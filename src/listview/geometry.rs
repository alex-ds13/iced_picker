//! Where the items are: list coordinates, drift and the anchor's target.
//!
//! List coordinates have y = 0 at the top of the list's bounds. The item at position
//! `p` starts at `content_y(p) = padding.top + drift + heights.offset_of(p)`. Drift is
//! a phantom spacer above the first item. Only four rules change it (section 4.4 of
//! the design): A absorbs unintended changes against the anchor, M moves the content
//! on purpose together with the anchor's target, S shrinks it while scrolling up and
//! Z removes it at the start of the list.

use super::heights::{Estimate, Heights};
use super::local::LocalLayout;

use iced::time::Instant;
use iced::{Padding, Rectangle};

/// At or above this visible top the list is at its start, where invariant I1 holds:
/// no drift and no anchor.
const START: f64 = 0.5;

/// The part of the viewport below the list's top, in list coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Visible {
    pub(super) top: f64,
    pub(super) height: f64,
}

impl Visible {
    /// Assumed by a layout that runs before the first redraw.
    pub(super) const GUESS: Self = Self {
        top: 0.0,
        height: 1024.0,
    };

    /// The viewport below the top of `bounds`, both in the same unscrolled space.
    ///
    /// The bottom is not clamped to the list's bounds. Their height is partly
    /// estimated, and a walk that only covered up to it would add a few rows per pass
    /// instead of covering the viewport at once.
    pub(super) fn new(viewport: Rectangle, bounds: Rectangle) -> Self {
        let top = f64::from(viewport.y - bounds.y).max(0.0);
        let bottom = f64::from(viewport.y + viewport.height - bounds.y);

        Self {
            top,
            height: (bottom - top).max(0.0),
        }
    }

    pub(super) fn bottom(self) -> f64 {
        self.top + self.height
    }

    pub(super) fn at_start(self) -> bool {
        self.top <= START
    }
}

/// What the anchor keeps still: the top of an item, or the bottom of the last one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Row(usize),
    End,
}

/// The builder settings that place items.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Settings {
    pub(super) padding: Padding,
    pub(super) spacing: f32,
    pub(super) estimate: Option<f32>,
    pub(super) anticipate: usize,
}

/// The positions of the first and last rows of a window, and its number of rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Span {
    pub(super) first: usize,
    pub(super) last: usize,
    pub(super) len: usize,
}

/// The geometry of a list, independent of its key type.
#[derive(Debug)]
pub(super) struct Geometry {
    pub(super) heights: Heights,
    settings: Settings,
    drift: f64,
    /// The content y the anchor's top must keep. `Some` exactly when there is an anchor.
    anchor_y: Option<f64>,
    /// The visible region of the last redraw. Layout walks to cover it.
    pub(super) visible: Option<Visible>,
    /// The visible region of the last walk.
    pub(super) laid_out: Option<Visible>,
    /// The geometry changed since the last walk, so the window may be wrong.
    pub(super) dirty: bool,
    /// The redraw timestamp of the last rule S.
    pub(super) shrunk_at: Option<Instant>,
    /// The constraints of the last full layout, `None` after a `diff` until the next
    /// full layout.
    pub(super) local: Option<LocalLayout>,
    /// The next redraw must use root layout for every pass. Set by changes outside a
    /// redraw, consumed by the next redraw and never by a layout.
    pub(super) block_next_redraw: bool,
    /// The redraw timestamp whose passes must all use root layout.
    pub(super) full_frame: Option<Instant>,
    /// The next layout is the root layout a redraw asked for.
    pub(super) redraw_layout: bool,
}

/// What a geometry-neutral walk must leave unchanged, besides every measured height
/// and the list's size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Snapshot {
    total: f64,
    drift: f64,
    estimate: Estimate,
}

impl Geometry {
    pub(super) fn new(settings: Settings) -> Self {
        let mut heights = Heights::new(Estimate::Provisional);
        heights.set_spacing(settings.spacing);
        heights.set_estimate(settings.estimate);

        Self {
            heights,
            settings,
            drift: 0.0,
            anchor_y: None,
            visible: None,
            laid_out: None,
            dirty: true,
            shrunk_at: None,
            local: None,
            block_next_redraw: false,
            full_frame: None,
            redraw_layout: false,
        }
    }

    /// Applies the builder settings. Returns whether they changed, in which case the
    /// caller absorbs the change against the anchor.
    pub(super) fn configure(&mut self, settings: Settings) -> bool {
        if settings == self.settings {
            return false;
        }

        self.heights.set_spacing(settings.spacing);
        self.heights.set_estimate(settings.estimate);
        self.settings = settings;
        self.dirty = true;

        true
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        Snapshot {
            total: self.total(),
            drift: self.drift,
            estimate: self.heights.estimate(),
        }
    }

    /// Makes every pass of the redraw at `now` use root layout, starting with the
    /// one the caller asks for.
    pub(super) fn full(&mut self, now: Instant) {
        self.full_frame = Some(now);
        self.redraw_layout = true;
    }

    pub(super) fn settings(&self) -> Settings {
        self.settings
    }

    pub(super) fn drift(&self) -> f64 {
        self.drift
    }

    pub(super) fn anchor_y(&self) -> Option<f64> {
        self.anchor_y
    }

    /// The content y of position 0.
    pub(super) fn base(&self) -> f64 {
        f64::from(self.settings.padding.top) + self.drift
    }

    pub(super) fn content_y(&self, position: usize) -> f64 {
        self.base() + self.heights.offset_of(position)
    }

    pub(super) fn bottom_of(&self, position: usize) -> f64 {
        self.content_y(position) + f64::from(self.heights.height(position))
    }

    /// The content y of the bottom of the last item, or of position 0 when empty.
    pub(super) fn end(&self) -> f64 {
        self.base() + self.heights.total()
    }

    pub(super) fn target_y(&self, target: Target) -> f64 {
        match target {
            Target::Row(position) => self.content_y(position),
            Target::End => self.end(),
        }
    }

    /// Whether the end of the list counts as visible, for choosing to pin it.
    ///
    /// This picks an anchor, it does not decide coverage, so it compares loosely:
    /// pinning the end instead of the first visible item makes a difference on screen
    /// smaller than the tolerance. Exact comparisons fail at the scrollable's largest
    /// translation. The list reports its height as `f32`, the scrollable computes
    /// that translation as `content - height` and the content adds the height back,
    /// both rounded. With 1985 rows of 1 px estimated at 1000 + 3/256 px and a
    /// 600.03125 px viewport, the end is reported at 958039.1875, the largest
    /// translation is 957439.125 and the visible bottom 958039.125, one `f32` ulp
    /// short. Leaving the end unpinned there let each pass shorten the list under the
    /// next pass's translation, and the third pass drew the viewport 399 px past the
    /// end.
    ///
    /// So the end is visible when it is at most four `f32` ulps below the visible
    /// bottom, at the magnitude of the two. Once `pinned`, it stays visible until it is
    /// more than 1 px below, or that tolerance if larger, so rounding cannot flip the
    /// anchor between the passes of a frame.
    pub(super) fn end_visible(&self, visible: Visible, pinned: bool) -> bool {
        if self.heights.is_empty() {
            return false;
        }

        let end = f64::from(self.reported_end());
        let bottom = visible.bottom();
        let tolerance = 4.0 * ulp(end.abs().max(bottom.abs()));

        let tolerance = if pinned {
            tolerance.max(1.0)
        } else {
            tolerance
        };

        end <= bottom + tolerance
    }

    /// The end as the list's reported height places it.
    fn reported_end(&self) -> f32 {
        self.total().max(0.0) as f32 - self.settings.padding.bottom
    }

    /// The height of the list.
    pub(super) fn total(&self) -> f64 {
        self.base() + self.heights.total() + f64::from(self.settings.padding.bottom)
    }

    /// Rule M: an intended move of the content. The anchor's target moves with it, so
    /// rule A does not undo it.
    pub(super) fn move_drift(&mut self, delta: f64) {
        self.drift += delta;

        if let Some(target) = &mut self.anchor_y {
            *target += delta;
        }

        self.dirty = true;
    }

    /// Rule A: after heights or positions changed, moves the content so the anchor is
    /// back at its target.
    pub(super) fn absorb(&mut self, anchor: Option<Target>) {
        if let (Some(anchor), Some(target)) = (anchor, self.anchor_y) {
            self.drift -= self.target_y(anchor) - target;
        }
    }

    /// Rule Z, the drift part: removes all drift at the start of the list. Returns
    /// whether there was any. The caller drops the anchor.
    pub(super) fn zero_drift(&mut self) -> bool {
        if self.drift == 0.0 {
            return false;
        }

        self.move_drift(-self.drift);

        true
    }

    /// Rule S: while scrolling up from `t0` to `t1`, shrinks the drift by the same
    /// ratio as the visible top, so it reaches 0 together with the top. Returns whether
    /// it applied.
    ///
    /// It applies at most once per redraw timestamp `now`. Shrinking positive drift
    /// shortens the list, so on the next pass of the same redraw the scrollable can
    /// clamp its translation and lower the visible top again. That decrease is the
    /// ancestors catching up, not scrolling, and shrinking again on every pass would
    /// leave the last drawn translation behind the last layout's content height.
    pub(super) fn shrink(&mut self, t0: f64, t1: f64, now: Instant) -> bool {
        if self.drift == 0.0 || self.shrunk_at == Some(now) || !(0.0 < t1 && t1 < t0) {
            return false;
        }

        let shrunk = self.drift * t1 / t0;
        self.move_drift(shrunk - self.drift);
        self.shrunk_at = Some(now);

        true
    }

    /// Pins the anchor where it is now.
    pub(super) fn pin(&mut self, target: Target) {
        self.anchor_y = Some(self.target_y(target));
    }

    /// Pins the anchor at the content y `y`.
    pub(super) fn pin_at(&mut self, y: f64) {
        self.anchor_y = Some(y);
    }

    pub(super) fn unpin(&mut self) {
        self.anchor_y = None;
    }

    /// The first item whose bottom is below `top`, else the last item.
    pub(super) fn first_visible(&self, top: f64) -> Option<usize> {
        let len = self.heights.len();

        if len == 0 {
            return None;
        }

        let position = self.heights.position_at(top - self.base());

        if self.bottom_of(position) > top || position + 1 == len {
            Some(position)
        } else {
            Some(position + 1)
        }
    }

    /// The first visible row, where the walk starts. A row ending at or above the top
    /// is not visible, also when the top lies in the spacing below it.
    pub(super) fn first_row(&self, visible: Visible) -> usize {
        self.first_visible(visible.top).unwrap_or(0)
    }

    /// Whether the row at `position` starts above the visible bottom, which is an
    /// exclusive end. Below it, a gap before the next row is spacing, not content.
    /// Agrees with [`last_row`](Self::last_row) exactly.
    pub(super) fn starts_before(&self, position: usize, visible: Visible) -> bool {
        self.heights.offset_of(position) < visible.bottom() - self.base()
    }

    /// Whether the row at `position` shows content inside `visible`.
    pub(super) fn covers(&self, position: usize, visible: Visible) -> bool {
        self.bottom_of(position) > visible.top && self.starts_before(position, visible)
    }

    /// The last row starting above the visible bottom, which is never after the row
    /// where a walk from [`first_row`](Self::first_row) stops.
    fn last_row(&self, visible: Visible) -> usize {
        let first = self.first_row(visible);

        self.heights
            .position_before(visible.bottom() - self.base())
            .max(first)
    }

    /// The coverage check of section 4.1: whether the window `span` no longer covers
    /// `visible` with half of the anticipation as hysteresis, or has grown more than
    /// four anticipations past what is visible. O(log n).
    pub(super) fn needs_walk(&self, visible: Visible, span: Option<Span>) -> bool {
        let len = self.heights.len();

        let Some(span) = span else {
            return len > 0 || self.laid_out.is_none();
        };

        if len == 0 || self.laid_out.is_none() || span.len != span.last - span.first + 1 {
            return true;
        }

        let anticipate = self.settings.anticipate;
        let margin = anticipate / 2;
        let first = self.first_row(visible);
        let last = self.last_row(visible);

        (span.first > 0 && first < span.first + margin)
            || (span.last + 1 < len && last + margin > span.last)
            || span.len > last - first + 1 + 4 * anticipate
    }
}

/// The distance from `value` to the next larger `f32`.
fn ulp(value: f64) -> f64 {
    let value = value as f32;

    if !value.is_finite() {
        return 0.0;
    }

    let next = f32::from_bits(value.abs().to_bits() + 1);
    f64::from(next) - f64::from(value.abs())
}

#[cfg(test)]
mod tests {
    use super::{Geometry, Settings, Span, Target, Visible, ulp};

    use std::time::Duration;

    use iced::time::Instant;
    use iced::{Padding, Rectangle};

    fn geometry(heights: &[Option<f32>], anticipate: usize) -> Geometry {
        let mut geometry = Geometry::new(Settings {
            padding: Padding::ZERO,
            spacing: 0.0,
            estimate: Some(40.0),
            anticipate,
        });
        geometry.heights.rebuild(heights.iter().copied());
        geometry.laid_out = Some(Visible::GUESS);
        geometry
    }

    fn visible(top: f64, height: f64) -> Visible {
        Visible { top, height }
    }

    fn span(first: usize, last: usize) -> Option<Span> {
        Some(Span {
            first,
            last,
            len: last - first + 1,
        })
    }

    #[test]
    fn visible_region_is_below_the_list_top_and_not_clamped_to_its_height() {
        let bounds = Rectangle::new([0.0, 100.0].into(), [800.0, 300.0].into());
        let viewport = Rectangle::new([0.0, 0.0].into(), [800.0, 600.0].into());
        assert_eq!(Visible::new(viewport, bounds), visible(0.0, 500.0));

        let viewport = Rectangle::new([0.0, 250.0].into(), [800.0, 600.0].into());
        assert_eq!(Visible::new(viewport, bounds), visible(150.0, 600.0));

        // A list entirely below the viewport has an empty region at its start.
        let viewport = Rectangle::new([0.0, 0.0].into(), [800.0, 50.0].into());
        assert_eq!(Visible::new(viewport, bounds), visible(0.0, 0.0));
        assert!(Visible::new(viewport, bounds).at_start());
    }

    #[test]
    fn coverage_check_uses_half_anticipation_and_lazy_eviction() {
        // 200 rows of 40 px, viewport 600 px. A walk at top 4000 keeps rows 96..=118.
        let geometry = geometry(&[Some(40.0); 200], 4);
        let window = span(96, 118);

        assert!(!geometry.needs_walk(visible(4000.0, 600.0), window));
        // One and a half rows either way stay inside the hysteresis.
        assert!(!geometry.needs_walk(visible(4060.0, 600.0), window));
        assert!(!geometry.needs_walk(visible(3940.0, 600.0), window));
        // Three rows down leave less than half the anticipation below.
        assert!(geometry.needs_walk(visible(4120.0, 600.0), window));
        assert!(geometry.needs_walk(visible(3880.0, 600.0), window));
        // A viewport ending exactly on a row boundary does not count the next row.
        assert!(!geometry.needs_walk(visible(4080.0, 600.0), window));

        // Far more rows than the visible ones plus four anticipations.
        assert!(geometry.needs_walk(visible(4000.0, 40.0), span(80, 118)));
        assert!(!geometry.needs_walk(visible(4000.0, 40.0), span(92, 108)));

        // A window that is not contiguous, or none at all.
        let gap = Some(Span {
            first: 96,
            last: 118,
            len: 22,
        });
        assert!(geometry.needs_walk(visible(4000.0, 600.0), gap));
        assert!(geometry.needs_walk(visible(4000.0, 600.0), None));

        // At the ends there is nothing more to add.
        assert!(!geometry.needs_walk(visible(0.0, 600.0), span(0, 18)));
        assert!(!geometry.needs_walk(visible(7400.0, 600.0), span(181, 199)));

        // Without anticipation only rows outside the window trigger a walk.
        let geometry = self::geometry(&[Some(40.0); 200], 0);
        assert!(!geometry.needs_walk(visible(4000.0, 600.0), span(100, 114)));
        assert!(geometry.needs_walk(visible(4001.0, 600.0), span(100, 114)));

        // Review P2-R1 F5: the bottom is an exact exclusive end on the 1/256 px grid.
        // With 1 px rows and no anticipation, a walk at the top keeps rows 0..=599.
        let geometry = self::geometry(&[Some(1.0); 1000], 0);
        let window = span(0, 599);
        assert_eq!(geometry.last_row(visible(0.0, 600.0)), 599);
        assert!(!geometry.needs_walk(visible(0.0, 600.0), window));
        // Half a grid unit more shows the top of row 600.
        let tiny = 1.0 / 2048.0;
        assert_eq!(geometry.last_row(visible(tiny, 600.0)), 600);
        assert!(geometry.needs_walk(visible(tiny, 600.0), window));
        // A bottom exactly on row 601's top excludes it, any amount past includes it.
        assert_eq!(geometry.last_row(visible(1.0, 600.0)), 600);
        assert_eq!(geometry.last_row(visible(1.0 + tiny, 600.0)), 601);
    }

    #[test]
    fn drift_rules_keep_the_anchor_target_in_step() {
        let mut geometry = geometry(&[Some(40.0); 200], 4);

        // R2-1: natural y 4400, target 4000, drift -400.
        geometry.pin(Target::Row(110));
        geometry.move_drift(-400.0);
        assert_eq!(geometry.anchor_y(), Some(4000.0));
        geometry.dirty = false;

        // Absorbing a change above the anchor keeps its target.
        geometry.heights.set(5, 80.0);
        geometry.absorb(Some(Target::Row(110)));
        assert_eq!(geometry.drift(), -440.0);
        assert_eq!(geometry.content_y(110), 4000.0);
        geometry.heights.set(5, 40.0);
        geometry.absorb(Some(Target::Row(110)));
        assert_eq!(geometry.drift(), -400.0);

        // Rule S moves the target with the drift, so rule A keeps the result.
        let now = Instant::now();
        assert!(!geometry.shrink(4000.0, 4100.0, now));
        assert!(!geometry.shrink(4000.0, 0.0, now));
        assert!(geometry.shrink(4000.0, 3900.0, now));
        assert!(geometry.dirty);
        assert_eq!(geometry.drift(), -390.0);
        assert_eq!(geometry.anchor_y(), Some(4010.0));
        geometry.absorb(Some(Target::Row(110)));
        assert_eq!(geometry.drift(), -390.0);

        // Review P2-R1 F1: once per redraw timestamp, whatever the later passes see.
        assert!(!geometry.shrink(3900.0, 3800.0, now));
        assert_eq!(geometry.drift(), -390.0);
        let later = now + Duration::from_millis(16);
        assert!(geometry.shrink(3900.0, 3800.0, later));
        assert_eq!(geometry.drift(), -380.0);

        // Rule Z removes all of it, exactly.
        assert!(geometry.zero_drift());
        assert_eq!(geometry.drift(), 0.0);
        assert!(!geometry.zero_drift());
        let last = later + Duration::from_millis(16);
        assert!(!geometry.shrink(4000.0, 3900.0, last));
    }

    #[test]
    fn end_visibility_tolerates_rounding_and_holds_a_pinned_end() {
        assert_eq!(ulp(958_039.0), 0.0625);
        assert_eq!(ulp(-958_039.0), 0.0625);
        assert_eq!(ulp(1.0), f64::from(f32::EPSILON));

        // 100 rows of 40 px: the end is at 4000.
        let geometry = geometry(&[Some(40.0); 100], 4);
        let below = |bottom: f64| visible(bottom - 600.0, 600.0);

        assert!(geometry.end_visible(below(4000.0), false));
        assert!(geometry.end_visible(below(4000.0 - 4.0 * ulp(4000.0)), false));
        assert!(!geometry.end_visible(below(4000.0 - 0.01), false));

        // A pinned end stays visible within 1 px.
        assert!(geometry.end_visible(below(4000.0 - 0.99), true));
        assert!(!geometry.end_visible(below(4000.0 - 1.01), true));

        assert!(!self::geometry(&[], 4).end_visible(below(4000.0), true));
    }

    #[test]
    fn first_visible_skips_rows_ending_at_the_top() {
        let geometry = geometry(&[Some(40.0), Some(0.0), Some(40.0), Some(40.0)], 4);
        assert_eq!(geometry.first_visible(0.0), Some(0));
        assert_eq!(geometry.first_visible(39.0), Some(0));
        // Row 0 ends at 40 and the zero-height row 1 owns no y, so row 2 is first.
        assert_eq!(geometry.first_visible(40.0), Some(2));
        assert_eq!(geometry.first_visible(500.0), Some(3));
        assert_eq!(self::geometry(&[], 4).first_visible(0.0), None);
    }
}
