//! Keys of the passed sequence, their positions and the heights cached by key.

use super::heights::{Heights, Recorded};

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::Hash;
use std::mem;
use std::ops::RangeInclusive;

/// Which key is at each position, the position of each key, and every height
/// measured so far, by key.
///
/// The heights indexed by position live in [`Heights`], which the caller owns so
/// geometry code needs no key type. Both always have the same length.
#[derive(Debug)]
pub(super) struct Model<K> {
    keys: Vec<K>,
    /// The first position of each key. Later duplicates are not reachable by key.
    positions: HashMap<K, u32>,
    /// Also kept for removed keys, so a key that comes back (after a filter change,
    /// for example) gets its height back.
    measured: HashMap<K, f32>,
    warned_duplicates: bool,
}

impl<K> Default for Model<K> {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            positions: HashMap::new(),
            measured: HashMap::new(),
            warned_duplicates: false,
        }
    }
}

impl<K: Clone + Eq + Hash> Model<K> {
    pub(super) fn len(&self) -> usize {
        self.keys.len()
    }

    pub(super) fn key(&self, position: usize) -> &K {
        &self.keys[position]
    }

    pub(super) fn position(&self, key: &K) -> Option<usize> {
        self.positions.get(key).map(|&position| position as usize)
    }

    /// The last height measured for the first item with `key`.
    #[cfg(test)]
    pub(super) fn cached(&self, key: &K) -> Option<f32> {
        self.measured.get(key).copied()
    }

    /// Compares `keys` with the current keys, stopping at the first difference.
    ///
    /// When they differ, rebuilds the positions and `heights` from the cache, and
    /// returns where each old position went. O(n) in both cases.
    pub(super) fn reconcile(
        &mut self,
        keys: impl IntoIterator<Item = K>,
        heights: &mut Heights,
    ) -> Option<Rebuilt<K>> {
        debug_assert_eq!(self.keys.len(), heights.len());

        let mut keys = keys.into_iter();
        let mut equal = 0;

        let first_difference = loop {
            match keys.next() {
                Some(key) if self.keys.get(equal) == Some(&key) => equal += 1,
                Some(key) => break Some(key),
                None if equal == self.keys.len() => return None,
                None => break None,
            }
        };

        let mut new_keys = self.keys[..equal].to_vec();
        new_keys.extend(first_difference);
        new_keys.extend(keys);

        let old_keys = mem::replace(&mut self.keys, new_keys);
        let old_positions = mem::take(&mut self.positions);
        let mut duplicates = 0;
        self.positions.reserve(self.keys.len());

        for (position, key) in self.keys.iter().enumerate() {
            match self.positions.entry(key.clone()) {
                Entry::Occupied(_) => duplicates += 1,
                Entry::Vacant(entry) => {
                    entry.insert(to_u32(position));
                }
            }
        }

        let moves = old_keys
            .iter()
            .enumerate()
            .map(|(old, key)| {
                let owner = old_positions
                    .get(key)
                    .is_some_and(|&position| position as usize == old);

                if owner {
                    self.positions.get(key).copied()
                } else {
                    None
                }
            })
            .collect();

        heights.rebuild(self.keys.iter().map(|key| self.measured.get(key).copied()));

        let warned = duplicates > 0 && !self.warned_duplicates;

        if warned {
            self.warned_duplicates = true;
            log::warn!(
                "list_view: {duplicates} items reuse the key of an earlier item. \
                 Only the first item with each key is reachable by key."
            );
        }

        Some(Rebuilt {
            #[cfg(test)]
            old_keys,
            old_positions,
            moves,
            #[cfg(test)]
            duplicates,
            #[cfg(test)]
            warned,
        })
    }

    /// Records the measured height of the item at `position`.
    ///
    /// Only the first item with a key writes the cache, so a later duplicate cannot
    /// replace the height restored for its key.
    pub(super) fn record(
        &mut self,
        position: usize,
        height: f32,
        heights: &mut Heights,
    ) -> Recorded {
        let recorded = heights.set(position, height);
        let key = &self.keys[position];

        let owner = self
            .positions
            .get(key)
            .is_some_and(|&owner| owner as usize == position);

        if recorded != Recorded::Invalid && owner {
            match self.measured.get_mut(key) {
                Some(cached) => *cached = height,
                None => {
                    self.measured.insert(key.clone(), height);
                }
            }
        }

        recorded
    }
}

/// How a structural change moved the previous keys.
#[derive(Debug)]
pub(super) struct Rebuilt<K> {
    #[cfg(test)]
    old_keys: Vec<K>,
    old_positions: HashMap<K, u32>,
    /// The new position of each old position. `None` when its key was removed, or
    /// when it was a later duplicate, which cannot be followed by key.
    moves: Vec<Option<u32>>,
    #[cfg(test)]
    duplicates: usize,
    #[cfg(test)]
    warned: bool,
}

impl<K: Eq + Hash> Rebuilt<K> {
    #[cfg(test)]
    pub(super) fn old_len(&self) -> usize {
        self.old_keys.len()
    }

    pub(super) fn new_position(&self, old: usize) -> Option<usize> {
        self.moves[old].map(|position| position as usize)
    }

    /// The old positions that cannot be followed: removed keys and later duplicates.
    #[cfg(test)]
    pub(super) fn removed(&self) -> impl Iterator<Item = (usize, &K)> + '_ {
        self.moves
            .iter()
            .zip(&self.old_keys)
            .enumerate()
            .filter(|(_, (moved, _))| moved.is_none())
            .map(|(old, (_, key))| (old, key))
    }

    /// Items in the new sequence whose key was already used by an earlier item.
    #[cfg(test)]
    pub(super) fn duplicates(&self) -> usize {
        self.duplicates
    }

    /// Whether this rebuild logged the duplicate warning, which happens once per list.
    #[cfg(test)]
    pub(super) fn warned(&self) -> bool {
        self.warned
    }

    /// The new position of `key` if it survived. Otherwise the first surviving old
    /// item after it, or the last one before it when none after it survived, among
    /// the old positions `within`.
    ///
    /// `None` when `key` was not in the old sequence or nothing `within` survived.
    pub(super) fn survivor(&self, key: &K, within: RangeInclusive<usize>) -> Option<usize> {
        let old = *self.old_positions.get(key)? as usize;
        let start = (*within.start()).min(self.moves.len());
        let end = within
            .end()
            .saturating_add(1)
            .min(self.moves.len())
            .max(start);
        let old = old.clamp(start, end);

        let after = self.moves[old..end].iter().flatten().next();
        let before = || self.moves[start..old].iter().rev().flatten().next();

        after.or_else(before).map(|&position| position as usize)
    }
}

fn to_u32(position: usize) -> u32 {
    u32::try_from(position).expect("list_view supports at most u32::MAX items")
}

#[cfg(test)]
mod tests {
    use super::{Model, Rebuilt};
    use crate::listview::heights::{Estimate, Heights};

    use std::fmt::Debug;
    use std::hash::Hash;

    fn reconcile<K: Clone + Eq + Hash>(
        model: &mut Model<K>,
        heights: &mut Heights,
        keys: &[K],
    ) -> Option<Rebuilt<K>> {
        let rebuilt = model.reconcile(keys.iter().cloned(), heights);
        assert_eq!(model.len(), keys.len());
        assert_eq!(heights.len(), keys.len());

        for (position, key) in keys.iter().enumerate() {
            assert!(model.key(position) == key);
        }

        rebuilt
    }

    fn moves<K: Eq + Hash>(rebuilt: &Rebuilt<K>) -> Vec<Option<usize>> {
        (0..rebuilt.old_len())
            .map(|old| rebuilt.new_position(old))
            .collect()
    }

    fn removed<K: Clone + Eq + Hash>(rebuilt: &Rebuilt<K>) -> Vec<(usize, K)> {
        rebuilt
            .removed()
            .map(|(old, key)| (old, key.clone()))
            .collect()
    }

    /// Checks the indexed heights and that the rebuilt tree agrees with them.
    fn assert_heights(heights: &Heights, expected: &[f32]) {
        let actual: Vec<f32> = (0..heights.len()).map(|p| heights.height(p)).collect();
        assert_eq!(actual, expected);

        let mut top = 0.0;

        for (position, &height) in expected.iter().enumerate() {
            assert_eq!(heights.offset_of(position), top);
            assert_eq!(heights.position_at(top), position);
            top += f64::from(height) + heights.spacing();
        }

        assert_eq!(heights.total(), (top - heights.spacing()).max(0.0));
    }

    fn record<K: Clone + Debug + Eq + Hash>(
        model: &mut Model<K>,
        heights: &mut Heights,
        key: K,
        height: f32,
    ) {
        let position = model.position(&key).expect("recorded key is present");
        model.record(position, height, heights);
        assert_eq!(model.cached(&key), Some(height), "{key:?}");
    }

    /// Every old position.
    const ALL: std::ops::RangeInclusive<usize> = 0..=usize::MAX;

    #[test]
    fn keyed_reconcile_survives_insert_remove_reorder_and_filter() {
        let mut model = Model::default();
        let mut heights = Heights::new(Estimate::given(10.0));
        heights.set_spacing(2.0);

        assert!(reconcile(&mut model, &mut heights, &[]).is_none());

        let rebuilt = reconcile(&mut model, &mut heights, &['a', 'b', 'c', 'd']).unwrap();
        assert_eq!(rebuilt.old_len(), 0);
        assert_heights(&heights, &[10.0; 4]);

        for (key, height) in [('a', 11.0), ('b', 12.0), ('c', 13.0), ('d', 14.0)] {
            record(&mut model, &mut heights, key, height);
        }

        assert!(reconcile(&mut model, &mut heights, &['a', 'b', 'c', 'd']).is_none());
        assert_heights(&heights, &[11.0, 12.0, 13.0, 14.0]);

        // Insert: the new key is estimated, the others keep their heights.
        let rebuilt = reconcile(&mut model, &mut heights, &['a', 'x', 'b', 'c', 'd']).unwrap();
        assert_eq!(moves(&rebuilt), [Some(0), Some(2), Some(3), Some(4)]);
        assert_eq!(removed(&rebuilt), []);
        assert_heights(&heights, &[11.0, 10.0, 12.0, 13.0, 14.0]);
        assert!(!heights.is_measured(1) && heights.is_measured(2));
        assert_eq!(model.position(&'b'), Some(2));

        // Remove: the next surviving key takes over a removed key.
        let rebuilt = reconcile(&mut model, &mut heights, &['a', 'x', 'c', 'd']).unwrap();
        assert_eq!(moves(&rebuilt), [Some(0), Some(1), None, Some(2), Some(3)]);
        assert_eq!(removed(&rebuilt), [(2, 'b')]);
        assert_eq!(rebuilt.survivor(&'b', ALL), Some(2));
        assert_eq!(rebuilt.survivor(&'x', ALL), Some(1));

        // Only the old positions given count: 'b' was at 2, 'c' at 3 and 'd' at 4.
        assert_eq!(rebuilt.survivor(&'b', 2..=2), None);
        assert_eq!(rebuilt.survivor(&'b', 2..=3), Some(2));
        assert_eq!(rebuilt.survivor(&'b', 1..=2), Some(1));
        assert_eq!(model.position(&'b'), None);
        assert_eq!(model.cached(&'b'), Some(12.0));
        assert_heights(&heights, &[11.0, 10.0, 13.0, 14.0]);

        // Reorder: heights follow their keys, and new measurements are cached by key.
        let rebuilt = reconcile(&mut model, &mut heights, &['d', 'c', 'x', 'a']).unwrap();
        assert_eq!(moves(&rebuilt), [Some(3), Some(2), Some(1), Some(0)]);
        assert_eq!(removed(&rebuilt), []);
        assert_heights(&heights, &[14.0, 13.0, 10.0, 11.0]);
        record(&mut model, &mut heights, 'a', 21.0);
        assert_heights(&heights, &[14.0, 13.0, 10.0, 21.0]);

        // Filter: the app passes the matching keys only.
        let rebuilt = reconcile(&mut model, &mut heights, &['c', 'a']).unwrap();
        assert_eq!(moves(&rebuilt), [None, Some(0), None, Some(1)]);
        assert_eq!(removed(&rebuilt), [(0, 'd'), (2, 'x')]);
        assert_heights(&heights, &[13.0, 21.0]);

        // Clearing the filter restores every cached key, the unmeasured one included.
        let rebuilt = reconcile(&mut model, &mut heights, &['a', 'b', 'c', 'd', 'x']).unwrap();
        assert_eq!(moves(&rebuilt), [Some(2), Some(0)]);
        assert_heights(&heights, &[21.0, 12.0, 13.0, 14.0, 10.0]);
        let measured: Vec<bool> = (0..5).map(|p| heights.is_measured(p)).collect();
        assert_eq!(measured, [true, true, true, true, false]);

        // Equal keys with a changed length.
        let longer = ['a', 'b', 'c', 'd', 'x', 'y'];
        let rebuilt = reconcile(&mut model, &mut heights, &longer).unwrap();
        assert_eq!(moves(&rebuilt), (0..5).map(Some).collect::<Vec<_>>());
        assert_heights(&heights, &[21.0, 12.0, 13.0, 14.0, 10.0, 10.0]);

        let rebuilt = reconcile(&mut model, &mut heights, &['a', 'b', 'c']).unwrap();
        assert_eq!(
            moves(&rebuilt),
            [Some(0), Some(1), Some(2), None, None, None]
        );
        assert_eq!(removed(&rebuilt), [(3, 'd'), (4, 'x'), (5, 'y')]);
        assert_heights(&heights, &[21.0, 12.0, 13.0]);

        // Equal length with a changed key.
        let rebuilt = reconcile(&mut model, &mut heights, &['a', 'z', 'c']).unwrap();
        assert_eq!(moves(&rebuilt), [Some(0), None, Some(2)]);
        assert_eq!(removed(&rebuilt), [(1, 'b')]);
        assert_heights(&heights, &[21.0, 10.0, 13.0]);

        // With no survivor after a removed key, the last one before it takes over.
        let rebuilt = reconcile(&mut model, &mut heights, &['a']).unwrap();
        assert_eq!(rebuilt.survivor(&'c', ALL), Some(0));
        assert_eq!(rebuilt.survivor(&'z', ALL), Some(0));
        assert_eq!(rebuilt.survivor(&'q', ALL), None);

        // Removing every row.
        let rebuilt = reconcile(&mut model, &mut heights, &[]).unwrap();
        assert_eq!(moves(&rebuilt), [None]);
        assert_eq!(rebuilt.survivor(&'a', ALL), None);
        assert!(heights.is_empty());
        assert_eq!(heights.total(), 0.0);
        assert!(reconcile(&mut model, &mut heights, &[]).is_none());
    }

    #[test]
    fn index_keys_preserve_surviving_cached_heights() {
        let mut model = Model::default();
        let mut heights = Heights::new(Estimate::given(50.0));

        // Ported from the v1 widget's unmeasured_items_use_the_estimated_height.
        model.reconcile(0..10, &mut heights).unwrap();
        assert_eq!(heights.offset_of(3), 150.0);
        model.record(1, 20.0, &mut heights);
        assert_eq!(heights.offset_of(3), 120.0);

        // Ported from the v1 widget's measured_heights_survive_a_change_in_length.
        for position in 0..10 {
            model.record(position, 20.0, &mut heights);
        }

        let shorter = model.reconcile(0..9, &mut heights).unwrap();
        assert_eq!(
            moves(&shorter),
            (0..9).map(Some).chain([None]).collect::<Vec<_>>()
        );
        assert_eq!(removed(&shorter), [(9, 9)]);
        assert_eq!(heights.offset_of(5), 100.0);
        assert_eq!(heights.len(), 9);

        // The v1 widget forgot index 9 when the list shrank. The key cache restores it.
        let longer = model.reconcile(0..11, &mut heights).unwrap();
        assert_eq!(moves(&longer), (0..9).map(Some).collect::<Vec<_>>());
        assert!(heights.is_measured(9));
        assert!(!heights.is_measured(10));
        assert_eq!(heights.height(10), 50.0);
        assert_eq!(heights.offset_of(10), 10.0 * 20.0);

        // L9: with index keys, cached heights follow the position, not the item. After a
        // prepend, the old first item (now at position 1) gets the height of position 1
        // and starts one row lower.
        let mut model = Model::default();
        let mut heights = Heights::new(Estimate::given(50.0));
        model.reconcile(0..5, &mut heights).unwrap();

        for position in 0..5 {
            model.record(position, (position + 1) as f32 * 10.0, &mut heights);
        }

        let prepended = model.reconcile(0..6, &mut heights).unwrap();
        assert_eq!(moves(&prepended), (0..5).map(Some).collect::<Vec<_>>());
        assert_heights(&heights, &[10.0, 20.0, 30.0, 40.0, 50.0, 50.0]);
        assert_eq!(heights.offset_of(1), 10.0);
    }

    #[test]
    fn duplicate_keys_have_one_lookup_owner() {
        let mut model = Model::default();
        let mut heights = Heights::new(Estimate::given(10.0));

        let rebuilt = reconcile(&mut model, &mut heights, &['a', 'b', 'a', 'c', 'b']).unwrap();
        assert_eq!(rebuilt.duplicates(), 2);
        assert!(rebuilt.warned());
        assert_eq!(model.position(&'a'), Some(0));
        assert_eq!(model.position(&'b'), Some(1));
        assert_eq!(model.position(&'c'), Some(3));

        // A later duplicate is still renderable and measurable by position, but it
        // cannot replace the height cached for the first item with its key.
        assert_eq!(*model.key(2), 'a');
        model.record(2, 99.0, &mut heights);
        assert_eq!(heights.height(2), 99.0);
        assert_eq!(model.cached(&'a'), None);
        record(&mut model, &mut heights, 'a', 15.0);
        assert_heights(&heights, &[15.0, 10.0, 99.0, 10.0, 10.0]);

        // The warning is logged once per list. Later duplicates are not followed by key.
        let rebuilt = reconcile(&mut model, &mut heights, &['b', 'a', 'a']).unwrap();
        assert_eq!(rebuilt.duplicates(), 1);
        assert!(!rebuilt.warned());
        assert_eq!(model.position(&'a'), Some(1));
        assert_eq!(moves(&rebuilt), [Some(1), Some(0), None, None, None]);
        assert_eq!(removed(&rebuilt), [(2, 'a'), (3, 'c'), (4, 'b')]);
        assert_heights(&heights, &[10.0, 15.0, 15.0]);
    }
}
