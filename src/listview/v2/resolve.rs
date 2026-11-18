//! The list's side of its tasks, run inside `operate` (sections 4.5 to 4.7, 6.2).
//!
//! `scroll_to` measures, then scrolls: the destination's rows are laid out here, with
//! the limits of the last full layout, so the offset is computed from real heights.
//! Rows outside the window get a temporary tree, which is dropped. Every answer that
//! changes geometry, the drift or the anchor marks the list dirty and blocks the next
//! redraw from the local path, and that block outlives any layout before it.

use super::geometry::Geometry;
use super::heights::Recorded;
use super::tasks::{Align, Answer, Request, Scroll};
use super::window::{Core, Landing};
use super::{Catalog, ListView, item_limits};

use std::any::Any;
use std::hash::Hash;

use iced::advanced::Widget;
use iced::advanced::layout::Limits;
use iced::advanced::widget::Tree;
use iced::advanced::widget::operation::Animation;

impl<T, K, W, Message, Theme, Renderer> ListView<'_, T, K, W, Message, Theme, Renderer>
where
    K: Clone + Eq + Hash + Send + 'static,
    W: Widget<Message, Theme, Renderer>,
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    /// Answers a task's request.
    pub(super) fn answer(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        request: Request,
    ) -> Answer {
        match request {
            Request::Scroll(scroll) => self.resolve(tree, renderer, scroll, false),
            Request::Settle {
                translation,
                offset,
            } => {
                let core = tree.state.downcast_mut::<Core<K>>();

                // A pending jump has not reached the scroller's translation yet, so the
                // translation this task saw is stale.
                if core.pending_landing().is_some() {
                    return Answer::Aligned;
                }

                let drift = core.geometry.drift();
                let top = f64::from(translation) - f64::from(offset);

                // Positive drift can only become offset while the offset stays
                // positive, and the list must stay away from its start, where rule Z
                // would remove the rest at once and move the content. What cannot be
                // removed stays until the list reaches its start.
                let removed = if drift > 0.0 {
                    drift.min((top - 1.0).max(0.0))
                } else {
                    drift
                };

                if removed == 0.0 {
                    return Answer::Aligned;
                }

                // Rule M: the content and the anchor's target move down by the drift,
                // and the offset by the same amount, so nothing moves on screen once
                // the scroller's translation catches up. Until then the anchor is a
                // landing, which the clamped intermediate passes must not re-pin.
                core.geometry.move_drift(-removed);
                core.geometry.block_next_redraw = true;

                if core.anchor.is_some() {
                    core.landing = Some(Landing::new(top - removed));
                }

                Answer::Offset {
                    offset: (f64::from(translation) - removed) as f32,
                    animation: Animation::Instant,
                }
            }
            Request::Focus { key, scroll } => {
                let core = tree.state.downcast_ref::<Core<K>>();

                let Some(position) = key_position(core, &*key) else {
                    return Answer::Unknown;
                };

                let materialized = core
                    .window
                    .binary_search_by_key(&position, |slot| slot.position)
                    .is_ok();

                match scroll {
                    _ if materialized => Answer::Materialized,
                    Some(scroll) => self.resolve(tree, renderer, scroll, true),
                    None => Answer::Unknown,
                }
            }
            Request::Defer { key, widget } => {
                let core = tree.state.downcast_mut::<Core<K>>();

                let Some(key) = key.downcast_ref::<K>() else {
                    return Answer::Unknown;
                };

                if core.model.position(key).is_none() {
                    return Answer::Unknown;
                }

                core.focus.deferred = Some((key.clone(), widget));
                core.geometry.block_next_redraw = true;

                Answer::Deferred
            }
        }
    }

    /// Resolves a scroll (section 4.5): measures the target and the rows between the
    /// viewport's top and the target's alignment point, then computes the offset. An
    /// instant jump also measures the rows below the target down to the viewport's
    /// bottom.
    ///
    /// For a focus, an item with no height is not scrolled to: it could not be laid
    /// out at the destination, and its focus would wait forever.
    fn resolve(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        scroll: Scroll,
        focus: bool,
    ) -> Answer {
        let Tree {
            state, children, ..
        } = tree;
        let core = state.downcast_mut::<Core<K>>();

        let (Some(local), Some(position)) = (core.geometry.local, key_position(core, &*scroll.key))
        else {
            return Answer::Unknown;
        };

        let limits = item_limits(local.width, &local.limits);
        let changed = self.measure(core, children, renderer, &limits, position);

        // The measurement is absorbed at once, which keeps the current view, so the
        // decisions below see the target where the next frame shows it. An answer
        // that does not scroll lays the list out again, as after any other change
        // outside a redraw.
        if changed {
            core.absorb();
        }

        let unscrolled = |core: &mut Core<K>, answer| {
            if changed {
                core.geometry.dirty = true;
                core.geometry.block_next_redraw = true;
            }

            answer
        };

        let height = f64::from(core.geometry.heights.height(position));

        // An item with no height, or a viewport with none, leaves nothing to lay the
        // item out in, so its focus would wait forever.
        if focus && (height == 0.0 || scroll.viewport <= 0.0) {
            return unscrolled(core, Answer::Unknown);
        }

        let viewport = f64::from(scroll.viewport);
        let list = f64::from(scroll.offset);

        // The current view in list coordinates. Comparing the translation directly
        // would ignore content above the list. After a jump that has not reached the
        // scroller yet, the view is the jump's destination.
        let top = match core.pending_landing() {
            Some(landing) => landing.top,
            None => f64::from(scroll.translation) - list,
        };
        let instant = scroll.animation == Animation::Instant;

        let fraction = match scroll.align {
            Align::Start => 0.0,
            Align::Center => 0.5,
            Align::End => 1.0,
            Align::Nearest => {
                let start = core.geometry.content_y(position);

                if start < top {
                    0.0
                } else if start + height > top + viewport {
                    1.0
                } else if instant {
                    // Still an instant scroll, to where the view is, so it stops a
                    // smooth scroll that a task started before.
                    return unscrolled(
                        core,
                        Answer::Offset {
                            offset: (top + list) as f32,
                            animation: Animation::Instant,
                        },
                    );
                } else {
                    return unscrolled(core, Answer::Aligned);
                }
            }
        };

        // The rows that will be between the viewport's top and the alignment point.
        let mut needed = fraction * (viewport - height);
        let mut row = position;

        while needed > 0.0 && row > 0 {
            row -= 1;
            self.measure(core, children, renderer, &limits, row);
            needed -=
                f64::from(core.geometry.heights.height(row)) + core.geometry.heights.spacing();
        }

        let destination = |geometry: &Geometry| {
            (list + geometry.content_y(position) + fraction * (height - viewport)).max(0.0)
        };

        let mut instant = instant;

        if !instant {
            // The current view must not move, so the measurements are absorbed against
            // the current anchor.
            core.absorb();

            // A smooth scroll clamps its target against the content height the scroller
            // knows, which does not include what was just measured. An instant one
            // stores the offset as it is.
            instant = destination(&core.geometry) > f64::from(scroll.content - scroll.viewport);
        }

        if instant {
            // The jump makes the drift unobservable, so it is folded, and the first row
            // visible at the destination is pinned for the frames until it is drawn.
            let drift = core.geometry.drift();
            core.geometry.move_drift(-drift);

            // Near the end, the rows below the target decide how far the scroller can
            // go. Measured down to the bottom of the destination, they give the parent
            // the content height to clamp against from the first layout on, so the
            // jump settles within the landing frame.
            let bottom = destination(&core.geometry) - list + viewport;
            let mut row = position + 1;

            while row < core.model.len() && core.geometry.content_y(row) < bottom {
                self.measure(core, children, renderer, &limits, row);
                row += 1;
            }

            core.pin_landing(destination(&core.geometry) - list);
        }

        core.geometry.dirty = true;
        core.geometry.block_next_redraw = true;

        Answer::Offset {
            offset: destination(&core.geometry) as f32,
            animation: if instant {
                Animation::Instant
            } else {
                scroll.animation
            },
        }
    }

    /// Lays out the row at `position` with the item limits and records its height. A
    /// row outside the window is built for it and dropped. Returns whether the
    /// indexed height changed.
    fn measure(
        &mut self,
        core: &mut Core<K>,
        children: &mut [Tree],
        renderer: &Renderer,
        limits: &Limits,
        position: usize,
    ) -> bool {
        let height = match core
            .window
            .binary_search_by_key(&position, |slot| slot.position)
        {
            Ok(index) => {
                let tree = &mut children[index];
                self.rows[index].layout(tree, renderer, limits);
                tree.size.height
            }
            Err(_) => {
                let mut row = (self.view)(position, &self.items[position]);
                let mut tree = Tree::new(&row);
                tree.diff(&mut row);
                row.layout(&mut tree, renderer, limits);
                tree.size.height
            }
        };

        let recorded = core
            .model
            .record(position, height, &mut core.geometry.heights);

        #[cfg(test)]
        core.resolved.push(position);

        recorded != Recorded::Unchanged
    }
}

/// The position of a key that came from a task, if it has the list's key type.
fn key_position<K: Clone + Eq + Hash + 'static>(core: &Core<K>, key: &dyn Any) -> Option<usize> {
    core.model.position(key.downcast_ref::<K>()?)
}
