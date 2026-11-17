//! Item heights in position order, indexed by one Fenwick tree.
//!
//! Positions are exact on a grid of 1/256 px. Heights and spacing are rounded to
//! the nearest unit of that grid and summed as integers, so offsets, lookups and
//! the total always agree and the total can never be negative. Rows placed from
//! these offsets differ from their real `f32` heights by at most half a unit
//! each, which is invisible.

#[cfg(test)]
use std::cell::Cell;

/// Grid units per pixel.
const UNIT: u32 = 256;

/// The largest height or spacing stored, in units: 2^30, which is 4,194,304 px.
/// Larger values are clamped.
///
/// With fewer than 2^32 items (positions are `u32`), every offset is at most
/// `n * 2 * MAX_UNITS < 2^63`, so the `i64` sums cannot overflow. Offsets are also
/// exact as `f64` while below 2^53 units (2^45 px).
const MAX_UNITS: u32 = 1 << 30;

/// Rounds a finite, nonnegative height to the nearest unit, clamped to `MAX_UNITS`.
fn quantize(pixels: f32) -> u32 {
    (f64::from(pixels) * f64::from(UNIT))
        .round()
        .min(f64::from(MAX_UNITS)) as u32
}

fn to_pixels(units: i64) -> f64 {
    units as f64 / f64::from(UNIT)
}

/// The height assumed for items that have not been measured yet, in units.
///
/// Its value is never below [`Estimate::MIN`], so unmeasured items always have an
/// extent and [`Heights::position_at`] cannot skip over them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Estimate {
    /// Set by the application. Measurements never replace it.
    Given(u32),
    /// No positive measurement exists yet.
    Provisional,
    /// Adopted from the first positive measurement.
    Measured(u32),
}

impl Estimate {
    pub(super) const MIN: f32 = 1.0;
    pub(super) const PROVISIONAL: f32 = 32.0;
    const MIN_UNITS: u32 = Self::MIN as u32 * UNIT;
    const PROVISIONAL_UNITS: u32 = Self::PROVISIONAL as u32 * UNIT;

    pub(super) fn given(height: f32) -> Self {
        if height.is_finite() {
            Self::Given(quantize(height.max(0.0)).max(Self::MIN_UNITS))
        } else {
            Self::Provisional
        }
    }

    pub(super) fn value(self) -> f32 {
        to_pixels(i64::from(self.units())) as f32
    }

    fn units(self) -> u32 {
        match self {
            Self::Given(units) | Self::Measured(units) => units,
            Self::Provisional => Self::PROVISIONAL_UNITS,
        }
    }
}

/// The outcome of recording one measured height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Recorded {
    /// The rounded height equals the indexed one. No tree node was touched.
    Unchanged,
    Changed,
    /// Negative or non-finite. Nothing was recorded.
    Invalid,
}

/// Heights of the items of the sequence, with the spacing between them.
///
/// Offsets are relative to the top of the first item. Item `p` starts at
/// `prefix(p) + spacing * p`.
#[derive(Debug)]
pub(super) struct Heights {
    /// In units, at most `MAX_UNITS`.
    heights: Vec<u32>,
    measured: Vec<bool>,
    /// 1-based Fenwick tree over `heights`. Spacing is not stored in it, so a spacing
    /// change needs no rebuild.
    tree: Vec<i64>,
    /// Running sum of `heights`, so the total needs no tree walk.
    sum: i64,
    /// In units, at most `MAX_UNITS`.
    spacing: u32,
    estimate: Estimate,
    /// Fenwick nodes visited, to check that lookups and updates stay logarithmic.
    #[cfg(test)]
    probes: Cell<usize>,
}

impl Heights {
    pub(super) fn new(estimate: Estimate) -> Self {
        Self {
            heights: Vec::new(),
            measured: Vec::new(),
            tree: vec![0],
            sum: 0,
            spacing: 0,
            estimate,
            #[cfg(test)]
            probes: Cell::new(0),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.heights.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    pub(super) fn estimate(&self) -> Estimate {
        self.estimate
    }

    pub(super) fn spacing(&self) -> f64 {
        to_pixels(i64::from(self.spacing))
    }

    /// Sets the gap between items. Negative and non-finite values count as 0.
    pub(super) fn set_spacing(&mut self, spacing: f32) {
        // A negative gap would make offsets decrease, which breaks the descent in
        // `position_at`.
        self.spacing = if spacing.is_finite() {
            quantize(spacing.max(0.0))
        } else {
            0
        };
    }

    /// Applies the application's estimate, `None` when it gives none.
    ///
    /// Returns whether the estimate changed. Dropping a given estimate goes back to
    /// the provisional one until the next positive measurement.
    pub(super) fn set_estimate(&mut self, given: Option<f32>) -> bool {
        let estimate = match (given, self.estimate) {
            (Some(height), _) => Estimate::given(height),
            (None, Estimate::Given(_)) => Estimate::Provisional,
            (None, current) => current,
        };

        if estimate == self.estimate {
            return false;
        }

        let rewrite = estimate.units() != self.estimate.units();
        self.estimate = estimate;

        if rewrite {
            self.rewrite_unmeasured();
        }

        true
    }

    /// The indexed height, measured or estimated, on the 1/256 px grid.
    pub(super) fn height(&self, position: usize) -> f32 {
        to_pixels(i64::from(self.heights[position])) as f32
    }

    pub(super) fn is_measured(&self, position: usize) -> bool {
        self.measured[position]
    }

    /// Replaces every height, `None` for an unmeasured item. O(n).
    ///
    /// Negative or non-finite heights count as unmeasured.
    pub(super) fn rebuild(&mut self, heights: impl IntoIterator<Item = Option<f32>>) {
        let estimate = self.estimate.units();
        self.heights.clear();
        self.measured.clear();

        for height in heights {
            let height = height.filter(|height| height.is_finite() && *height >= 0.0);
            self.heights.push(height.map_or(estimate, quantize));
            self.measured.push(height.is_some());
        }

        // Enforced in release builds too: the i64 overflow bound of the tree assumes
        // at most u32::MAX rows of at most MAX_UNITS plus one gap each.
        assert!(
            self.heights.len() <= u32::MAX as usize,
            "list_view supports at most u32::MAX items"
        );
        self.build_tree();
    }

    /// Records a measured height, rounded to the nearest 1/256 px. O(log n), except
    /// for the one-time adoption of the first positive measurement as the estimate,
    /// which rewrites every unmeasured item.
    ///
    /// [`Recorded::Unchanged`] compares the rounded heights, so jitter below half a
    /// unit is not a change. Local relayout relies on this comparison to decide
    /// whether a walk changed the geometry.
    pub(super) fn set(&mut self, position: usize, height: f32) -> Recorded {
        if !height.is_finite() || height < 0.0 {
            log::warn!("list_view: ignoring invalid height {height} at position {position}");
            return Recorded::Invalid;
        }

        let units = quantize(height);
        let previous = self.heights[position];
        self.measured[position] = true;

        let recorded = if units == previous {
            Recorded::Unchanged
        } else {
            self.heights[position] = units;
            Recorded::Changed
        };

        if self.estimate == Estimate::Provisional && units > 0 {
            self.estimate = Estimate::Measured(units.max(Estimate::MIN_UNITS));
            // The rebuild also picks up this item's new height.
            self.rewrite_unmeasured();
        } else if recorded == Recorded::Changed {
            self.add(position, i64::from(units) - i64::from(previous));
        }

        recorded
    }

    /// The top of the item at `position`. At `len` it is the bottom of the last item
    /// plus one gap. O(log n).
    pub(super) fn offset_of(&self, position: usize) -> f64 {
        debug_assert!(position <= self.len());
        to_pixels(self.prefix(position) + i64::from(self.spacing) * position as i64)
    }

    /// Every height and the `len - 1` gaps between items. O(1).
    pub(super) fn total(&self) -> f64 {
        match self.len() {
            0 => 0.0,
            len => to_pixels(self.sum + i64::from(self.spacing) * (len - 1) as i64),
        }
    }

    /// The item containing `y`, where each item owns its top edge and the gap below it.
    ///
    /// Before the first item this is the first item, past the end the last one, and 0
    /// when empty. An item with no height owns no `y` when spacing is zero. O(log n).
    pub(super) fn position_at(&self, y: f64) -> usize {
        let len = self.len();

        if len == 0 {
            return 0;
        }

        // Offsets are whole units, so `offset <= y` is `offset <= floor(y)` in units.
        // The cast saturates, which keeps far away values ordered, and NaN becomes 0.
        let y = (y * f64::from(UNIT)).floor() as i64;
        let (mut position, mut accumulated) = (0, 0);
        let mut step: usize = 1 << (usize::BITS - 1 - len.leading_zeros());

        while step > 0 {
            let next = position + step;

            // `position` is a multiple of `2 * step`, so node `next` covers exactly
            // `step` items and the gaps below them.
            if next <= len {
                self.probe();
                let weight = self.tree[next] + i64::from(self.spacing) * step as i64;

                if accumulated + weight <= y {
                    position = next;
                    accumulated += weight;
                }
            }

            step >>= 1;
        }

        position.min(len - 1)
    }

    /// The sum of the first `count` heights, without spacing, in units.
    fn prefix(&self, count: usize) -> i64 {
        let (mut node, mut sum) = (count, 0);

        while node > 0 {
            self.probe();
            sum += self.tree[node];
            node -= node.isolate_lowest_one();
        }

        sum
    }

    fn add(&mut self, position: usize, delta: i64) {
        self.sum += delta;
        let mut node = position + 1;

        while node < self.tree.len() {
            self.probe();
            self.tree[node] += delta;
            node += node.isolate_lowest_one();
        }
    }

    fn rewrite_unmeasured(&mut self) {
        let estimate = self.estimate.units();

        for (height, &measured) in self.heights.iter_mut().zip(&self.measured) {
            if !measured {
                *height = estimate;
            }
        }

        self.build_tree();
    }

    fn build_tree(&mut self) {
        let len = self.heights.len();
        self.tree.clear();
        self.tree.push(0);
        self.tree
            .extend(self.heights.iter().map(|&height| i64::from(height)));

        for node in 1..=len {
            let parent = node + node.isolate_lowest_one();

            if parent <= len {
                self.tree[parent] += self.tree[node];
            }
        }

        self.sum = self.heights.iter().map(|&height| i64::from(height)).sum();
    }

    fn probe(&self) {
        #[cfg(test)]
        self.probes.set(self.probes.get() + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::{Estimate, Heights, MAX_UNITS, Recorded, UNIT};

    /// xorshift64*, so the seeded cases are reproducible without a dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.next() % bound as u64) as usize
        }

        /// A quarter of the heights are zero. The others are multiples of 0.25, which
        /// are on the 1/256 px grid, so a naive `f64` running sum is exact and
        /// compares with `==`.
        fn height(&mut self) -> f32 {
            if self.below(4) == 0 {
                0.0
            } else {
                self.below(400) as f32 * 0.25
            }
        }

        /// Zeros, values below one unit, fractions, ordinary rows, and rows above
        /// the clamp.
        fn mixed(&mut self) -> f32 {
            let unit = self.unit() as f32;

            match self.below(6) {
                0 => 0.0,
                1 => unit * 0.004,
                2 => unit * 3.0,
                3 => unit * 1000.0,
                4 => unit * 1.0e5,
                _ => 1.0e6 + unit * 1.0e8,
            }
        }

        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1_u64 << 53) as f64
        }
    }

    /// Compares the index with a naive running sum over `model`.
    fn check(index: &Heights, model: &[f32], spacing: f64, rng: &mut Rng) {
        let len = model.len();
        assert_eq!(index.len(), len);
        assert_eq!(index.is_empty(), len == 0);

        let mut offsets = Vec::with_capacity(len + 1);
        let mut top = 0.0;

        for (position, &height) in model.iter().enumerate() {
            assert_eq!(index.height(position), height, "height({position})");
            offsets.push(top);
            top += f64::from(height) + spacing;
        }

        offsets.push(top);

        for (position, &offset) in offsets.iter().enumerate() {
            assert_eq!(index.offset_of(position), offset, "offset_of({position})");
        }

        // No gap above the first item or below the last one.
        let heights: f64 = model.iter().map(|&height| f64::from(height)).sum();
        let total = index.total();
        assert_eq!(total, heights + spacing * len.saturating_sub(1) as f64);

        // The last item whose top is at or above `y`, else the first one.
        let naive = |y: f64| (0..len).rev().find(|&p| offsets[p] <= y).unwrap_or(0);

        let mut ys = vec![
            -1.0e9,
            -1.0,
            -0.125,
            0.0,
            total,
            total + 0.125,
            total + 1.0e9,
        ];
        let stride = (len / 256).max(1);

        for &offset in offsets.iter().step_by(stride) {
            ys.extend([offset - 0.125, offset, offset + 0.125]);
        }

        for _ in 0..64 {
            ys.push(rng.unit() * (total + 32.0) - 16.0);
        }

        for y in ys {
            assert_eq!(index.position_at(y), naive(y), "position_at({y})");
        }
    }

    /// Compares the index with an integer model: `units[p]` is item `p`'s height in
    /// grid units.
    fn check_units(index: &Heights, units: &[i64], spacing: i64, rng: &mut Rng) {
        let len = units.len();
        let pixels = |units: i64| units as f64 / f64::from(UNIT);
        assert_eq!(index.len(), len);

        let mut offsets = Vec::with_capacity(len + 1);
        let mut top = 0;

        for (position, &height) in units.iter().enumerate() {
            assert_eq!(index.height(position), pixels(height) as f32);
            offsets.push(top);
            top += height + spacing;
        }

        offsets.push(top);
        let mut previous = 0.0;

        for (position, &offset) in offsets.iter().enumerate() {
            let actual = index.offset_of(position);
            assert_eq!(actual, pixels(offset), "offset_of({position})");
            assert!(actual >= previous, "offset_of({position}) decreased");
            previous = actual;
        }

        let total = index.total();
        assert!(total >= 0.0);
        assert_eq!(total, pixels((top - spacing).max(0)));

        // Round trip: an item whose top differs from the next item's top is found
        // at its own top. Coincident tops resolve to the last of them.
        for position in 0..len {
            let found = index.position_at(index.offset_of(position));

            if position + 1 == len || offsets[position + 1] > offsets[position] {
                assert_eq!(found, position, "position_at(offset_of({position}))");
            } else {
                assert!(found > position);
            }
        }

        let naive = |y: i64| (0..len).rev().find(|&p| offsets[p] <= y).unwrap_or(0);

        for _ in 0..64 {
            let y = (rng.unit() * (top as f64 + 512.0)) as i64 - 256;
            assert_eq!(index.position_at(pixels(y)), naive(y), "position_at({y})");
        }
    }

    fn probes(index: &Heights) -> usize {
        index.probes.replace(0)
    }

    #[test]
    fn fenwick_matches_naive_prefix_and_lookup() {
        const ESTIMATE: f32 = 8.0;

        for spacing in [0.0_f32, 3.5] {
            for (case, len) in [0_usize, 1, 2, 3, 8, 13, 64, 3000].into_iter().enumerate() {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 + case as u64);
                let mut index = Heights::new(Estimate::given(ESTIMATE));
                index.set_spacing(spacing);
                let spacing = index.spacing();

                let source: Vec<Option<f32>> = (0..len)
                    .map(|_| (rng.below(2) == 0).then(|| rng.height()))
                    .collect();
                index.rebuild(source.iter().copied());
                let mut model: Vec<f32> = source
                    .iter()
                    .map(|height| height.unwrap_or(ESTIMATE))
                    .collect();

                for (position, height) in source.iter().enumerate() {
                    assert_eq!(index.is_measured(position), height.is_some());
                }

                check(&index, &model, spacing, &mut rng);

                if len == 0 {
                    continue;
                }

                for _ in 0..4 {
                    for _ in 0..(len * 2).min(500) {
                        let (position, height) = (rng.below(len), rng.height());
                        assert_ne!(index.set(position, height), Recorded::Invalid);
                        model[position] = height;
                    }

                    check(&index, &model, spacing, &mut rng);
                }

                for (position, height) in model.iter_mut().enumerate() {
                    index.set(position, 0.0);
                    *height = 0.0;
                }

                check(&index, &model, spacing, &mut rng);
            }
        }

        // The descent weighs the gap below each item: y = 12 is in the gap below item 0.
        let mut index = Heights::new(Estimate::given(10.0));
        index.set_spacing(5.0);
        index.rebuild([None, None, None]);
        assert_eq!(index.total(), 40.0);
        assert_eq!(index.position_at(12.0), 0);
        assert_eq!(index.position_at(14.75), 0);
        assert_eq!(index.position_at(15.0), 1);
        assert_eq!(index.position_at(44.0), 2);

        // Spacing is outside the tree, so changing it needs no rebuild.
        index.set_spacing(0.0);
        assert_eq!(index.total(), 30.0);
        assert_eq!(index.position_at(12.0), 1);
        assert_eq!(index.offset_of(2), 20.0);
    }

    #[test]
    fn mixed_magnitudes_match_naive_integer_model() {
        const ESTIMATE: f32 = 8.0;

        // The model's own rounding: nearest unit, clamped.
        let quantize = |pixels: f32| {
            (f64::from(pixels) * f64::from(UNIT))
                .round()
                .min(f64::from(MAX_UNITS)) as i64
        };

        for spacing in [0.0_f32, 0.3, 7.0] {
            for (case, len) in [1_usize, 2, 5, 33, 1000].into_iter().enumerate() {
                let mut rng = Rng(0xD1B5_4A32_D192_ED03 + case as u64);
                let mut index = Heights::new(Estimate::given(ESTIMATE));
                index.set_spacing(spacing);
                let spacing = quantize(spacing);

                let mut source: Vec<Option<f32>> = (0..len)
                    .map(|_| (rng.below(3) != 0).then(|| rng.mixed()))
                    .collect();
                let units = |source: &[Option<f32>]| -> Vec<i64> {
                    source
                        .iter()
                        .map(|height| quantize(height.unwrap_or(ESTIMATE)))
                        .collect()
                };

                index.rebuild(source.iter().copied());
                check_units(&index, &units(&source), spacing, &mut rng);

                for _ in 0..3 {
                    for _ in 0..(len * 2).min(400) {
                        let (position, height) = (rng.below(len), rng.mixed());
                        index.set(position, height);
                        source[position] = Some(height);
                    }

                    check_units(&index, &units(&source), spacing, &mut rng);
                }

                // Shrink, then drive every row to zero.
                source.truncate(len.div_ceil(2));
                index.rebuild(source.iter().copied());
                check_units(&index, &units(&source), spacing, &mut rng);

                for (position, height) in source.iter_mut().enumerate() {
                    index.set(position, 0.0);
                    *height = Some(0.0);
                }

                check_units(&index, &units(&source), spacing, &mut rng);
            }
        }
    }

    #[test]
    fn fixed_point_index_fixes_review_counterexamples() {
        let huge = 2.0_f32.powi(53);
        let max = MAX_UNITS as f32 / UNIT as f32;

        // Removing a huge row and then the small one leaves an empty total, never a
        // negative one.
        let mut index = Heights::new(Estimate::given(10.0));
        index.rebuild([Some(huge), Some(1.0)]);
        assert_eq!(index.height(0), max);
        index.set(1, 0.0);
        index.set(0, 0.0);
        assert_eq!(index.total(), 0.0);
        assert_eq!(index.offset_of(2), 0.0);
        assert_eq!(index.position_at(-0.5), 0);

        // Small rows next to a huge one are not absorbed by it.
        index.rebuild([Some(1.0), Some(1.0), Some(huge * 2.0), Some(1.0)]);
        index.set(2, 0.0);
        assert_eq!(index.total(), 3.0);
        assert_eq!(index.offset_of(4), 3.0);

        // Prefix and descent sum the same integers, so a top always finds its row.
        let mut index = Heights::new(Estimate::given(10.0));
        index.rebuild(
            [1.0e-7, 0.0, 0.0, 0.0, 1.0e-7, 0.0, 100.0, 1.0]
                .into_iter()
                .map(Some),
        );
        assert_eq!(index.height(0), 0.0);
        assert_eq!(index.offset_of(7), 100.0);
        assert_eq!(index.position_at(index.offset_of(7)), 7);

        // Rows and spacing are clamped to the documented maximum.
        let mut index = Heights::new(Estimate::given(1.0e9));
        assert_eq!(index.estimate(), Estimate::Given(MAX_UNITS));
        index.set_spacing(1.0e9);
        assert_eq!(index.spacing(), f64::from(max));
        index.rebuild([None, Some(1.0e9)]);
        assert_eq!(index.height(1), max);
        assert_eq!(index.total(), 3.0 * f64::from(max));

        // Jitter below half a unit is not a change. Half a unit or more is.
        let mut index = Heights::new(Estimate::given(10.0));
        index.rebuild([None, None]);
        assert_eq!(index.set(0, 10.0), Recorded::Unchanged);
        assert_eq!(index.set(0, 10.001), Recorded::Unchanged);
        assert_eq!(index.height(0), 10.0);
        assert_eq!(index.set(0, 10.003), Recorded::Changed);
        assert_eq!(index.height(0), 10.0 + 1.0 / 256.0);

        // A positive height that rounds to zero is a zero, so it is not adopted.
        let mut index = Heights::new(Estimate::Provisional);
        index.rebuild([None, None]);
        assert_eq!(index.set(0, 0.001), Recorded::Changed);
        assert_eq!(index.height(0), 0.0);
        assert_eq!(index.estimate(), Estimate::Provisional);
    }

    #[test]
    fn zero_measurement_keeps_positive_unknown_estimate() {
        let mut index = Heights::new(Estimate::Provisional);
        index.rebuild([None; 5]);

        // A first real height of 0, with zero spacing, is not adopted as the estimate.
        assert_eq!(index.set(0, 0.0), Recorded::Changed);
        assert_eq!(index.estimate(), Estimate::Provisional);
        assert_eq!(index.height(0), 0.0);

        for position in 1..5 {
            assert!(index.height(position) >= Estimate::MIN);
            assert_eq!(index.height(position), Estimate::PROVISIONAL);
            assert!(!index.is_measured(position));
        }

        // Unknown rows keep an extent, so the descent stops at them instead of
        // running to the last row.
        assert_eq!(index.position_at(0.0), 1);
        assert_eq!(index.position_at(f64::from(Estimate::PROVISIONAL)), 2);
        assert_eq!(index.total(), 4.0 * f64::from(Estimate::PROVISIONAL));

        // The first positive measurement is adopted once and only rewrites unknown rows.
        assert_eq!(index.set(2, 20.0), Recorded::Changed);
        assert_eq!(index.estimate(), Estimate::Measured(20 * UNIT));
        assert_eq!(index.estimate().value(), 20.0);
        assert_eq!(index.set(3, 50.0), Recorded::Changed);
        assert_eq!(index.estimate(), Estimate::Measured(20 * UNIT));

        let heights: Vec<f32> = (0..5).map(|position| index.height(position)).collect();
        assert_eq!(heights, [0.0, 20.0, 20.0, 50.0, 20.0]);
        assert_eq!(index.offset_of(4), 90.0);
        assert_eq!(index.total(), 110.0);

        // Real zeros stay zero, measured later or restored by a rebuild.
        assert_eq!(index.set(1, 0.0), Recorded::Changed);
        assert_eq!(index.height(1), 0.0);
        index.rebuild([Some(0.0), None, Some(50.0)]);
        assert_eq!(index.estimate(), Estimate::Measured(20 * UNIT));
        assert_eq!(index.height(0), 0.0);
        assert_eq!(index.height(1), 20.0);
        assert_eq!(index.position_at(0.0), 1);

        // A positive height below the minimum adopts the minimum.
        let mut tiny = Heights::new(Estimate::Provisional);
        tiny.rebuild([None, None]);
        tiny.set(0, 0.25);
        assert_eq!(tiny.estimate(), Estimate::Measured(Estimate::MIN_UNITS));
        assert_eq!(tiny.height(0), 0.25);
        assert_eq!(tiny.height(1), Estimate::MIN);

        // A given estimate is never adopted from a measurement, and is never below 1.
        assert_eq!(Estimate::given(0.0), Estimate::Given(Estimate::MIN_UNITS));
        assert_eq!(Estimate::given(f32::NAN), Estimate::Provisional);

        let mut given = Heights::new(Estimate::given(10.0));
        given.rebuild([None; 3]);
        given.set(0, 0.0);
        given.set(1, 25.0);
        assert_eq!(given.estimate(), Estimate::Given(10 * UNIT));
        assert_eq!(given.height(2), 10.0);

        // Changing the given estimate rewrites only unknown rows. Dropping it goes back
        // to the provisional estimate, which the next positive measurement replaces.
        assert!(given.set_estimate(Some(12.0)));
        assert!(!given.set_estimate(Some(12.0)));
        assert_eq!(
            [given.height(0), given.height(1), given.height(2)],
            [0.0, 25.0, 12.0]
        );
        assert!(given.set_estimate(None));
        assert_eq!(given.estimate(), Estimate::Provisional);
        assert_eq!(given.height(2), Estimate::PROVISIONAL);
        assert!(!given.set_estimate(None));
        given.set(1, 30.0);
        assert_eq!(given.estimate(), Estimate::Measured(30 * UNIT));
        assert_eq!(given.height(2), 30.0);
        assert_eq!(given.total(), 60.0);
    }

    #[test]
    fn lookups_and_updates_visit_logarithmic_nodes() {
        let len: usize = (1 << 20) + 3;
        let mut index = Heights::new(Estimate::given(10.0));
        index.set_spacing(2.0);
        index.rebuild((0..len).map(|_| None));

        // One node per bit of the length, at most.
        let bound = (usize::BITS - len.leading_zeros()) as usize;
        probes(&index);

        for position in [0, 1, 2, len / 3, len / 2, len - 2, len - 1] {
            index.offset_of(position);
            assert!(probes(&index) <= bound);

            assert_eq!(index.set(position, 7.0), Recorded::Changed);
            assert!(probes(&index) <= bound);

            let total = index.total();
            assert_eq!(probes(&index), 0);

            // An unchanged height touches no node and does not churn the sum.
            assert_eq!(index.set(position, 7.0), Recorded::Unchanged);
            assert_eq!(probes(&index), 0);
            assert_eq!(index.total().to_bits(), total.to_bits());

            let y = index.offset_of(position);
            probes(&index);
            assert_eq!(index.position_at(y), position);
            assert!(probes(&index) <= bound);
        }

        index.offset_of(len);
        assert!(probes(&index) <= bound);
    }

    #[test]
    fn invalid_heights_are_not_recorded() {
        let mut index = Heights::new(Estimate::Provisional);
        index.rebuild([None, Some(5.0)]);

        for height in [f32::NAN, f32::INFINITY, -1.0] {
            assert_eq!(index.set(0, height), Recorded::Invalid);
        }

        assert!(!index.is_measured(0));
        assert_eq!(index.height(0), Estimate::PROVISIONAL);
        assert_eq!(index.estimate(), Estimate::Provisional);
        assert_eq!(index.total(), 37.0);

        // A rebuild treats invalid cached heights as unmeasured.
        index.rebuild([Some(f32::NAN), Some(-2.0), Some(3.0)]);
        assert!(!index.is_measured(0) && !index.is_measured(1));
        assert_eq!(index.total(), 67.0);
    }
}
