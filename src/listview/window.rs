//! The window: the items that exist as widgets, kept in the tree by key, and the
//! walk that chooses them.

use super::geometry::{Geometry, Settings, Span, Target, Visible};
use super::heights::Recorded;
use super::model::{Model, Rebuilt};
use super::selection::Input;
use super::{Catalog, ListView};

use std::cell::{Cell, RefCell};
use std::hash::Hash;
use std::marker::PhantomData;
use std::mem;
use std::ops::RangeInclusive;

use iced::Vector;
use iced::advanced::Widget;
use iced::advanced::layout::Limits;
use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{Id, Tree};
use iced::time::Instant;

/// The state of a list, stored in its tree.
#[derive(Debug)]
pub(super) struct Core<K> {
    pub(super) model: Model<K>,
    pub(super) geometry: Geometry,
    /// Sorted by position and aligned with the tree's children and the widget's rows.
    pub(super) window: Vec<Slot<K>>,
    /// What the anchor pins. Its target content y is in the geometry.
    pub(super) anchor: Option<Anchor<K>>,
    /// The key and position of the item drawn as hovered by the last draw. A cursor
    /// event that hovers another item requests a redraw. The position tells apart
    /// items with the same key.
    pub(super) drawn_hover: RefCell<Option<(K, usize)>>,
    /// The anchor was pinned by an instant scroll or `settle` for the viewport after
    /// their jump, which the stored visible region does not show yet.
    pub(super) landing: Option<Landing>,
    pub(super) focus: Focus<K>,
    pub(super) input: Input<K>,
    /// Positions measured by `resolve`, for the tests.
    #[cfg(test)]
    pub(super) resolved: Vec<usize>,
    /// Every walk, for the tests.
    #[cfg(test)]
    pub(super) walks: Vec<Walk>,
}

/// An anchor pinned for the viewport after a jump.
///
/// It pins its item through every pass and every layout until the list is drawn
/// with a window walked for the region the jump reached: the destination or, for a
/// destination the parent cannot reach, the region the last redraw pass saw, if the
/// list still has the height the parent clamped against in that pass. Rules Z and S,
/// the re-pins and the refresh leave it alone until then, so rows built for the
/// destination, messages and later siblings can change heights in that frame without
/// moving it. The next frame releases it before anything else, and is an ordinary
/// frame. A jump clamped to the start is released as soon as it is laid out there,
/// and rule Z applies.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Landing {
    /// The visible top the jump aims at, in list coordinates.
    pub(super) top: f64,
    /// The visible region of the latest redraw since the landing was pinned.
    pub(super) seen: Option<Visible>,
    /// The list's height in the latest redraw.
    pub(super) total: Option<f64>,
    /// The timestamp of the latest redraw.
    pub(super) frame: Option<Instant>,
    /// The list was drawn with the reached region laid out. Set by `draw`, which only
    /// has a shared reference.
    pub(super) drawn: Cell<bool>,
}

impl Landing {
    pub(super) fn new(top: f64) -> Self {
        Self {
            top,
            seen: None,
            total: None,
            frame: None,
            drawn: Cell::new(false),
        }
    }

    /// Whether the scroller may not show the destination yet. A task's view of the
    /// scroller is stale until then.
    pub(super) fn pending(&self) -> bool {
        !self.drawn.get()
    }
}

/// The list's own focus, and a focus waiting for its item to be laid out
/// (section 4.7). While a focus waits, the list holds the focus for it, so exactly
/// one widget is focused, and any focus change that visits the list replaces it.
#[derive(Debug)]
pub(super) struct Focus<K> {
    pub(super) focused: bool,
    /// The key of the item and the widget inside it to focus.
    pub(super) deferred: Option<(K, Id)>,
}

impl<K> Focusable for Focus<K> {
    fn is_focused(&self) -> bool {
        self.focused
    }

    /// An explicit focus of the list replaces a deferred one.
    fn focus(&mut self) {
        self.focused = true;
        self.deferred = None;
    }

    /// Every focus change that visits the list calls this or [`Self::focus`], so a
    /// later focus change cancels the deferred one.
    fn unfocus(&mut self) {
        self.focused = false;
        self.deferred = None;
    }
}

/// The anchor (section 4.4), with its target y kept by the geometry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Anchor<K> {
    /// The top of the first item with this key.
    Item(K),
    /// The bottom of the last item, while the visible region reaches it.
    End,
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
    #[cfg(test)]
    pub(super) built: usize,
    /// Rows laid out, each at most once.
    #[cfg(test)]
    pub(super) measured: usize,
    /// Measured rows whose indexed height changed, or whose height was invalid.
    pub(super) changed: usize,
    /// Rows dropped at the end.
    #[cfg(test)]
    pub(super) evicted: usize,
    /// Rows built and dropped by the same walk, which is wasted work.
    #[cfg(test)]
    pub(super) discarded: usize,
    /// The walk ran inside a redraw with cached limits, not in a full layout.
    #[cfg(test)]
    pub(super) local: bool,
}

/// What a redraw needs from the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Redraw {
    /// The window covers the visible region.
    Idle,
    /// The window must change, and nothing else did: a local walk may do it.
    Shift,
    /// Root layout, for every pass of this redraw.
    Full,
}

impl<K: Clone + Eq + Hash> Core<K> {
    pub(super) fn new(settings: Settings) -> Self {
        Self {
            model: Model::default(),
            geometry: Geometry::new(settings),
            window: Vec::new(),
            anchor: None,
            drawn_hover: RefCell::new(None),
            landing: None,
            focus: Focus {
                focused: false,
                deferred: None,
            },
            input: Input::default(),
            #[cfg(test)]
            resolved: Vec::new(),
            #[cfg(test)]
            walks: Vec::new(),
        }
    }

    pub(super) fn anchor_target(&self) -> Option<Target> {
        match self.anchor.as_ref()? {
            Anchor::Item(key) => self.model.position(key).map(Target::Row),
            Anchor::End => Some(Target::End),
        }
    }

    /// Rule A against the current anchor, then rule G against the region the list
    /// shows (or a pending landing's destination).
    pub(super) fn absorb(&mut self) {
        self.absorb_only();
        let _ = self.close_gap(self.shown());
    }

    /// Rule A alone, for a batch of measurements followed by rule G: one change of
    /// the batch alone can look like a deficit that the rest of it makes up.
    fn absorb_only(&mut self) {
        let anchor = self.anchor_target();
        self.geometry.absorb(anchor);
    }

    /// The region the list shows: the visible region of the last redraw, or the
    /// destination of a jump the scroller has not drawn yet.
    pub(super) fn shown(&self) -> Option<Visible> {
        let visible = self.geometry.visible?;

        Some(match self.pending_landing() {
            Some(landing) => Visible {
                top: landing.top,
                height: visible.height,
            },
            None => visible,
        })
    }

    /// The positions of the items in the region the list shows, before a change of
    /// the keys. A removed anchor is handed only to one of them.
    pub(super) fn shown_positions(&self) -> Option<RangeInclusive<usize>> {
        let shown = self.shown()?;

        if self.model.len() == 0 {
            return None;
        }

        Some(self.geometry.first_row(shown)..=self.geometry.last_row(shown))
    }

    /// Rule G: positive drift never shows as a gap.
    ///
    /// The drift spacer lies between the list's top padding and its first item. When
    /// it reaches into the region the list shows, the drift shrinks so the first item
    /// starts at the region's top, which moves the content and the anchor's target
    /// together (rule M): the visible items move up into the gap instead of leaving it
    /// blank. When the items from the first one down are shorter than the region,
    /// the parent would clamp its translation and open the gap again, so the drift is
    /// folded entirely and the list starts at its natural position. The parent clamps
    /// against the shorter content on its next update, and the next redraw checks
    /// again.
    ///
    /// `region` is the region the list shows, or after measurements while a landing
    /// is pending its destination. A redraw checks the region the parent actually
    /// shows, for a landing once that region is final: after an edit the destination
    /// can be out of reach, and the clamped region is what is drawn. A pending
    /// landing's destination moves with the content, so it still refers to the same
    /// items. Returns whether the drift changed.
    pub(super) fn close_gap(&mut self, region: Option<Visible>) -> bool {
        let drift = self.geometry.drift();

        let Some(shown) = region.filter(|_| drift > 0.0) else {
            return false;
        };

        let padding = f64::from(self.geometry.settings().padding.top);

        if padding + drift <= shown.top || padding >= shown.bottom() {
            return false;
        }

        // The items from the first one down, and the bottom padding.
        let below = self.geometry.total() - padding - drift;

        let kept = if below >= shown.height {
            (shown.top - padding).max(0.0)
        } else {
            0.0
        };

        self.geometry.move_drift(kept - drift);

        if let Some(landing) = &mut self.landing {
            landing.top += kept - drift;
        }

        true
    }

    pub(super) fn span(&self) -> Option<Span> {
        Some(Span {
            first: self.window.first()?.position,
            last: self.window.last()?.position,
            len: self.window.len(),
        })
    }

    /// Applies the builder settings, keeping the anchor in place when they move it.
    /// Returns whether they changed.
    pub(super) fn configure(&mut self, settings: Settings) -> bool {
        let changed = self.geometry.configure(settings);

        if changed {
            self.absorb();
        }

        changed
    }

    /// Applies per-item estimates set for the current sequence, keeping the anchor in
    /// place when they move it. Returns whether an indexed height changed.
    pub(super) fn apply_item_estimates(&mut self) -> bool {
        let changed = self.geometry.heights.apply_item_estimates();

        if changed {
            self.geometry.dirty = true;
            self.absorb();
        }

        changed
    }

    /// Follows a structural change of the keys (section 4.2): keeps the window rows
    /// and trees of surviving keys, hands a removed anchor's target to the next
    /// surviving key among the `shown` positions (from before the change), and
    /// absorbs before any window is chosen.
    pub(super) fn restructure(
        &mut self,
        rebuilt: &Rebuilt<K>,
        children: &mut Vec<Tree>,
        shown: Option<RangeInclusive<usize>>,
    ) {
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

        // A removed anchor goes to a survivor that was shown before the change, which
        // keeps what is still on screen in place. Handing it to an item that was not
        // shown would keep that item where the anchor was, with all the drift that
        // takes, and the drift would show as a gap.
        if let Some(Anchor::Item(anchor)) = &self.anchor
            && self.model.position(anchor).is_none()
        {
            match shown.and_then(|shown| rebuilt.survivor(anchor, shown)) {
                Some(position) => {
                    self.anchor = Some(Anchor::Item(self.model.key(position).clone()));
                }
                None => {
                    // Nothing that was shown remains, so there is no content to keep
                    // still: the drift is folded and the scrollable clamps as for any
                    // shorter content. Folding also keeps an empty list at the height
                    // of its padding.
                    let drift = self.geometry.drift();
                    self.geometry.move_drift(-drift);
                    self.unpin();
                }
            }
        }

        // An end pin has nothing to keep in an empty list, which is as tall as its
        // padding.
        if self.model.len() == 0 && self.anchor.is_some() {
            let drift = self.geometry.drift();
            self.geometry.move_drift(-drift);
            self.unpin();
        }

        self.absorb();
        self.geometry.dirty = true;
    }

    /// Handles a redraw at `now` with the given visible region (section 1.3).
    ///
    /// [`Redraw::Full`] marks the whole redraw for root layout, so its retries with
    /// the same timestamp stay on the full path: after a change made outside the
    /// redraw, with no current layout or cached limits, and for rules Z and S.
    pub(super) fn redraw(&mut self, visible: Visible, now: Instant) -> Redraw {
        // A landing drawn in an earlier frame is released before rules Z and S and the
        // refresh, so this frame treats its anchor like any other.
        if self
            .landing
            .as_ref()
            .is_some_and(|landing| landing.drawn.get() && landing.frame != Some(now))
        {
            self.landing = None;
        }

        // A deferred focus keeps the whole redraw on the full path, also the passes
        // after it commits.
        if mem::take(&mut self.geometry.block_next_redraw) || self.focus.deferred.is_some() {
            self.geometry.full_frame = Some(now);
        }

        let redraw = self.decide(visible, now);

        if redraw == Redraw::Full {
            self.geometry.full(now);
        }

        redraw
    }

    fn decide(&mut self, visible: Visible, now: Instant) -> Redraw {
        let previous = self.geometry.visible.replace(visible);

        // Rule G against the region the parent shows now, which it may have clamped
        // against shorter content. A change leaves the geometry dirty, so this redraw
        // takes the full path. A pending landing is checked once the region it reached
        // is final, below.
        if self.landing.is_none() {
            let _ = self.close_gap(Some(visible));
        }

        let current = !self.geometry.dirty
            && self.geometry.laid_out.is_some()
            && self.geometry.local.is_some();

        let shift = |core: &Self| {
            if core.geometry.needs_walk(visible, core.span()) {
                Redraw::Shift
            } else {
                Redraw::Idle
            }
        };

        if let Some(landing) = &mut self.landing {
            let walked = current && self.geometry.laid_out == Some(visible);
            let arrived = (visible.top - landing.top).abs() <= 1.0 || landing.seen == Some(visible);
            landing.seen = Some(visible);
            landing.frame = Some(now);
            landing.total = Some(self.geometry.total());

            if !(walked && arrived) {
                return Redraw::Full;
            }

            // The region the landing reached is the one drawn. When an edit put the
            // destination out of reach, it is clamped, and rule G applies to it.
            if self.close_gap(Some(visible)) {
                return Redraw::Full;
            } else if !visible.at_start() {
                return shift(self);
            }

            // A jump the parent clamped to the start shows the start, where rule Z
            // keeps the list's top in place as in any other frame there.
            self.landing = None;
        }

        if visible.at_start() {
            // Rule Z. It applies before the list is laid out too, so I1 holds at the
            // end of the next walk.
            let moved = self.geometry.zero_drift();
            self.unpin();

            return if moved || !current {
                Redraw::Full
            } else {
                shift(self)
            };
        }

        if !current {
            return Redraw::Full;
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
            return Redraw::Full;
        }

        // The refresh moves no content, so a local walk may follow it.
        self.pin_first_visible(visible);
        shift(self)
    }

    /// The anchor refresh: pins the end of the list while the visible region reaches
    /// it, else the first item whose bottom is below the visible top.
    ///
    /// At the end, the visible items are kept still from below. Measuring them then
    /// moves the content above, out of view, and the list's height stays as it is, so
    /// the parent scrollable never has to clamp its translation against a list that
    /// keeps getting shorter. Pinning the first visible item there instead would move
    /// the end, and with it the largest translation, on every pass.
    ///
    /// An item is found again through its key, which leads to the key's first
    /// position. A later duplicate is skipped for the next first occurrence, else the
    /// last one before it, so a position is never paired with another one's target.
    /// This scans the run of duplicates around the visible top.
    pub(super) fn pin_first_visible(&mut self, visible: Visible) {
        self.landing = None;

        if self
            .geometry
            .end_visible(visible, self.anchor == Some(Anchor::End))
        {
            self.anchor = Some(Anchor::End);
            self.geometry.pin(Target::End);
            return;
        }

        match self.first_visible_owner(visible.top) {
            Some(position) => {
                self.anchor = Some(Anchor::Item(self.model.key(position).clone()));
                self.geometry.pin(Target::Row(position));
            }
            None => self.unpin(),
        }
    }

    /// The first visible item that is its key's first occurrence, else the last one
    /// before it.
    fn first_visible_owner(&self, top: f64) -> Option<usize> {
        let first = self.geometry.first_visible(top)?;

        let owner =
            |position: usize| self.model.position(self.model.key(position)) == Some(position);

        (first..self.model.len())
            .find(|&position| owner(position))
            .or_else(|| (0..first).rev().find(|&position| owner(position)))
    }

    /// The item pin that replaces an end pin when the keys change: the first visible
    /// item, where it is now.
    ///
    /// Structural changes keep the visible surviving items still, wherever the
    /// change is. An end pin would instead follow the new sequence's end, so an
    /// appended item would push the visible items up, and unrelated new items would
    /// keep the old end. The caller pins this before reconciling, the keyed path then
    /// hands it to a survivor or folds the drift, and the next refresh pins the end
    /// again if it is still in view.
    pub(super) fn end_as_item(&self) -> Option<(K, f64)> {
        if self.anchor != Some(Anchor::End) {
            return None;
        }

        let visible = self.geometry.visible?;
        let position = self.first_visible_owner(visible.top)?;

        Some((
            self.model.key(position).clone(),
            self.geometry.content_y(position),
        ))
    }

    /// Marks a landing drawn when the list is drawn showing `visible`, laid out for it,
    /// and `visible` is the destination, or the region of the last redraw pass while
    /// the list keeps the height of that pass. The parent clamped that pass against
    /// this height, so it shows the same region in the next frame.
    pub(super) fn draw_landing(&self, visible: Visible) {
        let Some(landing) = &self.landing else {
            return;
        };

        let arrived = (visible.top - landing.top).abs() <= 1.0
            || (landing.seen == Some(visible) && landing.total == Some(self.geometry.total()));

        if arrived && !self.geometry.dirty && self.geometry.laid_out == Some(visible) {
            landing.drawn.set(true);
        }
    }

    /// The landing, while the scroller may not show its destination yet.
    pub(super) fn pending_landing(&self) -> Option<&Landing> {
        self.landing.as_ref().filter(|landing| landing.pending())
    }

    pub(super) fn unpin(&mut self) {
        self.anchor = None;
        self.landing = None;
        self.geometry.unpin();
    }

    /// Pins the first item visible at `top`, the top of the viewport after an instant
    /// jump, as a landing. Always an item: the rows from it down to the target are
    /// measured, which keeps the target exact. If the end is in view, the refresh of
    /// the frame after the landing is drawn pins it without moving anything.
    /// At the start there is no anchor (I1).
    pub(super) fn pin_landing(&mut self, top: f64) {
        let visible = Visible { top, height: 0.0 };

        match self.first_visible_owner(top) {
            Some(position) if !visible.at_start() => {
                self.anchor = Some(Anchor::Item(self.model.key(position).clone()));
                self.geometry.pin(Target::Row(position));
                self.landing = Some(Landing::new(top));
            }
            _ => self.unpin(),
        }
    }

    /// Checks invariants I1 and I2 (section 4.4) and the alignment of the window with
    /// the tree and the rows, in debug builds.
    #[cfg(debug_assertions)]
    fn check(&self, visible: Visible, children: &[Tree], rows: usize) {
        let geometry = &self.geometry;

        // A landing pins for the viewport after the jump, not for the stored one.
        if visible.at_start() && self.landing.is_none() {
            debug_assert_eq!(geometry.drift(), 0.0, "I1: drift at the start");
            debug_assert!(self.anchor.is_none(), "I1: anchor at the start");
        }

        debug_assert_eq!(self.anchor.is_some(), geometry.anchor_y().is_some());

        if let Some(target) = geometry.anchor_y() {
            let anchor = self.anchor_target().expect("the anchor key is present");
            let error = geometry.target_y(anchor) - target;
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
    /// `limits` are the item limits. `x` is the left edge of the rows. `local` tells
    /// a walk inside a redraw from one in a full layout. Both walk the same way.
    pub(super) fn sync(
        &mut self,
        core: &mut Core<K>,
        children: &mut Vec<Tree>,
        renderer: &Renderer,
        limits: &Limits,
        x: f32,
        local: bool,
    ) -> Walk {
        #[cfg(not(test))]
        let _ = local;

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
            batch: false,
            #[cfg(test)]
            new: Vec::new(),
            walk: Walk {
                #[cfg(test)]
                local,
                ..Walk::default()
            },
            _widget: PhantomData,
        };

        walker.run(visible);

        // Without an estimate from the application, the end of the first walk that
        // measured positive heights adopts their median. The unmeasured items above
        // the anchor change height then, which is absorbed before the rows are placed,
        // so nothing visible moves.
        if walker.core.geometry.heights.adopt() {
            walker.core.absorb();
        }

        walker.place(x);
        walker.core.geometry.laid_out = Some(visible);
        walker.core.geometry.dirty = false;

        #[cfg(debug_assertions)]
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
    /// The retained rows are being measured, and rule G waits for all of them.
    batch: bool,
    /// Whether each window row was built by this walk, aligned with the window, for
    /// the statistics of the tests.
    #[cfg(test)]
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

        #[cfg(test)]
        {
            self.new = vec![false; self.core.window.len()];
        }

        if len == 0 {
            #[cfg(test)]
            {
                self.walk.evicted = self.core.window.len();
            }

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

        // 1. The current rows pick up size changes. 2. Rule A runs after each one, and
        // rule G after all of them.
        self.batch = true;

        for index in 0..self.core.window.len() {
            self.measure(index);
        }

        self.batch = false;
        let shown = self.core.shown();
        let _ = self.core.close_gap(shown);

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
            // changes no drift. The same happens when the end of the list comes into
            // view, so the end is pinned before measurements shorten the list any more.
            // P4's instant scroll pins the destination's first row for the viewport
            // after the jump, which the stored visible region does not show yet. That
            // pin is a landing, left alone here until the frame after it is drawn
            // releases it.
            // Only a re-pin that changes the anchor counts. A refresh that picks the
            // same anchor again, as when no item covers an empty visible region, must
            // not keep the walk going.
            let mut repinned = false;

            if !visible.at_start() && self.stale_anchor(visible) {
                let before = (self.core.anchor.clone(), self.core.geometry.anchor_y());
                self.core.pin_first_visible(visible);
                repinned = before != (self.core.anchor.clone(), self.core.geometry.anchor_y());
            }

            let (first, last) = if self.core.anchor == Some(Anchor::End) {
                self.cover_up(visible)
            } else {
                self.cover_down(visible)
            };

            for position in last + 1..len.min((last + 1).saturating_add(anticipate)) {
                self.ensure(position);
            }

            for position in (first.saturating_sub(anticipate)..first).rev() {
                self.ensure(position);
            }

            if range == Some((first, last)) && !repinned {
                break (first, last);
            }

            range = Some((first, last));
        };

        // 7. Evict once, around the final anchored range.
        let lowest = first.saturating_sub(anticipate);
        let highest = last.saturating_add(anticipate).min(len - 1);

        let keep: Vec<bool> = self
            .core
            .window
            .iter()
            .map(|slot| (lowest..=highest).contains(&slot.position))
            .collect();

        #[cfg(test)]
        {
            self.walk.evicted = keep.iter().filter(|&&keep| !keep).count();
            self.walk.discarded = keep
                .iter()
                .zip(&self.new)
                .filter(|&(&keep, &new)| !keep && new)
                .count();
        }

        retain(&mut self.core.window, &keep);
        retain(self.rows, &keep);
        retain(self.children, &keep);
    }

    /// Covers `visible` from the first visible row down, adding each next row while it
    /// starts above the visible bottom. Returns the first and last rows.
    fn cover_down(&mut self, visible: Visible) -> (usize, usize) {
        let len = self.core.model.len();
        let first = self.core.geometry.first_row(visible);
        self.ensure(first);

        let mut last = first;

        while last + 1 < len && self.core.geometry.starts_before(last + 1, visible) {
            last += 1;
            self.ensure(last);
        }

        (first, last)
    }

    /// Covers `visible` from the pinned end up, adding each previous row while it ends
    /// below the visible top. Returns the first and last rows.
    ///
    /// With the end pinned, a measured row moves only the rows above it. Walking up
    /// from the end therefore stops as soon as real heights cover the top, whether
    /// the rows grow or shrink, and keeps every row it builds. Walking down instead
    /// would measure every row the estimates put in view, because each one that grows
    /// pushes the rows before it up and out.
    fn cover_up(&mut self, visible: Visible) -> (usize, usize) {
        // The last row starting above the visible bottom, as coverage counts it. The
        // rows after it, between it and the pinned end, are empty and not visible.
        let geometry = &self.core.geometry;
        let mut last = geometry
            .heights
            .position_before(visible.bottom() - geometry.base());

        loop {
            self.ensure(last);

            // Measuring it moves only the rows above it. If it turned out empty at
            // the bottom, the one before it is the last visible one.
            if last == 0 || self.core.geometry.starts_before(last, visible) {
                break;
            }

            last -= 1;
        }

        let mut first = last;

        while first > 0 && self.core.geometry.bottom_of(first - 1) > visible.top {
            first -= 1;
            self.ensure(first);
        }

        (first, last)
    }

    /// Whether the anchor no longer is what a refresh would pick: the end while it is
    /// visible, else an item covering visible content.
    fn stale_anchor(&self, visible: Visible) -> bool {
        if self.core.landing.is_some() {
            return false;
        }

        let geometry = &self.core.geometry;

        match self.core.anchor_target() {
            _ if geometry.end_visible(visible, self.core.anchor == Some(Anchor::End)) => {
                self.core.anchor != Some(Anchor::End)
            }
            Some(Target::Row(position)) => !geometry.covers(position, visible),
            Some(Target::End) | None => true,
        }
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

        #[cfg(test)]
        {
            self.walk.measured += 1;
        }

        if recorded != Recorded::Unchanged {
            self.walk.changed += 1;
        }

        if self.batch {
            core.absorb_only();
        } else {
            core.absorb();
        }
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

                #[cfg(test)]
                {
                    self.new.insert(index, true);
                    self.walk.built += 1;
                }

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
