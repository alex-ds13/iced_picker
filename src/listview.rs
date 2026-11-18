//! A virtualized vertical list for long sequences of items with variable heights.
//!
//! A [`ListView`] is content, like a `column`: the application puts it in its own
//! `scrollable`, with anything else above or below it. Only a window of items around
//! the visible region exists as widgets, built, laid out and drawn. Every other item
//! is a height in an index: measured once it has been shown, estimated before that.
//! The list never moves the scroll offset by itself. It keeps the visible items still
//! while items above them are measured, inserted or removed, by moving a spacer above
//! the first item (its drift).
//!
//! # When to use it
//!
//! Under a couple of thousand simple rows, a `column` in a `scrollable` is simpler:
//! it keeps every widget's state, every operation reaches every item, and iced
//! already skips drawing the rows outside the viewport. A list view pays off when the
//! sequence is long or its rows are expensive to build.
//!
//! # Example
//!
//! ```
//! use iced::widget::{Id, scrollable, text};
//! use iced::widget::operation::Animation;
//! use iced::{Fill, Task, Widget};
//! use iced_picker::listview::{self, Action, Align, Selection, list_view};
//!
//! const SCROLL: Id = Id::new("contacts-scroll");
//! const LIST: Id = Id::new("contacts");
//!
//! struct Contact {
//!     id: u64,
//!     name: String,
//! }
//!
//! struct App {
//!     contacts: Vec<Contact>,
//!     selection: Selection<u64>,
//! }
//!
//! #[derive(Debug, Clone)]
//! enum Message {
//!     Select(Action<u64>),
//!     Open(u64),
//! }
//!
//! impl App {
//!     fn view(&self) -> impl Widget<Message> + '_ {
//!         let list = list_view(&self.contacts, |_, contact| text(&contact.name))
//!             .key(|contact| contact.id)
//!             .id(LIST)
//!             .scroller(SCROLL)
//!             .selection(&self.selection, Message::Select)
//!             .multiple(true)
//!             .on_activate(Message::Open);
//!
//!         scrollable(list).id(SCROLL).height(Fill)
//!     }
//!
//!     fn update(&mut self, message: Message) -> Task<Message> {
//!         match message {
//!             Message::Select(action) => {
//!                 // Keyboard moves ask the application to bring the new cursor
//!                 // into view, because the list cannot scroll itself.
//!                 let reveal = action.reveal().copied();
//!                 self.selection.perform(action);
//!
//!                 if let Some(key) = reveal {
//!                     return listview::scroll_to(LIST, key, Align::Nearest, Animation::Instant);
//!                 }
//!             }
//!             Message::Open(_id) => {}
//!         }
//!
//!         Task::none()
//!     }
//! }
//! ```
//!
//! # Contracts and limitations
//!
//! * **Keys.** Without [`ListView::key`], an item is identified by its index:
//!   inserting or removing items above the viewport then shifts the visible
//!   content, and measured heights and widget state follow the index. With stable
//!   keys, edits above the first visible item move nothing on screen, except at
//!   the start of the list: there the list shows the sequence's current
//!   beginning, so items inserted at the top appear. When the item at the top of
//!   the viewport is removed, the next item that was on screen takes its place.
//!   When none of the items on screen remains, nothing is held: the scrollable
//!   shows what is now at its offset, clamped to the content. Duplicate keys are
//!   reachable by their first position only.
//! * **Filtering** is the application's: it passes the filtered sequence, and the
//!   list compares keys on every view to find what changed.
//! * **Width.** Items outside the window cannot report an intrinsic width, so a
//!   `Shrink` or `Fit` width takes the available width, like `Fill`.
//! * **The scroller.** [`scroll_to`], [`focus`] (for items outside the window) and
//!   [`settle`] move the scrollable named with [`ListView::scroller`]. It must be a
//!   vertical, start-anchored ancestor of the list with a unique id. Operations
//!   cannot tell a scrollable's direction or anchor, so this is not checked.
//! * **Exactness.** An instant [`scroll_to`] lands exactly, provided the items from
//!   the first one visible at the destination down to the target keep their height
//!   until the frame at the destination is drawn. A height change in the third update
//!   pass of a jump that changes how far the scrollable can go is drawn one frame
//!   late. A smooth one is approximate: items measured during the animation move the
//!   destination, and a destination beyond the content height the scrollable knows
//!   is reached instantly.
//! * **Widget state** of items outside the window is lost, like any widget removed
//!   from an iced tree. Operations by id reach the items in the window only. Use
//!   [`focus`] to focus a widget in an item outside it.
//! * **Drift.** Edits above the viewport, and items above it measured at another
//!   height than estimated, become drift: a spacer above the first item that keeps
//!   the visible items still. While there is drift, scrolling toward the start
//!   moves the items by more or less than the scroll (the drift shrinks with the
//!   distance to the start), and at the start it is gone. [`settle`] turns it into
//!   an offset change instead. Drift never shows as a gap: when the spacer would
//!   reach into the viewport, the items move up to the viewport's top, or to their
//!   natural position when they are too few to fill it.
//! * **Estimates.** Unmeasured items count with [`ListView::estimated_height`], at
//!   least 1 px. Without it, the estimate is the median of the heights of the items
//!   measured so far (those of 0 px left out, and items measured by [`scroll_to`]
//!   or [`focus`] included), the lower middle one for an even count, taken once at
//!   the end of the first layout that measured a positive height. 32 px is used
//!   before that. The median reduces the effect of an unusual item, such as an
//!   expanded first row, when several typical items are measured, but with few of
//!   them (for example with `.anticipate(0)` and a tall first item) the unusual one
//!   can still set it. An explicit estimate avoids that. The adoption is absorbed like any
//!   measurement, so nothing visible moves, and the estimate then stays. A jump
//!   into unmeasured content whose estimate is much larger than the real height can
//!   build and discard extra items in that frame.
//! * **Numeric range.** Heights and spacing are kept on a 1/256 px grid. A single
//!   item's height, the spacing and an explicit estimate are clamped to
//!   4,194,304 px, so a taller item cannot be scrolled through entirely. A clamped
//!   item measurement logs a warning once per list.
//!   Positions beyond about 8 million pixels lose sub-pixel precision (`f32`
//!   layout).
//! * **Layout.** Rows that change their own geometry on a redraw must ask for a
//!   layout. A window shift on a redraw lays out only the list when its last full
//!   layout's constraints still hold, no task, jump, waiting focus or drift
//!   correction is in progress, no widget asked for a layout earlier in that update,
//!   and the shift leaves every measured height, the drift, the estimate and the
//!   list's size unchanged. Otherwise the list asks for a layout of the whole
//!   application, which can take more than one update pass to settle, and inside a
//!   `Component` the component lays out its whole subtree after any change.
//! * **Selection.** It is the application's [`Selection`]. Keys act only while the
//!   list itself has the focus. A press that never reaches the list, because an
//!   overlay or a `Component` around an earlier widget captured it, cannot cancel a
//!   waiting [`focus`] or unfocus the list, as for iced's text inputs. Lists that
//!   share a selection see each other's actions only once the application handled
//!   them.
//! * **Groups.** An item with sub-items is one item, `column![sticky(header),
//!   sub_items..]`, whose header sticks while its sub-items scroll by. A stuck
//!   header is an overlay: give it its own button for the group.

mod geometry;
mod heights;
mod local;
mod model;
mod resolve;
mod selection;
mod style;
mod tasks;
mod widget;
mod window;

// The headless tests use iced's null renderer, which exists in debug builds only.
#[cfg(all(test, debug_assertions))]
mod tests;

pub use selection::{Action, Change, Selection};
pub use style::{Catalog, Status, Style, StyleFn, default};
pub use tasks::{Align, focus, scroll_to, settle};
pub use widget::{ListView, list_view};

use widget::item_limits;
