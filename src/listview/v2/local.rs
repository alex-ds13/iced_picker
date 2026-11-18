//! Local relayout (section 14.1): a window shift on a redraw lays out only the list,
//! with the limits of the last full layout, and asks for no root layout when the
//! walk leaves every measured height, the drift, the estimate and the list's size
//! unchanged.
//!
//! This follows the fork's `Component`, which lays out its subtree inside `update`
//! (`widget/src/component.rs:274-300`). It is safe because the runtime holds only
//! copies of positions and sizes: a successful walk changes neither the list's size
//! nor anything that places its siblings, and draw, operate and the overlays of the
//! same frame start from a fresh root `Layout` and read the children's trees
//! (`runtime/src/user_interface.rs:584-645`).
//!
//! This saves the root layout only. Inside a `Component`, any invalidation from its
//! subtree, the overlay one of a successful walk included, still makes the component
//! rebuild and lay out its whole view (`widget/src/component.rs:259-300`).

use super::geometry::Settings;
use super::window::{Core, Redraw};
use super::{Catalog, ListView, item_limits};

use std::hash::Hash;

use iced::advanced::layout::{Layout, Limits};
use iced::advanced::shell::Invalidation;
use iced::advanced::widget::Tree;
use iced::advanced::widget::operation::focusable;
use iced::advanced::{Shell, Widget};
use iced::time::Instant;
use iced::{Length, Rectangle, Size};

/// The constraints of the last full layout, reused by local relayout.
///
/// Every full layout replaces it and every `diff` clears it, so it never outlives a
/// change of the parent's limits, the builder settings or the widget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LocalLayout {
    /// The exact limits the list received.
    pub(super) limits: Limits,
    /// The size the list took.
    pub(super) size: Size,
    pub(super) sizing: Size<Length>,
    /// The width of the items.
    pub(super) width: f32,
    pub(super) settings: Settings,
}

#[cfg(test)]
thread_local! {
    /// Lets tests run the same trace with and without local relayout.
    pub(super) static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

#[cfg(test)]
fn enabled() -> bool {
    ENABLED.get()
}

#[cfg(not(test))]
fn enabled() -> bool {
    true
}

impl<T, K, W, Message, Theme, Renderer> ListView<'_, T, K, W, Message, Theme, Renderer>
where
    K: Clone + Eq + Hash + Send + 'static,
    W: Widget<Message, Theme, Renderer>,
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    /// The list's part of a redraw at `now`, before the rows get it: the drift
    /// rules, the anchor refresh and, when the window must change, a local walk or a
    /// root layout.
    pub(super) fn redraw(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
        now: Instant,
    ) {
        let core = tree.state.downcast_mut::<Core<K>>();
        let visible = super::geometry::Visible::new(*viewport, layout.bounds());

        match core.redraw(visible, now) {
            Redraw::Idle => self.commit_focus(tree, layout, renderer, viewport),
            Redraw::Full => shell.invalidate_layout(),
            Redraw::Shift => {
                let local = self.eligible(tree, layout, viewport, shell.invalidation(), now);

                if local.is_some_and(|local| self.walk_locally(tree, renderer, local)) {
                    // Overlays of the rows may have moved, appeared or gone.
                    shell.invalidate_overlay();
                } else {
                    tree.state.downcast_mut::<Core<K>>().geometry.full(now);
                    shell.invalidate_layout();
                }
            }
        }
    }

    /// The cached constraints, when a local walk is allowed at `now` (section 14.1,
    /// "Eligible events and cached constraints").
    fn eligible(
        &self,
        tree: &Tree,
        layout: Layout,
        viewport: &Rectangle,
        invalidation: Invalidation,
        now: Instant,
    ) -> Option<LocalLayout> {
        let geometry = &tree.state.downcast_ref::<Core<K>>().geometry;
        let local = geometry.local?;

        let visible = layout
            .bounds()
            .intersection(viewport)
            .is_some_and(|visible| visible.height > 0.0);

        // A Layout or Widgets request from an earlier widget means a root layout runs
        // anyway, and the list must not make it look cheaper than it is.
        let pending = matches!(
            invalidation,
            Invalidation::Layout(_) | Invalidation::Widgets
        );

        let core = tree.state.downcast_ref::<Core<K>>();

        // A landing and a deferred focus wait for frames that only root layout
        // brings, and keep every redraw on the full path until they are done.
        let allowed = enabled()
            && core.landing.is_none()
            && core.focus.deferred.is_none()
            && visible
            && !pending
            && !geometry.dirty
            && geometry.full_frame != Some(now)
            && local.size == tree.size
            && local.size == layout.size()
            && local.sizing == self.size()
            && local.settings == self.settings();

        allowed.then_some(local)
    }

    /// Focuses the deferred widget once its item is laid out (section 4.7), with the
    /// fork's global focus operation on the item's subtree. The first part of the
    /// focus task already unfocused every other widget.
    fn commit_focus(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
    ) {
        let core = tree.state.downcast_mut::<Core<K>>();

        // The list holds the focus for the item. Without it, something else took the
        // focus and the item must not take it back.
        if !core.focus.focused {
            core.focus.deferred = None;
            return;
        }

        let Some(index) = core.focus.deferred.as_ref().and_then(|(key, _)| {
            let position = core.model.position(key)?;

            core.window
                .binary_search_by_key(&position, |slot| slot.position)
                .ok()
        }) else {
            return;
        };

        let Some((_, widget)) = core.focus.deferred.take() else {
            return;
        };

        // The list held the focus for the item, and hands it over.
        core.focus.focused = false;

        let mut focus = focusable::focus::<()>(widget);

        if let Some((layout, tree)) = layout.iter_mut(&mut tree.children).nth(index) {
            self.rows[index].operate(tree, layout, viewport, renderer, &mut focus);
        }
    }

    /// The shared walk with the cached limits. Returns whether the geometry stayed
    /// exactly the same. Otherwise the walk is kept as it is, with its real
    /// measurements and absorption, and the caller asks for a root layout.
    fn walk_locally(&mut self, tree: &mut Tree, renderer: &Renderer, local: LocalLayout) -> bool {
        let Tree {
            state,
            children,
            size,
            ..
        } = tree;
        let core = state.downcast_mut::<Core<K>>();

        let before = core.geometry.snapshot();
        let limits = item_limits(local.width, &local.limits);
        let walk = self.sync(
            core,
            children,
            renderer,
            &limits,
            local.settings.padding.left,
            true,
        );
        let after = Size::new(local.size.width, core.geometry.total().max(0.0) as f32);

        // Exact comparisons: one changed row is enough, even if the total or the
        // rounded size happen to be equal.
        let neutral = walk.changed == 0
            && core.geometry.snapshot() == before
            && after == local.size
            && self.size() == local.sizing;

        if !neutral {
            core.geometry.dirty = true;
            *size = after;
        }

        neutral
    }
}
