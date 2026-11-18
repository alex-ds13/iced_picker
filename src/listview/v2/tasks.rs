//! Tasks that move the offset of the scrollable a list lives in, and focus a widget
//! inside one of its items (sections 4.5 to 4.7 of the design).
//!
//! The list never moves the offset itself. A task reaches the list and its scroller
//! through operations: the first traversal records the named scrollables it passes
//! and asks the list, through a `custom` call, to resolve the request. The list
//! measures the destination inside its `operate`, where it has its tree and the
//! renderer, and answers through a second `custom` call. A chained operation then
//! scrolls the scroller (`Outcome::Chain`, run at once by the shell). No request or
//! answer outlives the list's `operate`.
//!
//! Contract: the scroller named with [`ListView::scroller`](super::ListView::scroller)
//! has a unique id, is vertical and start-anchored, and contains the list. Operation
//! callbacks expose neither the direction nor the anchor of a scrollable, so this is
//! not checked beyond the scroller being reported before the list.

use std::any::Any;

use iced::Task;
use iced::advanced::widget::operation::focusable;
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, Scrollable};
use iced::advanced::widget::operation::{Animation, Operation, Outcome};
use iced::advanced::widget::{Id, operate};
use iced::{Rectangle, Size, Vector};

/// Where [`scroll_to`] puts the item in the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Its top at the top of the viewport.
    Start,
    /// Its middle at the middle of the viewport.
    Center,
    /// Its bottom at the bottom of the viewport.
    End,
    /// [`Align::Start`] when it starts above the viewport, [`Align::End`] when it
    /// ends below it, and no scroll when it is in view. In view, an instant request
    /// still stops a smooth scroll an earlier task started.
    Nearest,
}

/// Scrolls the scroller of the list with id `list` so the item with `key` is
/// aligned as asked.
///
/// `key` must have exactly the list's key type: `usize` for a list without
/// [`ListView::key`](super::ListView::key). A key of another type is an unknown key.
///
/// The destination is measured before the offset is computed, so
/// [`Animation::Instant`] lands exactly, provided the items from the first one
/// visible at the destination down to the target keep their height until that
/// frame is drawn. [`Animation::Smooth`] and [`Animation::Auto`] are approximate:
/// items measured during the animation move the destination. A destination beyond
/// the content height the scroller knows is reached instantly.
///
/// Nothing happens if the list, the key or the list's scroller is not found, or if
/// the scroller is not reported before the list. A jump past the content height the
/// scroller last saw can take a third update pass of the landing frame. A height
/// change in that third pass, after the scroller clamped it, is drawn one frame
/// late when it changes how far the scroller can go.
pub fn scroll_to<M, K>(list: impl Into<Id>, key: K, align: Align, animation: Animation) -> Task<M>
where
    M: Send + 'static,
    K: Clone + Send + 'static,
{
    operate(ScrollTo::new(list.into(), key, align, animation))
}

/// Focuses the widget with id `widget` inside the item with `key` of the list with
/// id `list`.
///
/// `key` must have exactly the list's key type: `usize` for a list without
/// [`ListView::key`](super::ListView::key). A key of another type is an unknown key.
///
/// A materialized item is focused at once, without a scroller. Otherwise the item is
/// scrolled into view first and focused on the first redraw it is laid out in,
/// unless something interrupts it. Until then the list holds the focus for it.
///
/// It is interrupted by a later focus change, by the removal of its key, by a key a
/// selectable list handles, and by any press the list receives: outside the list or
/// taken by a widget, the press also unfocuses the list, and inside it only cancels
/// the waiting focus, because the user acted before it landed. A press that never
/// reaches the list, because a capturing overlay or a `Component` around an earlier
/// widget stopped it, cannot cancel it. iced's own text inputs have the same limit:
/// they unfocus themselves only when they receive a press outside them.
///
/// Focusing an item off the window does nothing to the focus, or to what is on
/// screen, when the item has no height or the scroller's viewport has none: there is
/// nothing to lay it out in. Measuring the item to find that out is still recorded,
/// and the next redraw lays out the list again if it changed. Nothing changes either
/// if the list or the key is not found, or if a needed scroller is not. An item's
/// widget state is lost when it leaves the window.
///
/// A widget in an overlay of a materialized item, like a stuck `Sticky` header, is
/// focused through the overlays. A waiting focus commits in the item's own widgets
/// only: if its widget is in an overlay by then, the list keeps the focus and
/// nothing waits any more.
pub fn focus<M, K>(list: impl Into<Id>, key: K, widget: impl Into<Id>) -> Task<M>
where
    M: Send + 'static,
    K: Clone + Send + 'static,
{
    operate(FocusItem::new(list.into(), key, widget.into()))
}

/// Moves the scroller's offset instead of the content: the drift a list keeps above
/// its visible items after large edits is removed, and the offset changes by the
/// same amount, so nothing moves on screen.
///
/// Positive drift, a gap above the first item, is removed only as far as the offset
/// can shrink while the list stays away from its start. The rest stays until the
/// list reaches its start, where it is removed as usual. Until a frame at the new
/// offset is drawn, another `settle` does nothing, and a later [`scroll_to`] or
/// [`focus`] measures from the new offset.
pub fn settle<M>(list: impl Into<Id>) -> Task<M>
where
    M: Send + 'static,
{
    operate(Settle::new(list.into()))
}

/// What a task and a list exchange through the list's `custom` calls. Keys travel
/// as `Box<dyn Any>`, so tasks without a key type, like [`settle`], use the same
/// exchange, and a key of the wrong type is an unknown key.
pub(super) struct Exchange {
    /// The list's scroller id, for the task to find in what it recorded.
    pub(super) scroller: Option<Id>,
    pub(super) request: Option<Request>,
    pub(super) answer: Option<Answer>,
}

impl Exchange {
    pub(super) fn new(scroller: Option<Id>) -> Self {
        Self {
            scroller,
            request: None,
            answer: None,
        }
    }
}

pub(super) enum Request {
    Scroll(Scroll),
    /// Remove the drift. `translation` is the scroller's.
    Settle {
        translation: f32,
        /// The list's top in the scroller's content.
        offset: f32,
    },
    /// Focus inside the item, scrolling with `scroll` if it is not materialized.
    Focus {
        key: Box<dyn Any>,
        scroll: Option<Scroll>,
    },
    /// Focus `widget` inside the item once it is laid out.
    Defer {
        key: Box<dyn Any>,
        widget: Id,
    },
}

/// A scroll request, with what the task saw of the scroller.
pub(super) struct Scroll {
    pub(super) key: Box<dyn Any>,
    pub(super) align: Align,
    pub(super) animation: Animation,
    /// The scroller's viewport height.
    pub(super) viewport: f32,
    /// The scroller's vertical translation.
    pub(super) translation: f32,
    /// The list's top in the scroller's content.
    pub(super) offset: f32,
    /// The content height the scroller knows.
    pub(super) content: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Answer {
    /// The key or the list's layout is unknown: nothing happens.
    Unknown,
    /// The item is already where it should be.
    Aligned,
    /// The item is materialized.
    Materialized,
    /// Scroll the scroller to this offset.
    Offset { offset: f32, animation: Animation },
    /// The focus waits for the item to be laid out.
    Deferred,
}

/// A named scrollable seen by a task's first traversal.
#[derive(Debug, Clone)]
struct Scroller {
    id: Id,
    bounds: Rectangle,
    content: Size,
    translation: Vector,
}

/// The named scrollables of a traversal, in the order they were reported.
#[derive(Debug, Default)]
struct Scrollers(Vec<Scroller>);

impl Scrollers {
    fn record(&mut self, id: Option<&Id>, bounds: Rectangle, content: Size, translation: Vector) {
        if let Some(id) = id {
            self.0.push(Scroller {
                id: id.clone(),
                bounds,
                content,
                translation,
            });
        }
    }

    /// The scrollable named `id`, if it was reported before the list.
    fn find(&self, id: Option<&Id>) -> Option<&Scroller> {
        let id = id?;
        self.0.iter().rev().find(|scroller| &scroller.id == id)
    }
}

/// The exchange offered by the list with id `list`, if `id` and `state` are it.
fn exchange<'a>(list: &Id, id: Option<&Id>, state: &'a mut dyn Any) -> Option<&'a mut Exchange> {
    (id == Some(list))
        .then(|| state.downcast_mut::<Exchange>())
        .flatten()
}

fn scroll<K: 'static>(
    key: K,
    align: Align,
    animation: Animation,
    scroller: &Scroller,
    list: Rectangle,
) -> Scroll {
    Scroll {
        key: Box::new(key),
        align,
        animation,
        viewport: scroller.bounds.height,
        translation: scroller.translation.y,
        offset: list.y - scroller.bounds.y,
        content: scroller.content.height,
    }
}

/// The first traversal of [`scroll_to`].
pub(super) struct ScrollTo<K> {
    list: Id,
    key: K,
    align: Align,
    animation: Animation,
    scrollers: Scrollers,
    target: Option<Id>,
    answer: Option<Answer>,
}

impl<K> ScrollTo<K> {
    pub(super) fn new(list: Id, key: K, align: Align, animation: Animation) -> Self {
        Self {
            list,
            key,
            align,
            animation,
            scrollers: Scrollers::default(),
            target: None,
            answer: None,
        }
    }
}

impl<K, T> Operation<T> for ScrollTo<K>
where
    K: Clone + Send + 'static,
{
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Size,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        self.scrollers.record(id, bounds, content, translation);
    }

    fn custom(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Any) {
        let Some(exchange) = exchange(&self.list, id, state) else {
            return;
        };

        if let Some(answer) = exchange.answer.take() {
            self.answer = Some(answer);
            return;
        }

        if self.target.is_some() {
            return;
        }

        if let Some(scroller) = self.scrollers.find(exchange.scroller.as_ref()) {
            let request = scroll(
                self.key.clone(),
                self.align,
                self.animation,
                scroller,
                bounds,
            );
            exchange.request = Some(Request::Scroll(request));
            self.target = Some(scroller.id.clone());
        }
    }

    fn finish(&self) -> Outcome<T> {
        match (self.answer, &self.target) {
            (Some(Answer::Offset { offset, animation }), Some(scroller)) => {
                Outcome::Chain(Box::new(Land::<K> {
                    scroll: Some((scroller.clone(), offset, animation)),
                    defer: None,
                }))
            }
            _ => Outcome::None,
        }
    }
}

/// The first traversal of [`settle`].
pub(super) struct Settle {
    list: Id,
    scrollers: Scrollers,
    target: Option<Id>,
    answer: Option<Answer>,
}

impl Settle {
    pub(super) fn new(list: Id) -> Self {
        Self {
            list,
            scrollers: Scrollers::default(),
            target: None,
            answer: None,
        }
    }
}

impl<T> Operation<T> for Settle {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Size,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        self.scrollers.record(id, bounds, content, translation);
    }

    fn custom(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Any) {
        let Some(exchange) = exchange(&self.list, id, state) else {
            return;
        };

        if let Some(answer) = exchange.answer.take() {
            self.answer = Some(answer);
            return;
        }

        if self.target.is_some() {
            return;
        }

        if let Some(scroller) = self.scrollers.find(exchange.scroller.as_ref()) {
            exchange.request = Some(Request::Settle {
                translation: scroller.translation.y,
                offset: bounds.y - scroller.bounds.y,
            });
            self.target = Some(scroller.id.clone());
        }
    }

    fn finish(&self) -> Outcome<T> {
        match (self.answer, &self.target) {
            (Some(Answer::Offset { offset, animation }), Some(scroller)) => {
                Outcome::Chain(Box::new(Land::<()> {
                    scroll: Some((scroller.clone(), offset, animation)),
                    defer: None,
                }))
            }
            _ => Outcome::None,
        }
    }
}

/// The first traversal of [`focus`].
pub(super) struct FocusItem<K> {
    list: Id,
    key: K,
    widget: Id,
    scrollers: Scrollers,
    target: Option<Id>,
    asked: bool,
    answer: Option<Answer>,
}

impl<K> FocusItem<K> {
    pub(super) fn new(list: Id, key: K, widget: Id) -> Self {
        Self {
            list,
            key,
            widget,
            scrollers: Scrollers::default(),
            target: None,
            asked: false,
            answer: None,
        }
    }
}

impl<K, T> Operation<T> for FocusItem<K>
where
    K: Clone + Send + 'static,
    T: 'static,
{
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Size,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        self.scrollers.record(id, bounds, content, translation);
    }

    fn custom(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Any) {
        let Some(exchange) = exchange(&self.list, id, state) else {
            return;
        };

        if let Some(answer) = exchange.answer.take() {
            self.answer = Some(answer);
            return;
        }

        if self.asked {
            return;
        }

        self.asked = true;

        // Without a scroller only a materialized item can be focused.
        let scroller = self.scrollers.find(exchange.scroller.as_ref());
        let request = scroller.map(|scroller| {
            scroll(
                self.key.clone(),
                Align::Nearest,
                Animation::Instant,
                scroller,
                bounds,
            )
        });

        self.target = scroller.map(|scroller| scroller.id.clone());
        exchange.request = Some(Request::Focus {
            key: Box::new(self.key.clone()),
            scroll: request,
        });
    }

    fn finish(&self) -> Outcome<T> {
        let defer = Some((self.list.clone(), self.key.clone(), self.widget.clone()));

        match self.answer {
            Some(Answer::Materialized) => {
                Outcome::Chain(Box::new(focusable::focus(self.widget.clone())))
            }
            Some(Answer::Offset { offset, animation }) => Outcome::Chain(Box::new(Land {
                scroll: self
                    .target
                    .clone()
                    .map(|scroller| (scroller, offset, animation)),
                defer,
            })),
            Some(Answer::Aligned) => Outcome::Chain(Box::new(Land {
                scroll: None,
                defer,
            })),
            _ => Outcome::None,
        }
    }
}

/// The chained traversal: scrolls the scroller, and for a deferred focus first
/// unfocuses every focusable widget (the list's `focusable` call comes before its
/// `custom` call, so an older deferred focus is cancelled first) and then leaves the
/// focus with the list.
struct Land<K> {
    scroll: Option<(Id, f32, Animation)>,
    defer: Option<(Id, K, Id)>,
}

impl<K, T> Operation<T> for Land<K>
where
    K: Clone + Send + 'static,
{
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    /// The list takes the focus for the item and every other widget loses it.
    fn focusable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        state: &mut dyn focusable::Focusable,
    ) {
        if let Some((list, _, _)) = &self.defer {
            if id == Some(list) {
                state.focus();
            } else {
                state.unfocus();
            }
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Size,
        _translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        if let Some((scroller, offset, animation)) = &self.scroll
            && id == Some(scroller)
        {
            // An instant scroll stores the offset unclamped, so it is reached once
            // layout catches up with what the list measured.
            state.scroll_to(
                AbsoluteOffset {
                    x: None,
                    y: Some(*offset),
                },
                *animation,
                bounds,
                content,
            );
        }
    }

    fn custom(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Any) {
        let Some((list, key, widget)) = &self.defer else {
            return;
        };

        if let Some(exchange) = exchange(list, id, state)
            && exchange.answer.is_none()
        {
            exchange.request = Some(Request::Defer {
                key: Box::new(key.clone()),
                widget: widget.clone(),
            });
        }
    }
}
