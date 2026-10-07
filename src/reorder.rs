//! A vertical container whose rows can be dragged to reorder them.
//!
//! Press a row and move past a small deadband to drag it. The row floats under
//! the cursor while an insertion line marks where it will drop. A press and
//! release without movement is left to the row's own widget, so a button row
//! still registers a click. In live mode ([`Reorder::live`]) the reorder is
//! emitted while the drag happens and the empty slot follows the row, instead
//! of the insertion line.
use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Meta, Operation, Tree, tree};
use iced::advanced::{Shell, Widget};
use iced::alignment::Alignment;
use iced::{
    Border, Color, Element, Event, Length, Padding, Pixels, Point, Rectangle, Size, Vector,
    Widget as _, mouse, touch,
};

/// How far the cursor must travel from the press point before a press turns
/// into a drag. Below it, the press stays a plain click on the row.
const DEADBAND: f32 = 5.0;

/// A vertical list of rows that can be reordered by dragging.
#[allow(missing_debug_implementations)]
pub struct Reorder<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    children: Vec<Element<'a, Message, Theme, Renderer>>,
    spacing: f32,
    padding: Padding,
    width: Length,
    live: bool,
    on_reorder: Option<Box<dyn Fn(usize, usize) -> Message + 'a>>,
    class: Theme::Class<'a>,
}

impl<'a, Message, Theme, Renderer> Reorder<'a, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    /// Creates an empty [`Reorder`].
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
            spacing: 0.0,
            padding: Padding::ZERO,
            width: Length::Fill,
            live: false,
            on_reorder: None,
            class: Theme::default(),
        }
    }

    /// Creates a [`Reorder`] from the given rows.
    pub fn with_children(
        children: impl IntoIterator<Item = impl Widget<Message, Theme, Renderer> + 'a>,
    ) -> Self {
        Self {
            children: children.into_iter().map(|child| child.boxed()).collect(),
            ..Self::new()
        }
    }

    /// Adds a row to the end of the list.
    pub fn push(mut self, child: impl Widget<Message, Theme, Renderer> + 'a) -> Self {
        self.children.push(child.boxed());
        self
    }

    /// Sets the vertical spacing between rows.
    pub fn spacing(mut self, spacing: impl Into<Pixels>) -> Self {
        self.spacing = spacing.into().0;
        self
    }

    /// Sets the padding around the list.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    /// Sets the width of the list.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the message produced on a drop, given the moved row's index and its
    /// target index. Without it, rows are not draggable.
    pub fn on_reorder(mut self, on_reorder: impl Fn(usize, usize) -> Message + 'a) -> Self {
        self.on_reorder = Some(Box::new(on_reorder));
        self
    }

    /// Reorders live while dragging: as soon as the dragged row would land in
    /// a new slot, [`Self::on_reorder`] is emitted and the empty slot follows
    /// it, instead of showing an insertion line and emitting only on drop.
    ///
    /// This assumes the application applies every emitted reorder to its rows.
    /// One that is ignored leaves the tracked slot out of sync until the drop.
    pub fn live(mut self, live: bool) -> Self {
        self.live = live;
        self
    }

    /// Sets the style of the list.
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the style class of the list.
    #[must_use]
    pub fn class(mut self, class: impl Into<Theme::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }
}

impl<'a, Message, Theme, Renderer> Default for Reorder<'a, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    fn default() -> Self {
        Self::new()
    }
}

/// The drag state kept in the widget tree.
#[derive(Default)]
struct State {
    action: Action,
    layout: layout::flex::Cache,
}

#[derive(Default)]
enum Action {
    /// Nothing is being dragged.
    #[default]
    Idle,
    /// A row was pressed, but the cursor has not passed the deadband yet.
    Pending { index: usize, origin: Point },
    /// A row is being dragged.
    Dragging {
        index: usize,
        grab_offset: Vector,
        cursor: Point,
    },
}

impl Action {
    /// The row index the action refers to, if any.
    fn index(&self) -> Option<usize> {
        match self {
            Action::Idle => None,
            Action::Pending { index, .. } | Action::Dragging { index, .. } => Some(*index),
        }
    }
}

/// The index of the row whose bounds contain `point`, if any.
fn row_at(layout: Layout, children: &[Tree], point: Point) -> Option<usize> {
    layout
        .iter(children)
        .position(|(row, _)| row.bounds().contains(point))
}

/// The target index for a row dragged to `cursor`: the slot whose row the
/// cursor is over (the count of rows fully above it), clamped to the last row.
/// It is the `to` of a `remove(from)` then `insert(to, item)`.
fn drop_index(layout: Layout, children: &[Tree], cursor: Point) -> usize {
    let ins = layout
        .iter(children)
        .filter(|(row, _)| row.bounds().y + row.bounds().height < cursor.y)
        .count();
    ins.min(children.len().saturating_sub(1))
}

impl<Message, Theme, Renderer> Meta for Reorder<'_, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Reorder<'_, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&mut self, tree: &mut Tree) {
        // An insert or remove mid-drag shifts the grabbed row off its index, so
        // a changed row count cancels the drag before it moves the wrong row.
        if tree.children.len() != self.children.len() {
            tree.state.downcast_mut::<State>().action = Action::Idle;
        }
        tree.diff_children(&mut self.children);
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: Length::Shrink,
        }
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        let state = tree.state.downcast_mut::<State>();

        tree.size = layout::flex::resolve(
            layout::flex::Axis::Vertical,
            renderer,
            limits,
            self.width,
            Length::Shrink,
            self.padding,
            self.spacing,
            Alignment::Start,
            &mut tree.children,
            &mut self.children,
            &mut state.layout,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds(), viewport);
        operation.traverse(&mut |operation| {
            self.children
                .iter_mut()
                .zip(layout.iter_mut(&mut tree.children))
                .for_each(|(child, (layout, state))| {
                    child.operate(state, layout, viewport, renderer, operation);
                });
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // Drop a stored index that fell out of range since the last rebuild,
        // then decide whether a drag is active. Scoped so the state borrow ends
        // before the children are forwarded to below.
        let dragging = {
            let state = tree.state.downcast_mut::<State>();
            if state
                .action
                .index()
                .is_some_and(|i| i >= self.children.len())
            {
                state.action = Action::Idle;
            }
            matches!(state.action, Action::Dragging { .. })
        };

        // While not dragging, the children see every event, so a press and
        // release without movement still fires the row's own click. A drag
        // swallows everything except redraw housekeeping, which the children
        // still need to keep their visual state fresh.
        if !dragging
            || matches!(
                event,
                Event::Window(iced::window::Event::RedrawRequested(_))
            )
        {
            for (child, (layout, tree)) in self
                .children
                .iter_mut()
                .zip(layout.iter_mut(&mut tree.children))
            {
                child.update(tree, event, layout, cursor, renderer, shell, viewport);
            }
        }

        if self.on_reorder.is_none() {
            return;
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if let Some(origin) = cursor.position()
                    && let Some(index) = row_at(layout, &tree.children, origin)
                {
                    tree.state.downcast_mut::<State>().action = Action::Pending { index, origin };
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. }) => {
                let Some(position) = cursor.land().position() else {
                    return;
                };
                let state = tree.state.downcast_mut::<State>();
                match state.action {
                    Action::Pending { index, origin } if position.distance(origin) > DEADBAND => {
                        let row = layout
                            .iter(&tree.children)
                            .nth(index)
                            .map(|(row, _)| row.bounds().position())
                            .unwrap_or(origin);
                        state.action = Action::Dragging {
                            index,
                            grab_offset: origin - row,
                            cursor: position,
                        };
                        shell.capture_event();
                        shell.invalidate_overlay();
                        shell.request_redraw();
                    }
                    Action::Dragging {
                        index, grab_offset, ..
                    } => {
                        let index = if self.live {
                            let to = drop_index(layout, &tree.children, position);
                            if to != index
                                && let Some(on_reorder) = &self.on_reorder
                            {
                                shell.publish(on_reorder(index, to));
                                to
                            } else {
                                index
                            }
                        } else {
                            index
                        };
                        state.action = Action::Dragging {
                            index,
                            grab_offset,
                            cursor: position,
                        };
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. })
            | Event::Touch(touch::Event::FingerLost { .. }) => {
                // Take the drag result and reset in a scope so the state borrow
                // is gone before the release is forwarded to the children.
                let drag = {
                    let state = tree.state.downcast_mut::<State>();
                    let drag = match state.action {
                        Action::Dragging { index, cursor, .. } => Some((index, cursor)),
                        _ => None,
                    };
                    state.action = Action::Idle;
                    drag
                };

                if let Some((index, drop_cursor)) = drag {
                    // Forward the release once so the source row's button clears
                    // its pressed state. Levitate the cursor so it publishes nothing.
                    let cursor = cursor.levitate();
                    for (child, (layout, tree)) in self
                        .children
                        .iter_mut()
                        .zip(layout.iter_mut(&mut tree.children))
                    {
                        child.update(tree, event, layout, cursor, renderer, shell, viewport);
                    }

                    // A live drag already emitted every move while it happened.
                    if !self.live {
                        let to = drop_index(layout, &tree.children, drop_cursor);
                        if to != index
                            && let Some(on_reorder) = &self.on_reorder
                        {
                            shell.publish(on_reorder(index, to));
                        }
                    }
                    shell.capture_event();
                    shell.invalidate_overlay();
                    shell.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if matches!(
            tree.state.downcast_ref::<State>().action,
            Action::Dragging { .. }
        ) {
            return mouse::Interaction::Grabbing;
        }
        self.children
            .iter()
            .zip(layout.iter(&tree.children))
            .map(|(child, (layout, tree))| {
                child.mouse_interaction(tree, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let dragged = match tree.state.downcast_ref::<State>().action {
            Action::Dragging { index, .. } => Some(index),
            _ => None,
        };

        for (i, (child, (layout, tree))) in self
            .children
            .iter()
            .zip(layout.iter(&tree.children))
            .enumerate()
        {
            // The dragged row only appears in the overlay, so its slot stays
            // an empty gap of the same size.
            if Some(i) == dragged {
                continue;
            }
            child.draw(tree, renderer, theme, style, layout, cursor, viewport);
        }

        let Action::Dragging {
            index,
            cursor: drop_cursor,
            ..
        } = tree.state.downcast_ref::<State>().action
        else {
            return;
        };

        let appearance = <Theme as Catalog>::style(theme, &self.class, Status::Dragging);
        let rows: Vec<Rectangle> = layout
            .iter(&tree.children)
            .map(|(layout, _)| layout.bounds())
            .collect();

        if let Some(bounds) = rows.get(index) {
            renderer.fill_quad(
                Quad {
                    bounds: *bounds,
                    border: appearance.slot_border,
                    ..Quad::default()
                },
                appearance.slot_background,
            );
        }

        // A live drag moves the slot itself, so it needs no insertion line.
        if self.live {
            return;
        }

        // The insertion line sits at the gap the row would drop into.
        let ins = drop_index(layout, &tree.children, drop_cursor);

        // If hovering over the same place it was grabbed don't draw insertion line.
        if ins == index {
            return;
        }

        let content = layout.bounds();
        let x = content.x + self.padding.left;
        let width = (content.width - self.padding.left - self.padding.right).max(0.0);
        let y = if rows.is_empty() {
            content.y
        } else if ins == 0 {
            rows[0].y
        } else if ins >= rows.len() - 1 {
            rows[rows.len() - 1].y + rows[rows.len() - 1].height
        } else if ins < index {
            (rows[ins - 1].y + rows[ins - 1].height + rows[ins].y) / 2.0
        } else {
            (rows[ins].y + rows[ins].height + rows[ins + 1].y) / 2.0
        };
        renderer.fill_quad(
            Quad {
                bounds: Rectangle {
                    x,
                    y: y - appearance.line_width / 2.0,
                    width,
                    height: appearance.line_width,
                },
                ..Quad::default()
            },
            appearance.line_color,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        if let Action::Dragging {
            index, grab_offset, ..
        } = tree.state.downcast_ref::<State>().action
            && index < self.children.len()
            && index < tree.children.len()
        {
            let child_tree = &tree.children[index];
            let position = layout.position() + child_tree.translation + translation;

            return vec![overlay::Element::new(Box::new(PickedRow {
                content: &self.children[index],
                tree: child_tree,
                layout: Layout::new(child_tree.size).move_to(position),
                grab_offset,
                window,
            }))];
        }

        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}

/// The floating copy of the row being dragged, drawn translated to the cursor.
struct PickedRow<'a, 'b, Message, Theme, Renderer> {
    content: &'b Element<'a, Message, Theme, Renderer>,
    tree: &'b Tree,
    layout: Layout,
    grab_offset: Vector,
    window: Size,
}

impl<'a, 'b, Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for PickedRow<'a, 'b, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        cursor: mouse::Cursor,
    ) {
        // Place the row so it keeps its grab offset under the cursor, even
        // when its slot has moved since the drag started (live mode).
        let translation = cursor
            .position()
            .map(|position| (position - self.grab_offset) - self.layout.bounds().position())
            .unwrap_or(Vector::ZERO);

        renderer.with_layer(Rectangle::with_size(self.window), |renderer| {
            renderer.with_translation(translation, |renderer| {
                self.content.draw(
                    self.tree,
                    renderer,
                    theme,
                    style,
                    self.layout,
                    mouse::Cursor::Unavailable,
                    &Rectangle::INFINITE,
                );
            });
        });
    }
}

/// Creates a [`Reorder`] from the given rows.
pub fn reorder<'a, Message, Theme, Renderer>(
    children: impl IntoIterator<Item = impl Widget<Message, Theme, Renderer> + 'a>,
) -> Reorder<'a, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    Reorder::with_children(children)
}

/// The state of a [`Reorder`] for styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No row is being dragged.
    Idle,
    /// A row is being dragged.
    Dragging,
}

/// The appearance of a [`Reorder`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The color of the insertion line shown at the drop position.
    pub line_color: Color,
    /// The thickness of the insertion line.
    pub line_width: f32,
    /// The fill of the dragged row's now-empty slot.
    pub slot_background: Color,
    /// The border of the dragged row's now-empty slot.
    pub slot_border: Border,
}

/// The theme catalog of a [`Reorder`].
pub trait Catalog {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

/// A styling function for a [`Reorder`].
///
/// This is just a boxed closure: `Fn(&Theme, Status) -> Style`.
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for iced::Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

/// The default style of a [`Reorder`], accenting the insertion line with the
/// primary color.
pub fn default(theme: &iced::Theme, _status: Status) -> Style {
    let palette = theme.palette();

    Style {
        line_color: palette.primary.strong.color,
        line_width: 2.0,
        slot_background: Color {
            a: 0.2,
            ..Color::BLACK
        },
        slot_border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 2.0.into(),
        },
    }
}

// iced's null renderer is only available in debug builds.
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    use iced::widget::Space;
    use iced_core::shell::{Bus, Waker};
    use iced_core::window;

    type TestReorder = Reorder<'static, (usize, usize), iced::Theme, ()>;

    fn test_widget() -> TestReorder {
        Reorder::new()
            .push(Space::new().width(50).height(20))
            .push(Space::new().width(70).height(30))
            .push(Space::new().width(40).height(40))
            .width(Length::Shrink)
            .padding(4)
            .spacing(6)
            .on_reorder(|from, to| (from, to))
    }

    fn tree_for(reorder: &mut TestReorder) -> Tree {
        let mut tree = Tree::new(reorder);
        tree.diff(reorder);
        reorder.layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(200.0, 200.0)),
        );
        tree
    }

    fn send_event(
        reorder: &mut TestReorder,
        tree: &mut Tree,
        bus: &mut Bus<(usize, usize)>,
        event: mouse::Event,
        position: Point,
    ) -> bool {
        let mut shell = Shell::new(&window::Headless, Waker::noop(), bus);
        let layout = Layout::new(tree.size);
        reorder.update(
            tree,
            &Event::Mouse(event),
            layout,
            mouse::Cursor::Available(position),
            &(),
            &mut shell,
            &Rectangle::with_size(Size::new(200.0, 200.0)),
        );
        shell.is_event_captured()
    }

    #[test]
    fn flex_layout_stores_row_geometry_and_preserves_drag_state() {
        let mut reorder = test_widget();
        let mut tree = tree_for(&mut reorder);
        assert_eq!(tree.size, Size::new(78.0, 110.0));
        assert_eq!(tree.children[0].translation, Vector::new(4.0, 4.0));
        assert_eq!(tree.children[1].translation, Vector::new(4.0, 30.0));
        assert_eq!(tree.children[2].translation, Vector::new(4.0, 66.0));
        assert_eq!(tree.children[1].size, Size::new(70.0, 30.0));

        tree.state.downcast_mut::<State>().action = Action::Pending {
            index: 1,
            origin: Point::new(8.0, 35.0),
        };
        reorder.layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(200.0, 200.0)),
        );
        assert_eq!(tree.state.downcast_ref::<State>().action.index(), Some(1));
    }

    #[test]
    fn hit_testing_and_drop_indices_include_tree_translations() {
        let mut reorder = test_widget();
        let tree = tree_for(&mut reorder);
        let layout = Layout::new(tree.size).move_to(Point::new(100.0, 200.0));
        assert_eq!(
            row_at(layout, &tree.children, Point::new(105.0, 235.0)),
            Some(1)
        );
        assert_eq!(
            row_at(layout, &tree.children, Point::new(105.0, 226.0)),
            None
        );
        assert_eq!(
            drop_index(layout, &tree.children, Point::new(105.0, 190.0)),
            0
        );
        assert_eq!(
            drop_index(layout, &tree.children, Point::new(105.0, 235.0)),
            1
        );
        assert_eq!(
            drop_index(layout, &tree.children, Point::new(105.0, 500.0)),
            2
        );
        assert_eq!(drop_index(layout, &[], Point::ORIGIN), 0);
    }

    #[test]
    fn a_click_stays_pending_and_a_drag_emits_only_on_drop() {
        let mut reorder = test_widget();
        let mut tree = tree_for(&mut reorder);
        let mut bus = Bus::new();
        let origin = Point::new(8.0, 8.0);
        let pressed = mouse::Event::ButtonPressed(mouse::Button::Left);
        let released = mouse::Event::ButtonReleased(mouse::Button::Left);
        assert!(!send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            pressed,
            origin
        ));
        let within_deadband = Point::new(10.0, 8.0);
        assert!(!send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            mouse::Event::CursorMoved {
                position: within_deadband
            },
            within_deadband
        ));
        assert!(!send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            released,
            within_deadband
        ));
        assert!(bus.is_empty());
        assert!(matches!(
            tree.state.downcast_ref::<State>().action,
            Action::Idle
        ));

        send_event(&mut reorder, &mut tree, &mut bus, pressed, origin);
        let dragging = Point::new(20.0, 8.0);
        assert!(send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            mouse::Event::CursorMoved { position: dragging },
            dragging
        ));
        let layout = Layout::new(tree.size).move_to(Point::new(30.0, 40.0));
        let window = Size::new(200.0, 200.0);
        assert_eq!(
            reorder
                .overlay(
                    &mut tree,
                    layout,
                    &(),
                    &Rectangle::with_size(window),
                    Vector::new(10.0, 20.0),
                    window
                )
                .len(),
            1
        );

        let dropped = Point::new(20.0, 150.0);
        send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            mouse::Event::CursorMoved { position: dropped },
            dropped,
        );
        assert!(bus.is_empty());
        assert!(send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            released,
            dropped
        ));
        assert_eq!(bus.into_iter().collect::<Vec<_>>(), vec![(0, 2)]);
        assert!(matches!(
            tree.state.downcast_ref::<State>().action,
            Action::Idle
        ));
    }

    #[test]
    fn live_reordering_survives_diff_and_does_not_repeat_on_drop() {
        let mut reorder = test_widget().live(true);
        let mut tree = tree_for(&mut reorder);
        let mut bus = Bus::new();
        let origin = Point::new(8.0, 8.0);
        send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            mouse::Event::ButtonPressed(mouse::Button::Left),
            origin,
        );
        let dragging = Point::new(20.0, 8.0);
        send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            mouse::Event::CursorMoved { position: dragging },
            dragging,
        );
        let dropped = Point::new(20.0, 90.0);
        send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            mouse::Event::CursorMoved { position: dropped },
            dropped,
        );
        assert_eq!(tree.state.downcast_ref::<State>().action.index(), Some(2));
        assert_eq!(bus.len(), 1);

        reorder.children.rotate_left(1);
        tree.diff(&mut reorder);
        reorder.layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(200.0, 200.0)),
        );
        assert_eq!(tree.state.downcast_ref::<State>().action.index(), Some(2));
        send_event(
            &mut reorder,
            &mut tree,
            &mut bus,
            mouse::Event::ButtonReleased(mouse::Button::Left),
            dropped,
        );
        assert_eq!(bus.into_iter().collect::<Vec<_>>(), vec![(0, 2)]);
    }

    #[test]
    fn a_changed_row_count_cancels_the_drag() {
        let mut reorder = test_widget();
        let mut tree = tree_for(&mut reorder);
        tree.state.downcast_mut::<State>().action = Action::Dragging {
            index: 1,
            grab_offset: Vector::new(4.0, 5.0),
            cursor: Point::new(20.0, 90.0),
        };
        reorder.children.pop();
        tree.diff(&mut reorder);
        assert_eq!(tree.children.len(), 2);
        assert!(matches!(
            tree.state.downcast_ref::<State>().action,
            Action::Idle
        ));
    }
}
