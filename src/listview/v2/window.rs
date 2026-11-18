//! The window: the items that exist as widgets, kept in the tree by key, and the
//! walk that chooses them.

use super::geometry::{Geometry, Settings, Span, Visible};
use super::heights::Recorded;
use super::model::{Model, Rebuilt};
use super::{Catalog, ListView};

use std::cell::RefCell;
use std::hash::Hash;
use std::marker::PhantomData;
use std::mem;

use iced::Vector;
use iced::advanced::Widget;
use iced::advanced::layout::Limits;
use iced::advanced::widget::Tree;
use iced::time::Instant;

/// The state of a list, stored in its tree.
#[derive(Debug)]
pub(super) struct Core<K> {
    pub(super) model: Model<K>,
    pub(super) geometry: Geometry,
    /// Sorted by position and aligned with the tree's children and the widget's rows.
    pub(super) window: Vec<Slot<K>>,
    /// The pinned item. Its target content y is in the geometry.
    pub(super) anchor: Option<K>,
    /// The key and position of the item drawn as hovered by the last draw. A cursor
    /// event that hovers another item requests a redraw. The position tells apart
    /// items with the same key.
    pub(super) drawn_hover: RefCell<Option<(K, usize)>>,
    /// Every walk, for the tests.
    #[cfg(test)]
    pub(super) walks: Vec<Walk>,
}

/// An item of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Slot<K> {
    pub(super) position: usize,
    pub(super) key: K,
}

/// What one walk did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Walk {
    /// Rows built, which got a new tree.
    pub(super) built: usize,
    /// Rows laid out, each at most once.
    pub(super) measured: usize,
    /// Measured rows whose indexed height changed, or whose height was invalid.
    pub(super) changed: usize,
    /// Rows dropped at the end.
    pub(super) evicted: usize,
    /// Rows built and dropped by the same walk, which is wasted work.
    pub(super) discarded: usize,
}

impl<K: Clone + Eq + Hash> Core<K> {
    pub(super) fn new(settings: Settings) -> Self {
        Self {
            model: Model::default(),
            geometry: Geometry::new(settings),
            window: Vec::new(),
            anchor: None,
            drawn_hover: RefCell::new(None),
            #[cfg(test)]
            walks: Vec::new(),
        }
    }

    pub(super) fn anchor_position(&self) -> Option<usize> {
        self.anchor
            .as_ref()
            .and_then(|key| self.model.position(key))
    }

    /// Rule A against the current anchor.
    pub(super) fn absorb(&mut self) {
        let anchor = self.anchor_position();
        self.geometry.absorb(anchor);
    }

    pub(super) fn span(&self) -> Option<Span> {
        Some(Span {
            first: self.window.first()?.position,
            last: self.window.last()?.position,
            len: self.window.len(),
        })
    }

    /// Applies the builder settings, keeping the anchor in place when they move it.
    pub(super) fn configure(&mut self, settings: Settings) {
        if self.geometry.configure(settings) {
            self.absorb();
        }
    }

    /// Follows a structural change of the keys (section 4.2): keeps the window rows
    /// and trees of surviving keys, hands a removed anchor's target to the next
    /// surviving key, and absorbs before any window is chosen.
    pub(super) fn restructure(&mut self, rebuilt: &Rebuilt<K>, children: &mut Vec<Tree>) {
        let window = mem::take(&mut self.window);
        let trees = mem::take(children);

        let mut kept: Vec<(Slot<K>, Tree)> = window
            .into_iter()
            .zip(trees)
            .filter_map(|(slot, tree)| {
                let position = rebuilt.new_position(slot.position)?;
                Some((
                    Slot {
                        position,
                        key: slot.key,
                    },
                    tree,
                ))
            })
            .collect();

        kept.sort_by_key(|(slot, _)| slot.position);
        (self.window, *children) = kept.into_iter().unzip();

        if let Some(anchor) = &self.anchor
            && self.model.position(anchor).is_none()
        {
            match rebuilt.survivor(anchor) {
                Some(position) => self.anchor = Some(self.model.key(position).clone()),
                None => {
                    // Nothing that was shown remains, so there is no content to keep
                    // still. Folding the drift also keeps an empty list at the height
                    // of its padding.
                    let drift = self.geometry.drift();
                    self.geometry.move_drift(-drift);
                    self.unpin();
                }
            }
        }

        self.absorb();
        self.geometry.dirty = true;
    }

    /// Handles a redraw at `now` with the given visible region (section 1.3, full path
    /// only). Returns whether the list needs a new walk, which this phase does with a
    /// root layout.
    pub(super) fn redraw(&mut self, visible: Visible, now: Instant) -> bool {
        let previous = self.geometry.visible.replace(visible);
        let laid_out = !self.geometry.dirty && self.geometry.laid_out.is_some();

        if visible.at_start() {
            // Rule Z. It applies before the list is laid out too, so I1 holds at the
            // end of the next walk.
            let moved = self.geometry.zero_drift();
            self.unpin();

            return moved || !laid_out || self.geometry.needs_walk(visible, self.span());
        }

        if !laid_out {
            return true;
        }

        // Rule S, before any coverage work. Rule M moved the old anchor's target with
        // the drift, so it may now lie far below the viewport, and rows measured above
        // it would push the visible rows away while the walk covers them. Refreshing
        // the anchor on the shrunk geometry pins the first visible row instead and
        // keeps the shrink, because the new target is where that row is now.
        if let Some(previous) = previous
            && self.geometry.shrink(previous.top, visible.top, now)
        {
            self.pin_first_visible(visible);
            return true;
        }

        self.pin_first_visible(visible);
        self.geometry.needs_walk(visible, self.span())
    }

    /// The anchor refresh: pins the first item whose bottom is below the visible top.
    ///
    /// The anchor is found again through its key, which leads to the key's first
    /// position. A later duplicate is skipped for the next first occurrence, else the
    /// last one before it, so a position is never paired with another one's target.
    /// This scans the run of duplicates around the visible top.
    pub(super) fn pin_first_visible(&mut self, visible: Visible) {
        let Some(first) = self.geometry.first_visible(visible.top) else {
            self.unpin();
            return;
        };

        let owner =
            |position: usize| self.model.position(self.model.key(position)) == Some(position);
        let position = (first..self.model.len())
            .find(|&position| owner(position))
            .or_else(|| (0..first).rev().find(|&position| owner(position)));

        match position {
            Some(position) => {
                self.anchor = Some(self.model.key(position).clone());
                self.geometry.pin(position);
            }
            None => self.unpin(),
        }
    }

    pub(super) fn unpin(&mut self) {
        self.anchor = None;
        self.geometry.unpin();
    }

    /// Checks invariants I1 and I2 (section 4.4) and the alignment of the window with
    /// the tree and the rows.
    fn check(&self, visible: Visible, children: &[Tree], rows: usize) {
        let geometry = &self.geometry;

        if visible.at_start() {
            debug_assert_eq!(geometry.drift(), 0.0, "I1: drift at the start");
            debug_assert!(self.anchor.is_none(), "I1: anchor at the start");
        }

        debug_assert_eq!(self.anchor.is_some(), geometry.anchor_y().is_some());

        if let Some(target) = geometry.anchor_y() {
            let position = self.anchor_position().expect("the anchor key is present");
            let error = geometry.content_y(position) - target;
            debug_assert!(error.abs() < 1.0e-3, "I2: anchor off its target by {error}");
        } else {
            debug_assert_eq!(geometry.drift(), 0.0, "drift without an anchor");
        }

        debug_assert_eq!(children.len(), self.window.len());
        debug_assert_eq!(rows, self.window.len());
        debug_assert!(
            self.window
                .windows(2)
                .all(|pair| pair[0].position < pair[1].position)
        );
        debug_assert!(
            self.window
                .iter()
                .all(|slot| *self.model.key(slot.position) == slot.key)
        );
    }
}

impl<'a, T, K, W, Message, Theme, Renderer> ListView<'a, T, K, W, Message, Theme, Renderer>
where
    K: Clone + Eq + Hash,
    W: Widget<Message, Theme, Renderer>,
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    /// The window walk (section 4.3): lays out the current rows, then adds rows one at
    /// a time until the visible region of the last redraw and the anticipation are
    /// covered by measured heights, evicts once and places the rows.
    ///
    /// `limits` are the item limits. `x` is the left edge of the rows.
    pub(super) fn sync(
        &mut self,
        core: &mut Core<K>,
        children: &mut Vec<Tree>,
        renderer: &Renderer,
        limits: &Limits,
        x: f32,
    ) -> Walk {
        let visible = core.geometry.visible.unwrap_or(Visible::GUESS);

        let mut walker = Walker {
            items: self.items,
            view: &*self.view,
            rows: &mut self.rows,
            children,
            core,
            renderer,
            limits: *limits,
            fresh: Vec::new(),
            new: Vec::new(),
            walk: Walk::default(),
            _widget: PhantomData,
        };

        walker.run(visible);
        walker.place(x);
        walker.core.geometry.laid_out = Some(visible);
        walker.core.geometry.dirty = false;
        walker
            .core
            .check(visible, walker.children, walker.rows.len());

        #[cfg(test)]
        walker.core.walks.push(walker.walk);

        walker.walk
    }
}

/// The borrows of one walk.
struct Walker<'w, 'a, T, K, W, Message, Theme, Renderer> {
    items: &'a [T],
    view: &'w (dyn Fn(usize, &'a T) -> W + 'a),
    rows: &'w mut Vec<W>,
    children: &'w mut Vec<Tree>,
    core: &'w mut Core<K>,
    renderer: &'w Renderer,
    limits: Limits,
    /// Whether each window row was laid out in this walk, aligned with the window.
    fresh: Vec<bool>,
    /// Whether each window row was built by this walk, aligned with the window.
    new: Vec<bool>,
    walk: Walk,
    _widget: PhantomData<fn(Message, Theme)>,
}

impl<T, K, W, Message, Theme, Renderer> Walker<'_, '_, T, K, W, Message, Theme, Renderer>
where
    K: Clone + Eq + Hash,
    W: Widget<Message, Theme, Renderer>,
{
    fn run(&mut self, visible: Visible) {
        let len = self.core.model.len();
        let anticipate = self.core.geometry.settings().anticipate;
        self.fresh = vec![false; self.core.window.len()];
        self.new = vec![false; self.core.window.len()];

        if len == 0 {
            self.walk.evicted = self.core.window.len();
            self.core.window.clear();
            self.rows.clear();
            self.children.clear();
            return;
        }

        // I2 needs an anchor away from the start. There is none after every key was
        // replaced, and without one the rows measured above the first visible row would
        // push it down, one row after another.
        if !visible.at_start() && self.core.anchor.is_none() {
            self.core.pin_first_visible(visible);
        }

        // 1. The current rows pick up size changes. 2. Rule A runs after each one.
        for index in 0..self.core.window.len() {
            self.measure(index);
        }

        // 3-6. Coverage from the first visible row down, then the anticipation. The
        // range is recomputed until a pass adds nothing that moves it. Rows built are
        // normally all kept. One exception is accepted: a first row estimated far
        // taller than it is shrinks above the viewport when measured, and the rows
        // found below it before the anchor moves on are discarded, about its estimate
        // error divided by the real row height.
        let mut range = None;

        let (first, last) = loop {
            // The ordinary anchor stands for visible content: it is the first visible
            // row of the last refresh. Measurements (of the retained rows, or of the
            // anchor itself) can move it out of the visible region, and rows measured
            // between it and the viewport would then push the visible rows away while
            // the walk follows them. So it is refreshed on the current geometry, which
            // changes no drift. P4's instant scroll pins the destination's first row
            // for the viewport after the jump, which the stored visible region does
            // not show yet. That pin must be marked on the anchor and left alone here
            // until the next laid-out redraw refreshes it.
            if !visible.at_start() && !self.anchor_covers(visible) {
                self.core.pin_first_visible(visible);
            }

            let first = self.core.geometry.first_row(visible);
            self.ensure(first);

            let mut last = first;

            while last + 1 < len && self.core.geometry.starts_before(last + 1, visible) {
                last += 1;
                self.ensure(last);
            }

            for position in last + 1..len.min(last + 1 + anticipate) {
                self.ensure(position);
            }

            for position in (first.saturating_sub(anticipate)..first).rev() {
                self.ensure(position);
            }

            if range == Some((first, last)) {
                break (first, last);
            }

            range = Some((first, last));
        };

        // 7. Evict once, around the final anchored range.
        let lowest = first.saturating_sub(anticipate);
        let highest = (last + anticipate).min(len - 1);

        let keep: Vec<bool> = self
            .core
            .window
            .iter()
            .map(|slot| (lowest..=highest).contains(&slot.position))
            .collect();

        self.walk.evicted = keep.iter().filter(|&&keep| !keep).count();
        self.walk.discarded = keep
            .iter()
            .zip(&self.new)
            .filter(|&(&keep, &new)| !keep && new)
            .count();
        retain(&mut self.core.window, &keep);
        retain(self.rows, &keep);
        retain(self.children, &keep);
    }

    fn anchor_covers(&self, visible: Visible) -> bool {
        self.core
            .anchor_position()
            .is_some_and(|position| self.core.geometry.covers(position, visible))
    }

    /// Lays out the window row at `index` once per walk and records its height.
    fn measure(&mut self, index: usize) {
        if self.fresh[index] {
            return;
        }

        self.fresh[index] = true;

        let tree = &mut self.children[index];
        self.rows[index].layout(tree, self.renderer, &self.limits);

        let height = tree.size.height;
        let position = self.core.window[index].position;
        let core = &mut *self.core;
        let recorded = core
            .model
            .record(position, height, &mut core.geometry.heights);

        self.walk.measured += 1;

        if recorded != Recorded::Unchanged {
            self.walk.changed += 1;
        }

        core.absorb();
    }

    /// Makes sure the row at `position` is in the window and laid out in this walk.
    fn ensure(&mut self, position: usize) {
        let index = match self
            .core
            .window
            .binary_search_by_key(&position, |slot| slot.position)
        {
            Ok(index) => index,
            Err(index) => {
                let mut row = (self.view)(position, &self.items[position]);
                let mut tree = Tree::new(&row);
                tree.diff(&mut row);

                let key = self.core.model.key(position).clone();
                self.core.window.insert(index, Slot { position, key });
                self.rows.insert(index, row);
                self.children.insert(index, tree);
                self.fresh.insert(index, false);
                self.new.insert(index, true);
                self.walk.built += 1;

                index
            }
        };

        self.measure(index);
    }

    fn place(&mut self, x: f32) {
        let geometry = &self.core.geometry;

        for (slot, child) in self.core.window.iter().zip(self.children.iter_mut()) {
            child.translation = Vector::new(x, geometry.content_y(slot.position) as f32);
        }
    }
}

fn retain<T>(items: &mut Vec<T>, keep: &[bool]) {
    let mut keep = keep.iter();
    items.retain(|_| *keep.next().expect("one flag per item"));
}
