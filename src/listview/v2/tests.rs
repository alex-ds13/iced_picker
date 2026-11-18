//! Headless tests of the candidate widget with iced's null renderer.
//!
//! [`Harness`] plays the runtime and the shell: every redraw gets up to three update
//! passes with the same timestamp, a Layout invalidation lays out the root and
//! rebuilds overlays inside its pass, and the frame is drawn after the last pass even
//! if it invalidated. Lists live in `scrollable(column![header, list, footer])`.

use super::geometry::Visible;
use super::window::{Core, Walk};
use super::{Status, Style, StyleFn, list_view};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::shell::{Bus, Diff, Invalidation, Waker};
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, Scrollable, scroll_to};
use iced::advanced::widget::operation::{Animation, Operation};
use iced::advanced::widget::{Id, Meta, Tree, tree};
use iced::advanced::{Shell, Widget};
use iced::time::Instant;
use iced::widget::{column, scrollable, space, text};
use iced::{
    Color, Element, Event, Fill, Length, Padding, Point, Rectangle, Size, Theme, Vector,
    Widget as _, mouse,
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
        let mut drawn = self.drawn.borrow().clone();
        drawn.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y));

        let mut covered = from;

        for row in &drawn {
            assert!(
                row.bounds.y <= covered + 1.0e-3,
                "gap at {covered} before row {} at {}",
                row.key,
                row.bounds.y
            );
            covered = covered.max(row.bounds.y + row.bounds.height);
        }

        assert!(covered >= to, "drawn rows end at {covered}, before {to}");
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
                Height::Fixed(_) | Height::Area(_) => Length::Shrink,
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

        let intrinsic = match self.item.height {
            Height::Fixed(height) | Height::Fill(height) => height,
            Height::Area(area) => area / limits.max.width,
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
        _layout: Layout,
        _cursor: mouse::Cursor,
        _renderer: &(),
        _shell: &mut Shell<'_, ()>,
        _viewport: &Rectangle,
    ) {
        assert_eq!(tree.state.downcast_ref::<RowState>().key, self.item.key);

        let redraw = matches!(event, Event::Window(window::Event::RedrawRequested(_)));
        self.log.updates.borrow_mut().push((self.item.key, redraw));
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

/// The application of most tests: a list of probes between a header and a footer.
struct App {
    items: Vec<Item>,
    log: Rc<Log>,
    header: f32,
    scroller: bool,
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

        let mut list = list_view(&self.items, move |_, item: &Item| Probe {
            item: *item,
            log: Rc::clone(&log),
        })
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

        let content =
            column![space().height(self.header), list, space().height(FOOTER)].width(Fill);

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

    fn overlays(&mut self, root: &mut Root<'_>) {
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

        assert!(overlays.is_empty(), "the probes have no overlays");
    }

    /// One runtime update. It never draws: a redraw request is only reported.
    fn update(&mut self, root: &mut Root<'_>, event: &Event) -> Updated {
        self.overlays(root);

        let layout = Layout::new(self.tree.size);
        let viewport = self.viewport();
        let mut bus = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);

        root.update(
            &mut self.tree,
            event,
            layout,
            self.cursor,
            &(),
            &mut shell,
            &viewport,
        );

        let invalidation = shell.invalidation();
        let captured = shell.is_event_captured();
        let redraw = shell.redraw_request();
        drop(shell);
        assert!(bus.is_empty(), "nothing publishes messages in these tests");

        match invalidation {
            Invalidation::None => {}
            Invalidation::Overlay => self.overlays(root),
            Invalidation::Layout(diff) => {
                if diff == Diff::Perform {
                    self.tree.diff(root);
                }

                self.layout(root);
                self.overlays(root);
            }
            Invalidation::Widgets => panic!("no test rebuilds the view during a frame"),
        }

        Updated {
            invalidation,
            captured,
            redraw,
        }
    }

    fn next_redraw(&mut self) -> Event {
        self.now += Duration::from_millis(16);
        self.frame_start = self.root_layouts;

        Event::Window(window::Event::RedrawRequested(self.now))
    }

    /// A redraw: up to three passes with the same event, then a draw.
    fn frame(&mut self, root: &mut Root<'_>) -> Frame {
        let event = self.next_redraw();
        self.complete(root, &event, Vec::new())
    }

    /// Finishes a frame whose first passes the test ran itself.
    fn complete(
        &mut self,
        root: &mut Root<'_>,
        event: &Event,
        mut passes: Vec<Invalidation>,
    ) -> Frame {
        while passes.len() < 3 && passes.last() != Some(&Invalidation::None) {
            passes.push(self.update(root, event).invalidation);
        }

        self.draw(root);

        Frame {
            passes,
            root_layouts: self.root_layouts - self.frame_start,
        }
    }

    fn draw(&mut self, root: &mut Root<'_>) {
        self.log.drawn.take();

        root.draw(
            &self.tree,
            &mut (),
            &Theme::Light,
            &renderer::Style { text_color: TEXT },
            Layout::new(self.tree.size),
            self.cursor,
            &self.viewport(),
        );
    }

    fn operate(&mut self, root: &mut Root<'_>, operation: &mut dyn Operation) {
        let layout = Layout::new(self.tree.size);
        let viewport = self.viewport();
        root.operate(&mut self.tree, layout, &viewport, &(), operation);
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

    fn anchor(&self) -> (Option<u32>, Option<f64>) {
        let core = self.core();
        (core.anchor, core.geometry.anchor_y())
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

        if let Some(key) = core.anchor {
            let position = core.model.position(&key).unwrap();
            assert_eq!(Some(geometry.content_y(position)), geometry.anchor_y());
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
    // Every row it built is kept and only the previous rows are evicted.
    app.log.clear();
    let frame = ui.scroll(&mut root, 4000.0);
    assert_eq!(frame.root_layouts, 1);
    assert_eq!(ui.walks(), walks + 1);
    assert_eq!(
        ui.last_walk(),
        Walk {
            built: 23,
            measured: 30 + 23,
            changed: 0,
            evicted: 30,
            discarded: 0,
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

    let walks = ui.walks();
    let frame = ui.scroll(&mut root, 0.0);
    assert_eq!(frame.root_layouts, 1);
    assert_eq!(ui.walks(), walks + 1);
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

    // Without one after it, the last surviving key before it.
    app.items.retain(|item| item.key < 102);
    let mut root = app.view();
    ui.rebuild(&mut root);
    assert_eq!(ui.anchor(), (Some(99), Some(4005.0)));
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

    // Overlays are asked of the current rows only, at the same bounds.
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
    // neither row 11 nor row 24 shows any content.
    let frame = ui.scroll(&mut root, 595.0);
    assert_eq!(
        frame.passes,
        [Invalidation::Layout(Diff::Skip), Invalidation::None]
    );
    assert_eq!(ui.positions(), range(12..=23));
    assert_eq!(app.log.drawn_keys(), (12..=23).collect::<Vec<u32>>());

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    ui.assert_invariants();
}

#[test]
fn retained_measurements_repin_the_ordinary_anchor() {
    // Review P2-R2 F2: row 0 is 10,000 px tall and the only visible row at top 600.
    let mut items = rows(0..1000, 1.0);
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
    // that any change, in either direction, shows here.
    let walk = ui.last_walk();
    assert_eq!(walk.built, 1580);
    assert_eq!(walk.discarded, 972);
    assert_eq!(walk.evicted, 604 + 972);
    assert_eq!(ui.positions(), range(2004..=2611));

    app.log.assert_covered(10_000.0, 10_600.0);
    ui.assert_invariants();

    let frame = ui.frame(&mut root);
    assert_eq!(frame.passes, [Invalidation::None]);
    assert_eq!(frame.root_layouts, 0);
}
