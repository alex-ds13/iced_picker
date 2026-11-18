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
use super::window::{Anchor, Core, Walk};
use super::{Status, Style, StyleFn, list_view};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay::Group;
use iced::advanced::renderer;
use iced::advanced::shell::{Bus, Diff, Invalidation, Waker};
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, Scrollable, scroll_to};
use iced::advanced::widget::operation::{Animation, Operation};
use iced::advanced::widget::{Id, Meta, Tree, tree};
use iced::advanced::{Shell, Widget};
use iced::time::Instant;
use iced::widget::{column, scrollable, space, sticky, text};
use iced::{
    Color, Element, Event, Fill, Length, Padding, Point, Rectangle, Size, Theme, Vector,
    Widget as _, mouse, touch,
};
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
                Height::Fixed(_) | Height::Area(_) | Height::Group { .. } => Length::Shrink,
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
        })
    }

    fn layout(&mut self, tree: &mut Tree, _renderer: &(), limits: &layout::Limits) {
        self.log.layouts.borrow_mut().push((self.item.key, *limits));

        let grown = self.log.overrides.borrow().get(&self.item.key).copied();
        let intrinsic = match self.item.height {
            _ if let Some(height) = grown => height,
            Height::Fixed(height) | Height::Fill(height) => height,
            Height::Area(area) => area / limits.max.width,
            Height::Group { .. } => unreachable!("a group is a column, not a probe"),
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
                shell.invalidate_layout();
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
        _tree: &mut Tree,
        layout: Layout,
        _viewport: &Rectangle,
        _renderer: &(),
        _operation: &mut dyn Operation,
    ) {
        self.log
            .operated
            .borrow_mut()
            .push((self.item.key, layout.bounds()));
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

impl Widget<(), Theme, ()> for Sibling {
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fixed(self.height),
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
    height: Length,
    estimate: Option<f32>,
    anticipate: usize,
    spacing: f32,
    padding: Padding,
    highlight: Vec<u32>,
    text_color: Option<Color>,
}

impl App {
    fn new(items: Vec<Item>) -> Self {
        Self {
            items,
            log: Rc::default(),
            header: 0.0,
            scroller: true,
            direct: false,
            height: Fill,
            estimate: None,
            anticipate: 4,
            spacing: 0.0,
            padding: Padding::ZERO,
            highlight: Vec::new(),
            text_color: None,
        }
    }

    fn estimated(items: Vec<Item>, estimate: f32) -> Self {
        Self {
            estimate: Some(estimate),
            ..Self::new(items)
        }
    }

    fn view(&self) -> Root<'_> {
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
        let mut bus = Bus::new();
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

        assert!(bus.is_empty(), "nothing publishes messages in these tests");

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
        self.run(root, &event, Vec::new(), Some(view))
    }

    /// Finishes a frame whose first passes the test ran itself.
    fn complete(&mut self, root: &mut Root<'_>, event: &Event, passes: Vec<Invalidation>) -> Frame {
        self.run(root, event, passes, None)
    }

    /// The shell's retry loop (`winit/src/lib.rs:784-835`): at most three passes,
    /// Overlay and Layout retry, Widgets rebuilds the view before the next pass, and
    /// the third pass is drawn whatever it invalidated.
    fn run<'a>(
        &mut self,
        root: &mut Root<'a>,
        event: &Event,
        mut passes: Vec<Invalidation>,
        view: Option<&dyn Fn() -> Root<'a>>,
    ) -> Frame {
        while passes.len() < 3 && passes.last() != Some(&Invalidation::None) {
            if passes.last() == Some(&Invalidation::Widgets) {
                let view = view.expect("a Widgets invalidation needs the view");
                *root = view();
                self.rebuild(root);
                self.rebuilds += 1;
            }

            passes.push(self.update(root, event).invalidation);
        }

        self.draw(root);

        Frame {
            passes,
            root_layouts: self.root_layouts - self.frame_start,
        }
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

    fn walks(&self) -> usize {
        self.core().walks.len()
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

    // Without one after it, the last surviving key before it. Row 99 keeps the
    // target, and then the end of the list is in view, so the walk pins the end
    // where it now is (review P3-R1 F1).
    app.items.retain(|item| item.key < 102);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.anchor(), (None, Some(4045.0)));
    assert!(ui.end_pinned());
    assert_eq!(ui.screen_y(&mut root, 99), 0.0);
    assert_eq!(ui.core().geometry.drift(), 40.0);
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
    assert_eq!(visits.customs, 0);
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
fn candidate_implements_widget_for_production_renderer() {
    fn assert_widget<W: Widget<(), Theme, iced::Renderer>>(_widget: &W) {}

    let items = [3_u32, 1, 2];

    let list = list_view(&items, |_, number: &u32| text(number.to_string()))
        .key(|number: &u32| *number)
        .id(LIST)
        .width(Fill)
        .padding(4)
        .spacing(2)
        .estimated_height(20)
        .anticipate(2)
        .highlight([1])
        .style(super::default);
    assert_widget(&list);
    let _: Element<'_, (), Theme, iced::Renderer> = list.boxed();

    // Boxed rows of mixed widgets, indexed keys and a class.
    let class: StyleFn<'_, Theme> = Box::new(|_theme, _status| Style::default());
    let list = list_view(&items, |index, number: &u32| {
        if index == 0 {
            text(number.to_string()).boxed()
        } else {
            space().height(10).boxed()
        }
    })
    .class(class);
    assert_widget(&list);
}

#[test]
fn shrink_runs_once_per_redraw_so_the_drawn_translation_fits() {
    // Review P2-R1 F1: a 50 px footer below the list, positive drift, and a list so
    // short that the scrollable clamps its translation again after a shrink.
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

    // Pass 1 sees the translation clamped to 4450 - 600 and shrinks once. Pass 2 sees
    // it clamped to the shorter content, which is the ancestors catching up, not
    // scrolling, so it shrinks nothing and the layout stays.
    let frame = ui.frame(&mut root);
    assert_eq!(
        frame.passes,
        [Invalidation::Layout(Diff::Skip), Invalidation::None]
    );
    assert_eq!(ui.core().geometry.drift(), 3850.0);
    ui.assert_invariants();

    // The drawn translation is the clamp of the final content height, so the footer
    // ends at the bottom of the viewport.
    let content = ui.list().1.size.height + FOOTER;
    assert_eq!(content, 4300.0);
    assert_eq!(ui.translation(&mut root), content - WINDOW.height);
    assert_eq!(app.log.drawn_keys(), (100..110).collect::<Vec<u32>>());
    assert_eq!(app.log.screen()[&109], 4210.0 - 3700.0);

    // Nothing is left for a later frame.
    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(ui.core().geometry.drift(), 3850.0);
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
