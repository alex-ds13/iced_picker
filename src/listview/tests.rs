//! Headless tests of the candidate widget with iced's null renderer.
//!
//! [`Harness`] plays the runtime and the shell: every redraw gets up to three update
//! passes with the same timestamp, a Layout invalidation lays out the root and
//! rebuilds overlays inside its pass, and the frame is drawn after the last pass even
//! if it invalidated, and a Widgets invalidation rebuilds the view between passes.
//! Lists live in `scrollable(column![header, list, footer])`. "No root layout" is
//! what the shell reports: no pass with a Layout invalidation.

use super::geometry::Visible;
use super::heights::Estimate;
use super::tasks::{FocusItem, ScrollTo, Settle};
use super::window::{Anchor, Core, Walk};
use super::{Action, Align, Change, Selection, Status, Style, list_view};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay::Group;
use iced::advanced::renderer;
use iced::advanced::shell::{Bus, Diff, Invalidation, Tracking, Waker};
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, Scrollable, scroll_to};
use iced::advanced::widget::operation::{Animation, Focusable, Operation, Outcome, focusable};
use iced::advanced::widget::{Id, Meta, Tree, tree};
use iced::advanced::{Shell, Widget};
use iced::time::Instant;
use iced::widget::{
    Component, Row, button, column, component, scrollable, sensor, space, sticky, text,
};
use iced::{
    Color, Element, Event, Fill, Length, Padding, Point, Rectangle, Size, Theme, Vector,
    Widget as _, mouse, touch,
};
use iced_core::keyboard::key::{Named, NativeCode, Physical};
use iced_core::keyboard::{self, Key, Location, Modifiers};
use iced_core::window::{self, RedrawRequest};

type Root<'a> = Element<'a, (), Theme, ()>;

const SCROLL: Id = Id::new("list-scroll");
const LIST: Id = Id::new("list");
const WINDOW: Size = Size::new(800.0, 600.0);
const FOOTER: f32 = 50.0;
const TEXT: Color = Color::BLACK;
const RED: Color = Color::from_rgb(1.0, 0.0, 0.0);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Height {
    Fixed(f32),
    /// `Fill` height with this intrinsic height.
    Fill(f32),
    /// This area divided by the width.
    Area(f32),
    /// `column![sticky(header), items..]`. The header's key is the item's key, and
    /// item `i` (from 1) has key `key * 1000 + i`.
    Group {
        header: f32,
        items: u32,
        item: f32,
    },
    /// A probe inside a `button` that publishes on release.
    Button(f32),
    /// `column![sticky(header), items..]` with a real header: text, and a 50 px
    /// button on the right that publishes. Items as in [`Height::Group`].
    Header {
        header: f32,
        items: u32,
        item: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Item {
    key: u32,
    height: Height,
}

fn rows(keys: Range<u32>, height: f32) -> Vec<Item> {
    keys.map(|key| Item {
        key,
        height: Height::Fixed(height),
    })
    .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Drawn {
    key: u32,
    bounds: Rectangle,
    viewport: Rectangle,
    text_color: Color,
}

/// What the probe rows saw.
#[derive(Default)]
struct Log {
    serial: Cell<u64>,
    /// Keys whose row got a new tree, in order.
    created: RefCell<Vec<u32>>,
    layouts: RefCell<Vec<(u32, layout::Limits)>>,
    /// Keys that received an event, and whether it was a redraw.
    updates: RefCell<Vec<(u32, bool)>>,
    /// The rows drawn in the last frame.
    drawn: RefCell<Vec<Drawn>>,
    operated: RefCell<Vec<(u32, Rectangle)>>,
    overlays: RefCell<Vec<(u32, Rectangle)>>,
    statuses: RefCell<Vec<Status>>,
    key_calls: Cell<usize>,
    /// Every redraw a row received: its key, its layout's parent and its bounds.
    redraws: RefCell<Vec<(u32, Option<Rectangle>, Rectangle)>>,
    /// Rows that take this height on their next redraw and ask for a root layout.
    grow: RefCell<HashMap<u32, f32>>,
    /// Heights rows took in a redraw, over the item's height.
    overrides: RefCell<HashMap<u32, f32>>,
    /// Layouts of the header and footer.
    sibling_layouts: Cell<usize>,
    /// The footer's bounds in the last draw.
    footer: Cell<Option<Rectangle>>,
    /// Invalidations the header (0) and footer (1) request on their next redraws.
    inject: RefCell<[VecDeque<Option<Invalidation>>; 2]>,
    /// The header captures every press.
    capture_press: Cell<bool>,
    /// Rows that publish a message on their next redraw.
    publish: RefCell<HashSet<u32>>,
    /// Heights the next message handled by `Harness::frame_app` applies.
    on_message: RefCell<Vec<(u32, f32)>>,
    /// Rows wrapped in a sensor that publishes a message when they are shown.
    sensors: RefCell<HashSet<u32>>,
    /// What the footer does on its next redraws: give a row a height and ask for a
    /// layout, or nothing.
    footer_changes: RefCell<VecDeque<Option<(u32, f32)>>>,
    /// The number of the current update pass of a frame, from 1.
    pass: Cell<usize>,
    /// The passes in which a row grew or the footer changed a row.
    changed_in: RefCell<Vec<usize>>,
    /// The selection actions the list emitted.
    actions: RefCell<Vec<Action<u32>>>,
    /// The keys the list activated.
    activated: RefCell<Vec<u32>>,
}

impl Log {
    fn clear(&self) {
        self.created.take();
        self.layouts.take();
        self.updates.take();
        self.operated.take();
        self.overlays.take();
        self.statuses.take();
        self.key_calls.take();
        self.redraws.take();
    }

    fn laid_out(&self) -> Vec<u32> {
        self.layouts.borrow().iter().map(|(key, _)| *key).collect()
    }

    fn drawn_keys(&self) -> Vec<u32> {
        self.drawn.borrow().iter().map(|drawn| drawn.key).collect()
    }

    /// Screen y of each drawn row (the scrollable sits at the top of the window).
    fn screen(&self) -> HashMap<u32, f32> {
        self.drawn
            .borrow()
            .iter()
            .map(|drawn| (drawn.key, drawn.bounds.y - drawn.viewport.y))
            .collect()
    }

    /// Asserts that the drawn rows cover `from..to`, in content coordinates.
    fn assert_covered(&self, from: f32, to: f32) {
        self.assert_covered_within(from, to, 1.0e-3, "");
    }

    /// Like [`Self::assert_covered`], allowing gaps up to `slack` between rows, which
    /// is how far `f32` rounds row positions at large offsets.
    fn assert_covered_within(&self, from: f32, to: f32, slack: f32, case: &str) {
        let mut drawn = self.drawn.borrow().clone();
        drawn.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y));

        let mut covered = from;

        for row in &drawn {
            assert!(
                row.bounds.y <= covered + slack,
                "{case}: gap at {covered} before row {} at {}",
                row.key,
                row.bounds.y
            );
            covered = covered.max(row.bounds.y + row.bounds.height);
        }

        assert!(
            covered >= to,
            "{case}: drawn rows end at {covered}, before {to}"
        );
    }
}

/// The tree state of a probe row, to follow tree identity.
struct RowState {
    key: u32,
    serial: u64,
    focused: bool,
}

impl Focusable for RowState {
    fn is_focused(&self) -> bool {
        self.focused
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }
}

/// A focusable widget outside the list: the header.
const OUTSIDE: Id = Id::new("outside");

/// The id of the focusable field of the probe with `key`.
fn field(key: u32) -> Id {
    Id::from(format!("field-{key}"))
}

/// An instrumented row.
struct Probe {
    item: Item,
    log: Rc<Log>,
}

impl Meta for Probe {}

impl Widget<(), Theme, ()> for Probe {
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: match self.item.height {
                Height::Fill(_) => Length::Fill,
                Height::Fixed(_)
                | Height::Area(_)
                | Height::Group { .. }
                | Height::Button(_)
                | Height::Header { .. } => Length::Shrink,
            },
        }
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<RowState>()
    }

    fn state(&self) -> tree::State {
        let serial = self.log.serial.get() + 1;
        self.log.serial.set(serial);
        self.log.created.borrow_mut().push(self.item.key);

        tree::State::new(RowState {
            key: self.item.key,
            serial,
            focused: false,
        })
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &(), limits: &layout::Limits) {
        self.log.layouts.borrow_mut().push((self.item.key, *limits));

        let grown = self.log.overrides.borrow().get(&self.item.key).copied();
        let intrinsic = match self.item.height {
            _ if let Some(height) = grown => height,
            Height::Fixed(height) | Height::Fill(height) => height,
            Height::Area(area) => area / limits.max.width,
            Height::Group { .. } | Height::Button(_) | Height::Header { .. } => {
                unreachable!("not a probe")
            }
        };
        let size = self.size();

        tree.size = limits.resolve(size.width, size.height, Size::new(0.0, intrinsic));
    }

    fn draw(
        &self,
        tree: &Tree,
        _renderer: &mut (),
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        assert_eq!(tree.state.downcast_ref::<RowState>().key, self.item.key);

        self.log.drawn.borrow_mut().push(Drawn {
            key: self.item.key,
            bounds: layout.bounds(),
            viewport: *viewport,
            text_color: style.text_color,
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        _cursor: mouse::Cursor,
        _renderer: &(),
        shell: &mut Shell<'_, ()>,
        _viewport: &Rectangle,
    ) {
        assert_eq!(tree.state.downcast_ref::<RowState>().key, self.item.key);

        let redraw = matches!(event, Event::Window(window::Event::RedrawRequested(_)));
        self.log.updates.borrow_mut().push((self.item.key, redraw));

        if redraw {
            self.log
                .redraws
                .borrow_mut()
                .push((self.item.key, layout.parent(), layout.bounds()));

            let grown = self.log.grow.borrow_mut().remove(&self.item.key);

            if let Some(height) = grown {
                self.log
                    .overrides
                    .borrow_mut()
                    .insert(self.item.key, height);
                self.log.changed_in.borrow_mut().push(self.log.pass.get());
                shell.invalidate_layout();
            }

            if self.log.publish.borrow_mut().remove(&self.item.key) {
                shell.publish(());
            }
        }
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &(),
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        _viewport: &Rectangle,
        _renderer: &(),
        operation: &mut dyn Operation,
    ) {
        self.log
            .operated
            .borrow_mut()
            .push((self.item.key, layout.bounds()));

        let state = tree.state.downcast_mut::<RowState>();
        operation.focusable(Some(&field(self.item.key)), layout.bounds(), state);
    }

    fn overlay<'b>(
        &'b mut self,
        _tree: &'b mut Tree,
        layout: Layout,
        _renderer: &(),
        _viewport: &Rectangle,
        _translation: Vector,
        _window: Size,
    ) -> Vec<iced::advanced::overlay::Element<'b, (), Theme, ()>> {
        self.log
            .overlays
            .borrow_mut()
            .push((self.item.key, layout.bounds()));

        Vec::new()
    }
}

/// The header or the footer, which counts its layouts and requests the
/// invalidations queued for it on its redraws.
struct Sibling {
    /// 0 for the header, 1 for the footer.
    side: usize,
    height: f32,
    log: Rc<Log>,
}

impl Meta for Sibling {}

/// The focus of the header, which is the [`OUTSIDE`] widget.
#[derive(Default)]
struct Outside(bool);

impl Focusable for Outside {
    fn is_focused(&self) -> bool {
        self.0
    }

    fn focus(&mut self) {
        self.0 = true;
    }

    fn unfocus(&mut self) {
        self.0 = false;
    }
}

impl Widget<(), Theme, ()> for Sibling {
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fixed(self.height),
        }
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Outside>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Outside::default())
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        _viewport: &Rectangle,
        _renderer: &(),
        operation: &mut dyn Operation,
    ) {
        if self.side == 0 {
            let state = tree.state.downcast_mut::<Outside>();
            operation.focusable(Some(&OUTSIDE), layout.bounds(), state);
        }
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &(), limits: &layout::Limits) {
        self.log
            .sibling_layouts
            .set(self.log.sibling_layouts.get() + 1);
        let size = self.size();
        tree.size = limits.resolve(size.width, size.height, Size::ZERO);
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut (),
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        if self.side == 1 {
            self.log.footer.set(Some(layout.bounds()));
        }
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        _layout: Layout,
        _cursor: mouse::Cursor,
        _renderer: &(),
        shell: &mut Shell<'_, ()>,
        _viewport: &Rectangle,
    ) {
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            let injected = self.log.inject.borrow_mut()[self.side].pop_front();

            if let Some(Some(invalidation)) = injected {
                shell.invalidate(invalidation);
            }

            let change = if self.side == 1 {
                self.log.footer_changes.borrow_mut().pop_front()
            } else {
                None
            };

            if let Some(Some((key, height))) = change {
                self.log.overrides.borrow_mut().insert(key, height);
                self.log.changed_in.borrow_mut().push(self.log.pass.get());
                shell.invalidate_layout();
            }
        }

        if self.side == 0
            && self.log.capture_press.get()
            && let Event::Mouse(mouse::Event::ButtonPressed(_)) = event
        {
            shell.capture_event();
        }
    }
}

/// A probe, or for a group a column of a sticky header probe and item probes.
fn row(item: &Item, log: &Rc<Log>) -> Root<'static> {
    let probe = |key, height| Probe {
        item: Item {
            key,
            height: Height::Fixed(height),
        },
        log: Rc::clone(log),
    };

    match item.height {
        Height::Group {
            header,
            items,
            item: height,
        } => column(
            std::iter::once(sticky(probe(item.key, header)).boxed())
                .chain((1..=items).map(|index| probe(item.key * 1000 + index, height).boxed())),
        )
        .boxed(),
        Height::Button(height) => button(probe(item.key, height)).on_press(()).boxed(),
        Height::Header {
            header,
            items,
            item: height,
        } => {
            let header = Row::with_children([
                text("group").width(Fill).boxed(),
                button(space()).width(50).height(Fill).on_press(()).boxed(),
            ])
            .height(header);

            column(
                std::iter::once(sticky(header).boxed())
                    .chain((1..=items).map(|index| probe(item.key * 1000 + index, height).boxed())),
            )
            .boxed()
        }
        _ if log.sensors.borrow().contains(&item.key) => sensor(Probe {
            item: *item,
            log: Rc::clone(log),
        })
        .on_show(|_| ())
        .boxed(),
        _ => Probe {
            item: *item,
            log: Rc::clone(log),
        }
        .boxed(),
    }
}

/// The application of most tests: a list of probes between a header and a footer.
struct App {
    items: Vec<Item>,
    log: Rc<Log>,
    header: f32,
    scroller: bool,
    /// The list is the scrollable's whole content: no column, header or footer.
    direct: bool,
    /// The list's `scroller`.
    scroller_id: Option<Id>,
    height: Length,
    estimate: Option<f32>,
    anticipate: usize,
    spacing: f32,
    padding: Padding,
    highlight: Vec<u32>,
    text_color: Option<Color>,
    /// The application's selection, cloned into each view.
    selection: Option<Rc<RefCell<Selection<u32>>>>,
    multiple: bool,
    activate: bool,
}

impl App {
    fn new(items: Vec<Item>) -> Self {
        Self {
            items,
            log: Rc::default(),
            header: 0.0,
            scroller: true,
            direct: false,
            scroller_id: Some(SCROLL),
            height: Fill,
            estimate: None,
            anticipate: 4,
            spacing: 0.0,
            padding: Padding::ZERO,
            highlight: Vec::new(),
            text_color: None,
            selection: None,
            multiple: false,
            activate: false,
        }
    }

    fn estimated(items: Vec<Item>, estimate: f32) -> Self {
        Self {
            estimate: Some(estimate),
            ..Self::new(items)
        }
    }

    fn view(&self) -> Root<'_> {
        let list = self.list();

        let header = Sibling {
            side: 0,
            height: self.header,
            log: Rc::clone(&self.log),
        };
        let footer = Sibling {
            side: 1,
            height: FOOTER,
            log: Rc::clone(&self.log),
        };

        if self.direct {
            return scrollable(list)
                .id(SCROLL)
                .width(Fill)
                .height(self.height)
                .boxed();
        }

        let content = column![header, list, footer].width(Fill);

        if self.scroller {
            scrollable(content)
                .id(SCROLL)
                .width(Fill)
                .height(self.height)
                .boxed()
        } else {
            content.boxed()
        }
    }

    /// The list alone, for views built by a test.
    fn list(&self) -> Root<'_> {
        let log = Rc::clone(&self.log);
        let calls = Rc::clone(&self.log);

        let mut list = list_view(&self.items, move |_, item: &Item| row(item, &log))
            .key(move |item: &Item| {
                calls.key_calls.set(calls.key_calls.get() + 1);
                item.key
            })
            .id(LIST)
            .anticipate(self.anticipate)
            .spacing(self.spacing)
            .padding(self.padding)
            .highlight(self.highlight.iter().copied());

        if let Some(estimate) = self.estimate {
            list = list.estimated_height(estimate);
        }

        if let Some(color) = self.text_color {
            let log = Rc::clone(&self.log);

            list = list.style(move |_theme, status| {
                log.statuses.borrow_mut().push(status);

                Style {
                    text_color: Some(color),
                    ..Style::default()
                }
            });
        }

        if let Some(scroller) = &self.scroller_id {
            list = list.scroller(scroller.clone());
        }

        // The tests change the selection between views, so each view gets a copy.
        if let Some(selection) = &self.selection {
            let log = Rc::clone(&self.log);

            list = list
                .selection(selection.borrow().clone(), move |action| {
                    log.actions.borrow_mut().push(action)
                })
                .multiple(self.multiple);

            if self.activate {
                let log = Rc::clone(&self.log);
                list = list.on_activate(move |key| log.activated.borrow_mut().push(key));
            }
        }

        list.boxed()
    }

    /// Makes the list selectable, in multiple selection or not.
    fn selectable(mut self, multiple: bool) -> Self {
        self.selection = Some(Rc::default());
        self.multiple = multiple;
        self
    }

    /// The application's update for the list's messages: applies the logged actions
    /// to the selection, and returns the keys they ask to reveal.
    fn apply(&self) -> Vec<u32> {
        let selection = self.selection.as_ref().expect("a selectable list");

        self.log
            .actions
            .take()
            .into_iter()
            .filter_map(|action| {
                let reveal = action.reveal().copied();
                selection.borrow_mut().perform(action);
                reveal
            })
            .collect()
    }
}

/// The result of one runtime update.
#[derive(Debug)]
struct Updated {
    invalidation: Invalidation,
    captured: bool,
    redraw: RedrawRequest,
    /// The interaction the runtime returns for this update: the overlays' from before
    /// the base update, else the base tree's under the cursor it was given
    /// (`runtime/src/user_interface.rs:472-477`).
    interaction: mouse::Interaction,
}

/// The result of one frame.
#[derive(Debug)]
struct Frame {
    /// The invalidation of each update pass.
    passes: Vec<Invalidation>,
    /// The last pass published messages, which were handled after the draw.
    late: bool,
    root_layouts: usize,
}

/// The runtime and shell of the tests.
struct Harness {
    tree: Tree,
    log: Rc<Log>,
    window: Size,
    cursor: mouse::Cursor,
    now: Instant,
    root_layouts: usize,
    frame_start: usize,
    /// View rebuilds after a Widgets invalidation.
    rebuilds: usize,
    /// The number of walks when the current frame started.
    walk_start: usize,
    /// The runtime's overlay state: `Some` with the overlays' mouse interaction when
    /// the last update had overlays (`runtime/src/user_interface.rs:279-283`).
    overlay: Option<mouse::Interaction>,
    /// The messages published since the application last handled them, with their
    /// receipts, which the shell drops after the application's update for them
    /// (`winit/src/lib.rs:1247-1249`).
    bus: Bus<()>,
}

impl Harness {
    fn new(root: &mut Root<'_>, log: &Rc<Log>) -> Self {
        Self::sized(root, log, WINDOW)
    }

    fn sized(root: &mut Root<'_>, log: &Rc<Log>, window: Size) -> Self {
        let mut harness = Self {
            tree: Tree::new(&*root),
            log: Rc::clone(log),
            window,
            cursor: mouse::Cursor::Unavailable,
            now: Instant::now(),
            root_layouts: 0,
            frame_start: 0,
            rebuilds: 0,
            walk_start: 0,
            overlay: None,
            bus: Bus::new(),
        };

        harness.rebuild(root);
        harness
    }

    /// A new view, diffed and laid out like `UserInterface::build` does.
    fn rebuild(&mut self, root: &mut Root<'_>) {
        self.tree.diff(root);
        self.layout(root);
    }

    fn layout(&mut self, root: &mut Root<'_>) {
        root.layout(
            &mut self.tree,
            &(),
            &layout::Limits::new(Size::ZERO, self.window),
        );
        self.root_layouts += 1;
    }

    /// A window resize: the shell lays out again before the next redraw.
    fn resize(&mut self, root: &mut Root<'_>, window: Size) {
        self.window = window;
        self.layout(root);
    }

    fn viewport(&self) -> Rectangle {
        Rectangle::with_size(self.window)
    }

    /// Builds the overlays as the runtime does and returns their mouse interaction,
    /// `None` when there are none.
    fn overlays(&mut self, root: &mut Root<'_>) -> Option<mouse::Interaction> {
        let layout = Layout::new(self.tree.size);
        let viewport = self.viewport();
        let overlays = root.overlay(
            &mut self.tree,
            layout,
            &(),
            &viewport,
            Vector::ZERO,
            self.window,
        );

        (!overlays.is_empty()).then(|| Group::new(overlays).mouse_interaction(self.cursor, &()))
    }

    /// The interaction the runtime computes on the overlays that got the event: none
    /// without a cursor position (`runtime/src/user_interface.rs:286-297`).
    fn interaction(
        group: &mut Group<'_, (), Theme, ()>,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        cursor
            .position()
            .map(|position| group.mouse_interaction(mouse::Cursor::Available(position), &()))
            .unwrap_or_default()
    }

    /// Lays out the root for a Layout invalidation.
    fn invalidated(&mut self, root: &mut Root<'_>, invalidation: Invalidation) {
        if let Invalidation::Layout(diff) = invalidation {
            if diff == Diff::Perform {
                self.tree.diff(root);
            }

            self.layout(root);
        }
    }

    /// The cursor of the base tree: levitating while the overlays are interactive.
    fn base_cursor(&self) -> mouse::Cursor {
        match self.overlay {
            None | Some(mouse::Interaction::None) => self.cursor,
            Some(_) => self.cursor.levitate(),
        }
    }

    /// One runtime update, in the runtime's order
    /// (`runtime/src/user_interface.rs:232-384`): the overlays get the event first,
    /// then the base tree with a levitating cursor over interactive overlays. Layout
    /// and overlays are rebuilt inside the update. It never draws: a redraw request is
    /// only reported.
    fn update(&mut self, root: &mut Root<'_>, event: &Event) -> Updated {
        let layout = Layout::new(self.tree.size);
        let viewport = self.viewport();
        let window = self.window;
        let mut bus = std::mem::take(&mut self.bus);
        let mut invalidation = Invalidation::None;
        let mut redraw = RedrawRequest::Wait;
        let mut captured = false;

        // The overlays that got the event are kept, and their interaction computed on
        // them, unless they asked for new overlays or a layout
        // (`runtime/src/user_interface.rs:232-299`).
        let retained = {
            let overlays =
                root.overlay(&mut self.tree, layout, &(), &viewport, Vector::ZERO, window);

            if overlays.is_empty() {
                None
            } else {
                let mut group = Group::new(overlays);
                let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
                group.update(event, self.cursor, &(), &mut shell);

                invalidation = shell.invalidation();
                captured = shell.is_event_captured();
                redraw = shell.redraw_request();
                drop(shell);

                Some(match invalidation {
                    Invalidation::Layout(_) | Invalidation::Overlay => None,
                    _ => Some(Self::interaction(&mut group, self.cursor)),
                })
            }
        };

        self.overlay = match retained {
            None => None,
            Some(Some(interaction)) => Some(interaction),
            Some(None) => {
                self.invalidated(root, invalidation);

                let layout = Layout::new(self.tree.size);
                let overlays =
                    root.overlay(&mut self.tree, layout, &(), &viewport, Vector::ZERO, window);

                (!overlays.is_empty())
                    .then(|| Self::interaction(&mut Group::new(overlays), self.cursor))
            }
        };

        let overlaid = self.overlay.unwrap_or_default();
        let cursor = self.base_cursor();

        if !captured {
            let layout = Layout::new(self.tree.size);
            let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);

            root.update(
                &mut self.tree,
                event,
                layout,
                cursor,
                &(),
                &mut shell,
                &viewport,
            );

            let base = shell.invalidation();
            captured = shell.is_event_captured();
            redraw = redraw.min(shell.redraw_request());
            drop(shell);

            if captured {
                self.overlay = None;
            }

            if let Invalidation::Layout(_) | Invalidation::Overlay = base {
                self.invalidated(root, base);

                if let Some(interaction) = self.overlays(root) {
                    self.overlay = Some(interaction);
                }
            }

            invalidation = invalidation.max(base);
        }

        self.bus = bus;

        let interaction = if overlaid == mouse::Interaction::None {
            root.mouse_interaction(
                &self.tree,
                Layout::new(self.tree.size),
                cursor,
                &viewport,
                &(),
            )
        } else {
            overlaid
        };

        Updated {
            invalidation,
            captured,
            redraw,
            interaction,
        }
    }

    fn next_redraw(&mut self) -> Event {
        // Never behind the real clock, which smooth scrolling starts from.
        self.now = (self.now + Duration::from_millis(16)).max(Instant::now());
        self.frame_start = self.root_layouts;
        self.walk_start = self.walks();

        Event::Window(window::Event::RedrawRequested(self.now))
    }

    /// A redraw: up to three passes with the same event, then a draw.
    fn frame(&mut self, root: &mut Root<'_>) -> Frame {
        let event = self.next_redraw();
        self.complete(root, &event, Vec::new())
    }

    /// A redraw whose Widgets invalidations rebuild the view with `view`.
    fn frame_with<'a>(&mut self, root: &mut Root<'a>, view: &dyn Fn() -> Root<'a>) -> Frame {
        let event = self.next_redraw();
        self.run(root, &event, Vec::new(), Some(view), None)
    }

    /// A redraw of an application: `update` handles the messages published in a pass,
    /// given their number, and the view is rebuilt with `view`.
    fn frame_app<'a>(
        &mut self,
        root: &mut Root<'a>,
        view: &dyn Fn() -> Root<'a>,
        update: &dyn Fn(usize),
    ) -> Frame {
        let event = self.next_redraw();
        self.run(root, &event, Vec::new(), Some(view), Some(update))
    }

    /// Finishes a frame whose first passes the test ran itself.
    fn complete(&mut self, root: &mut Root<'_>, event: &Event, passes: Vec<Invalidation>) -> Frame {
        self.run(root, event, passes, None, None)
    }

    /// The shell's retry loop (`winit/src/lib.rs:784-835`): at most three passes.
    /// Overlay and Layout retry. Messages and Widgets retry after the application
    /// handles the messages and the view is rebuilt and laid out. The third pass is
    /// drawn whatever it invalidated, and its messages are handled after the draw.
    fn run<'a>(
        &mut self,
        root: &mut Root<'a>,
        event: &Event,
        mut passes: Vec<Invalidation>,
        view: Option<&dyn Fn() -> Root<'a>>,
        update: Option<&dyn Fn(usize)>,
    ) -> Frame {
        while passes.len() < 3
            && (passes.last() != Some(&Invalidation::None) || self.messages() > 0)
        {
            if self.messages() > 0 || passes.last() == Some(&Invalidation::Widgets) {
                self.handle(root, view, update);
            }

            self.log.pass.set(passes.len() + 1);
            passes.push(self.update(root, event).invalidation);
        }

        self.draw(root);
        let late = self.messages() > 0;

        if late {
            self.handle(root, view, update);
        }

        Frame {
            passes,
            late,
            root_layouts: self.root_layouts - self.frame_start,
        }
    }

    /// The messages published since the application last handled them.
    fn messages(&self) -> usize {
        self.bus.len()
    }

    /// Handles the messages of updates outside a frame, as the shell does before the
    /// next redraw. Returns how many there were.
    fn deliver<'a>(
        &mut self,
        root: &mut Root<'a>,
        view: &dyn Fn() -> Root<'a>,
        update: &dyn Fn(usize),
    ) -> usize {
        let messages = self.messages();

        if messages > 0 {
            self.handle(root, Some(view), Some(update));
        }

        messages
    }

    /// A left press and release at `point`, with `modifiers` held, as the runtime
    /// delivers them. Returns the press's update.
    fn click(&mut self, root: &mut Root<'_>, point: Point, modifiers: Modifiers) -> Updated {
        self.modifiers(root, modifiers);
        self.cursor = mouse::Cursor::Available(point);
        let press = self.update(
            root,
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        );
        let release = self.update(
            root,
            &Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        );
        assert!(!release.captured, "the list never captures a release");
        press
    }

    fn modifiers(&mut self, root: &mut Root<'_>, modifiers: Modifiers) {
        self.update(
            root,
            &Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)),
        );
    }

    /// A key press with `modifiers`.
    fn key(&mut self, root: &mut Root<'_>, key: Key, modifiers: Modifiers) -> Updated {
        self.update(
            root,
            &Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(),
                modified_key: key,
                physical_key: Physical::Unidentified(NativeCode::Unidentified),
                location: Location::Standard,
                modifiers,
                text: None,
                repeat: false,
            }),
        )
    }

    /// The application handles the pending messages, and the view is rebuilt and
    /// laid out (`winit/src/lib.rs:817-830`).
    fn handle<'a>(
        &mut self,
        root: &mut Root<'a>,
        view: Option<&dyn Fn() -> Root<'a>>,
        update: Option<&dyn Fn(usize)>,
    ) {
        let messages = self.messages();

        if messages > 0 {
            update.expect("messages need the application's update")(messages);
        }

        // The receipts go once the application handled their messages.
        self.bus = Bus::new();

        let view = view.expect("a rebuild needs the view");
        *root = view();
        self.rebuild(root);
        self.rebuilds += 1;
    }

    /// Draws the base tree, then the overlays when the last update had some
    /// (`runtime/src/user_interface.rs:578-618`).
    fn draw(&mut self, root: &mut Root<'_>) {
        self.log.drawn.take();

        let layout = Layout::new(self.tree.size);
        let viewport = self.viewport();
        let style = renderer::Style { text_color: TEXT };

        root.draw(
            &self.tree,
            &mut (),
            &Theme::Light,
            &style,
            layout,
            self.base_cursor(),
            &viewport,
        );

        if self.overlay.is_some() {
            let overlays = root.overlay(
                &mut self.tree,
                layout,
                &(),
                &viewport,
                Vector::ZERO,
                self.window,
            );

            if !overlays.is_empty() {
                Group::new(overlays).draw(&mut (), &Theme::Light, &style, self.cursor);
            }
        }
    }

    /// Operates on the base tree, then the overlays
    /// (`runtime/src/user_interface.rs:623-645`).
    fn operate(&mut self, root: &mut Root<'_>, operation: &mut dyn Operation) {
        let layout = Layout::new(self.tree.size);
        let viewport = self.viewport();
        let tree = &mut self.tree;

        operation.traverse(&mut |operation| {
            root.operate(tree, layout, &viewport, &(), operation);
        });

        let overlays = root.overlay(
            &mut self.tree,
            layout,
            &(),
            &viewport,
            Vector::ZERO,
            self.window,
        );

        if !overlays.is_empty() {
            Group::new(overlays).operate(&(), operation);
        }
    }

    /// Runs an operation and every operation it chains, as the shell runs a widget
    /// action (`winit/src/lib.rs:1694-1715`). The shell then requests a redraw, which
    /// is the test's next frame.
    fn run_task(&mut self, root: &mut Root<'_>, operation: impl Operation + 'static) {
        let mut current: Option<Box<dyn Operation>> = Some(Box::new(operation));

        while let Some(mut operation) = current.take() {
            self.operate(root, operation.as_mut());

            if let Outcome::Chain(next) = operation.finish() {
                current = Some(next);
            }
        }
    }

    /// The ids of the focused widgets.
    fn focused(&mut self, root: &mut Root<'_>) -> Vec<Id> {
        #[derive(Default)]
        struct Focused(Vec<Id>);

        impl Operation for Focused {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }

            fn focusable(
                &mut self,
                id: Option<&Id>,
                _bounds: Rectangle,
                state: &mut dyn Focusable,
            ) {
                if state.is_focused()
                    && let Some(id) = id
                {
                    self.0.push(id.clone());
                }
            }
        }

        let mut focused = Focused::default();
        self.operate(root, &mut focused);
        focused.0
    }

    /// The translation of every scrollable, in traversal order.
    fn translations(&mut self, root: &mut Root<'_>) -> Vec<(Option<Id>, f32)> {
        #[derive(Default)]
        struct Translations(Vec<(Option<Id>, f32)>);

        impl Operation for Translations {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }

            fn scrollable(
                &mut self,
                id: Option<&Id>,
                _bounds: Rectangle,
                _content: Size,
                translation: Vector,
                _state: &mut dyn Scrollable,
            ) {
                self.0.push((id.cloned(), translation.y));
            }
        }

        let mut translations = Translations::default();
        self.operate(root, &mut translations);
        translations.0
    }

    /// Sets the scrollable's offset like an instant `scroll_to` task.
    fn scroll_only(&mut self, root: &mut Root<'_>, y: f32) {
        let mut scroll = scroll_to::<()>(
            SCROLL,
            AbsoluteOffset {
                x: None,
                y: Some(y),
            },
            Animation::Instant,
        );

        self.operate(root, &mut scroll);
    }

    fn scroll(&mut self, root: &mut Root<'_>, y: f32) -> Frame {
        self.scroll_only(root, y);
        self.frame(root)
    }

    fn translation(&mut self, root: &mut Root<'_>) -> f32 {
        let mut translation = Translation::default();
        self.operate(root, &mut translation);
        translation.0.map_or(0.0, |translation| translation.y)
    }

    /// The list's tree and its origin in content coordinates.
    fn list(&self) -> (Vector, &Tree) {
        fn find(tree: &Tree, origin: Vector) -> Option<(Vector, &Tree)> {
            if tree.tag == tree::Tag::of::<Core<u32>>() {
                return Some((origin, tree));
            }

            tree.children
                .iter()
                .find_map(|child| find(child, origin + child.translation))
        }

        find(&self.tree, Vector::ZERO).expect("the view has a list")
    }

    /// A new view diffed into the tree without the layout that always follows.
    fn diff_only(&mut self, root: &mut Root<'_>) {
        self.tree.diff(root);
    }

    /// The interaction of the current trees: the overlays kept or rebuilt by the last
    /// update when they have one, else the base tree's. What the runtime returned for
    /// that update is [`Updated::interaction`]. Rebuilding the overlays here would
    /// lose state they keep in themselves.
    fn mouse_interaction(&self, root: &Root<'_>) -> mouse::Interaction {
        match self.overlay {
            Some(interaction) if interaction != mouse::Interaction::None => interaction,
            _ => root.mouse_interaction(
                &self.tree,
                Layout::new(self.tree.size),
                self.base_cursor(),
                &self.viewport(),
                &(),
            ),
        }
    }

    /// The walks of the current frame.
    fn frame_walks(&self) -> Vec<Walk> {
        self.core().walks[self.walk_start..].to_vec()
    }

    fn core_mut(&mut self) -> &mut Core<u32> {
        fn find(tree: &mut Tree) -> Option<&mut Tree> {
            if tree.tag == tree::Tag::of::<Core<u32>>() {
                return Some(tree);
            }

            tree.children.iter_mut().find_map(find)
        }

        find(&mut self.tree)
            .expect("the view has a list")
            .state
            .downcast_mut()
    }

    fn core(&self) -> &Core<u32> {
        self.list().1.state.downcast_ref()
    }

    /// The walks so far, none for a list without `u32` keys, which only some tests
    /// inspect.
    fn walks(&self) -> usize {
        fn find(tree: &Tree) -> Option<&Core<u32>> {
            if tree.tag == tree::Tag::of::<Core<u32>>() {
                return Some(tree.state.downcast_ref());
            }

            tree.children.iter().find_map(find)
        }

        find(&self.tree).map_or(0, |core| core.walks.len())
    }

    fn last_walk(&self) -> Walk {
        *self.core().walks.last().expect("a walk happened")
    }

    fn positions(&self) -> Vec<usize> {
        self.core()
            .window
            .iter()
            .map(|slot| slot.position)
            .collect()
    }

    fn keys(&self) -> Vec<u32> {
        self.core().window.iter().map(|slot| slot.key).collect()
    }

    /// The tree serial of every window row, checking that trees and slots agree.
    fn serials(&self) -> HashMap<u32, u64> {
        let (_, tree) = self.list();

        self.core()
            .window
            .iter()
            .zip(&tree.children)
            .map(|(slot, child)| {
                let row = child.state.downcast_ref::<RowState>();
                assert_eq!(row.key, slot.key, "tree and slot disagree");
                (row.key, row.serial)
            })
            .collect()
    }

    /// The screen y of a window row's top.
    fn screen_y(&mut self, root: &mut Root<'_>, key: u32) -> f32 {
        let translation = self.translation(root);
        let (origin, tree) = self.list();
        let core: &Core<u32> = tree.state.downcast_ref();
        let index = core
            .window
            .iter()
            .position(|slot| slot.key == key)
            .expect("the key is in the window");

        origin.y + tree.children[index].translation.y - translation
    }

    /// The anchored key, `None` at the end, and the target.
    fn anchor(&self) -> (Option<u32>, Option<f64>) {
        let core = self.core();

        let key = match core.anchor {
            Some(Anchor::Item(key)) => Some(key),
            Some(Anchor::End) | None => None,
        };

        (key, core.geometry.anchor_y())
    }

    fn end_pinned(&self) -> bool {
        self.core().anchor == Some(Anchor::End)
    }

    /// Checks I1 and I2 against the stored visible region.
    fn assert_invariants(&self) {
        let core = self.core();
        let geometry = &core.geometry;
        let visible = geometry.visible.unwrap_or(Visible::GUESS);

        // Rule G: positive drift does not reach into the visible region.
        let padding = f64::from(geometry.settings().padding.top);
        assert!(
            geometry.drift() <= 0.0 || padding + geometry.drift() <= visible.top + 1.0e-6,
            "a gap of {} px above the first item",
            padding + geometry.drift() - visible.top
        );

        if visible.at_start() {
            assert_eq!(geometry.drift(), 0.0);
            assert_eq!(core.anchor, None);
        }

        if let Some(target) = core.anchor_target() {
            assert_eq!(Some(geometry.target_y(target)), geometry.anchor_y());
        }
    }
}

/// Reads the named scrollable's translation.
#[derive(Default)]
struct Translation(Option<Vector>);

impl Operation for Translation {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        _content: Size,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&SCROLL) {
            self.0 = Some(translation);
        }
    }
}

/// Records the containers an operation visits and any custom state offered to it.
#[derive(Default)]
struct Visits {
    containers: Vec<(Option<Id>, Rectangle)>,
    customs: usize,
}

impl Operation for Visits {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle, _viewport: &Rectangle) {
        self.containers.push((id.cloned(), bounds));
    }

    fn custom(&mut self, _id: Option<&Id>, _bounds: Rectangle, _state: &mut dyn std::any::Any) {
        self.customs += 1;
    }
}

fn range(range: std::ops::RangeInclusive<usize>) -> Vec<usize> {
    range.collect()
}

#[test]
fn window_walk_covers_extreme_estimates() {
    // R2-3: estimate 1000 px, real rows 1 px, 600 px viewport at the start.
    let mut app = App::estimated(Vec::new(), 1000.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    drop(root);

    app.items = rows(0..1000, 1.0);
    app.log.clear();
    let mut root = app.view();
    let walks = ui.walks();
    ui.rebuild(&mut root);

    // One walk, which built every row it kept, one at a time and in walk order, and
    // measured each once: 600 visible rows and 4 anticipated ones.
    let built: Vec<u32> = (0..604).collect();
    assert_eq!(ui.walks(), walks + 1);
    assert_eq!(*app.log.created.borrow(), built);
    assert_eq!(app.log.laid_out(), built);
    assert_eq!(
        ui.last_walk(),
        Walk {
            built: 604,
            measured: 604,
            changed: 604,
            evicted: 0,
            discarded: 0,
            local: false,
        }
    );
    assert_eq!(ui.positions(), range(0..=603));
    ui.assert_invariants();

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    app.log.assert_covered(0.0, 600.0);
    drop(root);

    // R3-5: fresh keys in the middle, estimate 1 px, real rows 1000 px.
    let mut app = App::estimated(rows(0..20_000, 1.0), 1.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 10_000.0);
    assert_eq!(ui.anchor(), (Some(10_000), Some(10_000.0)));
    drop(root);

    app.items = (0..20_000)
        .map(|index| Item {
            key: 100_000 + index,
            height: Height::Fixed(1000.0),
        })
        .collect();
    app.log.clear();
    let mut root = app.view();
    ui.rebuild(&mut root);

    // The first visible row covers the viewport. Four rows below and four above
    // complete the window, and the rows above are absorbed against the new anchor.
    let built = vec![
        110_000, 110_001, 110_002, 110_003, 110_004, 109_999, 109_998, 109_997, 109_996,
    ];
    assert_eq!(*app.log.created.borrow(), built);
    assert_eq!(app.log.laid_out(), built);
    assert_eq!(ui.last_walk().built, 9);
    assert_eq!(ui.last_walk().evicted, 0);
    assert_eq!(ui.positions(), range(9_996..=10_004));
    assert_eq!(ui.anchor(), (Some(110_000), Some(10_000.0)));
    assert_eq!(ui.core().geometry.drift(), -4.0 * 999.0);
    ui.assert_invariants();

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(app.log.drawn_keys(), [110_000]);
    app.log.assert_covered(10_000.0, 10_600.0);
    assert_eq!(app.log.screen()[&110_000], 0.0);
    drop(root);

    // The list start: fewer than four rows precede the first visible one.
    let mut app = App::estimated(rows(0..2_000, 1.0), 1.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 2.0);
    drop(root);

    app.items = (0..2_000)
        .map(|index| Item {
            key: 10_000 + index,
            height: Height::Fixed(1000.0),
        })
        .collect();
    app.log.clear();
    let mut root = app.view();
    ui.rebuild(&mut root);

    let built = vec![10_002, 10_003, 10_004, 10_005, 10_006, 10_001, 10_000];
    assert_eq!(*app.log.created.borrow(), built);
    assert_eq!(app.log.laid_out(), built);
    assert_eq!(ui.positions(), range(0..=6));
    assert_eq!(ui.screen_y(&mut root, 10_002), 0.0);
    ui.assert_invariants();
}

#[test]
fn visible_rows_are_not_capped_at_1024() {
    // R1 M1: 1300 px of 1 px rows, with the provisional estimate.
    let app = App::new(rows(0..5_000, 1.0));
    let mut root = app.view();
    let mut ui = Harness::sized(&mut root, &app.log, Size::new(800.0, 1300.0));

    // The first layout has no viewport yet and covers its 1024 px guess.
    assert_eq!(ui.positions(), range(0..=1_027));

    let walks = ui.walks();
    let frame = ui.frame(&mut root);

    // One walk discovers the 276 missing rows at once, and the second pass is clean.
    assert_eq!(
        frame.passes,
        [Invalidation::Layout(Diff::Skip), Invalidation::None]
    );
    assert_eq!(ui.walks(), walks + 1);
    assert_eq!(ui.last_walk().built, 276);
    assert_eq!(ui.positions(), range(0..=1_303));
    app.log.assert_covered(0.0, 1300.0);
    assert_eq!(app.log.drawn.borrow().len(), 1300);
}

#[test]
fn hysteresis_and_lazy_eviction_bound_window() {
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    // The first layout covered its 1024 px guess, which lazy eviction tolerates.
    assert_eq!(ui.positions(), range(0..=29));

    // Less than half the anticipation: no walk, no layout, nothing measured.
    app.log.clear();
    let walks = ui.walks();
    let frame = ui.scroll(&mut root, 60.0);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(frame.root_layouts, 0);
    assert_eq!(ui.walks(), walks);
    assert!(app.log.layouts.borrow().is_empty());
    assert_eq!(app.log.key_calls.get(), 0);
    app.log.assert_covered(60.0, 660.0);

    // A larger shift covers both edges in one walk, which evicts once at the end.
    // Every row it built is kept and only the previous rows are evicted. The new rows
    // measure as estimated, so the walk is local and no root layout follows.
    app.log.clear();
    let frame = ui.scroll(&mut root, 4000.0);
    assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);
    assert_eq!(frame.root_layouts, 0);
    assert_eq!(ui.walks(), walks + 1);
    assert_eq!(
        ui.last_walk(),
        Walk {
            built: 23,
            measured: 30 + 23,
            changed: 0,
            evicted: 30,
            discarded: 0,
            local: true,
        }
    );
    assert_eq!(ui.positions(), range(96..=118));
    let mut built: Vec<u32> = (100..=118).collect();
    built.extend((96..100).rev());
    assert_eq!(*app.log.created.borrow(), built);
    app.log.assert_covered(4000.0, 4600.0);

    // Redraws and walks never call the key closure. Only a view rebuild compares
    // the keys, once per item.
    assert_eq!(app.log.key_calls.get(), 0);
    drop(root);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(app.log.key_calls.get(), 1000);
    ui.frame(&mut root);

    // Only rows that end outside the final window lose their state.
    let before = ui.serials();
    ui.scroll(&mut root, 4200.0);
    assert_eq!(ui.positions(), range(101..=123));
    let middle = ui.serials();

    for key in 101..=118 {
        assert_eq!(middle[&key], before[&key]);
    }

    ui.scroll(&mut root, 4000.0);
    assert_eq!(ui.positions(), range(96..=118));
    let after = ui.serials();

    for key in 96..=100 {
        assert!(after[&key] > before[&key], "row {key} was rebuilt");
    }

    for key in 101..=118 {
        assert_eq!(after[&key], before[&key]);
    }

    app.log.assert_covered(4000.0, 4600.0);
    drop(root);

    // Lazy eviction: a 400 px header comes into view, so only 200 px of the list
    // remain visible. The window keeps its edges but is far larger than needed.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.header = 400.0;
    app.anticipate = 1;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 400.0);
    assert_eq!(ui.positions(), range(0..=15));

    // An eviction-only walk is geometry neutral, so it is local.
    let walks = ui.walks();
    let frame = ui.scroll(&mut root, 0.0);
    assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);
    assert_eq!(frame.root_layouts, 0);
    assert_eq!(ui.walks(), walks + 1);
    assert!(ui.last_walk().local);
    assert_eq!(ui.last_walk().built, 0);
    assert_eq!(ui.last_walk().evicted, 10);
    assert_eq!(ui.positions(), range(0..=5));
    app.log.assert_covered(400.0, 600.0);
}

#[test]
fn huge_anticipation_materializes_everything_without_overflow() {
    // R3-F8: `usize::MAX` overflowed the window bounds and the shift check.
    for anticipate in [usize::MAX, usize::MAX / 4 + 1] {
        let mut app = App::estimated(rows(0..100, 40.0), 40.0);
        app.anticipate = anticipate;
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);
        assert_eq!(ui.positions(), range(0..=99));
        app.log.assert_covered(0.0, 600.0);

        ui.scroll(&mut root, 1000.0);
        assert_eq!(ui.positions(), range(0..=99));
        app.log.assert_covered(1000.0, 1600.0);

        ui.scroll(&mut root, 3400.0);
        assert_eq!(ui.positions(), range(0..=99));
        app.log.assert_covered(3400.0, 4000.0);
    }
}

#[test]
fn prepend_absorbs_before_window_eviction() {
    // R1 H2: 40 px rows measured, viewport top 4000, 1000 rows prepended.
    let mut app = App::estimated(rows(10_000..10_200, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    assert_eq!(ui.anchor(), (Some(10_100), Some(4000.0)));

    let keys = ui.keys();
    let serials = ui.serials();
    let screen = app.log.screen();
    assert_eq!(screen.len(), 15);
    drop(root);

    app.items.splice(0..0, rows(0..1000, 40.0));
    app.log.clear();
    let mut root = app.view();
    ui.rebuild(&mut root);

    // The walk starts from the absorbed geometry, so it keeps the old trees and
    // builds and evicts nothing.
    assert!(app.log.created.borrow().is_empty());
    assert_eq!(ui.last_walk().built, 0);
    assert_eq!(ui.last_walk().evicted, 0);
    assert_eq!(ui.positions(), range(1_096..=1_118));
    assert_eq!(ui.keys(), keys);
    assert_eq!(ui.serials(), serials);
    assert_eq!(ui.core().geometry.drift(), -40_000.0);
    assert_eq!(ui.anchor(), (Some(10_100), Some(4000.0)));
    ui.assert_invariants();

    let frame = ui.frame(&mut root);
    assert_eq!(frame.root_layouts, 0);
    assert_eq!(app.log.screen(), screen);
    app.log.assert_covered(4000.0, 4600.0);
}

#[test]
fn redraw_refreshes_anchor_without_layout() {
    // R1 H3 and R3-4: a scroll inside the hysteresis needs no layout, but the anchor
    // follows it, so collapsing the previous anchor row does not move row 101.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    assert_eq!(ui.anchor(), (Some(100), Some(4000.0)));

    let frame = ui.scroll(&mut root, 4045.0);
    assert_eq!(frame.root_layouts, 0);
    assert_eq!(ui.anchor(), (Some(101), Some(4040.0)));
    assert_eq!(app.log.screen()[&101], -5.0);
    drop(root);

    app.items[100].height = Height::Fixed(20.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.screen_y(&mut root, 101), -5.0);
    assert_eq!(ui.core().geometry.drift(), 20.0);
    ui.assert_invariants();

    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&101], -5.0);
    drop(root);

    // R2-4's stale ordinary anchor: after scrolling on, an anticipated row above the
    // viewport grows, and the first visible row stays put.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 20_000.0);
    ui.scroll(&mut root, 20_600.0);
    assert_eq!(ui.anchor(), (Some(515), Some(20_600.0)));
    assert!(ui.keys().contains(&512));
    drop(root);

    app.items[512].height = Height::Fixed(80.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&515], 0.0);
    ui.assert_invariants();
}

#[test]
fn intentional_drift_move_rebases_anchor() {
    // R2-1, negative drift: the anchor's natural y is 4400 and its target 4000.
    let mut app = App::estimated(rows(1000..1200, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    drop(root);

    app.items.splice(0..0, rows(0..10, 40.0));
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), -400.0);
    assert_eq!(ui.anchor(), (Some(1100), Some(4000.0)));
    assert_eq!(ui.core().geometry.content_y(110), 4000.0);

    // Scrolling up from 4000 to 3900 shrinks the drift by rule S, which moves the
    // target too, so the walk of that same pass keeps it.
    ui.scroll_only(&mut root, 3900.0);
    let redraw = ui.next_redraw();
    let pass = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(pass, Invalidation::Layout(Diff::Skip));
    assert_eq!(ui.core().geometry.drift(), -390.0);
    assert_eq!(ui.core().geometry.content_y(110), 4010.0);
    // The anchor is refreshed on the shrunk geometry (review P2-R1 F2): the first
    // visible row, pinned where it is now, so the shrink is kept.
    assert_eq!(ui.anchor(), (Some(1097), Some(3890.0)));
    ui.complete(&mut root, &redraw, vec![pass]);
    ui.assert_invariants();

    // Rule Z at the start: exactly zero drift and no anchor, which a later layout
    // cannot undo.
    let frame = ui.scroll(&mut root, 0.0);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(ui.anchor(), (None, None));
    drop(root);

    app.items[0].height = Height::Fixed(60.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&0], 0.0);
    assert_eq!(app.log.screen()[&1], 60.0);
    drop(root);

    // Positive drift: rows above the anchor were removed.
    let mut app = App::estimated(rows(1000..1200, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    drop(root);

    app.items.drain(0..10);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), 400.0);
    assert_eq!(ui.anchor(), (Some(1100), Some(4000.0)));

    ui.scroll_only(&mut root, 3900.0);
    let redraw = ui.next_redraw();
    let pass = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(pass, Invalidation::Layout(Diff::Skip));
    assert_eq!(ui.core().geometry.drift(), 390.0);
    assert_eq!(ui.core().geometry.content_y(90), 3990.0);
    assert_eq!(ui.anchor(), (Some(1097), Some(3870.0)));
    ui.complete(&mut root, &redraw, vec![pass]);
    ui.assert_invariants();

    let frame = ui.scroll(&mut root, 0.0);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(ui.anchor(), (None, None));
    assert_eq!(app.log.screen()[&1010], 0.0);
}

#[test]
fn removed_anchor_uses_next_survivor_then_previous() {
    let mut app = App::estimated(rows(0..200, 40.0), 40.0);
    app.padding = Padding::ZERO.vertical(5.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4005.0);
    assert_eq!(ui.anchor(), (Some(100), Some(4005.0)));
    assert_eq!(app.log.screen()[&100], 0.0);
    drop(root);

    // The next surviving key in the old order takes the removed anchor's target.
    app.items.retain(|item| !(100..=101).contains(&item.key));
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.anchor(), (Some(102), Some(4005.0)));
    assert_eq!(ui.screen_y(&mut root, 102), 0.0);
    ui.assert_invariants();
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&102], 0.0);
    drop(root);

    // Only a survivor that was shown takes it. Here every shown row goes, and row
    // 99 ended at the viewport's top: the drift is folded instead of keeping row 99
    // there, which would leave the viewport empty below it. The end is in view, so
    // the walk pins it, and the scrollable clamps to the shorter content: the last
    // rows end above the footer at the bottom.
    app.items.retain(|item| item.key < 102);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert!(ui.end_pinned());
    assert_eq!(ui.core().geometry.drift(), 0.0);
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 4060.0 - 600.0);
    assert_eq!(screen_bottom(&app.log, 99), 545.0);
    ui.assert_invariants();
    drop(root);

    // Removing every row leaves the padding and no drift.
    app.items.clear();
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.anchor(), (None, None));
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert!(ui.core().window.is_empty());
    assert!(ui.list().1.children.is_empty());
    assert_eq!(ui.list().1.size.height, 10.0);
    ui.frame(&mut root);
    assert!(app.log.drawn.borrow().is_empty());
}

#[test]
fn app_filtered_same_length_sequence_reconciles() {
    // R1 M3: the application filters, and the list follows the sequence it gets.
    struct Filtered {
        source: Vec<Item>,
        matching: HashSet<u32>,
        visible: Vec<usize>,
        log: Rc<Log>,
    }

    impl Filtered {
        fn refilter(&mut self) {
            self.visible = (0..self.source.len())
                .filter(|&index| self.matching.contains(&self.source[index].key))
                .collect();
        }

        fn view(&self) -> Root<'_> {
            let log = Rc::clone(&self.log);
            let source = &self.source;

            let list = list_view(&self.visible, move |_, &index: &usize| Probe {
                item: source[index],
                log: Rc::clone(&log),
            })
            .key(move |&index: &usize| source[index].key);

            scrollable(column![list].width(Fill))
                .id(SCROLL)
                .width(Fill)
                .height(Fill)
                .boxed()
        }
    }

    let (a, b) = (rows(1..2, 40.0)[0], rows(2..3, 40.0)[0]);
    let mut app = Filtered {
        source: vec![a, b],
        matching: HashSet::from([1]),
        visible: Vec::new(),
        log: Rc::default(),
    };
    app.refilter();
    assert_eq!(app.visible, [0]);

    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    assert_eq!(app.log.drawn_keys(), [1]);
    let serials = ui.serials();
    drop(root);

    // Same length, swapped data: the new filtered sequence still shows A.
    app.source.swap(0, 1);
    app.refilter();
    assert_eq!(app.visible, [1]);
    app.log.clear();
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(app.log.drawn_keys(), [1]);
    assert_eq!(ui.serials(), serials);
    assert!(app.log.created.borrow().is_empty());
    drop(root);

    // The predicate's input changes in place: now only B matches.
    app.matching = HashSet::from([2]);
    app.refilter();
    assert_eq!(app.visible, [0]);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(app.log.drawn_keys(), [2]);
    assert_eq!(ui.keys(), [2]);
}

#[test]
fn row_limits_and_coordinates_match_parent() {
    // Padding, spacing, a 100 px header, zero-height rows and a Fill row that takes
    // its intrinsic height.
    let heights = [
        Height::Fixed(40.0),
        Height::Fill(25.0),
        Height::Fixed(0.0),
        Height::Fixed(30.0),
        Height::Fixed(0.0),
        Height::Fill(10.0),
    ];
    let mut app = App::estimated(
        (0..)
            .zip(heights)
            .map(|(key, height)| Item { key, height })
            .collect(),
        40.0,
    );
    app.header = 100.0;
    app.spacing = 3.0;
    app.padding = Padding {
        top: 7.0,
        right: 13.0,
        bottom: 9.0,
        left: 11.0,
    };
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    for (_, limits) in app.log.layouts.borrow().iter() {
        assert_eq!(limits.min, Size::new(776.0, 0.0));
        assert_eq!(limits.max.width, 776.0);
        assert_eq!(limits.compression, Size::new(false, true));
        assert_eq!(limits.infinite, Size::new(false, true));
    }

    let (origin, tree) = ui.list();
    assert_eq!(origin, Vector::new(0.0, 100.0));
    assert_eq!(tree.size, Size::new(800.0, 7.0 + 105.0 + 5.0 * 3.0 + 9.0));

    let placed: Vec<(Vector, Size)> = tree
        .children
        .iter()
        .map(|child| (child.translation, child.size))
        .collect();
    let expected: Vec<(Vector, Size)> = [
        (7.0, 40.0),
        (50.0, 25.0),
        (78.0, 0.0),
        (81.0, 30.0),
        (114.0, 0.0),
        (117.0, 10.0),
    ]
    .into_iter()
    .map(|(y, height)| (Vector::new(11.0, y), Size::new(776.0, height)))
    .collect();
    assert_eq!(placed, expected);

    // Drawn rows use the same coordinates. Zero-height rows have nothing to draw.
    let drawn: Vec<(u32, Rectangle)> = app
        .log
        .drawn
        .borrow()
        .iter()
        .map(|drawn| (drawn.key, drawn.bounds))
        .collect();
    let rectangle =
        |y: f32, height: f32| Rectangle::new(Point::new(11.0, 100.0 + y), Size::new(776.0, height));
    assert_eq!(
        drawn,
        [
            (0, rectangle(7.0, 40.0)),
            (1, rectangle(50.0, 25.0)),
            (3, rectangle(81.0, 30.0)),
            (5, rectangle(117.0, 10.0)),
        ]
    );
    drop(root);

    // An empty list is as tall as its padding.
    app.items.clear();
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.list().1.size, Size::new(800.0, 16.0));
    assert!(ui.list().1.children.is_empty());
    ui.frame(&mut root);
    drop(root);

    // Without a scrollable, the list virtualizes against the window: 500 px of it
    // are visible below the header.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.scroller = false;
    app.header = 100.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    assert_eq!(ui.positions(), range(0..=16));
    app.log.assert_covered(100.0, 600.0);
    drop(root);

    // A clipped viewport: the scrollable is 300 px tall, 200 px of it below the header.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.height = Length::Fixed(300.0);
    app.header = 100.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    assert_eq!(ui.positions(), range(0..=8));
    app.log.assert_covered(100.0, 300.0);
    drop(root);

    // A width change lays out every window row again and absorbs against the anchor.
    let items = (0..1000)
        .map(|key| Item {
            key,
            height: Height::Area(40.0 * 800.0),
        })
        .collect();
    let app = App::estimated(items, 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    assert_eq!(ui.anchor(), (Some(100), Some(4000.0)));
    let keys = ui.keys();

    app.log.clear();
    ui.resize(&mut root, Size::new(400.0, 600.0));
    assert_eq!(app.log.laid_out(), keys);
    assert!(
        app.log
            .layouts
            .borrow()
            .iter()
            .all(|(_, limits)| limits.max.width == 400.0)
    );
    assert_eq!(ui.core().geometry.heights.height(99), 80.0);
    assert_eq!(ui.screen_y(&mut root, 100), 0.0);
    ui.assert_invariants();

    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&100], 0.0);
    assert_eq!(app.log.screen()[&101], 80.0);
    app.log.assert_covered(4000.0, 4600.0);
}

#[test]
fn child_first_nonselectable_events_and_traversal() {
    let mut app = App::estimated(rows(0..100, 40.0), 40.0);
    app.highlight = vec![12];
    app.text_color = Some(RED);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 400.0);

    // The window of the first layout's 1024 px guess still covers this viewport.
    let window: Vec<u32> = (0..=29).collect();
    assert_eq!(ui.keys(), window);

    // Every materialized row gets each event once, and without selection the list
    // captures nothing.
    let once = |log: &Log, redraw: bool| {
        let updates = log.updates.take();
        assert!(updates.iter().all(|&(_, kind)| kind == redraw));
        updates
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<u32>>()
    };

    ui.cursor = mouse::Cursor::Available(Point::new(100.0, 90.0));
    app.log.clear();

    for event in [
        mouse::Event::CursorMoved {
            position: Point::new(100.0, 90.0),
        },
        mouse::Event::ButtonPressed(mouse::Button::Left),
        mouse::Event::ButtonReleased(mouse::Button::Left),
    ] {
        let captured = ui.update(&mut root, &Event::Mouse(event)).captured;
        assert!(!captured);
        assert_eq!(once(&app.log, false), window);
    }

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(once(&app.log, true), window);

    // Only rows that intersect the viewport are drawn, with the style's text color
    // and their status.
    let drawn = app.log.drawn.borrow().clone();
    assert_eq!(app.log.drawn_keys(), (10..=24).collect::<Vec<u32>>());

    for row in &drawn {
        assert_eq!(row.text_color, RED);
        assert_eq!(
            row.bounds,
            Rectangle::new(
                Point::new(0.0, 40.0 * row.key as f32),
                Size::new(800.0, 40.0)
            )
        );
    }

    let statuses = app.log.statuses.take();
    assert_eq!(statuses.len(), drawn.len());

    for (row, status) in drawn.iter().zip(statuses) {
        // The cursor is at screen y 90, content y 490, over row 12.
        assert_eq!(
            status,
            Status {
                hovered: row.key == 12,
                highlighted: row.key == 12,
                selected: false,
                cursor: false,
            }
        );
    }

    // Operations visit the list as a container with its id and only the current row
    // trees, at their bounds. No row is measured for them.
    app.log.clear();
    let mut visits = Visits::default();
    ui.operate(&mut root, &mut visits);
    assert!(visits.containers.contains(&(
        Some(LIST),
        Rectangle::new(Point::ORIGIN, Size::new(800.0, 4000.0))
    )));
    // The list offers its task exchange once, and nothing asks.
    assert_eq!(visits.customs, 1);
    assert!(ui.core().resolved.is_empty());
    assert!(app.log.layouts.borrow().is_empty());

    let expected: Vec<(u32, Rectangle)> = window
        .iter()
        .map(|&key| {
            let bounds = Rectangle::new(Point::new(0.0, 40.0 * key as f32), Size::new(800.0, 40.0));
            (key, bounds)
        })
        .collect();
    assert_eq!(*app.log.operated.borrow(), expected);

    // Operations also traverse the overlays, which the rows were asked for.
    assert_eq!(*app.log.overlays.borrow(), expected);

    // Overlays are asked of the current rows only, at the same bounds.
    app.log.overlays.take();
    ui.update(&mut root, &Event::Mouse(mouse::Event::CursorLeft));
    assert_eq!(*app.log.overlays.borrow(), expected);
    assert!(app.log.layouts.borrow().is_empty());
}

#[test]
fn public_api_constructs_with_null_and_production_renderers() {
    use crate::listview::{
        self as public, Action, Align, Catalog, Change, ListView, Selection, Status,
        Style as PublicStyle, StyleFn as PublicStyleFn, list_view as constructor,
    };

    fn assert_widget<M, W: Widget<M, Theme, iced::Renderer>>(_widget: &W) {}

    /// A message that is not `Clone`.
    struct Selected(Action<u32>);

    let items = [3_u32, 1, 2];

    // Stable keys, homogeneous rows, every builder.
    let list = constructor(&items, |_, number: &u32| text(number.to_string()))
        .key(|number: &u32| *number)
        .id(LIST)
        .scroller(SCROLL)
        .width(Fill)
        .padding(4)
        .spacing(2)
        .estimated_height(20)
        .anticipate(2)
        .highlight([1])
        .style(public::default);
    assert_widget::<(), _>(&list);
    let _: Element<'_, (), Theme, iced::Renderer> = list.boxed();

    // Boxed rows of mixed widgets, index keys and a class.
    let class: PublicStyleFn<'_, Theme> =
        Box::new(|_theme, _status: Status| PublicStyle::default());
    let list = constructor(&items, |index, number: &u32| {
        if index == 0 {
            text(number.to_string()).boxed()
        } else {
            space().height(10).boxed()
        }
    })
    .class(class);
    assert_widget::<(), _>(&list);
    let _ = <Theme as Catalog>::default();

    // A selection, borrowed or owned, with a message that is not `Clone`.
    let selection = Selection::new();
    let list: ListView<'_, u32, u32, _, Selected, Theme, iced::Renderer> =
        constructor(&items, |_, number: &u32| text(number.to_string()))
            .key(|number: &u32| *number)
            .selection(&selection, Selected)
            .multiple(true)
            .on_activate(|key| Selected(Change::Select(key).into()));
    assert_widget::<Selected, _>(&list);

    let owned = constructor(&items, |_, number: &u32| text(number.to_string()))
        .key(|number: &u32| *number)
        .selection(selection.clone(), Selected);
    assert_widget::<Selected, _>(&owned);

    // The tasks, for the same message and key types.
    let _: iced::Task<Selected> = public::scroll_to(LIST, 1_u32, Align::Start, Animation::Instant);
    let _: iced::Task<Selected> = public::focus(LIST, 1_u32, field(1));
    let _: iced::Task<Selected> = public::settle(LIST);

    // An application reads the action out of its message.
    let _ = |message: Selected| message.0.reveal().copied();

    // The same constructor with the null renderer, laid out and drawn.
    let log = Rc::new(Log::default());
    let items: Vec<Item> = rows(0..20, 40.0);
    let mut root: Root<'_> = scrollable(
        constructor(&items, |_, item| row(item, &log))
            .key(|item: &Item| item.key)
            .id(LIST)
            .selection(Selection::new(), |_| ()),
    )
    .id(SCROLL)
    .height(Fill)
    .boxed();
    let mut ui = Harness::new(&mut root, &log);
    ui.frame(&mut root);
    log.assert_covered(0.0, 600.0);
}

#[test]
fn gap_rule_folds_drift_above_too_few_rows() {
    // Fix F1, from review P2-R1 F1's scenario: a 50 px footer below the list,
    // positive drift, and a list so short that the scrollable clamps its translation
    // again after any shrink. Rule S's once-per-redraw guard has its own case,
    // `shrink_runs_once_per_redraw_while_the_parent_clamps`.
    let mut app = App::estimated(rows(0..200, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    drop(root);

    app.items.drain(0..100);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), 4000.0);
    drop(root);

    app.items.truncate(10);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.list().1.size.height, 4400.0);

    // Pass 1 sees the translation clamped to 4450 - 600, where the drift would show
    // as a 150 px gap above row 100 (fix F1). The ten rows cannot fill the viewport
    // from its top, so rule G folds the drift. Pass 2 sees the translation clamped
    // to the content that is left, the start.
    let frame = ui.frame(&mut root);
    assert_eq!(
        frame.passes,
        [
            Invalidation::Layout(Diff::Skip),
            Invalidation::Layout(Diff::Skip),
            Invalidation::None
        ]
    );
    assert_eq!(ui.core().geometry.drift(), 0.0);
    ui.assert_invariants();

    // The drawn translation is the clamp of the final content height: the rows from
    // the top, and the footer below them.
    let content = ui.list().1.size.height + FOOTER;
    assert_eq!(content, 450.0);
    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(app.log.drawn_keys(), (100..110).collect::<Vec<u32>>());
    assert_eq!(app.log.screen()[&100], 0.0);

    // Nothing is left for a later frame.
    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(ui.core().geometry.drift(), 0.0);
}

#[test]
fn shrink_repins_so_the_walk_keeps_what_it_builds() {
    // Review P2-R1 F2: 1 px rows, drift -10,000 after a prepend, and rows that are now
    // 1000 px tall between the viewport and the old anchor.
    let mut app = App::estimated(rows(0..30_000, 1.0), 1.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 10_000.0);
    drop(root);

    app.items.splice(0..0, rows(100_000..110_000, 1.0));

    for item in &mut app.items[18_000..18_600] {
        item.height = Height::Fixed(1000.0);
    }

    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), -10_000.0);
    assert_eq!(ui.anchor(), (Some(10_000), Some(10_000.0)));
    let window = ui.positions().len();

    // Rule S makes the drift -9000. The anchor is the first visible row of the shrunk
    // geometry, so the rows measured above it cannot push the visible rows away.
    ui.scroll_only(&mut root, 9000.0);
    let redraw = ui.next_redraw();
    let pass = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(pass, Invalidation::Layout(Diff::Skip));
    assert_eq!(ui.core().geometry.drift(), -9000.0);
    assert_eq!(ui.anchor(), (Some(8000), Some(9000.0)));

    let walk = ui.last_walk();
    assert_eq!(walk.built, 9);
    assert_eq!(walk.discarded, 0);
    assert_eq!(walk.evicted, window);
    assert_eq!(ui.positions(), range(17_996..=18_004));

    ui.complete(&mut root, &redraw, vec![pass]);
    ui.assert_invariants();
    app.log.assert_covered(9000.0, 9600.0);
    assert_eq!(app.log.screen()[&8000], 0.0);
}

#[test]
fn later_duplicate_is_never_the_anchor() {
    // Review P2-R1 F3: positions 0 and 10 share key 0. Position 10 becomes the first
    // visible row, but its key leads back to position 0.
    let mut items = rows(0..100, 40.0);
    items[10].key = 0;
    let app = App::estimated(items, 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);

    let frame = ui.scroll(&mut root, 400.0);
    assert_eq!(frame.root_layouts, 0);
    assert_eq!(ui.anchor(), (Some(11), Some(440.0)));
    drop(root);

    // An unchanged rebuild absorbs nothing.
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    ui.assert_invariants();

    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&11], 40.0);
}

#[test]
fn hover_redraw_follows_the_drawn_frame() {
    // Review P2-R1 F4: the hover to compare with is the one the last draw showed.
    let app = App::estimated(rows(0..100, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 400.0);

    let move_to = |ui: &mut Harness, root: &mut Root<'_>, y: f32| {
        let position = Point::new(100.0, y);
        ui.cursor = mouse::Cursor::Available(position);
        let event = Event::Mouse(mouse::Event::CursorMoved { position });
        ui.update(root, &event).redraw
    };

    // Entering row 12 (screen y 90, content y 490) asks for a frame. After it was
    // drawn, moving inside the row does not.
    assert_eq!(move_to(&mut ui, &mut root, 90.0), RedrawRequest::NextFrame);
    ui.frame(&mut root);
    assert_eq!(move_to(&mut ui, &mut root, 95.0), RedrawRequest::Wait);

    // Scrolling under the still cursor draws row 13 as hovered. Moving back onto row
    // 12, which the last cursor event also hovered, asks for a frame, and the harness
    // does not draw one by itself.
    ui.scroll(&mut root, 440.0);
    let drawn = app.log.drawn.borrow().len();
    assert_eq!(move_to(&mut ui, &mut root, 50.0), RedrawRequest::NextFrame);
    assert_eq!(app.log.drawn.borrow().len(), drawn);
}

#[test]
fn partially_visible_bottom_row_is_built() {
    // Review P2-R1 F5: 1 px rows without anticipation.
    let mut app = App::estimated(rows(0..2_000, 1.0), 1.0);
    app.anticipate = 0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    assert_eq!(ui.positions(), range(0..=599));

    // Half a grid unit of scrolling shows the top of row 600.
    let tiny = 1.0 / 2048.0;
    ui.scroll(&mut root, tiny);
    assert_eq!(ui.positions(), range(0..=600));
    app.log.assert_covered(tiny, 600.0 + tiny);

    // A bottom exactly on row 601's top leaves it out.
    let frame = ui.scroll(&mut root, 1.0);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert_eq!(ui.positions(), range(1..=600));
    app.log.assert_covered(1.0, 601.0);
}

#[test]
fn spacing_at_the_viewport_edges_is_not_missing_content() {
    // Review P2-R2 F1: with spacing 10 and no anticipation, row 11 is [550, 590) and
    // row 12 starts at 600, exactly at the bottom of the viewport.
    let mut app = App::estimated(rows(0..100, 40.0), 40.0);
    app.spacing = 10.0;
    app.anticipate = 0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);

    // Lazy eviction trims the window of the first layout's guess once, and the
    // confirmation pass is idle.
    let frame = ui.frame(&mut root);
    assert_eq!(
        frame.passes,
        [Invalidation::Layout(Diff::Skip), Invalidation::None]
    );
    assert_eq!(ui.positions(), range(0..=11));

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(frame.root_layouts, 0);

    // The top in the spacing above row 12 and the bottom in the spacing below row 23:
    // neither row 11 nor row 24 shows any content. The shift is geometry neutral.
    let frame = ui.scroll(&mut root, 595.0);
    assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);
    assert_eq!(ui.positions(), range(12..=23));
    assert_eq!(app.log.drawn_keys(), (12..=23).collect::<Vec<u32>>());

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    ui.assert_invariants();
}

#[test]
fn retained_measurements_repin_the_ordinary_anchor() {
    // Review P2-R2 F2: row 0 is 10,000 px tall and the only visible row at top 600.
    // The estimated end stays below the viewport, so the ordinary anchor applies.
    let mut items = rows(0..2000, 1.0);
    items[0].height = Height::Fixed(10_000.0);
    let mut app = App::estimated(items, 1.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 600.0);
    assert_eq!(ui.positions(), range(0..=4));
    assert_eq!(ui.anchor(), (Some(0), Some(0.0)));
    drop(root);

    // Row 0 collapses to 1 px and the unmeasured rows below the window grow to
    // 1000 px. Remeasuring row 0 leaves the anchor above the viewport, so the walk
    // pins the first visible row before discovering rows and keeps all it builds.
    app.items[0].height = Height::Fixed(1.0);

    for item in &mut app.items[5..] {
        item.height = Height::Fixed(1000.0);
    }

    let mut root = app.view();
    ui.rebuild(&mut root);

    let walk = ui.last_walk();
    assert_eq!(walk.built, 9);
    assert_eq!(walk.discarded, 0);
    assert_eq!(walk.evicted, 5);
    assert_eq!(ui.positions(), range(596..=604));
    assert_eq!(ui.anchor(), (Some(600), Some(600.0)));
    ui.assert_invariants();

    ui.frame(&mut root);
    app.log.assert_covered(600.0, 1200.0);
    assert_eq!(app.log.screen()[&600], 0.0);
}

#[test]
fn duplicate_keys_have_separate_hover() {
    // Review P2-R2 F3: positions 0 and 1 share key 0.
    let mut items = rows(0..100, 40.0);
    items[1].key = 0;
    let app = App::estimated(items, 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let move_to = |ui: &mut Harness, root: &mut Root<'_>, y: f32| {
        let position = Point::new(100.0, y);
        ui.cursor = mouse::Cursor::Available(position);
        let event = Event::Mouse(mouse::Event::CursorMoved { position });
        ui.update(root, &event).redraw
    };

    assert_eq!(move_to(&mut ui, &mut root, 10.0), RedrawRequest::NextFrame);
    ui.frame(&mut root);
    assert_eq!(move_to(&mut ui, &mut root, 30.0), RedrawRequest::Wait);

    // The second occurrence is another item, even with the same key.
    assert_eq!(move_to(&mut ui, &mut root, 50.0), RedrawRequest::NextFrame);
}

#[test]
fn overestimated_start_row_is_bounded_by_the_estimate_error() {
    // Review P2-R3: real rows of 1 px under an explicit 1000 px estimate. The first
    // frame at the start measures rows 0..=1027 for the layout's guess and keeps
    // 0..=603.
    let app = App::estimated(rows(0..5000, 1.0), 1000.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    assert_eq!(ui.positions(), range(0..=603));

    // A jump to 10,000 lands in unmeasured row 1036, estimated at [9028, 10028). It
    // becomes the anchor and is measured as 1 px, so the rows the walk then finds
    // below it until the viewport is covered end above the viewport once the anchor
    // moves to the first visible row, and are discarded.
    let frame = ui.scroll(&mut root, 10_000.0);
    assert_eq!(
        frame.passes,
        [Invalidation::Layout(Diff::Skip), Invalidation::None]
    );

    // Accepted limitation (see `ListView`): the discarded rows are about the start
    // row's estimate error divided by the real row height. These counts are pinned so
    // that any change, in either direction, shows here. The local walk does that work
    // and, having changed heights, falls back to a root layout, whose walk builds
    // nothing more.
    let walks = &ui.core().walks;
    let (local, full) = (walks[walks.len() - 2], walks[walks.len() - 1]);
    assert!(local.local && !full.local);
    assert_eq!(local.built, 1580);
    assert_eq!(local.discarded, 972);
    assert_eq!(local.evicted, 604 + 972);
    assert_eq!((full.built, full.changed, full.evicted), (0, 0, 0));
    assert_eq!(ui.positions(), range(2004..=2611));

    app.log.assert_covered(10_000.0, 10_600.0);
    ui.assert_invariants();

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(frame.root_layouts, 0);
}

/// Whether no walk of `walks` was local.
fn all_full(walks: &[Walk]) -> bool {
    walks.iter().all(|walk| !walk.local)
}

#[test]
fn remeasured_cached_window_shifts_never_layout_root() {
    // FP1: rows are 40 px and the estimate 30 px, so a first visit changes heights and
    // takes the full path. Shifts over measured rows never lay out the root.
    let app = App::estimated(rows(0..1000, 40.0), 30.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    for y in (400..=6000).step_by(400) {
        ui.scroll(&mut root, y as f32);
    }

    assert_eq!(ui.core().geometry.drift(), 0.0);
    let siblings = app.log.sibling_layouts.get();

    for y in [
        2000.0, 1200.0, 4400.0, 3000.0, 2600.0, 4800.0, 1000.0, 1160.0, 3400.0,
    ] {
        app.log.clear();
        let frame = ui.scroll(&mut root, y);

        // The shell saw an Overlay invalidation and then an idle confirmation pass.
        assert_eq!(
            frame.passes,
            [Invalidation::Overlay, Invalidation::None],
            "{y}"
        );
        assert_eq!(frame.root_layouts, 0);

        let walks = ui.frame_walks();
        assert_eq!(walks.len(), 1);
        assert!(walks[0].local);
        assert_eq!(walks[0].changed, 0);
        assert_eq!(walks[0].discarded, 0);

        // Rows entering the window are measured again, with unchanged heights.
        let entering = app.log.created.borrow().clone();
        let laid_out = app.log.laid_out();
        assert!(!entering.is_empty());
        assert!(entering.iter().all(|key| laid_out.contains(key)));

        app.log.assert_covered(y, y + 600.0);
        ui.assert_invariants();
    }

    assert_eq!(app.log.sibling_layouts.get(), siblings);
    assert_eq!(ui.core().geometry.drift(), 0.0);
}

#[test]
fn measurement_mismatch_falls_back_even_with_equal_total() {
    // FP2: rows 117 and 118 are measured at 4000, change while the viewport is away,
    // and come back as anticipated rows below the visible ones at 4080, where they
    // move neither the anchor nor the drift.
    let check = |changes: &[(usize, f32)], len: u32| {
        let mut app = App::estimated(rows(0..len, 40.0), 40.0);
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.scroll(&mut root, 4000.0);
        ui.scroll(&mut root, 8000.0);
        drop(root);

        for &(index, height) in changes {
            app.items[index].height = Height::Fixed(height);
        }

        let mut root = app.view();
        ui.rebuild(&mut root);

        // Nothing measured changed, so the next redraw is not blocked.
        assert!(!ui.core().geometry.block_next_redraw);
        let before = (ui.list().1.size, ui.core().geometry.heights.total());

        let frame = ui.scroll(&mut root, 4080.0);
        assert_eq!(
            frame.passes,
            [Invalidation::Layout(Diff::Skip), Invalidation::None]
        );

        let walks = ui.frame_walks();
        assert!(walks[0].local && walks[0].changed > 0);
        assert!(all_full(&walks[1..]));

        assert_eq!(ui.anchor(), (Some(102), Some(4080.0)));
        assert_eq!(app.log.screen()[&102], 0.0);
        app.log.assert_covered(4080.0, 4680.0);
        ui.assert_invariants();

        (
            before,
            (ui.list().1.size, ui.core().geometry.heights.total()),
        )
    };

    // One row 1 px taller.
    let ((size, total), (after, after_total)) = check(&[(117, 41.0)], 1000);
    assert_eq!(after_total, total + 1.0);
    assert_eq!(after.height, size.height + 1.0);

    // One row 1 px taller and the next 1 px shorter: neither total nor size change.
    let ((size, total), (after, after_total)) = check(&[(117, 41.0), (118, 39.0)], 1000);
    assert_eq!(after_total, total);
    assert_eq!(after, size);

    // One grid unit on a 1.6M px list, which the f32 size cannot show.
    let ((size, total), (after, after_total)) = check(&[(117, 40.0 + 1.0 / 256.0)], 40_000);
    assert_eq!(after_total, total + 1.0 / 256.0);
    assert_eq!(after, size);
}

#[test]
fn constraint_cache_and_estimate_changes_use_full_path() {
    // FP3. Downward shifts only, so rule S never applies.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let local = |ui: &mut Harness, root: &mut Root<'_>, y: f32| {
        let frame = ui.scroll(root, y);
        assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);
        assert!(ui.frame_walks()[0].local);
    };
    let full = |ui: &mut Harness, root: &mut Root<'_>, y: f32| {
        let frame = ui.scroll(root, y);
        assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
        assert!(all_full(&ui.frame_walks()));
    };

    local(&mut ui, &mut root, 2000.0);

    // A diff that the redraw sees before the layout that always follows it.
    drop(root);
    let mut root = app.view();
    ui.diff_only(&mut root);
    assert!(ui.core().geometry.local.is_none());
    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert!(all_full(&ui.frame_walks()));
    assert!(ui.core().geometry.local.is_some());
    local(&mut ui, &mut root, 2400.0);

    // Changed padding, then spacing. The rebuild's layout caches them, and the next
    // redraw is full whatever it shifts. Only the one after it is local again.
    drop(root);
    app.padding = Padding::ZERO.vertical(8.0).horizontal(10.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert!(ui.core().geometry.block_next_redraw);
    full(&mut ui, &mut root, 2800.0);
    local(&mut ui, &mut root, 3200.0);
    assert_eq!(
        ui.core().geometry.local.unwrap().settings.padding,
        app.padding
    );

    drop(root);
    app.spacing = 4.0;
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert!(ui.core().geometry.block_next_redraw);
    full(&mut ui, &mut root, 3600.0);
    local(&mut ui, &mut root, 4000.0);

    // New limits reach the list through a full layout, which replaces the cache.
    // The local walks after it lay rows out with the new width.
    ui.resize(&mut root, Size::new(600.0, 600.0));
    assert_eq!(ui.core().geometry.local.unwrap().limits.max.width, 600.0);
    app.log.clear();
    local(&mut ui, &mut root, 4800.0);
    assert!(!app.log.created.borrow().is_empty());
    assert!(
        app.log
            .layouts
            .borrow()
            .iter()
            .all(|(_, limits)| limits.max.width == 580.0)
    );
    drop(root);

    // Adopting the first positive measurement as the estimate is a full-path result,
    // even when it equals the provisional 32 px. Rows 0..=106 are 0 px tall and
    // measured by the first layout, rows from 107 on are 32 px.
    let items = (0..2000)
        .map(|key| Item {
            key,
            height: Height::Fixed(if key < 107 { 0.0 } else { 32.0 }),
        })
        .collect();
    let mut app = App::new(items);
    app.spacing = 10.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    assert_eq!(ui.core().geometry.heights.estimate(), Estimate::Provisional);
    local(&mut ui, &mut root, 200.0);

    let frame = ui.scroll(&mut root, 800.0);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    let walks = ui.frame_walks();
    assert!(walks[0].local);
    assert_eq!(walks[0].changed, 0);
    assert_eq!(
        ui.core().geometry.heights.estimate(),
        Estimate::Measured(32 * 256)
    );
    // Rows 81..=106 are 0 px tall between 10 px gaps and row 107 starts at 1070.
    assert_eq!(ui.positions(), range(77..=118));
    ui.assert_invariants();
}

#[test]
fn drift_or_task_barrier_covers_every_retry() {
    // FP4.
    let mut app = App::estimated(rows(1000..1400, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    drop(root);

    // Negative drift from a prepend. The structural change blocks the next redraw.
    app.items.splice(0..0, rows(0..10, 40.0));
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), -400.0);
    assert!(ui.core().geometry.block_next_redraw);

    let frame = ui.scroll(&mut root, 4400.0);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert!(all_full(&ui.frame_walks()));

    // Downward scrolling with unchanged nonzero drift may be local.
    let frame = ui.scroll(&mut root, 4800.0);
    assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);
    assert!(ui.frame_walks()[0].local);
    assert_eq!(ui.core().geometry.drift(), -400.0);

    // Rule S: every pass of that redraw is full, also a retry that needs a walk
    // because the ancestors moved the viewport again in between.
    ui.scroll_only(&mut root, 4400.0);
    let redraw = ui.next_redraw();
    let first = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(first, Invalidation::Layout(Diff::Skip));
    assert_ne!(ui.core().geometry.drift(), -400.0);

    ui.scroll_only(&mut root, 3600.0);
    let second = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(second, Invalidation::Layout(Diff::Skip));
    ui.complete(&mut root, &redraw, vec![first, second]);
    assert!(ui.frame_walks().len() >= 2);
    assert!(all_full(&ui.frame_walks()));
    app.log.assert_covered(3600.0, 4200.0);

    // Rule Z likewise.
    ui.scroll_only(&mut root, 0.0);
    let redraw = ui.next_redraw();
    let first = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(first, Invalidation::Layout(Diff::Skip));
    assert_eq!(ui.core().geometry.drift(), 0.0);

    ui.scroll_only(&mut root, 300.0);
    let second = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(second, Invalidation::Layout(Diff::Skip));
    ui.complete(&mut root, &redraw, vec![first, second]);
    assert!(all_full(&ui.frame_walks()));

    // A later stable shift is local again.
    let frame = ui.scroll(&mut root, 1200.0);
    assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);

    // A geometry change made by an operation, as the tasks of a later phase will
    // make, blocks the next redraw. A full layout in between does not lift it.
    let core = ui.core_mut();
    core.geometry.move_drift(40.0);
    core.geometry.block_next_redraw = true;
    drop(root);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert!(ui.core().geometry.block_next_redraw);

    let frame = ui.scroll(&mut root, 1600.0);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert!(all_full(&ui.frame_walks()));
    let frame = ui.scroll(&mut root, 2000.0);
    assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);
    ui.assert_invariants();
}

#[test]
fn input_deltas_wait_for_redraw_viewport() {
    // FP5: real input through the parent scrollable. The list never walks on it, and
    // the next redraw covers the translation the scrollable actually has.
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let input = |ui: &mut Harness, root: &mut Root<'_>, event: Event| {
        let walks = ui.walks();
        let updated = ui.update(root, &event);
        assert_eq!(ui.walks(), walks, "no walk on {event:?}");
        assert!(!matches!(
            updated.invalidation,
            Invalidation::Layout(_) | Invalidation::Widgets
        ));
    };
    let covered = |ui: &mut Harness, root: &mut Root<'_>| {
        ui.frame(root);
        let translation = ui.translation(root);
        app.log.assert_covered(translation, translation + 600.0);
        ui.assert_invariants();
        translation
    };

    // Pixels scroll at once. The translation the content sees only changes on the
    // next redraw (`widget/src/scrollable.rs:2629-2640`).
    ui.cursor = mouse::Cursor::Available(Point::new(400.0, 300.0));
    let pixels = mouse::ScrollDelta::Pixels { x: 0.0, y: -500.0 };
    input(
        &mut ui,
        &mut root,
        Event::Mouse(mouse::Event::WheelScrolled { delta: pixels }),
    );
    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(covered(&mut ui, &mut root), 500.0);

    // Lines scroll smoothly, over several redraws.
    let lines = mouse::ScrollDelta::Lines { x: 0.0, y: -10.0 };
    input(
        &mut ui,
        &mut root,
        Event::Mouse(mouse::Event::WheelScrolled { delta: lines }),
    );
    let mut previous = 500.0;
    let mut moving = 0;

    for _ in 0..120 {
        let translation = covered(&mut ui, &mut root);

        if translation == previous && moving > 0 {
            break;
        }

        if translation != previous {
            moving += 1;
        }

        previous = translation;
    }

    assert!(moving > 1, "the smooth scroll took several redraws");
    assert_eq!(previous, 500.0 + 1200.0);

    // Touch drags scroll by the finger's movement.
    let finger = touch::Finger(0);
    let start = Point::new(400.0, 300.0);
    ui.cursor = mouse::Cursor::Available(start);
    input(
        &mut ui,
        &mut root,
        Event::Touch(touch::Event::FingerPressed {
            id: finger,
            position: start,
        }),
    );
    let end = Point::new(400.0, 100.0);
    ui.cursor = mouse::Cursor::Available(end);
    input(
        &mut ui,
        &mut root,
        Event::Touch(touch::Event::FingerMoved {
            id: finger,
            position: end,
        }),
    );
    assert_eq!(ui.translation(&mut root), 1700.0);
    assert_eq!(covered(&mut ui, &mut root), 1900.0);
    input(
        &mut ui,
        &mut root,
        Event::Touch(touch::Event::FingerLifted {
            id: finger,
            position: end,
        }),
    );

    // Dragging the scrollbar's scroller.
    let content = ui.tree.children[0].size.height;
    let scroller = 1900.0 / content * 600.0;
    ui.cursor = mouse::Cursor::Available(Point::new(795.0, scroller + 1.0));
    input(
        &mut ui,
        &mut root,
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
    );
    assert_eq!(ui.translation(&mut root), 1900.0);

    let position = Point::new(795.0, scroller + 101.0);
    ui.cursor = mouse::Cursor::Available(position);
    input(
        &mut ui,
        &mut root,
        Event::Mouse(mouse::Event::CursorMoved { position }),
    );
    assert_eq!(ui.translation(&mut root), 1900.0);
    let dragged = covered(&mut ui, &mut root);
    assert!(
        dragged > 1900.0 + 5000.0,
        "the scroller was dragged to {dragged}"
    );
    input(
        &mut ui,
        &mut root,
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    );
}

#[test]
fn local_tree_geometry_is_visible_without_root_layout() {
    // FP6: after a local shift, draw, operate, mouse interaction and overlays see the
    // new rows from the trees, and the siblings keep their bounds.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.header = 100.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 400.0);
    ui.frame(&mut root);

    let (origin, list) = ui.list();
    let list = Rectangle::new(Point::ORIGIN + origin, list.size);
    let footer = app.log.footer.get();
    let siblings = app.log.sibling_layouts.get();

    // The first pass walks locally and reports only an Overlay invalidation.
    ui.scroll_only(&mut root, 2100.0);
    let redraw = ui.next_redraw();
    app.log.clear();
    let pass = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(pass, Invalidation::Overlay);
    assert_eq!(ui.root_layouts, ui.frame_start);
    assert!(ui.frame_walks()[0].local);

    let rows: Vec<(u32, Rectangle)> = {
        let (origin, tree) = ui.list();
        ui.core()
            .window
            .iter()
            .zip(&tree.children)
            .map(|(slot, child)| {
                let position = Point::ORIGIN + origin + child.translation;
                (slot.key, Rectangle::new(position, child.size))
            })
            .collect()
    };
    assert_eq!(rows.first().map(|(key, _)| *key), Some(46));

    // Every row of the final window got this redraw once, with the list as its
    // parent and its new bounds. Evicted rows got none.
    let redraws: Vec<(u32, Option<Rectangle>, Rectangle)> = rows
        .iter()
        .map(|&(key, bounds)| (key, Some(list), bounds))
        .collect();
    assert_eq!(*app.log.redraws.borrow(), redraws);

    // The overlays rebuilt inside the pass come from the new trees.
    let overlays = app.log.overlays.borrow().clone();
    assert_eq!(overlays[overlays.len() - rows.len()..], rows[..]);

    let frame = ui.complete(&mut root, &redraw, vec![pass]);
    assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);
    assert_eq!(frame.root_layouts, 0);

    // Draw.
    for drawn in app.log.drawn.borrow().iter() {
        let (_, bounds) = rows.iter().find(|(key, _)| *key == drawn.key).unwrap();
        assert_eq!(drawn.bounds, *bounds);
    }

    app.log.assert_covered(2100.0, 2700.0);

    // Operate.
    app.log.clear();
    ui.operate(&mut root, &mut Visits::default());
    assert_eq!(*app.log.operated.borrow(), rows);

    // Mouse interaction over row 60, which the local walk built (screen y 400).
    ui.cursor = mouse::Cursor::Available(Point::new(100.0, 400.0));
    assert_eq!(ui.mouse_interaction(&root), mouse::Interaction::Pointer);

    // The siblings and the list kept their geometry.
    let (origin, tree) = ui.list();
    assert_eq!(Rectangle::new(Point::ORIGIN + origin, tree.size), list);
    assert_eq!(app.log.footer.get(), footer);
    assert_eq!(app.log.sibling_layouts.get(), siblings);
}

#[test]
fn local_eviction_matches_full_walk() {
    // FP7: the same trace with local relayout on and off gives the same windows,
    // translations, tree identities and geometry.
    #[derive(Debug, PartialEq)]
    struct State {
        positions: Vec<usize>,
        keys: Vec<u32>,
        translations: Vec<Vector>,
        serials: Vec<u64>,
        anchor: (Option<u32>, Option<f64>),
        drift: f64,
        total: u64,
        size: Size,
    }

    fn state(ui: &Harness) -> State {
        let (_, tree) = ui.list();
        let core = ui.core();

        State {
            positions: ui.positions(),
            keys: ui.keys(),
            translations: tree
                .children
                .iter()
                .map(|child| child.translation)
                .collect(),
            serials: tree
                .children
                .iter()
                .map(|child| child.state.downcast_ref::<RowState>().serial)
                .collect(),
            anchor: ui.anchor(),
            drift: core.geometry.drift(),
            total: core.geometry.heights.total().to_bits(),
            size: tree.size,
        }
    }

    fn trace(local: bool) -> (Vec<State>, Vec<Walk>) {
        super::local::ENABLED.set(local);

        // Every seventh row is 0 px tall, so first visits change heights.
        let items = (0..400)
            .map(|key| Item {
                key,
                height: Height::Fixed(if key % 7 == 0 { 0.0 } else { 40.0 }),
            })
            .collect();
        let mut app = App::estimated(items, 40.0);
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        let mut states = Vec::new();

        for y in [
            0.0, 600.0, 1200.0, 1800.0, 2400.0, 1800.0, 1300.0, 1360.0, 700.0, 2200.0, 400.0, 0.0,
            30.0, 2000.0,
        ] {
            let total = ui.core().geometry.heights.total().to_bits();
            ui.scroll(&mut root, y);
            states.push(state(&ui));

            // A local walk that kept the geometry did not touch the height sums.
            if ui.frame_walks().iter().all(|walk| walk.local) {
                assert_eq!(ui.core().geometry.heights.total().to_bits(), total);
            }
        }

        drop(root);
        app.items.clear();
        let mut root = app.view();
        ui.rebuild(&mut root);
        ui.frame(&mut root);
        states.push(state(&ui));

        super::local::ENABLED.set(true);
        (states, ui.core().walks.clone())
    }

    let (local, local_walks) = trace(true);
    let (full, full_walks) = trace(false);
    assert_eq!(local, full);

    assert!(local_walks.iter().any(|walk| walk.local));
    assert!(all_full(&full_walks));

    // Eviction happens once per walk, after the final range is known.
    for walk in local_walks.iter().chain(&full_walks) {
        assert_eq!(walk.discarded, 0);
    }

    for walk in local_walks.iter().filter(|walk| walk.local) {
        assert!(walk.measured > 0);
    }

    assert!(local.last().unwrap().keys.is_empty());
}

#[test]
fn child_and_sibling_invalidations_are_not_downgraded() {
    // FP8.
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let view = || app.view();
    let mut root = view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 400.0);

    // A row the local walk builds grows on that same redraw and asks for a root
    // layout, which repairs the geometry before the draw.
    app.log.grow.borrow_mut().insert(55, 80.0);
    let frame = ui.scroll(&mut root, 2000.0);
    assert_eq!(
        frame.passes,
        [Invalidation::Layout(Diff::Skip), Invalidation::None]
    );
    let walks = ui.frame_walks();
    assert!(walks[0].local && walks[0].changed == 0);
    assert!(all_full(&walks[1..]));
    assert_eq!(ui.core().geometry.heights.height(55), 80.0);
    let screen = app.log.screen();
    assert_eq!(screen[&56] - screen[&55], 80.0);
    app.log.assert_covered(2000.0, 2600.0);

    // A retained row does the same.
    app.log.grow.borrow_mut().insert(60, 120.0);
    let frame = ui.scroll(&mut root, 2200.0);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert!(ui.frame_walks()[0].local);
    assert_eq!(ui.core().geometry.heights.height(60), 120.0);
    app.log.assert_covered(2200.0, 2800.0);

    // The header, before the list, asks for a root layout: no local attempt.
    app.log.inject.borrow_mut()[0].push_back(Some(Invalidation::Layout(Diff::Skip)));
    let frame = ui.scroll(&mut root, 2600.0);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert!(all_full(&ui.frame_walks()));
    app.log.assert_covered(2600.0, 3200.0);

    // The footer, after a local walk, asks for one: it is not downgraded.
    app.log.inject.borrow_mut()[1].push_back(Some(Invalidation::Layout(Diff::Skip)));
    let frame = ui.scroll(&mut root, 3000.0);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    let walks = ui.frame_walks();
    assert!(walks[0].local);
    assert!(all_full(&walks[1..]));
    app.log.assert_covered(3000.0, 3600.0);

    // Widgets from the header: the shell rebuilds the view between passes, and the
    // retry with the same timestamp stays full.
    app.log.inject.borrow_mut()[0].push_back(Some(Invalidation::Widgets));
    ui.scroll_only(&mut root, 3400.0);
    let rebuilds = ui.rebuilds;
    let frame = ui.frame_with(&mut root, &view);
    assert_eq!(frame.passes, [Invalidation::Widgets, Invalidation::None]);
    assert_eq!(ui.rebuilds, rebuilds + 1);
    assert!(all_full(&ui.frame_walks()));
    app.log.assert_covered(3400.0, 4000.0);

    // Widgets from the footer after a local walk.
    app.log.inject.borrow_mut()[1].push_back(Some(Invalidation::Widgets));
    ui.scroll_only(&mut root, 3800.0);
    let frame = ui.frame_with(&mut root, &view);
    assert_eq!(frame.passes, [Invalidation::Widgets, Invalidation::None]);
    assert_eq!(ui.rebuilds, rebuilds + 2);
    assert!(ui.frame_walks()[0].local);
    app.log.assert_covered(3800.0, 4400.0);

    // At the retry limit: the third pass asks for Widgets, and the frame is drawn
    // without a rebuild or a fourth pass. The list's geometry is stable, so the
    // drawn rows still cover the viewport.
    app.log.inject.borrow_mut()[1].extend([
        Some(Invalidation::Overlay),
        Some(Invalidation::Overlay),
        Some(Invalidation::Widgets),
    ]);
    let frame = ui.frame_with(&mut root, &view);
    assert_eq!(
        frame.passes,
        [
            Invalidation::Overlay,
            Invalidation::Overlay,
            Invalidation::Widgets
        ]
    );
    assert_eq!(ui.rebuilds, rebuilds + 2);
    app.log.assert_covered(3800.0, 4400.0);
}

/// Asserts the frame just drawn at the end of the list: the translation is the
/// largest one the final content height allows, the footer ends at the bottom of
/// the viewport, the drawn rows cover the viewport down to the footer, and the end
/// of the list is pinned.
fn assert_drawn_at_end(ui: &mut Harness, root: &mut Root<'_>, log: &Log) {
    let content = ui.tree.children[0].size.height;
    let translation = ui.translation(root);
    assert_eq!(translation, content - WINDOW.height);

    let footer = log.footer.get().expect("the footer is drawn");
    assert_eq!(footer.y + footer.height, content);
    assert_eq!(footer.y + footer.height - translation, WINDOW.height);

    log.assert_covered(translation, footer.y);
    assert!(ui.end_pinned());
    ui.assert_invariants();
}

#[test]
fn end_of_list_keeps_its_place_while_rows_are_measured() {
    // Review P3-R1 F1: a jump to the largest translation, over rows estimated very
    // differently from their real height. The end of the list is pinned, so measuring
    // the rows in view moves only the content above them, the content height and the
    // largest translation stay put, and the walk goes up from the end, keeping every
    // row it builds.
    let jump = |items: Vec<Item>, estimate: f32| {
        let app = App::estimated(items, estimate);
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);

        let content = ui.tree.children[0].size.height;
        let frame = ui.scroll(&mut root, content - WINDOW.height);
        assert!(frame.passes.len() <= 3);
        assert_eq!(frame.passes.last(), Some(&Invalidation::None));
        assert_eq!(ui.tree.children[0].size.height, content);

        let walks = ui.frame_walks();
        assert!(walks.iter().all(|walk| walk.discarded == 0));
        assert_drawn_at_end(&mut ui, &mut root, &app.log);

        let built = walks.iter().map(|walk| walk.built).sum::<usize>();
        let frame = ui.frame(&mut root);
        assert_eq!(frame.passes, [Invalidation::None]);
        built
    };

    // The reviewer's case: 1 px rows estimated at 1000 px. Without the end pin each
    // pass shortened the list below the translation of the next one, and the last
    // drawn frame was blank.
    let built = jump(rows(0..2000, 1.0), 1000.0);
    assert_eq!(built, 550 + 4);

    // A realistic overestimate: 20 px rows estimated at 40 px.
    let built = jump(rows(0..2000, 20.0), 40.0);
    assert_eq!(built, 28 + 4);

    // Growth: 40 px rows estimated at 20 px. Fourteen real rows cover the 550 px of
    // list above the footer.
    let built = jump(rows(0..2000, 40.0), 20.0);
    assert_eq!(built, 14 + 4);
}

#[test]
fn growth_at_the_end_moves_the_content_above() {
    // Review P3-R1 F1: a list shorter than the viewport below a 1000 px header,
    // scrolled to the end. When its last row grows, the end stays where it is and the
    // rows above move up, so the footer stays at the bottom of the viewport.
    let mut app = App::estimated(rows(0..20, 40.0), 40.0);
    app.header = 1000.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 1250.0);
    assert_drawn_at_end(&mut ui, &mut root, &app.log);
    assert_eq!(app.log.screen()[&19], 510.0);
    drop(root);

    app.items[19].height = Height::Fixed(140.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), -100.0);

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert_drawn_at_end(&mut ui, &mut root, &app.log);
    assert_eq!(app.log.screen()[&19], 410.0);
    assert_eq!(app.log.screen()[&18], 370.0);

    // Scrolling back to the start resolves the drift, as everywhere else.
    ui.scroll(&mut root, 0.0);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(ui.anchor(), (None, None));
}

#[test]
fn empty_visible_region_ends_the_walk() {
    // Review P3-R1 F1 follow-up: a 0 px tall scrollable scrolled to a row boundary.
    // No row shows content in the empty visible region, and the walk still ends.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.height = Length::Fixed(0.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let frame = ui.scroll(&mut root, 4000.0);
    assert!(frame.passes.len() <= 3);
    assert_eq!(ui.anchor(), (Some(100), Some(4000.0)));
    assert!(ui.positions().contains(&100));
    ui.assert_invariants();
}

#[test]
fn sticky_group_overlay_follows_a_local_shift() {
    // Review P3-R1 F2: item 20 is `column![sticky(header), items..]`, a 30 px header
    // and five 40 px items, at content y 800..1030. The fork's real `Sticky`, with
    // the harness running overlays in the runtime's order.
    let mut items = rows(0..100, 40.0);
    items[20].height = Height::Group {
        header: 30.0,
        items: 5,
        item: 40.0,
    };
    let app = App::estimated(items, 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3000.0);
    assert!(!ui.keys().contains(&20));
    assert_eq!(ui.overlay, None);

    let group = Rectangle::new(Point::new(0.0, 800.0), Size::new(800.0, 230.0));
    let stuck = Rectangle::new(Point::ORIGIN, Size::new(800.0, 30.0));

    // Scrolling to 850 puts the header above the viewport while the group is in it.
    // The cached window shift is local, and its new rows get this redraw once: the
    // group's items with the group as their parent, and the sticky, which sticks and
    // asks for new overlays instead of passing the redraw on.
    ui.scroll_only(&mut root, 850.0);
    let redraw = ui.next_redraw();
    app.log.clear();
    let pass = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(pass, Invalidation::Overlay);
    assert!(ui.frame_walks()[0].local);
    assert!(ui.overlay.is_some());

    let redraws = app.log.redraws.take();
    assert!(redraws.iter().all(|&(key, _, _)| key != 20));

    for index in 1..=5 {
        let key = 20_000 + index;
        let bounds = Rectangle::new(
            Point::new(0.0, 830.0 + 40.0 * (index - 1) as f32),
            Size::new(800.0, 40.0),
        );
        let received: Vec<_> = redraws.iter().filter(|redraw| redraw.0 == key).collect();
        assert_eq!(received, [&(key, Some(group), bounds)]);
    }

    // The next pass delivers the redraw to the overlay first: the header, clamped to
    // the top of the viewport in screen coordinates, with its group as its parent.
    let frame = ui.complete(&mut root, &redraw, vec![pass]);
    assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);
    assert_eq!(frame.root_layouts, 0);
    assert_eq!(
        *app.log.redraws.borrow().first().unwrap(),
        (20, Some(group), stuck)
    );

    // The base tree does not draw the stuck header and the overlay draws it there.
    let drawn: Vec<Drawn> = app
        .log
        .drawn
        .borrow()
        .iter()
        .copied()
        .filter(|drawn| drawn.key == 20)
        .collect();
    assert_eq!(drawn.len(), 1);
    assert_eq!((drawn[0].bounds, drawn[0].viewport), (stuck, stuck));

    // Over the stuck header the overlay owns the interaction, and the base tree's
    // cursor levitates.
    let position = Point::new(100.0, 10.0);
    ui.cursor = mouse::Cursor::Available(position);
    ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::CursorMoved { position }),
    );
    assert_eq!(ui.overlay, Some(mouse::Interaction::Pointer));
    assert!(ui.base_cursor().position().is_none());
    assert_eq!(ui.mouse_interaction(&root), mouse::Interaction::Pointer);

    // Operations reach the header through the overlay only.
    app.log.clear();
    ui.operate(&mut root, &mut Visits::default());
    let operated: Vec<(u32, Rectangle)> = app
        .log
        .operated
        .borrow()
        .iter()
        .copied()
        .filter(|(key, _)| *key == 20)
        .collect();
    assert_eq!(operated, [(20, stuck)]);
    ui.cursor = mouse::Cursor::Unavailable;

    // On the last permitted pass: two passes go by, the parent moves the viewport, and
    // the third pass shifts the window locally and creates the overlay. It is drawn
    // with that frame.
    ui.scroll(&mut root, 3000.0);
    assert_eq!(ui.overlay, None);
    app.log.inject.borrow_mut()[1]
        .extend([Some(Invalidation::Overlay), Some(Invalidation::Overlay)]);

    let redraw = ui.next_redraw();
    let first = ui.update(&mut root, &redraw).invalidation;
    let second = ui.update(&mut root, &redraw).invalidation;
    ui.scroll_only(&mut root, 850.0);
    let third = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(
        [first, second, third],
        [
            Invalidation::Overlay,
            Invalidation::Overlay,
            Invalidation::Overlay
        ]
    );
    assert!(ui.frame_walks().iter().any(|walk| walk.local));

    ui.complete(&mut root, &redraw, vec![first, second, third]);
    let drawn: Vec<Rectangle> = app
        .log
        .drawn
        .borrow()
        .iter()
        .filter(|drawn| drawn.key == 20)
        .map(|drawn| drawn.bounds)
        .collect();
    assert_eq!(drawn, [stuck]);
}

#[test]
fn end_is_visible_as_the_parent_reports_it() {
    // Review P3-R2 F1: the list is the scrollable's whole content, with no footer and
    // no bottom padding. The first frame measures rows 0..=1027 as 1 px and leaves 965
    // rows at a fractional estimate, so the exact height is not an f32.
    let jump = |estimate: f32, exact: f64, reported: f32| {
        let mut app = App::estimated(rows(0..1993, 1.0), estimate);
        app.direct = true;
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);

        assert_eq!(ui.core().geometry.total(), exact);
        let content = ui.tree.children[0].size.height;
        assert_eq!(content, reported);

        let frame = ui.scroll(&mut root, content - WINDOW.height);
        assert!(frame.passes.len() <= 3);
        assert_eq!(frame.passes.last(), Some(&Invalidation::None));
        assert!(ui.end_pinned());

        // The list kept its reported height, the translation is the largest it
        // allows, and the drawn rows cover the viewport down to the list's end.
        assert_eq!(ui.tree.children[0].size.height, content);
        let translation = ui.translation(&mut root);
        assert_eq!(translation, content - WINDOW.height);
        app.log.assert_covered(translation, content);
        ui.assert_invariants();

        let frame = ui.frame(&mut root);
        assert_eq!(frame.passes, [Invalidation::None]);
    };

    // The reviewer's case: the reported height rounds down.
    jump(1000.0 + 1.0 / 256.0, 966_031.769_531_25, 966_031.75);

    // The reported height rounds up.
    jump(
        1000.0 + 3.0 / 256.0,
        966_039.308_593_75,
        966_039.0 + 5.0 / 16.0,
    );
}

#[test]
fn empty_rows_at_the_end_agree_with_coverage() {
    // Review P3-R2 F2: rows 100..=199 are 0 px tall but estimated at 40 px, and the
    // list is the scrollable's whole content. The jump to the estimated end measures
    // them, and the walk keeps only the empty rows coverage counts as anticipation, so
    // later frames are idle.
    let items = (0..200)
        .map(|key| Item {
            key,
            height: Height::Fixed(if key < 100 { 40.0 } else { 0.0 }),
        })
        .collect();
    let mut app = App::estimated(items, 40.0);
    app.direct = true;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let frame = ui.scroll(&mut root, 7400.0);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert!(ui.end_pinned());
    assert_eq!(ui.positions(), range(81..=103));
    app.log.assert_covered(7400.0, 8000.0);

    for _ in 0..3 {
        let frame = ui.frame(&mut root);
        assert_eq!(frame.passes, [Invalidation::None]);
        assert!(ui.frame_walks().is_empty());
    }

    ui.assert_invariants();
}

#[test]
fn structural_changes_keep_visible_items_while_the_end_is_pinned() {
    // Review P3-R2 F3: 100 rows of 40 px and the footer, scrolled to the largest
    // translation 3450, so the end is pinned and row 87 is at screen y 30. Keyed
    // changes keep the visible surviving rows still, wherever they are.
    let pinned = || {
        let app = App::estimated(rows(0..100, 40.0), 40.0);
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);
        ui.scroll(&mut root, 3450.0);
        assert!(ui.end_pinned());
        assert_eq!(app.log.screen()[&87], 30.0);
        drop(root);
        (app, ui)
    };

    // An item appended at the end.
    let (mut app, mut ui) = pinned();
    app.items.push(rows(1000..1001, 40.0)[0]);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(ui.screen_y(&mut root, 87), 30.0);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&87], 30.0);
    ui.assert_invariants();
    drop(root);

    // An item prepended at the start.
    let (mut app, mut ui) = pinned();
    app.items.insert(0, rows(1000..1001, 40.0)[0]);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), -40.0);
    assert_eq!(ui.screen_y(&mut root, 87), 30.0);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&87], 30.0);
    drop(root);

    // The last five rows removed: the visible rows stay until the scrollable clamps
    // its translation to the shorter content, and the end is pinned again.
    let (mut app, mut ui) = pinned();
    app.items.truncate(95);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(ui.screen_y(&mut root, 87), 30.0);
    ui.frame(&mut root);
    assert_drawn_at_end(&mut ui, &mut root, &app.log);
    drop(root);

    // Every item replaced: nothing survives, so the drift is folded and the two new
    // rows are where they belong. The walk then pins the new end, which is in view.
    let (mut app, mut ui) = pinned();
    app.items = rows(2000..2002, 40.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(ui.anchor(), (None, Some(80.0)));
    assert_eq!(ui.list().1.size.height, 80.0);
    ui.frame(&mut root);
    assert_eq!(ui.anchor(), (None, None));
    assert_eq!(app.log.drawn_keys(), [2000, 2001]);
    app.log.assert_covered(0.0, 80.0);
}

/// A widget whose overlay keeps its interaction in the overlay object itself: it
/// shows a pointer until it gets an event.
struct Flagged;

impl Meta for Flagged {}

impl Widget<(), Theme, ()> for Flagged {
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &(), limits: &layout::Limits) {
        tree.size = limits.max;
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut (),
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }

    fn overlay<'b>(
        &'b mut self,
        _tree: &'b mut Tree,
        _layout: Layout,
        _renderer: &(),
        _viewport: &Rectangle,
        _translation: Vector,
        _window: Size,
    ) -> Vec<iced::advanced::overlay::Element<'b, (), Theme, ()>> {
        vec![iced::advanced::overlay::Element::new(Box::new(Flag {
            pointer: true,
        }))]
    }
}

struct Flag {
    pointer: bool,
}

impl iced::advanced::Overlay<(), Theme, ()> for Flag {
    fn draw(
        &self,
        _renderer: &mut (),
        _theme: &Theme,
        _style: &renderer::Style,
        _cursor: mouse::Cursor,
    ) {
    }

    fn update(
        &mut self,
        _event: &Event,
        _cursor: mouse::Cursor,
        _renderer: &(),
        _shell: &mut Shell<'_, ()>,
    ) {
        self.pointer = false;
    }

    fn mouse_interaction(&self, _cursor: mouse::Cursor, _renderer: &()) -> mouse::Interaction {
        if self.pointer {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }
}

#[test]
fn harness_keeps_the_overlays_that_got_the_event() {
    // Review P3-R2 F4: without an invalidation, the runtime computes the interaction
    // on the overlays that just handled the event, not on rebuilt ones.
    let log = Rc::new(Log::default());
    let mut root: Root<'_> = Flagged.boxed();
    let mut ui = Harness::new(&mut root, &log);

    let position = Point::new(10.0, 10.0);
    ui.cursor = mouse::Cursor::Available(position);
    let updated = ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::CursorMoved { position }),
    );
    assert_eq!(updated.invalidation, Invalidation::None);
    assert_eq!(ui.overlay, Some(mouse::Interaction::None));
    assert_eq!(ui.base_cursor(), ui.cursor);
}

#[test]
fn harness_operates_on_the_base_tree_inside_traverse() {
    // Review P3-R2 F5: an operation that never traverses sees nothing of the base
    // tree, as in the runtime.
    #[derive(Default)]
    struct Shallow(usize);

    impl Operation for Shallow {
        fn traverse(&mut self, _operate: &mut dyn FnMut(&mut dyn Operation)) {}

        fn container(&mut self, _id: Option<&Id>, _bounds: Rectangle, _viewport: &Rectangle) {
            self.0 += 1;
        }

        fn scrollable(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            _content: Size,
            _translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            self.0 += 1;
        }
    }

    let app = App::estimated(rows(0..100, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let mut shallow = Shallow::default();
    ui.operate(&mut root, &mut shallow);
    assert_eq!(shallow.0, 0);

    // An operation that traverses still reaches the list and its rows.
    let mut visits = Visits::default();
    ui.operate(&mut root, &mut visits);
    assert!(visits.containers.iter().any(|(id, _)| id == &Some(LIST)));
}

/// The distance from `value` to the next larger `f32`.
fn ulp32(value: f32) -> f32 {
    f32::from_bits(value.abs().to_bits() + 1) - value.abs()
}

/// Checks a frame drawn after a jump: the translation is within the largest one the
/// final content allows, the drawn rows cover the viewport wherever the list has
/// content there (with the `f32` rounding of row positions at this magnitude), and
/// returns the translation.
fn assert_frame_fits(ui: &mut Harness, root: &mut Root<'_>, log: &Log, case: &str) -> f32 {
    let height = ui.window.height;
    let content = ui.tree.children[0].size.height;
    let translation = ui.translation(root);
    assert!(
        translation <= (content - height).max(0.0),
        "{case}: translation {translation} past the largest one for content {content}"
    );

    let (origin, list) = ui.list();
    let start = origin.y + (ui.core().geometry.base() as f32).max(0.0);
    let end = origin.y + list.size.height;
    let from = translation.max(start);
    let to = (translation + height).min(end);

    if to > from {
        // Rows are placed on the 1/256 px grid of the heights, half a unit from their
        // real height at most, and their positions are rounded to `f32`.
        let slack = (4.0 * ulp32(to)).max(1.0 / 256.0);
        log.assert_covered_within(from, to - slack, slack, case);
    }

    translation
}

#[test]
fn end_stays_pinned_one_ulp_short_of_the_viewport() {
    // Review P3-R3 F1: the scrollable's largest translation plus the viewport height
    // ends one `f32` ulp above the reported end of the list.
    let mut app = App::estimated(rows(0..1985, 1.0), 1000.0 + 3.0 / 256.0);
    app.direct = true;
    let mut root = app.view();
    let mut ui = Harness::sized(&mut root, &app.log, Size::new(800.0, 600.031_25));
    ui.frame(&mut root);

    assert_eq!(ui.core().geometry.total(), 958_039.214_843_75);
    let content = ui.tree.children[0].size.height;
    assert_eq!(content, 958_039.0 + 3.0 / 16.0);

    let frame = ui.scroll(&mut root, content - 600.031_25);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert!(ui.end_pinned());
    assert_eq!(ui.tree.children[0].size.height, content);

    let translation = assert_frame_fits(&mut ui, &mut root, &app.log, "counterexample");
    assert_eq!(translation, 957_439.0 + 1.0 / 8.0);
    ui.assert_invariants();

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
}

#[test]
fn jumps_near_the_end_stay_covered() {
    // Review P3-R3 F1: seeded random lists, viewports and estimates, jumping to the
    // parent's largest translation and to offsets just above it.
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }

        fn unit(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1_u64 << 24) as f32
        }
    }

    let mut random = Random(0x5EED_1157_0F2A_B3C4);

    for case in 0..400 {
        let direct = random.below(2) == 0;
        let height = match random.below(3) {
            0 => 600.0,
            1 => (100 * 32 + random.below(800 * 32)) as f32 / 32.0,
            _ => 100.0 + random.unit() * 800.0,
        };

        let real = [0.37, 1.0, 2.5, 13.0, 40.0, 97.3][random.below(6) as usize];
        let estimate = match random.below(4) {
            0 => None,
            1 => Some(real * (0.5 + 2.0 * random.unit())),
            2 => Some(real * 1000.0 + random.below(256) as f32 / 256.0),
            _ => Some((real / 20.0).max(1.0)),
        };

        // Up to a few million px, as estimated or real.
        let largest = estimate.unwrap_or(32.0).max(real * 2.7);
        let len = (50 + random.below(5000))
            .min((3.0e6 / largest) as u64)
            .max(1) as u32;

        let items = (0..len)
            .map(|key| {
                let height = match random.below(10) {
                    0 => 0.0,
                    1 => real * 2.7,
                    _ => real,
                };

                Item {
                    key,
                    height: Height::Fixed(height),
                }
            })
            .collect();

        let mut app = App::new(items);
        app.direct = direct;
        app.estimate = estimate;

        let case = format!(
            "case {case}: direct {direct}, viewport {height}, real {real}, \
             estimate {estimate:?}, {len} rows"
        );

        let mut root = app.view();
        let mut ui = Harness::sized(&mut root, &app.log, Size::new(800.0, height));
        ui.frame(&mut root);

        for jump in 0..3 {
            let largest = (ui.tree.children[0].size.height - height).max(0.0);
            let offset = match jump {
                0 => largest,
                _ => (largest - random.unit() * 2.0 * height).max(0.0),
            };

            let frame = ui.scroll(&mut root, offset);
            let case = format!("{case}, jump {jump} to {offset}");
            assert!(
                frame.passes.last() == Some(&Invalidation::None) || frame.root_layouts == 0,
                "{case}: {frame:?}"
            );
            assert_frame_fits(&mut ui, &mut root, &app.log, &case);
            ui.assert_invariants();

            let frame = ui.frame(&mut root);
            assert_eq!(frame.passes, [Invalidation::None], "{case}: next frame");
        }
    }
}

/// A widget that opens an overlay with a pointer when it gets an event, and has no
/// interaction itself.
struct Opener;

#[derive(Default)]
struct Opened(bool);

impl Meta for Opener {}

impl Widget<(), Theme, ()> for Opener {
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Opened>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Opened::default())
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &(), limits: &layout::Limits) {
        tree.size = limits.max;
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut (),
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        _event: &Event,
        _layout: Layout,
        _cursor: mouse::Cursor,
        _renderer: &(),
        shell: &mut Shell<'_, ()>,
        _viewport: &Rectangle,
    ) {
        let opened = tree.state.downcast_mut::<Opened>();

        if !opened.0 {
            opened.0 = true;
            shell.invalidate_overlay();
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        _layout: Layout,
        _renderer: &(),
        _viewport: &Rectangle,
        _translation: Vector,
        _window: Size,
    ) -> Vec<iced::advanced::overlay::Element<'b, (), Theme, ()>> {
        if tree.state.downcast_ref::<Opened>().0 {
            vec![iced::advanced::overlay::Element::new(Box::new(Flag {
                pointer: true,
            }))]
        } else {
            Vec::new()
        }
    }
}

#[test]
fn update_reports_the_runtime_interaction() {
    // Review P3-R3 F2: the runtime returns the interaction of the overlay phase, which
    // had no overlay, and the base tree's under the cursor it gave the base tree. The
    // overlay the base update opened only shows in later queries.
    let log = Rc::new(Log::default());
    let mut root: Root<'_> = Opener.boxed();
    let mut ui = Harness::new(&mut root, &log);

    let position = Point::new(10.0, 10.0);
    ui.cursor = mouse::Cursor::Available(position);
    let updated = ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::CursorMoved { position }),
    );
    assert_eq!(updated.invalidation, Invalidation::Overlay);
    assert_eq!(updated.interaction, mouse::Interaction::None);
    assert_eq!(ui.mouse_interaction(&root), mouse::Interaction::Pointer);
}

/// The screen y of the bottom of a drawn row.
fn screen_bottom(log: &Log, key: u32) -> f32 {
    let drawn = log.drawn.borrow();
    let row = drawn
        .iter()
        .find(|drawn| drawn.key == key)
        .expect("the row is drawn");

    row.bounds.y + row.bounds.height - row.viewport.y
}

fn scroll_task(key: u32, align: Align, animation: Animation) -> ScrollTo<u32> {
    ScrollTo::new(LIST, key, align, animation)
}

/// The list scrolled to `y` in a direct scrollable, 100 rows of 40 px.
fn direct(items: Vec<Item>) -> App {
    let mut app = App::estimated(items, 40.0);
    app.direct = true;
    app
}

#[test]
fn named_scroller_is_not_an_earlier_sibling() {
    // R1 H1: an anonymous scrollable at y 0 with 40,000 px of content comes before the
    // list at y 220, inside the named outer scroller.
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let view = || -> Root<'_> {
        let inner = scrollable(space().height(40_000)).height(200).width(Fill);

        scrollable(column![inner, space().height(20), app.list()].width(Fill))
            .id(SCROLL)
            .width(Fill)
            .height(Fill)
            .boxed()
    };

    let mut root = view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(50, Align::Start, Animation::Instant));
    ui.frame(&mut root);
    assert_eq!(
        ui.translations(&mut root),
        [(Some(SCROLL), 2220.0), (None, 0.0)]
    );
    assert_eq!(app.log.screen()[&50], 0.0);
}

#[test]
fn only_named_scrollable_moves_with_equal_bounds() {
    // R2-6: the list scrolls inside an anonymous scrollable with the bounds of the
    // named one around it, whose content is that scrollable and a 100 px footer.
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let view = || -> Root<'_> {
        let inner = scrollable(app.list()).width(Fill).height(600);

        scrollable(column![inner, space().height(100)].width(Fill))
            .id(SCROLL)
            .width(Fill)
            .height(Fill)
            .boxed()
    };

    let mut root = view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    // Only the named one moves, as far as it can.
    ui.run_task(&mut root, scroll_task(50, Align::Start, Animation::Instant));
    ui.frame(&mut root);
    assert_eq!(
        ui.translations(&mut root),
        [(Some(SCROLL), 100.0), (None, 0.0)]
    );
    drop(root);

    // A scroller that is not there, or reported after the list, is a no-op that
    // changes nothing in the list.
    let unchanged = |ui: &mut Harness, root: &mut Root<'_>| {
        let before = (ui.anchor(), ui.core().geometry.drift());
        ui.run_task(root, scroll_task(500, Align::Start, Animation::Instant));
        let core = ui.core();
        assert!(core.resolved.is_empty());
        assert!(core.landing.is_none() && !core.geometry.block_next_redraw);
        assert_eq!((ui.anchor(), ui.core().geometry.drift()), before);
        ui.frame(root);
        assert!(ui.translations(root).iter().all(|(_, y)| *y == 0.0));
    };

    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.scroller_id = Some(Id::new("missing"));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    unchanged(&mut ui, &mut root);
    drop(root);

    const LATER: Id = Id::new("later");
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.scroller_id = Some(LATER);
    let view = || -> Root<'_> {
        let later = scrollable(space().height(1000)).id(LATER).height(100);

        scrollable(column![app.list(), later].width(Fill))
            .id(SCROLL)
            .width(Fill)
            .height(Fill)
            .boxed()
    };
    let mut root = view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    unchanged(&mut ui, &mut root);
    drop(root);

    // R1 M2, R2-7: a horizontal scroller is outside the contract. The callbacks give
    // no direction, so the task cannot tell, and the vertical offset has no effect.
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let view = || -> Root<'_> {
        scrollable(app.list())
            .horizontal()
            .id(SCROLL)
            .width(Fill)
            .height(Fill)
            .boxed()
    };
    let mut root = view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.run_task(
        &mut root,
        scroll_task(500, Align::Start, Animation::Instant),
    );
    ui.frame(&mut root);
    assert_eq!(ui.translations(&mut root), [(Some(SCROLL), 0.0)]);
}

#[test]
fn settle_rebases_anchor_and_uses_unclamped_absolute_offset() {
    // R1 H5, R2-2: 14,000 px of rows of which the list reports 10,000, with drift
    // -4000, the viewport at 9000.
    let mut app = direct(rows(1000..1250, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 9000.0);
    drop(root);

    app.items.splice(0..0, rows(0..100, 40.0));
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(ui.core().geometry.drift(), -4000.0);
    assert_eq!(ui.tree.children[0].size.height, 10_000.0);
    assert_eq!(app.log.screen()[&1225], 0.0);

    // The drift is removed and the anchor's target moves with it to 13,000. The
    // offset is stored unclamped and reached once layout catches up.
    ui.run_task(&mut root, Settle::new(LIST));
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(ui.anchor(), (Some(1225), Some(13_000.0)));
    assert!(ui.core().landing.is_some() && ui.core().geometry.block_next_redraw);

    let frame = ui.frame(&mut root);
    assert!(all_full(&ui.frame_walks()));
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert_eq!(ui.translation(&mut root), 13_000.0);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(app.log.screen()[&1225], 0.0);
    assert!(ui.core().pending_landing().is_none());
    ui.assert_invariants();
    drop(root);

    // Positive drift: rows above were removed.
    let mut app = direct(rows(1000..1250, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 9000.0);
    drop(root);

    app.items.drain(0..100);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(ui.core().geometry.drift(), 4000.0);

    ui.run_task(&mut root, Settle::new(LIST));
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 5000.0);
    assert_eq!(app.log.screen()[&1225], 0.0);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    drop(root);

    // Without a scroller nothing changes.
    let mut app = direct(rows(1000..1250, 40.0));
    app.scroller_id = None;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 9000.0);
    drop(root);

    app.items.splice(0..0, rows(0..100, 40.0));
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);

    let anchor = ui.anchor();
    ui.run_task(&mut root, Settle::new(LIST));
    assert_eq!(ui.core().geometry.drift(), -4000.0);
    assert_eq!(ui.anchor(), anchor);
    assert!(ui.core().landing.is_none() && !ui.core().geometry.block_next_redraw);
}

#[test]
fn instant_scroll_measures_destination_before_commit() {
    // R2-5, R3-3: row 0 was measured at 40 px and is now 2000 px tall, off-window.
    let mut app = direct(rows(0..100, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3000.0);
    drop(root);

    app.items[0].height = Height::Fixed(2000.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(0, Align::End, Animation::Instant));
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 1400.0);
    assert_eq!(screen_bottom(&app.log, 0), 600.0);
    drop(root);

    // A target taller than the viewport: its bottom or its top is aligned.
    app.items[50].height = Height::Fixed(2000.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(50, Align::End, Animation::Instant));
    ui.frame(&mut root);
    assert_eq!(screen_bottom(&app.log, 50), 600.0);

    ui.run_task(&mut root, scroll_task(50, Align::Start, Animation::Instant));
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&50], 0.0);
    drop(root);

    // Unreachable alignments of the first rows clamp at offset 0.
    let app = direct(rows(0..100, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 2000.0);

    ui.run_task(&mut root, scroll_task(1, Align::End, Animation::Instant));
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(screen_bottom(&app.log, 1), 80.0);

    ui.scroll(&mut root, 2000.0);
    ui.run_task(&mut root, scroll_task(2, Align::Center, Animation::Instant));
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 0.0);
    ui.assert_invariants();
}

#[test]
fn instant_scroll_survives_two_layouts_before_redraw() {
    // R3-1: 100 rows of 40 px, top 3000, row 99 now 2000 px tall. End gives 5360.
    let mut app = direct(rows(0..100, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3000.0);
    drop(root);

    app.items[99].height = Height::Fixed(2000.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(99, Align::End, Animation::Instant));
    assert!(ui.core().landing.is_some());
    assert_eq!(ui.anchor(), (Some(99), Some(3960.0)));

    // A message lays the list out again before the redraw. The landing holds.
    drop(root);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert!(ui.core().landing.is_some());
    assert_eq!(ui.anchor(), (Some(99), Some(3960.0)));
    assert!(ui.core().geometry.block_next_redraw);

    // Every convergence pass is on the full path, and the frame drawn is exact.
    let frame = ui.frame(&mut root);
    assert!(frame.passes.len() <= 3);
    assert!(all_full(&ui.frame_walks()));
    assert_eq!(ui.translation(&mut root), 5360.0);
    assert_eq!(screen_bottom(&app.log, 99), 600.0);
    assert!(ui.core().pending_landing().is_none());
    ui.assert_invariants();

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
}

#[test]
fn same_update_size_changes_are_in_scroll_resolution() {
    // R3-2 as accepted: row 90 grows to 1000 px in the update that returns the task,
    // so the view rebuild and its layout run before the task.
    let land = |row_99: f32, expected: f32| {
        let mut app = direct(rows(0..100, 40.0));
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);
        ui.scroll(&mut root, 3000.0);
        drop(root);

        app.items[90].height = Height::Fixed(1000.0);
        app.items[99].height = Height::Fixed(row_99);
        let mut root = app.view();
        ui.rebuild(&mut root);
        ui.run_task(&mut root, scroll_task(99, Align::End, Animation::Instant));
        ui.frame(&mut root);

        assert_eq!(ui.translation(&mut root), expected);
        assert_eq!(screen_bottom(&app.log, 99), 600.0);
    };

    land(40.0, 4360.0);
    land(2000.0, 6320.0);
}

#[test]
fn landing_frame_size_change_documents_l10() {
    // R4 F1: the task measures rows 85..=99 at 40 px and answers 3400. Row 90 then
    // grows to 1000 px in the landing redraw, between the landing anchor (row 85)
    // and the target, so the target moves by that change: L10, not exact.
    let app = direct(rows(0..100, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3000.0);

    ui.core_mut().resolved.clear();
    ui.run_task(&mut root, scroll_task(99, Align::End, Animation::Instant));
    let mut resolved = ui.core().resolved.clone();
    resolved.sort_unstable();
    assert_eq!(resolved, range(85..=99));
    assert_eq!(ui.anchor(), (Some(85), Some(3400.0)));

    app.log.grow.borrow_mut().insert(90, 1000.0);
    ui.frame(&mut root);
    let translation = ui.translation(&mut root);
    assert_eq!(translation, 3400.0);

    // Row 99 is now below the viewport, so its bottom comes from the geometry.
    let bottom = ui.core().geometry.bottom_of(99) as f32 - translation;
    assert_eq!(bottom, 1560.0);

    // On a later, ordinary frame a row built by a local walk grows in that redraw:
    // its layout request is kept, and the geometry is repaired before the draw.
    ui.frame(&mut root);
    app.log.grow.borrow_mut().insert(30, 80.0);
    let frame = ui.scroll(&mut root, 1000.0);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert!(ui.frame_walks()[0].local);
    assert_eq!(ui.core().geometry.heights.height(30), 80.0);
    app.log.assert_covered(1000.0, 1600.0);
}

#[test]
fn no_landing_ownership_survives_an_instant_task() {
    // R1 H4: an upward instant request with drift -400 lands exactly. Rule S does not
    // shrink the drift of a landing, and the drift was folded anyway.
    let mut app = direct(rows(1000..1200, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 4000.0);
    drop(root);

    app.items.splice(0..0, rows(0..10, 40.0));
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(ui.core().geometry.drift(), -400.0);

    ui.run_task(
        &mut root,
        scroll_task(1050, Align::Start, Animation::Instant),
    );
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&1050], 0.0);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&1050], 0.0);
    assert!(ui.core().landing.is_none());
    drop(root);

    // R3-4: Start on row 100, already at the top of the viewport. Nothing is pending
    // after the frame: a scroll within the hysteresis needs no layout and refreshes
    // the anchor, and collapsing row 100 leaves row 101 where it is.
    let mut app = direct(rows(0..1000, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 4000.0);

    ui.run_task(
        &mut root,
        scroll_task(100, Align::Start, Animation::Instant),
    );
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 4000.0);
    assert!(ui.core().pending_landing().is_none());

    let frame = ui.scroll(&mut root, 4045.0);
    assert_eq!(frame.root_layouts, 0);
    assert_eq!(ui.anchor(), (Some(101), Some(4040.0)));
    drop(root);

    app.items[100].height = Height::Fixed(20.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.screen_y(&mut root, 101), -5.0);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&101], -5.0);
    drop(root);

    // R2-4: after an instant task and a later scroll, an anticipated row above the
    // viewport grows, and the first visible row stays put. No target of the task is
    // left to hold anything else.
    let mut app = direct(rows(0..1000, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.run_task(
        &mut root,
        scroll_task(500, Align::Start, Animation::Instant),
    );
    ui.frame(&mut root);
    ui.scroll(&mut root, 20_600.0);
    assert_eq!(ui.anchor(), (Some(515), Some(20_600.0)));
    drop(root);

    app.items[512].height = Height::Fixed(80.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&515], 0.0);
}

#[test]
fn instant_far_scroll_all_alignments_with_drift() {
    // 60,010 rows: 32 px real, 40 px estimated, so every destination is unmeasured
    // and measured by the task, and every landing leaves some drift for the next.
    let mut app = direct(rows(0..60_000, 32.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 200_000.0);
    drop(root);

    app.items.splice(0..0, rows(100_000..100_010, 32.0));
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_ne!(ui.core().geometry.drift(), 0.0);

    // Offsets are around 2 million px, where f32 positions are 0.25 px apart.
    let near = |actual: f32, expected: f32| (actual - expected).abs() <= 0.25;

    for (key, align, above) in [
        (52_000, Align::Start, 0),
        (53_000, Align::Center, 9),
        (54_000, Align::End, 18),
        (55_000, Align::Nearest, 18),
        (5_000, Align::Start, 0),
        (4_000, Align::Center, 9),
        (3_000, Align::End, 18),
        (2_000, Align::Nearest, 0),
    ] {
        ui.core_mut().resolved.clear();
        ui.run_task(&mut root, scroll_task(key, align, Animation::Instant));

        // The target, the rows above it, then the rows below it down to the bottom of
        // the destination: 18 rows of 32 px fill 600 px.
        let position = key as usize + 10;
        let expected: Vec<usize> = (position - above..=position)
            .rev()
            .chain(position + 1..=position + 18 - above)
            .collect();
        assert_eq!(ui.core().resolved, expected, "{key} {align:?}");

        let frame = ui.frame(&mut root);
        assert_eq!(frame.passes.last(), Some(&Invalidation::None));

        let top = app.log.screen()[&key];
        let case = format!("{key} {align:?}: top at {top}");

        match align {
            Align::Start => assert!(near(top, 0.0), "{case}"),
            Align::Center => assert!(near(top + 16.0, 300.0), "{case}"),
            Align::End => assert!(near(top + 32.0, 600.0), "{case}"),
            Align::Nearest if above > 0 => assert!(near(top + 32.0, 600.0), "{case}"),
            Align::Nearest => assert!(near(top, 0.0), "{case}"),
        }

        assert_ne!(ui.core().geometry.drift(), 0.0);
        ui.assert_invariants();
    }
}

#[test]
fn smooth_overflow_downgrades_to_instant() {
    // R4 F2: the scroller knows 4000 px of content, row 99 is now 2000 px tall and
    // End needs 5360. A smooth scroll would clamp its target to 3400, so the task
    // jumps instantly, with a landing, and lands exactly.
    let mut app = direct(rows(0..100, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3000.0);
    drop(root);

    app.items[99].height = Height::Fixed(2000.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(99, Align::End, Animation::Smooth));
    assert!(ui.core().landing.is_some());
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 5360.0);
    assert_eq!(screen_bottom(&app.log, 99), 600.0);
    drop(root);

    // Within the known content a smooth scroll stays smooth and holds no landing: the
    // ordinary anchor follows the view on every frame of the animation.
    let app = direct(rows(0..100, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3000.0);

    ui.run_task(&mut root, scroll_task(50, Align::Start, Animation::Smooth));
    assert!(ui.core().landing.is_none());

    let mut frames = 0;

    while ui.translation(&mut root) != 2000.0 {
        ui.frame(&mut root);
        frames += 1;
        assert!(frames < 120, "the animation ends");
        assert!(ui.core().landing.is_none());
        let translation = ui.translation(&mut root);
        app.log.assert_covered(translation, translation + 600.0);
    }

    assert!(frames > 1);
    assert_eq!(app.log.screen()[&50], 0.0);
}

#[test]
fn nearest_includes_header_offset() {
    // R4 F4: row 13 is at list y 520..560, with H 600 and T 0. Below a 100 px header
    // it ends at 660 in the scroller, so Nearest scrolls by 60.
    let mut app = App::estimated(rows(0..100, 40.0), 40.0);
    app.header = 100.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(
        &mut root,
        scroll_task(13, Align::Nearest, Animation::Instant),
    );
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 60.0);
    assert_eq!(screen_bottom(&app.log, 13), 600.0);
    drop(root);

    // Without the header it is in view: no scroll, no landing, no barrier.
    let app = App::estimated(rows(0..100, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(
        &mut root,
        scroll_task(13, Align::Nearest, Animation::Instant),
    );
    assert!(ui.core().landing.is_none() && !ui.core().geometry.block_next_redraw);
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 0.0);
}

#[test]
fn focus_validates_before_changing_global_focus() {
    // R1 M8, R2-8: one 40 px row in a 600 px viewport, the list fits and has no
    // scroller. A materialized item is focused through the global focus chain.
    let mut app = App::estimated(rows(0..1, 40.0), 40.0);
    app.scroller_id = None;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, focusable::focus(OUTSIDE));
    assert_eq!(ui.focused(&mut root), [OUTSIDE]);
    ui.run_task(&mut root, FocusItem::new(LIST, 0_u32, field(0)));
    assert_eq!(ui.focused(&mut root), [field(0)]);

    // An unknown key or list changes nothing.
    ui.run_task(&mut root, focusable::focus(OUTSIDE));
    ui.run_task(&mut root, FocusItem::new(LIST, 7_u32, field(7)));
    assert_eq!(ui.focused(&mut root), [OUTSIDE]);
    ui.run_task(&mut root, FocusItem::new(Id::new("nope"), 0_u32, field(0)));
    assert_eq!(ui.focused(&mut root), [OUTSIDE]);
    drop(root);

    // An item off the window needs the missing scroller: nothing changes.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.scroller_id = None;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, focusable::focus(OUTSIDE));
    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    assert_eq!(ui.focused(&mut root), [OUTSIDE]);
    assert!(ui.core().focus.deferred.is_none());
    assert!(ui.core().resolved.is_empty());
}

#[test]
fn deferred_focus_is_cancelled_only_by_supersession_or_key_removal() {
    // R2-9: focus on an item off the window unfocuses everything else and waits, with
    // the list holding the focus for it. A focus change before the redraw visits the
    // list and cancels it, so the outside widget is the only focused one.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, focusable::focus(OUTSIDE));
    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    assert_eq!(ui.focused(&mut root), [LIST]);
    assert_eq!(ui.core().focus.deferred, Some((500, field(500))));

    ui.run_task(&mut root, focusable::focus(OUTSIDE));
    assert!(ui.core().focus.deferred.is_none());
    ui.frame(&mut root);
    ui.frame(&mut root);
    assert_eq!(ui.focused(&mut root), [OUTSIDE]);

    // Removing the item's key cancels it too. The list keeps the focus it held.
    ui.run_task(&mut root, FocusItem::new(LIST, 700_u32, field(700)));
    assert!(ui.core().focus.deferred.is_some());
    drop(root);

    app.items.retain(|item| item.key != 700);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert!(ui.core().focus.deferred.is_none());
    ui.frame(&mut root);
    assert_eq!(ui.focused(&mut root), [LIST]);
}

#[test]
fn clamped_intermediate_window_keeps_deferred_focus() {
    // R4 F3: 350 rows of 40 px, 14,000 px of which the list reports 10,000, drift
    // -4000, the viewport at 9000. Focusing the last row lands at 13,400, but the
    // first pass can only reach 9400.
    let mut app = direct(rows(1000..1250, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 9000.0);
    drop(root);

    app.items.splice(0..0, rows(0..100, 40.0));
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(ui.core().geometry.drift(), -4000.0);

    ui.run_task(&mut root, FocusItem::new(LIST, 1249_u32, field(1249)));
    assert!(ui.core().focus.deferred.is_some());

    // The intermediate window lacks the item and keeps the deferred focus.
    let redraw = ui.next_redraw();
    let first = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(first, Invalidation::Layout(Diff::Skip));
    assert_eq!(ui.translation(&mut root), 9400.0);
    assert!(!ui.keys().contains(&1249));
    assert!(ui.core().focus.deferred.is_some());

    // The translation catches up, the item is laid out and the focus commits. The
    // deferred focus kept every pass on the full path.
    let frame = ui.complete(&mut root, &redraw, vec![first]);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert!(all_full(&ui.frame_walks()));
    assert_eq!(ui.translation(&mut root), 13_400.0);
    assert!(ui.core().focus.deferred.is_none());
    assert_eq!(ui.focused(&mut root), [field(1249)]);
    assert_eq!(screen_bottom(&app.log, 1249), 600.0);
}

#[test]
fn ordinary_operations_do_not_measure_or_steal_focus() {
    // R1 M5 as narrowed: generic operations reach the materialized rows and the
    // overlays, with their chains followed, and never make the list measure.
    let mut items = rows(0..100, 40.0);
    items[20].height = Height::Group {
        header: 30.0,
        items: 5,
        item: 40.0,
    };
    let app = App::estimated(items, 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    // `focus_next` counts first and focuses in its chained operation. A list
    // without a selection handles no keys and is not a Tab stop (R3-F16).
    ui.run_task(&mut root, focusable::focus_next());
    assert_eq!(ui.focused(&mut root), [OUTSIDE]);
    ui.run_task(&mut root, focusable::focus_next());
    assert_eq!(ui.focused(&mut root), [field(0)]);

    // The stuck header of a group is only reached through its overlay.
    ui.scroll(&mut root, 850.0);
    ui.frame(&mut root);
    assert!(ui.overlay.is_some());
    ui.run_task(&mut root, focusable::focus(field(20)));
    assert_eq!(ui.focused(&mut root), [field(20)]);

    assert!(ui.core().resolved.is_empty());
    assert!(ui.core().landing.is_none());
}

#[test]
fn only_a_selectable_or_focused_list_is_a_tab_stop() {
    // R3-F16: a selectable list is a Tab stop before its rows.
    let app = App::estimated(rows(0..100, 40.0), 40.0).selectable(false);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, focusable::focus_next());
    assert_eq!(ui.focused(&mut root), [OUTSIDE]);
    ui.run_task(&mut root, focusable::focus_next());
    assert_eq!(ui.focused(&mut root), [LIST]);
    ui.run_task(&mut root, focusable::focus_next());
    assert_eq!(ui.focused(&mut root), [field(0)]);
    drop(root);

    // A list without a selection still holds the focus for a waiting focus, and the
    // next Tab takes it back: the list is visited while it is focused, then skipped.
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, focusable::focus(OUTSIDE));
    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    assert_eq!(ui.focused(&mut root), [LIST]);
    assert_eq!(ui.core().focus.deferred, Some((500, field(500))));

    ui.run_task(&mut root, focusable::focus_next());
    let focused = ui.focused(&mut root);
    assert_eq!(focused.len(), 1);
    assert_ne!(focused, [LIST]);
    assert!(!ui.core().focus.focused);
    assert!(ui.core().focus.deferred.is_none());

    ui.run_task(&mut root, focusable::focus(OUTSIDE));
    ui.run_task(&mut root, focusable::focus_next());
    assert_ne!(ui.focused(&mut root), [LIST]);
}

/// The list inside a [`Component`], alone or between siblings in the component.
struct Wrapped<'a> {
    app: &'a App,
    siblings: bool,
}

impl<'a> Component<'a, (), Theme, ()> for Wrapped<'a> {
    type State = ();
    type Event = ();

    fn update(&self, _state: &mut (), _event: (), _renderer: &()) -> Option<()> {
        None
    }

    fn view(&self, _state: &()) -> impl iced::Widget<(), Theme, ()> + 'a {
        if self.siblings {
            let footer = Sibling {
                side: 1,
                height: FOOTER,
                log: Rc::clone(&self.app.log),
            };

            column![space().height(30), self.app.list(), footer]
                .width(Fill)
                .boxed()
        } else {
            self.app.list()
        }
    }
}

fn wrapped(app: &App, siblings: bool) -> Root<'_> {
    scrollable(component(Wrapped { app, siblings }))
        .id(SCROLL)
        .width(Fill)
        .height(Fill)
        .boxed()
}

#[test]
fn list_inside_a_component_shifts_tasks_and_keeps_barriers() {
    for siblings in [false, true] {
        let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
        let mut root = wrapped(&app, siblings);
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);
        ui.scroll(&mut root, 400.0);
        let offset = if siblings { 30.0 } else { 0.0 };

        // A local shift asks for no root layout, but the component rebuilds its view
        // and lays out its whole subtree after any invalidation of its children
        // (`widget/src/component.rs:269-300`): the list walks again, and its
        // siblings in the component are laid out too.
        let siblings_before = app.log.sibling_layouts.get();
        let frame = ui.scroll(&mut root, 2000.0 + offset);
        assert_eq!(frame.root_layouts, 0, "siblings {siblings}");
        let walks = ui.frame_walks();
        assert!(walks[0].local);
        assert!(walks.len() > 1 && !walks[1].local);
        assert_eq!(
            app.log.sibling_layouts.get() > siblings_before,
            siblings,
            "siblings {siblings}"
        );
        app.log.assert_covered(2000.0 + offset, 2600.0 + offset);

        // Tasks reach the list through the component.
        ui.run_task(
            &mut root,
            scroll_task(900, Align::Start, Animation::Instant),
        );
        ui.frame(&mut root);
        assert_eq!(app.log.screen()[&900], 0.0);
        drop(root);

        // An operation-time barrier survives a root layout that skips the list: the
        // component keeps its child's layout while its limits do not change.
        app.items.splice(0..0, rows(10_000..10_010, 40.0));
        let mut root = wrapped(&app, siblings);
        ui.rebuild(&mut root);
        ui.frame(&mut root);
        assert_eq!(ui.core().geometry.drift(), -400.0);

        ui.run_task(&mut root, Settle::new(LIST));
        let walks = ui.walks();
        ui.layout(&mut root);
        assert_eq!(ui.walks(), walks);
        assert!(ui.core().geometry.dirty && ui.core().geometry.block_next_redraw);

        let frame = ui.frame(&mut root);
        assert_eq!(frame.passes.last(), Some(&Invalidation::None));
        assert!(all_full(&ui.frame_walks()));
        assert_eq!(ui.core().geometry.drift(), 0.0);
        assert_eq!(app.log.screen()[&900], 0.0);
        ui.assert_invariants();
    }
}

#[test]
fn public_tasks_build_with_any_message_and_key() {
    // The tasks are `Task`s of the application's message, for any key type.
    #[derive(Debug, Clone)]
    enum Message {}

    let _: iced::Task<Message> = super::scroll_to(LIST, 7_u32, Align::Nearest, Animation::Auto);
    let _: iced::Task<Message> = super::scroll_to(LIST, "key", Align::Center, Animation::Smooth);
    let _: iced::Task<Message> = super::focus(LIST, 7_usize, field(7));
    let _: iced::Task<Message> = super::settle(LIST);

    for align in [Align::Start, Align::Center, Align::End, Align::Nearest] {
        let _ = scroll_task(0, align, Animation::Instant);
    }
}

#[test]
fn deferred_focus_waits_for_its_item_and_blocks_local_walks() {
    // R4 F3, the user's side: focus on an item off the window, then the user scrolls
    // elsewhere before it is reached. The focus waits for the item, keeping every
    // window shift on the full path, and commits when the item is laid out.
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    ui.scroll_only(&mut root, 4000.0);
    ui.frame(&mut root);
    assert!(all_full(&ui.frame_walks()));
    assert!(ui.core().pending_landing().is_none());
    assert!(ui.core().focus.deferred.is_some());

    for y in [4400.0, 4800.0] {
        let frame = ui.scroll(&mut root, y);
        assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
        assert!(all_full(&ui.frame_walks()));
        assert!(ui.core().focus.deferred.is_some());
    }

    assert_eq!(ui.focused(&mut root), [LIST]);

    ui.scroll(&mut root, 19_440.0);
    assert!(ui.core().focus.deferred.is_none());
    assert_eq!(ui.focused(&mut root), [field(500)]);

    // Once it committed, shifts are local again.
    let frame = ui.scroll(&mut root, 19_840.0);
    assert_eq!(frame.passes, [Invalidation::Overlay, Invalidation::None]);
}

/// The focused widgets, which are never more than one.
fn single_focus(ui: &mut Harness, root: &mut Root<'_>) -> Vec<Id> {
    let focused = ui.focused(root);
    assert!(focused.len() <= 1, "several focused widgets: {focused:?}");
    focused
}

#[test]
fn landing_survives_a_layout_for_the_old_viewport() {
    // Review P4-R1 F1: rows 0..=99 are 40 px and row 100 was measured as 0 px at the
    // end. Back at the start, row 100 becomes 1000 px off the window. End on row 99
    // lands at 3400 with row 85 pinned. A full layout for the old viewport before the
    // redraw clears dirty, but must not release the landing.
    let mut items = rows(0..101, 40.0);
    items[100].height = Height::Fixed(0.0);
    let mut app = direct(items);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3400.0);
    assert!(ui.keys().contains(&100));
    ui.scroll(&mut root, 0.0);
    drop(root);

    app.items[100].height = Height::Fixed(1000.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(99, Align::End, Animation::Instant));
    assert_eq!(ui.anchor(), (Some(85), Some(3400.0)));
    drop(root);

    let mut root = app.view();
    ui.rebuild(&mut root);
    assert!(!ui.core().geometry.dirty);
    assert!(ui.core().landing.is_some());

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert_eq!(ui.translation(&mut root), 3400.0);
    assert_eq!(screen_bottom(&app.log, 99), 600.0);
    assert!(ui.core().pending_landing().is_none());
    ui.assert_invariants();
}

#[test]
fn landing_is_exempt_from_z_while_clamped() {
    // Review P4-R1 F2: 100 rows of 0 px, so the parent knows no content. Rows 49 and
    // 50 become 1000 px off the window. End on row 50 answers 400, but the first
    // redraw is clamped to the start, where rule Z must not drop the landing: the
    // walk there measures row 49 above it.
    let mut app = direct(rows(0..100, 0.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    assert_eq!(ui.positions(), range(95..=99));
    drop(root);

    app.items[49].height = Height::Fixed(1000.0);
    app.items[50].height = Height::Fixed(1000.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(50, Align::End, Animation::Instant));
    assert!(ui.core().landing.is_some());

    let redraw = ui.next_redraw();
    let first = ui.update(&mut root, &redraw).invalidation;
    assert_eq!(ui.translation(&mut root), 0.0);
    assert!(ui.core().landing.is_some());

    let frame = ui.complete(&mut root, &redraw, vec![first]);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert_eq!(ui.translation(&mut root), 400.0);
    assert_eq!(screen_bottom(&app.log, 50), 600.0);
    ui.assert_invariants();
}

#[test]
fn deferred_focus_is_held_by_the_list() {
    // Review P4-R1 F3: while a focus waits, the list holds it, so every focus change
    // visits the list, and exactly one widget is ever focused.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.header = 100.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let defer = |ui: &mut Harness, root: &mut Root<'_>| {
        ui.run_task(root, focusable::focus(OUTSIDE));
        ui.run_task(root, FocusItem::new(LIST, 500_u32, field(500)));
        assert_eq!(single_focus(ui, root), [LIST]);
        assert!(ui.core().focus.deferred.is_some());
    };

    let settles = |ui: &mut Harness, root: &mut Root<'_>, expected: &[Id]| {
        assert!(ui.core().focus.deferred.is_none());

        for _ in 0..3 {
            ui.frame(root);
            assert_eq!(single_focus(ui, root), expected);
        }
    };

    // `focus_next` and `focus_previous` move on from the list. The next one is the
    // field of the first row, whose state leaves with it when the jump to row 500
    // evicts it (L5), and the item never takes the focus.
    defer(&mut ui, &mut root);
    ui.run_task(&mut root, focusable::focus_next());
    assert_eq!(single_focus(&mut ui, &mut root), [field(0)]);
    assert!(ui.core().focus.deferred.is_none());

    for _ in 0..3 {
        ui.frame(&mut root);
        assert!(!single_focus(&mut ui, &mut root).contains(&field(500)));
    }

    ui.scroll(&mut root, 0.0);
    defer(&mut ui, &mut root);
    ui.run_task(&mut root, focusable::focus_previous());
    settles(&mut ui, &mut root, &[OUTSIDE]);

    // Focusing the list replaces the item: the list stays focused.
    ui.scroll(&mut root, 0.0);
    defer(&mut ui, &mut root);
    ui.run_task(&mut root, focusable::focus(LIST));
    settles(&mut ui, &mut root, &[LIST]);

    // A press outside the list unfocuses it.
    ui.scroll(&mut root, 0.0);
    defer(&mut ui, &mut root);
    ui.cursor = mouse::Cursor::Available(Point::new(100.0, 50.0));
    ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
    );
    ui.cursor = mouse::Cursor::Unavailable;
    settles(&mut ui, &mut root, &[]);

    // Uninterrupted, the list hands the focus to the item.
    ui.scroll(&mut root, 0.0);
    defer(&mut ui, &mut root);
    ui.frame(&mut root);
    assert_eq!(single_focus(&mut ui, &mut root), [field(500)]);
}

#[test]
fn nearest_without_scroll_still_lays_out_what_it_measured() {
    // Review P4-R1 F4: row 13 is laid out at 40 px, and its next layout gives 80 px.
    // Nearest measures it, finds [520, 600) still in view and does not scroll, but the
    // change must reach the layout: row 14 moves to 600 and the list grows.
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    let height = ui.list().1.size.height;

    app.log.overrides.borrow_mut().insert(13, 80.0);
    ui.run_task(
        &mut root,
        scroll_task(13, Align::Nearest, Animation::Instant),
    );
    assert!(ui.core().geometry.dirty && ui.core().geometry.block_next_redraw);
    assert!(ui.core().landing.is_none());

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(ui.screen_y(&mut root, 14), 600.0);
    assert_eq!(ui.list().1.size.height, height + 40.0);
    ui.assert_invariants();
    drop(root);

    // Review P4-R3 F3: with the end pinned, the growth moves the rows above it up, so
    // Nearest decides after absorbing it. 100 rows and the footer at translation 3450:
    // row 87 is at [3480, 3520), and 80 px tall it is at [3440, 3520), partly above
    // the viewport, so it is aligned at the top.
    let app = App::estimated(rows(0..100, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3450.0);
    assert!(ui.end_pinned());
    assert_eq!(app.log.screen()[&87], 30.0);

    app.log.overrides.borrow_mut().insert(87, 80.0);
    ui.run_task(
        &mut root,
        scroll_task(87, Align::Nearest, Animation::Instant),
    );
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&87], 0.0);
    ui.assert_invariants();
}

#[test]
fn zero_height_focus_target_is_a_no_op() {
    // Review P4-R1 F5: an item with no height off the window cannot be revealed, so
    // focusing it changes nothing and leaves nothing waiting.
    // Selectable, so the list can hold the focus before the task.
    let app = direct(rows(0..100, 0.0)).selectable(false);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    assert!(!ui.keys().contains(&50));

    ui.run_task(&mut root, focusable::focus(LIST));
    ui.run_task(&mut root, FocusItem::new(LIST, 50_u32, field(50)));
    assert_eq!(single_focus(&mut ui, &mut root), [LIST]);
    assert!(ui.core().focus.deferred.is_none());
    assert!(ui.core().landing.is_none());
    assert!(!ui.core().geometry.block_next_redraw);
}

#[test]
fn settle_keeps_the_screen_with_large_positive_drift() {
    // Review P4-R1 F6: 100 rows of 40 px removed above the viewport leave drift
    // 4000, with row 100 at the top of the viewport at 4000. Removing all the drift
    // would need offset 0, at the start, where rule Z removes it at once and moves
    // the content, so settle leaves 1 px, and nothing moves. (The review's numbers
    // had only ten rows left, which showed the drift as a gap; rule G now folds it,
    // see `filters_never_leave_a_gap`.)
    let mut app = App::estimated(rows(0..200, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    drop(root);

    app.items.drain(0..100);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(ui.core().geometry.drift(), 4000.0);
    assert_eq!(ui.translation(&mut root), 4000.0);

    let screen = app.log.screen();
    assert_eq!(screen[&100], 0.0);

    ui.run_task(&mut root, Settle::new(LIST));
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 1.0);
    assert_eq!(ui.core().geometry.drift(), 1.0);
    assert_eq!(app.log.screen(), screen);

    // The rest stays until the list reaches its start.
    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(ui.core().geometry.drift(), 1.0);
    ui.scroll(&mut root, 0.0);
    assert_eq!(ui.core().geometry.drift(), 0.0);
}

#[test]
fn landing_release_waits_for_rows_growing_in_their_first_redraw() {
    // Review P4-R2 F1: rows 0..=99 are 40 px, row 100 was measured as 0 px at the
    // end, and row 98 is now 1000 px off the window. End on row 99 lands at 4360 with
    // row 98 pinned at 3920. The walk for the destination builds row 100, which grows
    // to 1000 px in its first redraw, after the landing would have been released and
    // the end pinned. The landing pins row 98 until its frame is drawn.
    let land = |grow: u32| {
        let mut items = rows(0..101, 40.0);
        items[100].height = Height::Fixed(0.0);
        let mut app = direct(items);
        app.anticipate = 1;
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);
        ui.scroll(&mut root, 3400.0);
        assert!(ui.keys().contains(&100));
        ui.scroll(&mut root, 0.0);
        drop(root);

        app.items[98].height = Height::Fixed(1000.0);
        let mut root = app.view();
        ui.rebuild(&mut root);
        ui.frame(&mut root);

        ui.run_task(&mut root, scroll_task(99, Align::End, Animation::Instant));
        assert_eq!(ui.anchor(), (Some(98), Some(3920.0)));
        app.log.grow.borrow_mut().insert(grow, 1000.0);

        ui.frame(&mut root);
        assert_eq!(ui.translation(&mut root), 4360.0, "row {grow}");
        assert_eq!(screen_bottom(&app.log, 99), 600.0, "row {grow}");
        assert_eq!(ui.core().geometry.heights.height(grow as usize), 1000.0);
        ui.assert_invariants();

        let frame = ui.frame(&mut root);
        assert_eq!(frame.passes.last(), Some(&Invalidation::None));
        assert_eq!(screen_bottom(&app.log, 99), 600.0, "row {grow}");
        assert!(ui.core().landing.is_none());
    };

    // Below the target.
    land(100);

    // Above the landing's item, which L10 does not cover either: exact as well. Rows
    // between the landing's item and the target are L10's (see
    // `landing_frame_size_change_documents_l10`).
    land(97);
}

#[test]
fn presses_cancel_a_waiting_focus_even_when_captured() {
    // Review P4-R2 F2: a press the header captured before the list still reaches the
    // list in the column, which unfocuses and cancels the waiting focus.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.header = 100.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    assert_eq!(single_focus(&mut ui, &mut root), [LIST]);

    app.log.capture_press.set(true);
    ui.cursor = mouse::Cursor::Available(Point::new(100.0, 300.0));
    let updated = ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
    );
    assert!(updated.captured);
    app.log.capture_press.set(false);
    ui.cursor = mouse::Cursor::Unavailable;
    assert!(ui.core().focus.deferred.is_none());
    assert!(single_focus(&mut ui, &mut root).is_empty());

    // An uncaptured press inside the list cancels the waiting focus, and the list
    // keeps the focus it held.
    ui.scroll(&mut root, 0.0);
    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    ui.cursor = mouse::Cursor::Available(Point::new(100.0, 300.0));
    ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
    );
    ui.cursor = mouse::Cursor::Unavailable;
    assert!(ui.core().focus.deferred.is_none());
    ui.frame(&mut root);
    assert_eq!(single_focus(&mut ui, &mut root), [LIST]);

    // The commit needs the list to hold the focus.
    ui.scroll(&mut root, 0.0);
    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    ui.core_mut().focus.focused = false;
    ui.frame(&mut root);
    assert!(ui.core().focus.deferred.is_none());
    assert!(single_focus(&mut ui, &mut root).is_empty());
}

#[test]
fn focus_needs_room_to_lay_out_its_item() {
    // Review P4-R2 F3: a scroller with no height cannot show the item, so focusing an
    // item off the window does nothing.
    let mut app = App::estimated(rows(0..1000, 40.0), 40.0);
    app.anticipate = 0;
    app.height = Length::Fixed(0.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, focusable::focus(OUTSIDE));
    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    assert_eq!(single_focus(&mut ui, &mut root), [OUTSIDE]);
    assert!(ui.core().focus.deferred.is_none());
    assert!(ui.core().landing.is_none());
    drop(root);

    // With room, items shorter than, as tall as and taller than the viewport, below
    // and above it, are aligned inside the exclusive edges and their focus commits.
    for height in [10.0, 600.0, 2000.0] {
        let mut items = rows(0..1000, 40.0);
        items[500].height = Height::Fixed(height);
        items[100].height = Height::Fixed(height);
        let mut app = App::estimated(items, 40.0);
        app.anticipate = 0;
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);

        for key in [500, 100] {
            ui.run_task(&mut root, FocusItem::new(LIST, key, field(key)));
            ui.frame(&mut root);
            assert_eq!(single_focus(&mut ui, &mut root), [field(key)], "{height}");
            assert!(ui.core().focus.deferred.is_none());
        }
    }
}

#[test]
fn consecutive_tasks_before_a_redraw() {
    // Review P4-R2 F4: tasks run before the redraw that applies the previous one, so
    // the scroller's translation they see is stale.

    // Two settles: the second does nothing, and nothing moves (P4-R1 F6's numbers).
    let mut app = App::estimated(rows(0..200, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    drop(root);

    app.items.drain(0..100);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    let screen = app.log.screen();

    ui.run_task(&mut root, Settle::new(LIST));
    ui.run_task(&mut root, Settle::new(LIST));
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 1.0);
    assert_eq!(ui.core().geometry.drift(), 1.0);
    assert_eq!(app.log.screen(), screen);
    drop(root);

    // Nearest after a settle measures from the settled view: row 1228 is in it, at
    // screen y 120, so nothing scrolls.
    let mut app = direct(rows(1000..1250, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 9000.0);
    drop(root);

    app.items.splice(0..0, rows(0..100, 40.0));
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);

    ui.run_task(&mut root, Settle::new(LIST));
    ui.run_task(
        &mut root,
        scroll_task(1228, Align::Nearest, Animation::Instant),
    );
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 13_000.0);
    assert_eq!(app.log.screen()[&1228], 120.0);

    // A settle after an instant scroll does nothing: the scroll wins.
    ui.run_task(
        &mut root,
        scroll_task(1100, Align::Start, Animation::Instant),
    );
    ui.run_task(&mut root, Settle::new(LIST));
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&1100], 0.0);
    drop(root);

    // Two instant scrolls: the later one wins, exactly.
    let app = direct(rows(0..1000, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(
        &mut root,
        scroll_task(200, Align::Start, Animation::Instant),
    );
    ui.run_task(&mut root, scroll_task(600, Align::End, Animation::Instant));
    ui.frame(&mut root);
    assert_eq!(screen_bottom(&app.log, 600), 600.0);
    ui.assert_invariants();
}

#[test]
fn zero_height_focus_target_records_its_measurement_only() {
    // Review P4-R2 F5: row 500 is estimated at 40 px but has no height. Checking it
    // records the measurement, which the end-pinned list absorbs above the visible
    // rows, but neither the focus nor anything on screen changes, and the next
    // redraw lays the list out again.
    let mut items = rows(0..1000, 40.0);
    items[500].height = Height::Fixed(0.0);
    let mut app = App::estimated(items, 40.0);
    app.header = 100.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let content = ui.tree.children[0].size.height;
    ui.scroll(&mut root, content - WINDOW.height);
    assert!(ui.end_pinned());
    ui.run_task(&mut root, focusable::focus(OUTSIDE));
    ui.frame(&mut root);
    let screen = app.log.screen();
    let drift = ui.core().geometry.drift();

    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    assert_eq!(single_focus(&mut ui, &mut root), [OUTSIDE]);
    assert!(ui.core().focus.deferred.is_none() && ui.core().landing.is_none());
    assert_eq!(ui.core().geometry.drift(), drift + 40.0);
    assert!(ui.core().geometry.dirty);

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes[0], Invalidation::Layout(Diff::Skip));
    assert_eq!(app.log.screen(), screen);
    assert_eq!(single_focus(&mut ui, &mut root), [OUTSIDE]);
    ui.assert_invariants();
}

#[test]
fn landing_holds_through_messages_of_its_frame() {
    // Review P4-R3 F1: rows 0..=99 are 40 px and row 100 was measured as 0 px at the
    // end. Row 99 has a sensor whose message, on its first redraw at the destination,
    // makes row 100 1000 px tall. End on row 99 lands at 3400 with row 85 pinned. The
    // message is handled and the view laid out again between two passes of the
    // landing frame, which still pins row 85, so row 99 stays at the bottom.
    let mut items = rows(0..101, 40.0);
    items[100].height = Height::Fixed(0.0);
    let mut app = direct(items);
    app.anticipate = 1;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3400.0);
    ui.scroll(&mut root, 0.0);

    app.log.sensors.borrow_mut().insert(99);
    let view = || app.view();
    let update = |_| {
        app.log.overrides.borrow_mut().insert(100, 1000.0);
    };
    root = view();
    ui.rebuild(&mut root);
    ui.frame_app(&mut root, &view, &update);

    ui.run_task(&mut root, scroll_task(99, Align::End, Animation::Instant));
    assert_eq!(ui.anchor(), (Some(85), Some(3400.0)));

    let rebuilds = ui.rebuilds;
    let frame = ui.frame_app(&mut root, &view, &update);
    assert_eq!(
        ui.rebuilds,
        rebuilds + 1,
        "the message was handled in the frame"
    );
    assert_eq!(frame.passes.len(), 3);
    assert_eq!(ui.core().geometry.heights.height(100), 1000.0);
    assert_eq!(ui.translation(&mut root), 3400.0);
    assert_eq!(screen_bottom(&app.log, 99), 600.0);
    ui.assert_invariants();

    let frame = ui.frame_app(&mut root, &view, &update);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(screen_bottom(&app.log, 99), 600.0);
    assert!(ui.core().landing.is_none());
    assert_eq!(ui.anchor(), (Some(85), Some(3400.0)));
}

#[test]
fn landing_holds_through_a_later_sibling_layout() {
    // Review P4-R3 F1: the same jump in the column, where the footer asks for a
    // layout after the list's pass, while row 100 grows to 1000 px.
    let mut items = rows(0..101, 40.0);
    items[100].height = Height::Fixed(0.0);
    let mut app = App::estimated(items, 40.0);
    app.anticipate = 1;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3450.0);
    ui.scroll(&mut root, 0.0);

    ui.run_task(&mut root, scroll_task(99, Align::End, Animation::Instant));
    assert_eq!(ui.anchor(), (Some(85), Some(3400.0)));
    app.log
        .footer_changes
        .borrow_mut()
        .extend([None, Some((100, 1000.0))]);

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes.len(), 3);
    assert_eq!(ui.core().geometry.heights.height(100), 1000.0);
    assert_eq!(screen_bottom(&app.log, 99), 600.0);

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(screen_bottom(&app.log, 99), 600.0);
}

#[test]
fn drawn_landing_is_not_pending() {
    // Review P4-R3 F2: rows 97 and 98 are 1000 px before the task, which measures row
    // 98 and lands End on row 99 at 4360. The landing frame measures row 97, which
    // leaves drift -960, and row 100 grows in its first redraw. Once that frame is
    // drawn, tasks and the user's scrolling treat the anchor like any other.
    let landed = || {
        let mut items = rows(0..101, 40.0);
        items[100].height = Height::Fixed(0.0);
        let mut app = direct(items);
        app.anticipate = 1;
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);
        ui.scroll(&mut root, 3400.0);
        ui.scroll(&mut root, 0.0);
        drop(root);

        app.items[97].height = Height::Fixed(1000.0);
        app.items[98].height = Height::Fixed(1000.0);
        let mut root = app.view();
        ui.rebuild(&mut root);
        ui.frame(&mut root);

        ui.run_task(&mut root, scroll_task(99, Align::End, Animation::Instant));
        app.log.grow.borrow_mut().insert(100, 1000.0);
        ui.frame(&mut root);
        assert_eq!(ui.translation(&mut root), 4360.0);
        assert_eq!(ui.core().geometry.drift(), -960.0);
        assert_eq!(screen_bottom(&app.log, 99), 600.0);
        assert!(ui.core().landing.is_some() && ui.core().pending_landing().is_none());
        drop(root);

        (app, ui)
    };

    // A settle right away removes the drift. The view is rebuilt, as after any
    // message, which lays out the list again.
    let (app, mut ui) = landed();
    let mut root = app.view();
    ui.rebuild(&mut root);
    let screen = app.log.screen();
    ui.run_task(&mut root, Settle::new(LIST));
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 5320.0);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(app.log.screen(), screen);
    drop(root);

    // Scrolling up right away shrinks it (rule S).
    let (app, mut ui) = landed();
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.scroll(&mut root, 4000.0);
    assert!(ui.core().landing.is_none());
    let drift = ui.core().geometry.drift();
    assert!((drift - -960.0 * 4000.0 / 4360.0).abs() < 1.0e-6, "{drift}");
}

#[test]
fn aligned_instant_scroll_stops_a_smooth_one() {
    // Review P4-R3 F4: a smooth scroll to row 500 has not started when an instant
    // Nearest on row 5, which is in view, arrives. The later task wins, and the view
    // stays where it is.
    let app = App::estimated(rows(0..1000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(500, Align::Start, Animation::Smooth));
    ui.run_task(
        &mut root,
        scroll_task(5, Align::Nearest, Animation::Instant),
    );
    assert!(ui.core().landing.is_none());

    for _ in 0..60 {
        ui.frame(&mut root);
    }

    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(app.log.screen()[&5], 200.0);
}

/// SplitMix64, for the seeded property test.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<T: Copy>(&mut self, choices: &[T]) -> T {
        choices[self.below(choices.len())]
    }

    /// A row height: some empty, some fractional, some taller than the viewport.
    fn height(&mut self) -> f32 {
        match self.below(10) {
            0 => 0.0,
            1 | 2 => self.below(15_000) as f32 / 100.0,
            3 => 300.0 + self.below(1500) as f32,
            _ => 10.0 + self.below(110) as f32,
        }
    }
}

/// When a property case changes a row's height.
#[derive(Debug, Clone, Copy, PartialEq)]
enum When {
    /// A message handled before the task.
    Before,
    /// A message handled after the task, before the landing frame.
    AfterTask,
    /// The row's next redraw, which asks for a layout.
    Redraw,
    /// A message published by the target in a pass of the landing frame.
    Message,
    /// The footer, in a pass of the landing frame.
    Sibling,
}

/// What a property case checks right after its landing frame.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Then {
    Idle,
    Settle,
    ScrollUp,
}

const ALIGNS: [Align; 4] = [Align::Start, Align::Center, Align::End, Align::Nearest];

/// One case of `instant_landing_property`. Returns how many passes the landing frame
/// took, and how many frames drew before the landing was drawn.
fn landing_case(seed: u64) -> (usize, usize) {
    let mut rng = Rng(seed);
    let len = 2 + rng.below(200);
    let items = (0..len as u32)
        .map(|key| Item {
            key,
            height: Height::Fixed(rng.height()),
        })
        .collect();

    let mut app = App::new(items);
    app.estimate = rng.pick(&[None, Some(8.0), Some(40.0), Some(33.3), Some(250.0)]);
    app.spacing = rng.pick(&[0.0, 0.0, 2.5, 7.0]);
    app.anticipate = rng.below(5);
    app.direct = rng.chance(50);
    app.header = if app.direct {
        0.0
    } else {
        rng.pick(&[0.0, 37.5])
    };

    let key = rng.below(len) as u32;
    let align = rng.pick(&ALIGNS);
    let warm = rng.chance(60).then(|| rng.below(1000) as f32 / 1000.0);
    let start = rng.below(1000) as f32 / 1000.0;
    let then = rng.pick(&[Then::Idle, Then::Settle, Then::ScrollUp]);
    let mut timings = vec![When::Before, When::AfterTask, When::Redraw, When::Message];

    if !app.direct {
        timings.push(When::Sibling);
    }

    let changes: Vec<(When, f32, usize)> = (0..rng.below(4))
        .map(|_| (rng.pick(&timings), rng.height(), rng.below(3)))
        .collect();
    let case = format!(
        "seed {seed:#x}: {len} rows, estimate {:?}, spacing {}, anticipate {}, \
         direct {}, header {}, key {key}, {align:?}, warm {warm:?}, start {start}, \
         {then:?}, changes {changes:?}",
        app.estimate, app.spacing, app.anticipate, app.direct, app.header,
    );

    let log = Rc::clone(&app.log);
    let view = || app.view();
    let update = |_| {
        let changes = log.on_message.take();
        log.overrides.borrow_mut().extend(changes);
    };
    let changed = |row: usize, height: f32| {
        log.overrides.borrow_mut().insert(row as u32, height);
    };

    let mut root = view();
    let mut ui = Harness::new(&mut root, &log);
    ui.frame_app(&mut root, &view, &update);

    for fraction in warm.into_iter().chain([start]) {
        let content = ui.tree.children[0].size.height;
        ui.scroll_only(&mut root, (content - WINDOW.height).max(0.0) * fraction);
        ui.frame_app(&mut root, &view, &update);
        ui.frame_app(&mut root, &view, &update);
    }

    for &(when, height, _) in &changes {
        if when == When::Before {
            changed(rng.below(len), height);
            root = view();
            ui.rebuild(&mut root);
        }
    }

    let before = ui.translation(&mut root);
    ui.run_task(&mut root, scroll_task(key, align, Animation::Instant));

    // The scroller reports its new translation only after its next update. Without a
    // landing, the view is either at the start, with no anchor, or kept by Nearest.
    let landed = ui.core().landing.is_some();
    let aligned = !landed && ui.core().anchor.is_some();

    // L10 covers the rows from the first one visible at the destination down to the
    // target. Every other row may change.
    let first = match ui.core().anchor {
        None => 0,
        Some(Anchor::Item(anchor)) => anchor as usize,
        Some(Anchor::End) => {
            let (origin, _) = ui.list();
            let top = f64::from(before - origin.y);
            ui.core().geometry.first_visible(top).unwrap_or(0)
        }
    };
    let target = key as usize;
    let covered = first.min(target)..=first.max(target);
    let outside: Vec<usize> = (0..len).filter(|row| !covered.contains(row)).collect();
    let near: Vec<usize> = outside
        .iter()
        .copied()
        .filter(|&row| row + 3 >= *covered.start() && row <= covered.end() + 3)
        .collect();

    for &(when, height, slot) in &changes {
        if outside.is_empty() || when == When::Before {
            continue;
        }

        let row = if near.is_empty() || rng.chance(30) {
            rng.pick(&outside)
        } else {
            rng.pick(&near)
        };

        match when {
            When::Before => {}
            When::AfterTask => {
                changed(row, height);
                root = view();
                ui.rebuild(&mut root);
            }
            When::Redraw => {
                log.grow.borrow_mut().insert(row as u32, height);
            }
            When::Message => {
                log.publish.borrow_mut().insert(key);
                log.on_message.borrow_mut().push((row as u32, height));
            }
            When::Sibling => {
                let mut queue = log.footer_changes.borrow_mut();
                queue.extend(std::iter::repeat_n(None, slot));
                queue.push_back(Some((row as u32, height)));
            }
        }
    }

    log.changed_in.take();
    let mut frame = ui.frame_app(&mut root, &view, &update);
    let passes = frame.passes.len();
    let mut frames = 1;
    let changed_last = log.changed_in.borrow().contains(&3) || frame.late;

    // Changes that did not happen in the landing frame (a row never redrawn, a
    // footer pass that never ran) are dropped.
    log.grow.take();
    log.publish.take();
    log.footer_changes.take();
    log.on_message.take();

    // Only a change in the third pass, after the parent clamped it, leaves the
    // landing for the next frame: the frame is drawn with that pass's translation.
    while ui.core().pending_landing().is_some() {
        assert!(
            frames < 2 && changed_last,
            "{case}: the landing is still pending after {frames} frames: {:?}",
            frame.passes
        );
        frame = ui.frame_app(&mut root, &view, &update);
        frames += 1;
    }

    let translation = ui.translation(&mut root);
    let content = ui.tree.children[0].size.height;
    let viewport = WINDOW.height;
    let reachable = |y: f32| y.clamp(0.0, (content - viewport).max(0.0));
    let (origin, _) = ui.list();
    let geometry = &ui.core().geometry;
    let y = origin.y + geometry.content_y(target) as f32;
    let height = geometry.heights.height(target);
    let close = |a: f32, b: f32| (a - b).abs() <= 0.02;
    let start_at = reachable(y);
    let end_at = reachable(y - (viewport - height));

    let ok = match align {
        Align::Start => close(translation, start_at),
        Align::Center => close(translation, reachable(y - (viewport - height) / 2.0)),
        Align::End => close(translation, end_at),
        Align::Nearest if aligned => {
            close(translation, before)
                && y >= translation - 0.02
                && y + height <= translation + viewport + 0.02
        }
        Align::Nearest => close(translation, start_at) || close(translation, end_at),
    };
    assert!(
        ok,
        "{case}: translation {translation} for row {target} at {y}, {height} px tall, \
         content {content}, passes {:?}, before {before}, landed {landed}, aligned {aligned}",
        frame.passes
    );
    ui.assert_invariants();

    let screen = log.screen();
    let drift = ui.core().geometry.drift();
    let visible = ui.core().geometry.visible.expect("a redraw happened");

    match then {
        Then::Idle => {
            let frame = ui.frame_app(&mut root, &view, &update);
            assert_eq!(frame.passes, [Invalidation::None], "{case}");
            assert_eq!(log.screen(), screen, "{case}");
        }
        Then::Settle => {
            let top = f64::from(translation - origin.y);
            let removed = if drift > 0.0 {
                drift.min((top - 1.0).max(0.0))
            } else {
                drift
            };

            ui.run_task(&mut root, Settle::new(LIST));
            ui.frame_app(&mut root, &view, &update);
            let moved = ui.translation(&mut root);
            assert!(
                close(moved, translation - removed as f32),
                "{case}: settle moved to {moved}, removing {removed}"
            );

            for (key, y) in log.screen() {
                if let Some(before) = screen.get(&key) {
                    assert!(close(y, *before), "{case}: row {key} at {y}, was {before}");
                }
            }
        }
        Then::ScrollUp if visible.top > 2.0 => {
            let up = (visible.top * (1 + rng.below(9)) as f64 / 10.0) as f32;
            ui.scroll_only(&mut root, translation - up);
            ui.frame_app(&mut root, &view, &update);
            let shrunk = ui.core().geometry.shrunk_at == Some(ui.now);
            let closed = drift > 0.0 && ui.core().geometry.drift() < drift;
            assert!(ui.core().landing.is_none(), "{case}");
            assert!(
                drift == 0.0 || shrunk || closed || ui.core().geometry.visible.unwrap().at_start(),
                "{case}: rule S did not apply to drift {drift}"
            );
        }
        Then::ScrollUp => {}
    }

    ui.assert_invariants();

    (passes, frames)
}

#[test]
fn instant_landing_property() {
    // Review P4-R3: random lists, start offsets, instant scrolls and height changes
    // outside L10's rows, at every time a change can come in.
    let started = std::time::Instant::now();
    let mut passes = [0; 4];
    let mut late = Vec::new();

    for case in 0..400_u64 {
        let seed = 0x5EED_0000 + case * 0x9E37;
        let result = std::panic::catch_unwind(|| landing_case(seed));

        match result {
            Ok((count, frames)) => {
                passes[count] += 1;
                if frames > 1 {
                    late.push(format!("{seed:#x}"));
                }
            }
            Err(panic) => {
                eprintln!("instant_landing_property: case {case} failed, seed {seed:#x}");
                std::panic::resume_unwind(panic);
            }
        }
    }

    eprintln!(
        "instant_landing_property: 400 cases in {:?}, landing frames by passes {:?}, \
         drawn one frame late: {late:?}",
        started.elapsed(),
        &passes[1..]
    );
}

/// Delivers the messages of the last input to the application, which applies the
/// logged actions to its selection. Returns those actions.
fn delivered<'a>(ui: &mut Harness, root: &mut Root<'a>, app: &'a App) -> Vec<Action<u32>> {
    let actions = app.log.actions.borrow().clone();
    ui.deliver(root, &|| app.view(), &|_| {
        app.apply();
    });
    actions
}

/// The one action of the last input, which must not ask for a reveal.
fn pointed<'a>(ui: &mut Harness, root: &mut Root<'a>, app: &'a App) -> Change<u32> {
    let actions = delivered(ui, root, app);
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(actions[0].reveal(), None);
    actions[0].change.clone()
}

/// The one action of the last key, which must ask to reveal `reveal`.
fn typed<'a>(
    ui: &mut Harness,
    root: &mut Root<'a>,
    app: &'a App,
    reveal: Option<u32>,
) -> Change<u32> {
    let actions = delivered(ui, root, app);
    assert_eq!(actions.len(), 1, "{actions:?}");
    assert_eq!(actions[0].reveal().copied(), reveal);
    actions[0].change.clone()
}

/// Replaces the application's selection, and rebuilds the view.
fn reselect<'a>(ui: &mut Harness, root: &mut Root<'a>, app: &'a App, selection: Selection<u32>) {
    *app.selection.as_ref().unwrap().borrow_mut() = selection;
    *root = app.view();
    ui.rebuild(root);
}

fn selected(changes: impl IntoIterator<Item = Change<u32>>) -> Selection<u32> {
    let mut selection = Selection::new();

    for change in changes {
        selection.perform(Action::mouse(change));
    }

    selection
}

fn span(
    anchor: u32,
    cursor: u32,
    keys: impl IntoIterator<Item = u32>,
    additive: bool,
) -> Change<u32> {
    Change::Range {
        anchor,
        cursor,
        keys: keys.into_iter().collect(),
        additive,
    }
}

const NONE: Modifiers = Modifiers::empty();
const CTRL: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;

fn both() -> Modifiers {
    Modifiers::COMMAND | Modifiers::SHIFT
}

/// The middle of row `key` of 40 px rows at translation 0.
fn on_row(key: u32) -> Point {
    Point::new(100.0, 40.0 * key as f32 + 20.0)
}

fn named(named: Named) -> Key {
    Key::Named(named)
}

#[test]
fn selection_perform_matches_resolved_actions() {
    let sorted = |selection: &Selection<u32>| {
        let mut keys: Vec<u32> = selection.iter().copied().collect();
        keys.sort_unstable();
        keys
    };
    let state = |selection: &Selection<u32>| {
        (
            sorted(selection),
            selection.anchor().copied(),
            selection.cursor().copied(),
        )
    };

    let mut selection = Selection::new();
    assert!(selection.is_empty() && selection.cursor().is_none());

    selection.perform(Action::mouse(Change::Select(3)));
    assert_eq!(state(&selection), (vec![3], Some(3), Some(3)));

    selection.perform(Action::mouse(Change::Toggle(5)));
    assert_eq!(state(&selection), (vec![3, 5], Some(5), Some(5)));

    selection.perform(Action::mouse(Change::Toggle(3)));
    assert_eq!(state(&selection), (vec![5], Some(3), Some(3)));

    selection.perform(Action::mouse(span(5, 8, 5..=8, false)));
    assert_eq!(state(&selection), (vec![5, 6, 7, 8], Some(5), Some(8)));

    selection.perform(Action::mouse(span(1, 2, [1, 2], true)));
    assert_eq!(
        state(&selection),
        (vec![1, 2, 5, 6, 7, 8], Some(1), Some(2))
    );

    selection.perform(Action::keyboard(Change::Cursor(9)));
    assert_eq!(
        state(&selection),
        (vec![1, 2, 5, 6, 7, 8], Some(1), Some(9))
    );

    selection.perform(Action::keyboard(Change::All(vec![4, 9])));
    assert_eq!(state(&selection), (vec![4, 9], Some(1), Some(9)));
    assert!(selection.contains(&4) && !selection.contains(&1));
    assert_eq!(selection.len(), 2);

    // Empty data.
    selection.perform(Action::keyboard(Change::All(Vec::new())));
    assert_eq!(state(&selection), (vec![], Some(1), Some(9)));

    selection.perform(Action::mouse(span(2, 6, 2..=6, false)));
    selection.perform(Action::keyboard(Change::Clear));
    assert_eq!(state(&selection), (vec![], Some(2), Some(6)));

    // Removed keys are dropped by the application, with the anchor and cursor.
    selection.perform(Action::mouse(span(3, 6, 3..=6, false)));
    selection.retain(|key| key % 2 == 0);
    assert_eq!(state(&selection), (vec![4, 6], None, Some(6)));

    selection.select(7);
    assert_eq!(state(&selection), (vec![7], Some(7), Some(7)));
    selection.clear();
    assert_eq!(state(&selection), (vec![], Some(7), Some(7)));

    // Ranges follow the filtered sequence the application passes, in its order.
    let mut app = App::new(rows(0..1, 40.0)).selectable(true);
    app.items = [10, 3, 7, 42, 5]
        .into_iter()
        .map(|key| Item {
            key,
            height: Height::Fixed(40.0),
        })
        .collect();
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.click(&mut root, on_row(1), NONE);
    assert_eq!(pointed(&mut ui, &mut root, &app), Change::Select(3));
    ui.click(&mut root, on_row(3), SHIFT);
    assert_eq!(
        pointed(&mut ui, &mut root, &app),
        span(3, 42, [3, 7, 42], false)
    );
    ui.cursor = mouse::Cursor::Unavailable;
    drop(root);

    // The anchor left the sequence: the range starts at the cursor, and without
    // either at the clicked item.
    app.items.remove(1);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    ui.click(&mut root, on_row(3), SHIFT);
    assert_eq!(
        pointed(&mut ui, &mut root, &app),
        span(42, 5, [42, 5], false)
    );

    reselect(&mut ui, &mut root, &app, selected([Change::Select(99)]));
    ui.click(&mut root, on_row(1), SHIFT);
    assert_eq!(pointed(&mut ui, &mut root, &app), span(7, 7, [7], false));
}

#[test]
fn click_table_and_origin_are_distinct() {
    // Ten 40 px rows and a 30 px bottom padding, which is the empty area below the
    // items, at screen y 400..430.
    let below = Point::new(100.0, 415.0);
    let app = |multiple| {
        let mut app = App::new(rows(0..10, 40.0)).selectable(multiple);
        app.padding = Padding {
            bottom: 30.0,
            ..Padding::ZERO
        };
        app
    };

    // Single selection: plain, Shift and Ctrl+Shift select, also the selected item
    // (review P5-R1 F2), and Ctrl clears the selected item and selects another.
    let single = app(false);
    let mut root = single.view();
    let mut ui = Harness::new(&mut root, &single.log);
    ui.frame(&mut root);

    for (key, modifiers, change) in [
        (2, NONE, Change::Select(2)),
        (4, SHIFT, Change::Select(4)),
        (5, both(), Change::Select(5)),
        (5, both(), Change::Select(5)),
        (5, CTRL, Change::Clear),
        (6, CTRL, Change::Select(6)),
    ] {
        let press = ui.click(&mut root, on_row(key), modifiers);
        assert!(press.captured);
        assert_eq!(pointed(&mut ui, &mut root, &single), change, "{key}");
        assert!(ui.core().focus.focused);
    }

    ui.click(&mut root, below, NONE);
    assert_eq!(pointed(&mut ui, &mut root, &single), Change::Clear);
    drop(root);

    // Multiple selection: plain selects, Ctrl toggles, Shift replaces with a range
    // from the anchor and Ctrl+Shift adds it.
    let multiple = app(true);
    let mut root = multiple.view();
    let mut ui = Harness::new(&mut root, &multiple.log);
    ui.frame(&mut root);

    for (key, modifiers, change) in [
        (2, NONE, Change::Select(2)),
        (4, CTRL, Change::Toggle(4)),
        (6, SHIFT, span(4, 6, 4..=6, false)),
        (1, both(), span(4, 1, 1..=4, true)),
    ] {
        assert!(ui.click(&mut root, on_row(key), modifiers).captured);
        assert_eq!(pointed(&mut ui, &mut root, &multiple), change, "{key}");
    }

    ui.click(&mut root, below, NONE);
    assert_eq!(pointed(&mut ui, &mut root, &multiple), Change::Clear);

    // The anchor falls back to the cursor, then to the clicked item.
    reselect(
        &mut ui,
        &mut root,
        &multiple,
        selected([Change::Select(99), Change::Cursor(3)]),
    );
    ui.click(&mut root, on_row(7), SHIFT);
    assert_eq!(
        pointed(&mut ui, &mut root, &multiple),
        span(3, 7, 3..=7, false)
    );

    reselect(
        &mut ui,
        &mut root,
        &multiple,
        selected([Change::Select(99)]),
    );
    ui.click(&mut root, on_row(7), both());
    assert_eq!(
        pointed(&mut ui, &mut root, &multiple),
        span(7, 7, [7], true)
    );

    // The same change from the keyboard reveals its cursor key, from the mouse not.
    reselect(&mut ui, &mut root, &multiple, selected([Change::Select(2)]));
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert_eq!(
        typed(&mut ui, &mut root, &multiple, Some(3)),
        Change::Select(3)
    );
    ui.click(&mut root, on_row(3), NONE);
    assert_eq!(pointed(&mut ui, &mut root, &multiple), Change::Select(3));

    ui.key(&mut root, named(Named::ArrowDown), SHIFT);
    assert_eq!(
        typed(&mut ui, &mut root, &multiple, Some(4)),
        span(3, 4, 3..=4, false)
    );
    ui.click(&mut root, on_row(4), SHIFT);
    assert_eq!(
        pointed(&mut ui, &mut root, &multiple),
        span(3, 4, 3..=4, false)
    );
    drop(root);

    // A double click on the same item activates once. A triple click does not
    // activate again, nor do quick clicks on two items.
    let mut activating = app(true);
    activating.activate = true;
    let mut root = activating.view();
    let mut ui = Harness::new(&mut root, &activating.log);
    ui.frame(&mut root);

    for key in [3, 3, 3] {
        ui.click(&mut root, on_row(key), NONE);
        delivered(&mut ui, &mut root, &activating);
    }

    assert_eq!(*activating.log.activated.borrow(), [3]);

    ui.click(&mut root, on_row(5), NONE);
    ui.click(&mut root, on_row(6), NONE);
    delivered(&mut ui, &mut root, &activating);
    assert_eq!(*activating.log.activated.borrow(), [3]);
    drop(root);

    // Two clicks at the same point on different items, because item 5 was removed
    // in between, are not a double click either.
    let mut root = activating.view();
    ui.rebuild(&mut root);
    ui.click(&mut root, on_row(5), NONE);
    delivered(&mut ui, &mut root, &activating);
    drop(root);

    activating.items.remove(5);
    let mut root = activating.view();
    ui.rebuild(&mut root);
    ui.click(&mut root, on_row(5), NONE);
    assert_eq!(pointed(&mut ui, &mut root, &activating), Change::Select(6));
    assert_eq!(*activating.log.activated.borrow(), [3]);
}

#[test]
fn keyboard_table_uses_current_heights_and_focus() {
    // Rows of 40 px, rows 10..=19 100 px, 2 px spacing: row 5 is at 210, row 10 at
    // 420 and row 13 at 726.
    let app = |multiple| {
        let mut items = rows(0..100, 40.0);

        for item in &mut items[10..20] {
            item.height = Height::Fixed(100.0);
        }

        let mut app = App::new(items).selectable(multiple);
        app.spacing = 2.0;
        app.activate = true;
        app
    };

    let single = app(false);
    let mut root = single.view();
    let mut ui = Harness::new(&mut root, &single.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 1500.0);
    ui.scroll(&mut root, 0.0);

    // Not focused: keys reach other widgets.
    let key = ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert!(!key.captured && single.log.actions.borrow().is_empty());

    // Focused without a cursor, moves start at the first visible item.
    ui.run_task(&mut root, focusable::focus(LIST));
    assert!(ui.key(&mut root, named(Named::ArrowDown), NONE).captured);
    assert_eq!(
        typed(&mut ui, &mut root, &single, Some(0)),
        Change::Select(0)
    );

    for (key, modifiers, target) in [
        (Named::ArrowDown, NONE, 1),
        (Named::ArrowUp, NONE, 0),
        (Named::ArrowUp, NONE, 0),
        (Named::End, NONE, 99),
        (Named::Home, NONE, 0),
        (Named::ArrowDown, CTRL, 1),
        (Named::ArrowDown, SHIFT, 2),
        (Named::ArrowDown, both(), 3),
        (Named::ArrowDown, NONE, 4),
        (Named::ArrowDown, NONE, 5),
        (Named::PageDown, NONE, 13),
        (Named::PageUp, NONE, 3),
    ] {
        assert!(ui.key(&mut root, named(key), modifiers).captured);
        assert_eq!(
            typed(&mut ui, &mut root, &single, Some(target)),
            Change::Select(target),
            "{key:?} {modifiers:?}"
        );
    }

    // Page moves use the heights as measured now: row 12 becomes 300 px.
    single.log.overrides.borrow_mut().insert(12, 300.0);
    root = single.view();
    ui.rebuild(&mut root);
    reselect(&mut ui, &mut root, &single, selected([Change::Select(5)]));
    ui.key(&mut root, named(Named::PageDown), NONE);
    assert_eq!(
        typed(&mut ui, &mut root, &single, Some(12)),
        Change::Select(12)
    );

    // Space selects the cursor, Ctrl+A is not handled in single selection, Escape
    // clears a selection and Enter activates the cursor.
    ui.key(&mut root, named(Named::Space), CTRL);
    assert_eq!(
        typed(&mut ui, &mut root, &single, Some(12)),
        Change::Select(12)
    );
    assert!(!ui.key(&mut root, Key::Character("a".into()), CTRL).captured);
    assert!(ui.key(&mut root, named(Named::Escape), NONE).captured);
    assert_eq!(typed(&mut ui, &mut root, &single, None), Change::Clear);
    assert!(!ui.key(&mut root, named(Named::Escape), NONE).captured);
    assert!(ui.key(&mut root, named(Named::Enter), NONE).captured);
    assert_eq!(ui.deliver(&mut root, &|| single.view(), &|_| ()), 1);
    assert_eq!(*single.log.activated.borrow(), [12]);

    // Keys the list does not handle are left alone, also Tab.
    for key in [named(Named::Tab), Key::Character("x".into())] {
        assert!(!ui.key(&mut root, key, NONE).captured);
    }

    assert!(single.log.actions.borrow().is_empty());

    // A press elsewhere unfocuses the list, and keys are not handled again.
    ui.cursor = mouse::Cursor::Unavailable;
    ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
    );
    assert!(!ui.key(&mut root, named(Named::ArrowDown), NONE).captured);
    drop(root);

    // Multiple selection: Ctrl moves the cursor only, Shift and Ctrl+Shift make
    // ranges from the anchor, Ctrl+Space toggles and Ctrl+A selects all.
    let multiple = app(true);
    let mut root = multiple.view();
    let mut ui = Harness::new(&mut root, &multiple.log);
    ui.frame(&mut root);
    reselect(&mut ui, &mut root, &multiple, selected([Change::Select(2)]));
    ui.run_task(&mut root, focusable::focus(LIST));

    for (key, modifiers, reveal, change) in [
        (Named::ArrowDown, CTRL, Some(3), Change::Cursor(3)),
        (Named::ArrowDown, SHIFT, Some(4), span(2, 4, 2..=4, false)),
        (Named::ArrowDown, both(), Some(5), span(2, 5, 2..=5, true)),
        (Named::Space, CTRL, Some(5), Change::Toggle(5)),
        (Named::Home, SHIFT, Some(0), span(5, 0, 0..=5, false)),
        (Named::Space, NONE, Some(0), Change::Select(0)),
    ] {
        assert!(ui.key(&mut root, named(key), modifiers).captured);
        assert_eq!(
            typed(&mut ui, &mut root, &multiple, reveal),
            change,
            "{key:?} {modifiers:?}"
        );
    }

    assert!(ui.key(&mut root, Key::Character("a".into()), CTRL).captured);
    assert_eq!(
        typed(&mut ui, &mut root, &multiple, None),
        Change::All((0..100).collect())
    );
    drop(root);

    // An empty list handles nothing.
    let mut empty = app(true);
    empty.items.clear();
    let mut root = empty.view();
    let mut ui = Harness::new(&mut root, &empty.log);
    ui.frame(&mut root);
    ui.run_task(&mut root, focusable::focus(LIST));
    assert!(!ui.key(&mut root, named(Named::ArrowDown), NONE).captured);
    assert!(!ui.key(&mut root, Key::Character("a".into()), CTRL).captured);
    assert!(empty.log.actions.borrow().is_empty());
}

/// A key, the application's update, the reveal task it returns, and the next frame.
fn reveal_step<'a>(ui: &mut Harness, root: &mut Root<'a>, app: &'a App, key: Named) {
    ui.key(root, named(key), NONE);
    let reveals = RefCell::new(Vec::new());
    ui.deliver(root, &|| app.view(), &|_| {
        reveals.borrow_mut().extend(app.apply());
    });

    for key in reveals.take() {
        ui.run_task(root, scroll_task(key, Align::Nearest, Animation::Instant));
    }

    ui.frame(root);
}

#[test]
fn keyboard_reveal_goes_through_the_scroll_task() {
    // The application applies the action and returns `scroll_to` for its reveal key,
    // which the shell runs before the next frame. Row 14 ends at the viewport's
    // bottom, so Down to row 15 scrolls it into view at the bottom.
    let app = App::new(rows(0..100, 40.0)).selectable(false);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    reselect(&mut ui, &mut root, &app, selected([Change::Select(14)]));
    ui.run_task(&mut root, focusable::focus(LIST));

    reveal_step(&mut ui, &mut root, &app, Named::ArrowDown);
    assert_eq!(ui.translation(&mut root), 40.0);
    assert_eq!(screen_bottom(&app.log, 15), 600.0);

    // In view: the reveal does not scroll.
    reveal_step(&mut ui, &mut root, &app, Named::ArrowUp);
    assert_eq!(ui.translation(&mut root), 40.0);

    // A page down lands its target at the bottom, and Home at the top.
    reveal_step(&mut ui, &mut root, &app, Named::PageDown);
    assert_eq!(screen_bottom(&app.log, 29), 600.0);
    reveal_step(&mut ui, &mut root, &app, Named::Home);
    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(app.log.screen()[&0], 0.0);
}

#[test]
fn row_button_wins_and_nonselectable_text_press_is_ignored() {
    // Row 3 is a probe in a `button`, 50 px tall with the button's padding, at
    // screen y 120..170.
    let mut items = rows(0..10, 40.0);
    items[3].height = Height::Button(40.0);
    let app = App::new(items.clone()).selectable(true);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    // An item press the list handles is captured once, and focuses the list.
    let press = ui.click(&mut root, on_row(1), NONE);
    assert!(press.captured);
    assert_eq!(pointed(&mut ui, &mut root, &app), Change::Select(1));
    assert!(ui.core().focus.focused);

    // The button takes its press: the list drops the focus and selects nothing. The
    // button publishes on the release, which the list does not capture.
    let rebuilds = ui.rebuilds;
    let press = ui.click(&mut root, Point::new(100.0, 145.0), NONE);
    assert!(press.captured);
    assert!(!ui.core().focus.focused);
    assert!(app.log.actions.borrow().is_empty());
    assert_eq!(delivered(&mut ui, &mut root, &app), []);
    assert_eq!(
        ui.rebuilds,
        rebuilds + 1,
        "the button's message was handled"
    );

    // A right press selects nothing and is not captured.
    ui.cursor = mouse::Cursor::Available(on_row(5));
    let press = ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
    );
    assert!(!press.captured && app.log.actions.borrow().is_empty());
    drop(root);

    // Without a selection the list captures nothing, so the runtime can select the
    // text of the items, and takes no focus.
    let plain = App::new(items);
    let mut root = plain.view();
    let mut ui = Harness::new(&mut root, &plain.log);
    ui.frame(&mut root);
    let press = ui.click(&mut root, on_row(1), NONE);
    assert!(!press.captured);
    assert!(!ui.core().focus.focused);
    assert_eq!(ui.messages(), 0);
}

#[test]
fn touch_tap_selects_without_claiming_scroll_press() {
    let app = App::new(rows(0..100, 40.0)).selectable(true);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let finger = touch::Finger(1);
    let touch = |ui: &mut Harness,
                 root: &mut Root<'_>,
                 event: fn(touch::Finger, Point) -> touch::Event,
                 y: f32| {
        let position = Point::new(100.0, y);
        ui.cursor = mouse::Cursor::Available(position);
        ui.update(root, &Event::Touch(event(finger, position)))
    };
    let pressed = |id, position| touch::Event::FingerPressed { id, position };
    let moved = |id, position| touch::Event::FingerMoved { id, position };
    let lifted = |id, position| touch::Event::FingerLifted { id, position };
    let lost = |id, position| touch::Event::FingerLost { id, position };

    // A drag scrolls the parent, which got the press, and selects nothing.
    touch(&mut ui, &mut root, pressed, 300.0);
    touch(&mut ui, &mut root, moved, 260.0);
    touch(&mut ui, &mut root, lifted, 260.0);
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 40.0);
    assert!(app.log.actions.borrow().is_empty());

    // A tap selects the item under the finger, with no reveal, and focuses the list.
    touch(&mut ui, &mut root, pressed, 100.0);
    touch(&mut ui, &mut root, lifted, 103.0);
    assert_eq!(pointed(&mut ui, &mut root, &app), Change::Select(3));
    assert!(ui.core().focus.focused);

    // A lost finger, and a finger that moved too far, do not tap.
    touch(&mut ui, &mut root, pressed, 100.0);
    touch(&mut ui, &mut root, lost, 100.0);
    touch(&mut ui, &mut root, lifted, 100.0);
    touch(&mut ui, &mut root, pressed, 100.0);
    touch(&mut ui, &mut root, moved, 110.0);
    touch(&mut ui, &mut root, lifted, 110.0);
    assert!(app.log.actions.borrow().is_empty());
}

/// Rows of 40 px with item 20 a group of a 30 px header and five 40 px items at
/// content y 800..1030, the header made of `header`.
fn grouped(header: bool) -> App {
    let mut items = rows(0..100, 40.0);
    items[20].height = if header {
        Height::Header {
            header: 30.0,
            items: 5,
            item: 40.0,
        }
    } else {
        Height::Group {
            header: 30.0,
            items: 5,
            item: 40.0,
        }
    };
    App::estimated(items, 40.0)
}

/// The bounds the probe with `key` was drawn at in the last frame.
fn drawn_at(log: &Log, key: u32) -> Vec<Rectangle> {
    log.drawn
        .borrow()
        .iter()
        .filter(|drawn| drawn.key == key)
        .map(|drawn| drawn.bounds)
        .collect()
}

#[test]
fn sticky_group_local_and_full_paths_keep_parent_bounds() {
    // Item 50 is a group of a 30 px header and twenty 40 px items, 830 px at content
    // y 2000. End on it from the top is a full-path jump that leaves its header above
    // the viewport while the group fills it: the header sticks at the top.
    let mut items = rows(0..100, 40.0);
    items[50].height = Height::Group {
        header: 30.0,
        items: 20,
        item: 40.0,
    };
    let app = App::estimated(items, 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(50, Align::End, Animation::Instant));
    app.log.clear();
    let frame = ui.frame(&mut root);
    assert!(frame.root_layouts > 0);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert_eq!(ui.translation(&mut root), 2230.0);

    let group = Rectangle::new(Point::new(0.0, 2000.0), Size::new(800.0, 830.0));
    let stuck = Rectangle::new(Point::ORIGIN, Size::new(800.0, 30.0));
    assert_eq!(drawn_at(&app.log, 50), [stuck]);
    app.log.assert_covered(2230.0, 2830.0);

    // The new sticky got each pass's redraw at most once: it sticks on its first
    // one, and the overlay passes on the next.
    let header: Vec<_> = app
        .log
        .redraws
        .borrow()
        .iter()
        .filter(|redraw| redraw.0 == 50)
        .copied()
        .collect();
    assert_eq!(header, [(50, Some(group), stuck)]);

    // A local shift inside the group keeps it stuck without a root layout. The
    // items get their group as parent.
    app.log.clear();
    let frame = ui.scroll(&mut root, 2400.0);
    assert_eq!(frame.root_layouts, 0);
    assert_eq!(drawn_at(&app.log, 50), [stuck]);
    assert!(
        app.log
            .redraws
            .borrow()
            .iter()
            .filter(|redraw| redraw.0 > 50_000)
            .all(|redraw| redraw.1 == Some(group))
    );
    app.log.assert_covered(2430.0, 3000.0);

    // Back above, the header is in the base tree again.
    ui.scroll(&mut root, 1900.0);
    assert_eq!(
        drawn_at(&app.log, 50),
        [Rectangle::new(
            Point::new(0.0, 2000.0),
            Size::new(800.0, 30.0)
        )]
    );
    assert_eq!(ui.overlay, None);
}

#[test]
fn stuck_header_button_is_the_only_group_click_handler() {
    // Review R1 M6: at 850 the group's header is stuck at screen y 0..30, drawn by
    // an overlay: text, and a button at x 750..800.
    let app = grouped(true).selectable(true);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 850.0);
    ui.click(&mut root, Point::new(100.0, 300.0), NONE);
    assert_eq!(pointed(&mut ui, &mut root, &app), Change::Select(24));

    // Over the text the overlay is idle, the base cursor levitates, and the press
    // selects nothing. As for iced's text inputs it is outside the list.
    let text = Point::new(100.0, 10.0);
    ui.cursor = mouse::Cursor::Available(text);
    ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::CursorMoved { position: text }),
    );
    assert_eq!(ui.overlay, Some(mouse::Interaction::Idle));
    assert!(ui.base_cursor().is_levitating());

    let press = ui.click(&mut root, text, NONE);
    assert!(!press.captured);
    assert!(app.log.actions.borrow().is_empty());
    assert!(!ui.core().focus.focused);
    assert_eq!(ui.messages(), 0);

    // The header's button handles its click: it publishes the application's
    // message on the release, and the list selects nothing.
    let button = Point::new(775.0, 10.0);
    ui.cursor = mouse::Cursor::Available(button);
    ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::CursorMoved { position: button }),
    );
    let press = ui.click(&mut root, button, NONE);
    assert!(press.captured);
    assert_eq!(ui.messages(), 1);
    assert_eq!(delivered(&mut ui, &mut root, &app), []);
    ui.cursor = mouse::Cursor::Unavailable;

    // Not stuck, the header is in the base tree, where a press on its text selects
    // the group like any part of the item.
    ui.frame(&mut root);
    ui.scroll(&mut root, 700.0);
    assert_eq!(ui.overlay, None);
    ui.click(&mut root, Point::new(100.0, 115.0), NONE);
    assert_eq!(pointed(&mut ui, &mut root, &app), Change::Select(20));
}

#[test]
fn third_pass_draw_has_covered_rows_and_stuck_header() {
    // Review R1 M1, the Sticky case: a full-path jump whose third pass still asks
    // for new overlays is drawn right after it. Rows are 60 px, estimated at 40, so
    // the scroller clamps the jump on the first pass and reaches it on the second,
    // where the new header sticks. The footer asks for new overlays on the third.
    // Item 80 is a group of a 30 px header and twenty 60 px items, 1230 px.
    let mut items = rows(0..100, 60.0);
    items[80].height = Height::Group {
        header: 30.0,
        items: 20,
        item: 60.0,
    };
    let app = App::estimated(items, 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, scroll_task(80, Align::End, Animation::Instant));
    app.log.inject.borrow_mut()[1].extend([None, None, Some(Invalidation::Overlay)]);
    app.log.clear();
    let frame = ui.frame(&mut root);
    assert_eq!(
        frame.passes,
        [
            Invalidation::Layout(Diff::Skip),
            Invalidation::Layout(Diff::Skip),
            Invalidation::Overlay
        ]
    );

    // The frame drawn after that pass shows the stuck header at the top, and rows
    // under it cover the viewport.
    let translation = ui.translation(&mut root);
    let stuck = Rectangle::new(Point::ORIGIN, Size::new(800.0, 30.0));
    assert_eq!(drawn_at(&app.log, 80), [stuck]);
    app.log.assert_covered(translation, translation + 600.0);
    assert_eq!(screen_bottom(&app.log, 80_020), 600.0);
}

#[test]
fn keys_while_a_focus_waits() {
    // A waiting focus holds the list's focus. Keys the list does not handle leave
    // it waiting. A key it handles cancels it, as a press would: the user acted
    // before it landed.
    let app = App::estimated(rows(0..1000, 40.0), 40.0).selectable(false);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(&mut root, FocusItem::new(LIST, 500_u32, field(500)));
    assert!(ui.core().focus.deferred.is_some());

    for key in [
        named(Named::Tab),
        Key::Character("x".into()),
        named(Named::Escape),
    ] {
        assert!(!ui.key(&mut root, key, NONE).captured);
    }

    assert!(ui.core().focus.deferred.is_some());

    assert!(ui.key(&mut root, named(Named::ArrowDown), NONE).captured);
    assert!(ui.core().focus.deferred.is_none());
    assert_eq!(typed(&mut ui, &mut root, &app, Some(0)), Change::Select(0));

    ui.frame(&mut root);
    assert_eq!(single_focus(&mut ui, &mut root), [LIST]);
    drop(root);

    // Without one, the same key lets the focus land.
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.run_task(&mut root, FocusItem::new(LIST, 700_u32, field(700)));
    ui.key(&mut root, named(Named::Tab), NONE);
    ui.frame(&mut root);
    assert_eq!(single_focus(&mut ui, &mut root), [field(700)]);
}

#[test]
fn component_redraws_keep_selection_and_sticky() {
    // A component re-runs the redraw of its view after it rebuilds it, so rows get
    // a redraw twice in an invalidating pass. The header still sticks once and a
    // click still emits one action.
    let app = grouped(false).selectable(false);
    let mut root = wrapped(&app, false);
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 3000.0);

    app.log.clear();
    ui.scroll(&mut root, 850.0);
    let stuck = Rectangle::new(Point::ORIGIN, Size::new(800.0, 30.0));
    assert_eq!(drawn_at(&app.log, 20), [stuck]);
    app.log.assert_covered(850.0, 1450.0);
    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(drawn_at(&app.log, 20), [stuck]);

    ui.click(&mut root, Point::new(100.0, 300.0), NONE);
    let actions = app.log.actions.take();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].change, Change::Select(24));
}

#[test]
fn overlay_only_focus_targets_need_generic_operations() {
    // At 850 the header of group 20, a probe with a focusable field, is stuck and
    // exists only in the overlay. The generic focus operation, which the list's
    // `focus` chains for a materialized item, reaches it there.
    let app = grouped(false);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 850.0);
    assert!(ui.overlay.is_some());

    ui.run_task(&mut root, FocusItem::new(LIST, 20_u32, field(20)));
    assert_eq!(single_focus(&mut ui, &mut root), [field(20)]);

    // A waiting focus commits in the item's base tree, where a stuck header is not.
    // Then the list keeps the focus, and nothing waits any more. The list is not
    // selectable, so only a waiting focus focuses it: set its state directly.
    ui.run_task(&mut root, focusable::unfocus());
    let core = ui.core_mut();
    core.focus.focused = true;
    core.focus.deferred = Some((20, field(20)));
    ui.frame(&mut root);
    assert!(ui.core().focus.deferred.is_none());
    assert_eq!(single_focus(&mut ui, &mut root), [LIST]);
}

#[test]
fn selected_and_cursor_statuses_follow_the_selection_and_focus() {
    let mut app = App::new(rows(0..10, 40.0)).selectable(true);
    app.text_color = Some(RED);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    reselect(
        &mut ui,
        &mut root,
        &app,
        selected([Change::Select(2), Change::Toggle(4)]),
    );

    let statuses = |ui: &mut Harness, root: &mut Root<'_>| {
        app.log.statuses.take();
        ui.frame(root);
        let drawn = app.log.drawn_keys();
        let statuses = app.log.statuses.take();
        assert_eq!(statuses.len(), drawn.len());

        drawn
            .into_iter()
            .zip(statuses)
            .filter(|(_, status)| status.selected || status.cursor)
            .map(|(key, status)| (key, status.selected, status.cursor))
            .collect::<Vec<_>>()
    };

    // The cursor shows only while the list has the focus.
    assert_eq!(
        statuses(&mut ui, &mut root),
        [(2, true, false), (4, true, false)]
    );

    ui.run_task(&mut root, focusable::focus(LIST));
    assert_eq!(
        statuses(&mut ui, &mut root),
        [(2, true, false), (4, true, true)]
    );
}

#[test]
fn batched_input_resolves_against_emitted_actions() {
    // Review P5-R1 F1: the runtime delivers a batch of events before the application
    // applies any of their messages. Each event resolves against the actions the
    // list emitted before it.
    let mut app = App::new(rows(0..20, 40.0)).selectable(true);
    app.text_color = Some(RED);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    reselect(&mut ui, &mut root, &app, selected([Change::Select(2)]));
    ui.run_task(&mut root, focusable::focus(LIST));

    let batch = |ui: &mut Harness, root: &mut Root<'_>, keys: &[(Named, Modifiers)]| {
        for &(key, modifiers) in keys {
            assert!(ui.key(root, named(key), modifiers).captured);
        }

        app.log
            .actions
            .borrow()
            .iter()
            .map(|action| (action.change.clone(), action.reveal().copied()))
            .collect::<Vec<_>>()
    };

    let actions = batch(
        &mut ui,
        &mut root,
        &[(Named::ArrowDown, NONE), (Named::ArrowDown, NONE)],
    );
    assert_eq!(
        actions,
        [(Change::Select(3), Some(3)), (Change::Select(4), Some(4))]
    );

    // The list draws the application's selection, not what it emitted.
    app.log.statuses.take();
    ui.draw(&mut root);
    let statuses: Vec<_> = app
        .log
        .drawn_keys()
        .into_iter()
        .zip(app.log.statuses.take())
        .filter(|(_, status)| status.selected)
        .map(|(key, _)| key)
        .collect();
    assert_eq!(statuses, [2]);

    delivered(&mut ui, &mut root, &app);
    let selection = app.selection.as_ref().unwrap().borrow().clone();
    assert_eq!(selection.cursor(), Some(&4));

    // Shift extends the range from the same anchor, and a click in the same batch
    // starts from where the keys left the cursor.
    let actions = batch(
        &mut ui,
        &mut root,
        &[(Named::ArrowDown, SHIFT), (Named::ArrowDown, SHIFT)],
    );
    assert_eq!(
        actions,
        [
            (span(4, 5, 4..=5, false), Some(5)),
            (span(4, 6, 4..=6, false), Some(6)),
        ]
    );
    ui.click(&mut root, on_row(9), both());
    assert_eq!(
        app.log.actions.borrow().last().unwrap().change,
        span(4, 9, 4..=9, true)
    );
    delivered(&mut ui, &mut root, &app);

    // The application ignores an action: once it handled the message, the next key
    // starts from what it keeps, with no redraw in between.
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    ui.deliver(&mut root, &|| app.view(), &|_| {
        app.log.actions.take();
    });
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert_eq!(
        typed(&mut ui, &mut root, &app, Some(10)),
        Change::Select(10)
    );

    // The application applies something else: the next key starts from that.
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    ui.deliver(&mut root, &|| app.view(), &|_| {
        app.log.actions.take();
        app.selection
            .as_ref()
            .unwrap()
            .borrow_mut()
            .perform(Change::Select(15).into());
    });
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert_eq!(
        typed(&mut ui, &mut root, &app, Some(16)),
        Change::Select(16)
    );
    drop(root);

    // Single selection: a Ctrl click on the item a click of the same batch selected
    // clears it.
    let single = App::new(rows(0..20, 40.0)).selectable(false);
    let mut root = single.view();
    let mut ui = Harness::new(&mut root, &single.log);
    ui.frame(&mut root);
    ui.click(&mut root, on_row(3), NONE);
    ui.click(&mut root, on_row(3), CTRL);
    let changes: Vec<_> = delivered(&mut ui, &mut root, &single)
        .into_iter()
        .map(|action| action.change)
        .collect();
    assert_eq!(changes, [Change::Select(3), Change::Clear]);
}

#[test]
fn focus_changes_from_input_request_a_redraw() {
    // Review P5-R1 F3: the cursor's border depends on the list's focus. Ten rows and
    // the footer at screen y 400..450.
    let mut items = rows(0..10, 40.0);
    items[3].height = Height::Button(40.0);
    let app = App::new(items).selectable(true);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    let focus = |ui: &mut Harness, root: &mut Root<'_>| {
        ui.click(root, on_row(1), NONE);
        app.log.actions.take();
        ui.bus = Bus::new();
        ui.cursor = mouse::Cursor::Available(Point::new(100.0, 425.0));
        ui.update(
            root,
            &Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(100.0, 425.0),
            }),
        );
        ui.frame(root);
        assert!(ui.core().focus.focused);
    };

    // A right press on the footer unfocuses the list without any message.
    focus(&mut ui, &mut root);
    let press = ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
    );
    assert!(!ui.core().focus.focused);
    assert!(!press.captured && ui.messages() == 0);
    assert_eq!(press.redraw, RedrawRequest::NextFrame);

    // So does a press a row's button takes.
    focus(&mut ui, &mut root);
    ui.cursor = mouse::Cursor::Available(Point::new(100.0, 145.0));
    let press = ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
    );
    assert!(press.captured && !ui.core().focus.focused);
    assert_eq!(press.redraw, RedrawRequest::NextFrame);

    // A press that changes no focus asks for nothing.
    ui.cursor = mouse::Cursor::Available(Point::new(100.0, 425.0));
    let press = ui.update(
        &mut root,
        &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
    );
    assert_eq!(press.redraw, RedrawRequest::Wait);
}

#[test]
fn moves_skip_later_duplicates() {
    // Review P5-R1 F4: a key's later duplicates resolve to its first position, so
    // moves step over them.
    let keyed = |keys: &[u32]| {
        let mut app = App::new(rows(0..1, 40.0)).selectable(false);
        app.items = keys
            .iter()
            .map(|&key| Item {
                key,
                height: Height::Fixed(40.0),
            })
            .collect();
        app
    };

    let moves = |app: &App, start: u32, steps: &[(Named, u32)]| {
        let mut root = app.view();
        let mut ui = Harness::new(&mut root, &app.log);
        ui.frame(&mut root);
        reselect(&mut ui, &mut root, app, selected([Change::Select(start)]));
        ui.run_task(&mut root, focusable::focus(LIST));

        for &(key, target) in steps {
            ui.key(&mut root, named(key), NONE);
            assert_eq!(
                typed(&mut ui, &mut root, app, Some(target)),
                Change::Select(target),
                "{key:?}"
            );
        }
    };

    // [A, A, B]
    moves(
        &keyed(&[1, 1, 2]),
        1,
        &[
            (Named::ArrowDown, 2),
            (Named::ArrowDown, 2),
            (Named::ArrowUp, 1),
            (Named::End, 2),
        ],
    );

    // [A, B, A, C], and End on a later duplicate.
    moves(
        &keyed(&[1, 2, 1, 3]),
        2,
        &[
            (Named::ArrowDown, 3),
            (Named::ArrowUp, 2),
            (Named::ArrowUp, 1),
        ],
    );
    moves(&keyed(&[1, 2, 1]), 1, &[(Named::End, 2), (Named::Home, 1)]);

    // Pages that end inside a run of duplicates go on to the next item down, or
    // back to the run's first position up.
    let mut keys = vec![0];
    keys.extend(std::iter::repeat_n(1, 30));
    keys.push(2);
    moves(
        &keyed(&keys),
        0,
        &[(Named::PageDown, 2), (Named::PageUp, 1)],
    );
}

#[test]
fn page_point_in_spacing_goes_to_the_item_below() {
    // Rows of 40 px with 10 px spacing under a 5 px header: the visible height is
    // 595, so a page down from row 0 points at 595, in the spacing below row 11
    // (550..590). The item below it is the target.
    let mut app = App::new(rows(0..100, 40.0)).selectable(false);
    app.spacing = 10.0;
    app.header = 5.0;
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    reselect(&mut ui, &mut root, &app, selected([Change::Select(0)]));
    ui.run_task(&mut root, focusable::focus(LIST));

    ui.key(&mut root, named(Named::PageDown), NONE);
    assert_eq!(
        typed(&mut ui, &mut root, &app, Some(12)),
        Change::Select(12)
    );
}

#[test]
fn applications_change_the_selection_directly() {
    // Review P5-R1 F5: the same changes, and set-style methods, without a list.
    let mut selection: Selection<u32> = [2, 4].into_iter().collect();
    assert!(selection.contains(&2) && selection.contains(&4));
    assert_eq!(selection.cursor(), None);

    let action = Action::from(Change::Toggle(4));
    assert_eq!(action.reveal(), None);
    selection.perform(action);
    assert_eq!(selection.iter().copied().collect::<Vec<_>>(), [2]);
    assert_eq!(selection.cursor(), Some(&4));

    selection.perform(Change::All(vec![1, 2, 3]).into());
    selection.perform(Change::Cursor(3).into());
    selection.perform(span(1, 2, [1, 2], false).into());
    assert_eq!(selection.len(), 2);
    assert_eq!(
        (selection.anchor(), selection.cursor()),
        (Some(&1), Some(&2))
    );

    assert!(selection.insert(9) && !selection.insert(9));
    assert!(selection.remove(&1) && !selection.remove(&1));
    selection.extend([5, 6]);
    selection.set_cursor(Some(6));
    selection.set_anchor(None);

    let mut keys: Vec<u32> = selection.iter().copied().collect();
    keys.sort_unstable();
    assert_eq!(keys, [2, 5, 6, 9]);
    assert_eq!((selection.anchor(), selection.cursor()), (None, Some(&6)));
}

/// A component that owns its selection in its state.
struct Owning<'a> {
    items: &'a [u32],
    states: Rc<RefCell<Vec<Selection<u32>>>>,
    /// How many of the next actions to ignore.
    ignore: Rc<Cell<usize>>,
    /// Every action received.
    received: Rc<RefCell<Vec<Change<u32>>>>,
}

impl<'a> Component<'a, (), Theme, ()> for Owning<'a> {
    type State = Selection<u32>;
    type Event = Action<u32>;

    fn update(
        &self,
        state: &mut Selection<u32>,
        action: Action<u32>,
        _renderer: &(),
    ) -> Option<()> {
        self.received.borrow_mut().push(action.change.clone());

        if self.ignore.get() > 0 {
            self.ignore.set(self.ignore.get() - 1);
            return None;
        }

        state.perform(action);
        self.states.borrow_mut().push(state.clone());
        None
    }

    fn view(&self, state: &Selection<u32>) -> impl iced::Widget<Action<u32>, Theme, ()> + 'a {
        list_view(self.items, |_, _| space().width(Fill).height(40))
            .key(|key: &u32| *key)
            .id(LIST)
            .selection(state.clone(), |action| action)
            .multiple(true)
    }
}

#[test]
fn component_owns_its_selection() {
    // Review P5-R1 F6: a component cannot lend its state to the list for the list's
    // lifetime, so it passes a copy, and updates the state with the actions.
    let items: Vec<u32> = (0..20).collect();
    let states = Rc::new(RefCell::new(Vec::new()));
    let view = || -> Root<'_> {
        scrollable(component(Owning {
            items: &items,
            states: Rc::clone(&states),
            ignore: Rc::default(),
            received: Rc::default(),
        }))
        .id(SCROLL)
        .width(Fill)
        .height(Fill)
        .boxed()
    };

    let log = Rc::new(Log::default());
    let mut root = view();
    let mut ui = Harness::new(&mut root, &log);
    ui.frame(&mut root);

    let last = |states: &Rc<RefCell<Vec<Selection<u32>>>>| {
        let state = states.borrow().last().unwrap().clone();
        let mut keys: Vec<u32> = state.iter().copied().collect();
        keys.sort_unstable();
        (keys, state.anchor().copied(), state.cursor().copied())
    };

    ui.click(&mut root, on_row(2), NONE);
    assert_eq!(last(&states), (vec![2], Some(2), Some(2)));

    // The component rebuilt the list with its new state.
    ui.click(&mut root, on_row(5), SHIFT);
    assert_eq!(last(&states), (vec![2, 3, 4, 5], Some(2), Some(5)));

    ui.key(&mut root, named(Named::ArrowDown), SHIFT);
    ui.key(&mut root, named(Named::ArrowDown), SHIFT);
    assert_eq!(last(&states), (vec![2, 3, 4, 5, 6, 7], Some(2), Some(7)));
    assert_eq!(ui.messages(), 0);
    ui.frame(&mut root);
}

#[test]
fn harness_keeps_receipts_until_the_application_handles_them() {
    // Review P5-R2 E: a tracked message is processed only once the application's
    // update handled it, as in the shell.
    let app = App::new(rows(0..10, 40.0)).selectable(false);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.click(&mut root, on_row(2), NONE);
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    let pending: Vec<Tracking> = ui
        .core()
        .input
        .pending
        .iter()
        .map(|(tracking, _)| tracking.clone())
        .collect();
    assert_eq!(pending.len(), 2);
    assert!(pending.iter().all(|tracking| !tracking.is_processed()));

    delivered(&mut ui, &mut root, &app);
    assert!(pending.iter().all(Tracking::is_processed));
}

#[test]
fn component_handling_drops_the_pending_action() {
    // Review P5-R2 A: a component handles the list's actions at once, and here
    // ignores the first one. Its state, rebuilt into the list unchanged, is the
    // truth: the next key starts from it.
    let items: Vec<u32> = (0..20).collect();
    let states = Rc::new(RefCell::new(Vec::new()));
    let ignore = Rc::new(Cell::new(0));
    let received = Rc::new(RefCell::new(Vec::new()));
    let view = || -> Root<'_> {
        scrollable(component(Owning {
            items: &items,
            states: Rc::clone(&states),
            ignore: Rc::clone(&ignore),
            received: Rc::clone(&received),
        }))
        .id(SCROLL)
        .width(Fill)
        .height(Fill)
        .boxed()
    };

    let log = Rc::new(Log::default());
    let mut root = view();
    let mut ui = Harness::new(&mut root, &log);
    ui.frame(&mut root);
    ui.click(&mut root, on_row(2), NONE);

    ignore.set(1);
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert_eq!(
        received.borrow()[1..],
        [Change::Select(3), Change::Select(3)]
    );
    assert_eq!(states.borrow().last().unwrap().cursor(), Some(&3));
}

/// A component that forwards the list's actions to the application, and gives the
/// list a new copy of the application's selection on every view.
struct Forwarding<'a> {
    items: &'a [u32],
    selection: &'a RefCell<Selection<u32>>,
    forwarded: &'a RefCell<Vec<Action<u32>>>,
}

impl<'a> Component<'a, (), Theme, ()> for Forwarding<'a> {
    type State = ();
    type Event = Action<u32>;

    fn update(&self, _state: &mut (), action: Action<u32>, _renderer: &()) -> Option<()> {
        self.forwarded.borrow_mut().push(action);
        Some(())
    }

    fn view(&self, _state: &()) -> impl iced::Widget<Action<u32>, Theme, ()> + 'a {
        let selection = self.selection.borrow();
        let mut fresh: Selection<u32> = selection.iter().copied().collect();
        fresh.set_anchor(selection.anchor().copied());
        fresh.set_cursor(selection.cursor().copied());

        list_view(self.items, |_, _| space().width(Fill).height(40))
            .key(|key: &u32| *key)
            .id(LIST)
            .selection(fresh, |action| action)
    }
}

#[test]
fn forwarded_actions_stay_pending_over_new_copies() {
    // Review P5-R2 B: the component rebuilds its view with a new copy of the same
    // selection before the application handles the forwarded action, which is
    // still pending: the next key builds on it.
    let items: Vec<u32> = (0..20).collect();
    let selection = RefCell::new(Selection::new());
    selection.borrow_mut().select(2);
    let forwarded = RefCell::new(Vec::new());
    let view = || -> Root<'_> {
        scrollable(component(Forwarding {
            items: &items,
            selection: &selection,
            forwarded: &forwarded,
        }))
        .id(SCROLL)
        .width(Fill)
        .height(Fill)
        .boxed()
    };
    let update = |_| {
        for action in forwarded.take() {
            selection.borrow_mut().perform(action);
        }
    };

    let log = Rc::new(Log::default());
    let mut root = view();
    let mut ui = Harness::new(&mut root, &log);
    ui.frame(&mut root);
    ui.run_task(&mut root, focusable::focus(LIST));

    ui.key(&mut root, named(Named::ArrowDown), NONE);
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    let changes: Vec<_> = forwarded
        .borrow()
        .iter()
        .map(|action| action.change.clone())
        .collect();
    assert_eq!(changes, [Change::Select(3), Change::Select(4)]);

    assert_eq!(ui.deliver(&mut root, &view, &update), 2);
    assert_eq!(selection.borrow().cursor(), Some(&4));
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert_eq!(
        forwarded
            .borrow()
            .last()
            .map(|action| action.change.clone()),
        Some(Change::Select(5))
    );
}

#[test]
fn a_restored_state_is_not_mistaken_for_an_older_one() {
    // Review P5-R2 C: the application applies a key's action differently, then
    // restores the state from before it (an undo). The list resolves the next key
    // against the restored state.
    let app = App::new(rows(0..20, 40.0)).selectable(false);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    reselect(&mut ui, &mut root, &app, selected([Change::Select(2)]));
    let before = app.selection.as_ref().unwrap().borrow().clone();
    ui.run_task(&mut root, focusable::focus(LIST));

    ui.key(&mut root, named(Named::ArrowDown), NONE);
    ui.deliver(&mut root, &|| app.view(), &|_| {
        app.log.actions.take();
        app.selection.as_ref().unwrap().borrow_mut().select(15);
    });

    reselect(&mut ui, &mut root, &app, before);
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert_eq!(typed(&mut ui, &mut root, &app, Some(3)), Change::Select(3));
}

#[test]
fn lists_sharing_a_selection_see_each_other_after_the_update() {
    // Review P5-R2 D, a documented limit: pending actions belong to the list that
    // emitted them. Two lists show one selection, cursor 2. Down in the first one
    // and a Ctrl click on key 3 in the second, in one batch: the second list has not
    // seen the first one's Select(3), so it selects 3 instead of clearing it.
    let app = App::new(rows(0..5, 40.0)).selectable(false);
    let view = || -> Root<'_> {
        scrollable(column![app.list(), app.list()].width(Fill))
            .id(SCROLL)
            .width(Fill)
            .height(Fill)
            .boxed()
    };
    let update = |_| {
        app.apply();
    };

    let mut root = view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame_app(&mut root, &view, &update);
    ui.click(&mut root, on_row(2), NONE);
    ui.deliver(&mut root, &view, &update);

    ui.key(&mut root, named(Named::ArrowDown), NONE);
    ui.click(
        &mut root,
        Point::new(100.0, 200.0 + 40.0 * 3.0 + 20.0),
        CTRL,
    );
    let changes: Vec<_> = app
        .log
        .actions
        .borrow()
        .iter()
        .map(|action| action.change.clone())
        .collect();
    assert_eq!(changes, [Change::Select(3), Change::Select(3)]);
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Notify {
    Apply,
    Alter,
    Ignore,
}

/// A component that owns its selection, handles each action as `mode` says, and
/// notifies its parent, which forwards the action's receipt.
struct Notifying<'a> {
    items: &'a [u32],
    mode: Rc<Cell<Notify>>,
    received: Rc<RefCell<Vec<Change<u32>>>>,
}

impl<'a> Component<'a, (), Theme, ()> for Notifying<'a> {
    type State = Selection<u32>;
    type Event = Action<u32>;

    fn update(
        &self,
        state: &mut Selection<u32>,
        action: Action<u32>,
        _renderer: &(),
    ) -> Option<()> {
        self.received.borrow_mut().push(action.change.clone());

        match (self.mode.get(), &action.change) {
            (Notify::Alter, Change::Toggle(_)) => state.select(9),
            (Notify::Apply | Notify::Alter, _) => state.perform(action),
            (Notify::Ignore, _) => {}
        }

        Some(())
    }

    fn view(&self, state: &Selection<u32>) -> impl iced::Widget<Action<u32>, Theme, ()> + 'a {
        list_view(self.items, |_, _| space().width(Fill).height(40))
            .key(|key: &u32| *key)
            .id(LIST)
            .selection(state.clone(), |action| action)
            .multiple(true)
    }
}

#[test]
fn replayed_effects_match_what_the_owner_already_applied() {
    // Review P5-R3 F1: the component applies or alters an action and notifies its
    // parent, so the action stays pending over a state that may already contain
    // it. Replaying it changes nothing. Empty selection, cursor 2.
    let items: Vec<u32> = (0..20).collect();
    let mode = Rc::new(Cell::new(Notify::Apply));
    let received = Rc::new(RefCell::new(Vec::new()));
    let view = || -> Root<'_> {
        scrollable(component(Notifying {
            items: &items,
            mode: Rc::clone(&mode),
            received: Rc::clone(&received),
        }))
        .id(SCROLL)
        .width(Fill)
        .height(Fill)
        .boxed()
    };

    let run = |handling: Notify| {
        let log = Rc::new(Log::default());
        let mut root = view();
        let mut ui = Harness::new(&mut root, &log);
        ui.frame(&mut root);

        mode.set(Notify::Apply);
        ui.click(&mut root, on_row(2), NONE);
        ui.key(&mut root, named(Named::Escape), NONE);
        ui.deliver(&mut root, &view, &|_| ());
        received.take();

        mode.set(handling);
        ui.key(&mut root, named(Named::Space), CTRL);
        assert!(ui.key(&mut root, named(Named::Escape), NONE).captured);
        assert_eq!(received.take(), [Change::Toggle(2), Change::Clear]);
        ui.deliver(&mut root, &view, &|_| ());
        (ui, root)
    };

    // Applied: the toggle selected 2, so Escape clears it.
    run(Notify::Apply);

    // Altered: the list assumes the toggle until the parent handled the message,
    // then follows the component's state, whose cursor is 9.
    let (mut ui, mut root) = run(Notify::Alter);
    mode.set(Notify::Apply);
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert_eq!(received.take(), [Change::Select(10)]);
    drop(root);

    // Ignored: the same until handled, and then nothing is selected for Escape.
    let (mut ui, mut root) = run(Notify::Ignore);
    assert!(!ui.key(&mut root, named(Named::Escape), NONE).captured);
}

#[test]
fn working_copy_is_kept_until_a_receipt_or_the_selection_changes() {
    // Review P5-R3 F2: a batch of 100 Ctrl+A on 20,000 items applies each effect
    // once. The copy is rebuilt when a receipt is processed in the batch, or when
    // the list is given another selection.
    let app = App::new(rows(0..20_000, 40.0)).selectable(true);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.run_task(&mut root, focusable::focus(LIST));

    let counts = |ui: &Harness| (ui.core().input.applied, ui.core().input.rebases);
    let (applied, rebases) = counts(&ui);

    for _ in 0..100 {
        assert!(ui.key(&mut root, Key::Character("a".into()), CTRL).captured);
    }

    assert_eq!(counts(&ui), (applied + 100, rebases));
    app.log.actions.take();

    // The first message is handled: the copy is rebuilt from the other 99.
    let mut messages: Vec<_> = ui.bus.drain().collect();
    messages.remove(0);

    for (message, receipt) in messages {
        ui.bus.forward(message, receipt);
    }

    ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert_eq!(counts(&ui), (applied + 100 + 99 + 1, rebases + 1));

    // Another view, which may supply another selection: the 100 pending effects
    // are applied to it again, then the new one.
    root = app.view();
    ui.rebuild(&mut root);
    ui.key(&mut root, named(Named::ArrowDown), NONE);
    assert_eq!(counts(&ui), (applied + 200 + 100 + 1, rebases + 2));
    app.log.actions.take();
}

#[test]
fn example_filter_reorder_expand_remove_and_selection_flow() {
    // The application keeps its data and passes the items that match its query,
    // with stable keys: here the even keys, 40 px each. Its update applies each
    // action before it returns the reveal task.
    let data = RefCell::new(rows(0..400, 40.0));
    let divisor = Cell::new(2);
    let mut app = App::estimated(Vec::new(), 40.0).selectable(true);
    let filter = |app: &mut App| {
        app.items = data
            .borrow()
            .iter()
            .copied()
            .filter(|item| item.key % divisor.get() == 0)
            .collect();
    };
    filter(&mut app);

    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 2000.0);
    assert_eq!(app.log.screen()[&100], 0.0);

    // A click, then keys whose reveal tasks go through the scroller.
    ui.click(&mut root, Point::new(100.0, 100.0), NONE);
    assert_eq!(pointed(&mut ui, &mut root, &app), Change::Select(104));
    reveal_step(&mut ui, &mut root, &app, Named::ArrowDown);
    assert_eq!(ui.translation(&mut root), 2000.0);
    reveal_step(&mut ui, &mut root, &app, Named::End);
    assert_eq!(screen_bottom(&app.log, 398), 600.0);
    reveal_step(&mut ui, &mut root, &app, Named::Home);
    assert_eq!(app.log.screen()[&0], 0.0);

    ui.scroll(&mut root, 2000.0);
    ui.click(&mut root, Point::new(100.0, 100.0), NONE);
    ui.click(&mut root, Point::new(100.0, 140.0), CTRL);
    ui.click(&mut root, Point::new(100.0, 180.0), CTRL);
    delivered(&mut ui, &mut root, &app);
    let selection = Rc::clone(app.selection.as_ref().unwrap());
    let selected = || {
        let mut keys: Vec<u32> = selection.borrow().iter().copied().collect();
        keys.sort_unstable();
        keys
    };
    assert_eq!(selected(), [104, 106, 108]);
    let serial = ui.serials()[&100];
    drop(root);

    // A reorder of the same length above the viewport moves nothing on screen.
    data.borrow_mut().swap(10, 20);
    filter(&mut app);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&100], 0.0);
    assert_eq!(ui.serials()[&100], serial);
    drop(root);

    // A new query: the application drops the selected keys that left.
    divisor.set(4);
    filter(&mut app);
    let present: HashSet<u32> = app.items.iter().map(|item| item.key).collect();
    app.selection
        .as_ref()
        .unwrap()
        .borrow_mut()
        .retain(|key| present.contains(key));
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&100], 0.0);
    assert_eq!(selected(), [104, 108]);
    drop(root);

    // Expanding an item just above the viewport, and removing one, keep the first
    // visible item in place. Removing a selected item drops it from the selection.
    data.borrow_mut()[96].height = Height::Group {
        header: 30.0,
        items: 4,
        item: 40.0,
    };
    data.borrow_mut()
        .retain(|item| item.key != 92 && item.key != 104);
    filter(&mut app);
    app.selection
        .as_ref()
        .unwrap()
        .borrow_mut()
        .retain(|&key| key != 104);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&100], 0.0);
    assert_eq!(ui.core().geometry.heights.height(23), 190.0);
    assert_eq!(selected(), [108]);
    ui.assert_invariants();
}

#[test]
fn migration_keeps_index_height_cache_contract() {
    // The two regressions of the previous widget, through the public constructor
    // without keys, where an item's index is its key. Rows are 20 px, estimated at
    // 50 px.
    fn indexed<'a>(items: &'a [Item], log: &Rc<Log>) -> Root<'a> {
        let log = Rc::clone(log);

        scrollable(
            crate::listview::list_view(items, move |_, item| row(item, &log)).estimated_height(50),
        )
        .id(SCROLL)
        .height(Fill)
        .boxed()
    }

    let log = Rc::new(Log::default());
    let view = |items| indexed(items, &log);
    let offset = |ui: &Harness, position: usize| {
        let tree = ui.tree.children[0].state.downcast_ref::<Core<usize>>();
        tree.geometry.heights.offset_of(position)
    };

    // Unmeasured items count with the estimate, measured ones with their height.
    let items = rows(0..1000, 20.0);
    let mut root = view(&items);
    let mut ui = Harness::new(&mut root, &log);
    ui.frame(&mut root);
    let measured = (0..1000)
        .take_while(|&position| {
            ui.tree.children[0]
                .state
                .downcast_ref::<Core<usize>>()
                .geometry
                .heights
                .is_measured(position)
        })
        .count();
    assert!(measured > 30 && measured < 100);
    assert_eq!(
        offset(&ui, 500),
        20.0 * measured as f64 + 50.0 * (500 - measured) as f64
    );
    drop(root);

    // Measured heights survive a change in length: they follow the index. A
    // returning index gets its height back too, where the previous widget used
    // the estimate.
    let items = rows(0..100, 20.0);
    let mut root = view(&items);
    let mut ui = Harness::new(&mut root, &log);
    ui.frame(&mut root);

    for y in [400.0, 800.0, 1200.0, 1400.0] {
        ui.scroll(&mut root, y);
    }

    assert_eq!(offset(&ui, 100), 2000.0);
    drop(root);

    let shorter = rows(0..99, 20.0);
    let mut root = view(&shorter);
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(offset(&ui, 50), 1000.0);
    assert_eq!(offset(&ui, 99), 1980.0);
    drop(root);

    // The new index counts with the estimate until a layout measures it.
    let longer = rows(0..101, 20.0);
    let mut root = view(&longer);
    ui.diff_only(&mut root);
    assert_eq!(offset(&ui, 100), 2000.0);
    assert_eq!(offset(&ui, 101), 2050.0);
    ui.layout(&mut root);
    assert_eq!(offset(&ui, 101), 2020.0);
}

#[test]
fn filters_never_leave_a_gap() {
    // Fix F1, the user's scenario: 5,000 rows of 40 px in the column with the
    // footer, scrolled to the middle, and a filter that keeps 5 rows, none of them
    // shown. The removed anchor has no shown survivor, so the drift is folded and
    // the scrollable clamps to the 5 rows: they are drawn from the top.
    let mut app = App::estimated(rows(0..5000, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 100_000.0);
    drop(root);

    app.items.retain(|item| item.key % 1000 == 555);
    let mut root = app.view();
    ui.rebuild(&mut root);
    let frame = ui.frame(&mut root);
    assert!(frame.passes.len() <= 3);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    let content = ui.list().1.size.height + FOOTER;
    assert_eq!(content, 250.0);
    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    assert_eq!(app.log.drawn_keys(), [555, 1555, 2555, 3555, 4555]);
    assert_eq!(app.log.screen()[&555], 0.0);
    ui.assert_invariants();
    assert_eq!(ui.frame(&mut root).passes, [Invalidation::None]);
    drop(root);

    // One shown row is kept, row 2505 at screen 200, with the 2,505 rows above it
    // removed and the rows after it kept. It takes the anchor, and the drift would
    // leave 200 px blank above it: rule G moves it up to the viewport's top, with the
    // rows below it.
    app.items = rows(0..5000, 40.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.scroll(&mut root, 100_000.0);
    assert_eq!(app.log.screen()[&2505], 200.0);
    drop(root);

    app.items.retain(|item| item.key >= 2505);
    let mut root = app.view();
    ui.rebuild(&mut root);
    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert_eq!(app.log.screen()[&2505], 0.0);
    app.log
        .assert_covered(ui.translation(&mut root), ui.translation(&mut root) + 600.0);
    ui.assert_invariants();
    assert_eq!(ui.frame(&mut root).passes, [Invalidation::None]);
    drop(root);

    // Only that row is kept: it cannot fill the viewport, so the drift is folded and
    // it ends up at the top of the list, at the start.
    app.items.retain(|item| item.key == 2505);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(app.log.screen()[&2505], 0.0);
    ui.assert_invariants();
    drop(root);

    // Every shown row is removed, and many rows below them are kept: no shown
    // survivor, so the drift is folded, and the rows that are now at the scroller's
    // translation fill the viewport: the 40 rows removed from 2490 bring row 2540
    // to the top, where row 2500 was.
    app.items = rows(0..5000, 40.0);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.scroll(&mut root, 100_000.0);
    drop(root);

    app.items.retain(|item| !(2490..2530).contains(&item.key));
    let mut root = app.view();
    ui.rebuild(&mut root);
    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    let translation = ui.translation(&mut root);
    app.log.assert_covered(translation, translation + 600.0);
    assert_eq!(app.log.screen()[&2540], 0.0);
    assert_eq!(ui.core().geometry.drift(), 0.0);
    ui.assert_invariants();
    assert_eq!(ui.frame(&mut root).passes, [Invalidation::None]);
}

#[test]
fn gap_rule_checks_the_drawn_region_of_an_unreachable_landing() {
    // Review F1-R1 F1: an instant Start on row 100, then, before the landing frame,
    // a filter keeps only rows 100..=109. The destination at 4000 is out of reach
    // (400 px of rows), so the drawn region is clamped, and the drift must not show
    // there as a gap.
    let mut app = direct(rows(0..200, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);

    ui.run_task(
        &mut root,
        scroll_task(100, Align::Start, Animation::Instant),
    );
    drop(root);

    app.items.retain(|item| (100..=109).contains(&item.key));
    let mut root = app.view();
    ui.rebuild(&mut root);
    // The frame takes all three passes: the clamp is final on the second one, rule G
    // folds the drift there, and the third lays out the rows from the start.
    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes.len(), 3);
    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(app.log.screen()[&100], 0.0);
    app.log.assert_covered(0.0, 400.0);
    assert_eq!(ui.core().geometry.drift(), 0.0);

    // The landing was drawn at the start and goes with the next frame, rule Z's.
    assert_eq!(ui.frame(&mut root).passes, [Invalidation::None]);
    assert_eq!(app.log.screen()[&100], 0.0);
    ui.assert_invariants();
    drop(root);

    // The same through a waiting focus, which lands the same way: no gap, and the
    // focus commits.
    app.items = rows(0..200, 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.run_task(&mut root, FocusItem::new(LIST, 105_u32, field(105)));
    assert!(ui.core().focus.deferred.is_some());
    drop(root);

    app.items.retain(|item| (100..=109).contains(&item.key));
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(ui.translation(&mut root), 0.0);
    assert_eq!(app.log.screen()[&100], 0.0);
    app.log.assert_covered(0.0, 400.0);
    ui.frame(&mut root);
    assert_eq!(single_focus(&mut ui, &mut root), [field(105)]);
    ui.assert_invariants();
}

#[test]
fn gap_rule_waits_for_the_retained_rows() {
    // Review F1-R1 F2: 15 rows left after 100 removed above, end pinned. In one
    // update row 100 shrinks to 0 and row 101 grows to 80, so neither the total nor
    // row 102's position changes. Measured one by one, the first change alone
    // would look like rows too short to fill the viewport.
    let mut app = App::estimated(rows(0..200, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 4000.0);
    drop(root);

    app.items.drain(0..100);
    app.items.truncate(15);
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert!(ui.end_pinned());
    let before = app.log.screen()[&102];

    app.log
        .overrides
        .borrow_mut()
        .extend([(100, 0.0), (101, 80.0)]);
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&102], before);
    ui.assert_invariants();
}

#[test]
fn shown_items_are_taken_before_new_settings() {
    // Review F1-R1 F3: one update removes rows 100..=109 and sets 50 px spacing. Row
    // 110 was shown at y 400 before it, so it takes the removed anchor's place.
    let mut app = direct(rows(0..1000, 40.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 4000.0);
    assert_eq!(app.log.screen()[&110], 400.0);
    drop(root);

    app.items.retain(|item| !(100..=109).contains(&item.key));
    app.spacing = 50.0;
    let mut root = app.view();
    ui.rebuild(&mut root);
    ui.frame(&mut root);
    assert_eq!(app.log.screen()[&110], 0.0);
    ui.assert_invariants();
}

#[test]
fn shrink_runs_once_per_redraw_while_the_parent_clamps() {
    // Review F1-R1 F4, rule S's guard without a gap: 26 rows of 40 px left after 100
    // removed above, at 5000. The first pass shrinks the drift. The next passes see
    // the parent clamp against the shorter content, which is not scrolling, so they
    // shrink nothing more.
    let mut app = App::estimated(rows(0..300, 40.0), 40.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 5000.0);
    drop(root);

    app.items.drain(0..100);
    app.items.truncate(26);
    let mut root = app.view();
    ui.rebuild(&mut root);
    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes.len(), 3);
    assert_eq!(ui.core().geometry.drift(), 3592.0);
    assert_eq!(ui.translation(&mut root), 4082.0);
    assert_eq!(ui.core().geometry.shrunk_at, Some(ui.now));
    ui.assert_invariants();
}

#[test]
fn estimate_adopts_the_median_of_the_first_walk() {
    use super::heights::Estimate;

    let estimate = |ui: &Harness| ui.core().geometry.heights.estimate();

    // A tall first row, like an expanded item, among normal rows: the unmeasured
    // rows count with the normal height, not the tall one.
    let mut items = rows(0..1000, 40.0);
    items[0].height = Height::Fixed(100.0);
    let app = App::new(items);
    let mut root = app.view();
    let ui = Harness::new(&mut root, &app.log);
    assert_eq!(estimate(&ui), Estimate::Measured(40 * 256));
    assert_eq!(ui.list().1.size.height, 100.0 + 999.0 * 40.0);
    drop(root);

    // Mixed heights: their median, the lower middle one of an even count.
    let items: Vec<Item> = (0..1000)
        .map(|key| Item {
            key,
            height: Height::Fixed([20.0, 50.0, 30.0, 90.0][key as usize % 4]),
        })
        .collect();
    let app = App::new(items);
    let mut root = app.view();
    let ui = Harness::new(&mut root, &app.log);
    assert_eq!(estimate(&ui), Estimate::Measured(30 * 256));
    drop(root);

    // A first walk of empty rows adopts nothing. The next walk that measures
    // positive heights does.
    let app = App::new(rows(0..10, 0.0));
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    assert_eq!(estimate(&ui), Estimate::Provisional);

    for key in 0..10 {
        app.log.overrides.borrow_mut().insert(key, 25.0);
    }

    ui.rebuild(&mut root);
    assert_eq!(estimate(&ui), Estimate::Measured(25 * 256));
    drop(root);

    // An explicit estimate is never replaced.
    let app = App::estimated(rows(0..1000, 40.0), 100.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.scroll(&mut root, 20_000.0);
    assert_eq!(estimate(&ui), Estimate::Given(100 * 256));
}

#[test]
fn adopting_an_estimate_keeps_the_visible_rows_still() {
    use super::heights::Estimate;

    // Rows of 30 px estimated at 80, scrolled into the middle, where most rows above
    // were never measured. Dropping the estimate goes back to the provisional one,
    // and the next walk adopts 30: the unmeasured rows above change height, and the
    // visible rows stay where they are.
    let mut app = App::estimated(rows(0..2000, 30.0), 80.0);
    let mut root = app.view();
    let mut ui = Harness::new(&mut root, &app.log);
    ui.frame(&mut root);
    ui.scroll(&mut root, 40_000.0);
    let screen = app.log.screen();
    drop(root);

    app.estimate = None;
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(
        ui.core().geometry.heights.estimate(),
        Estimate::Measured(30 * 256)
    );

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes.last(), Some(&Invalidation::None));
    assert_eq!(app.log.screen(), screen);
    ui.assert_invariants();
    assert_eq!(ui.frame(&mut root).passes, [Invalidation::None]);
}
