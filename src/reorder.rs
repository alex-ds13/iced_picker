//! A vertical container whose rows can be dragged to reorder them.
//!
//! Press a row and move past a small deadband to drag it. The row floats under
//! the cursor while an insertion line marks where it will drop. A press and
//! release without movement is left to the row's own widget, so a button row
//! still registers a click.
use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Shell, Widget};
use iced::alignment::Alignment;
use iced::{
    Color, Element, Event, Length, Padding, Pixels, Point, Rectangle, Size, Vector, mouse, touch,
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
            on_reorder: None,
            class: Theme::default(),
        }
    }

    /// Creates a [`Reorder`] from the given rows.
    pub fn with_children(
        children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        let mut list = Self::new();
        for child in children {
            list = list.push(child);
        }
        list
    }

    /// Adds a row to the end of the list.
    pub fn push(mut self, child: impl Into<Element<'a, Message, Theme, Renderer>>) -> Self {
        self.children.push(child.into());
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
        origin: Point,
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
fn row_at(layout: Layout<'_>, point: Point) -> Option<usize> {
    layout
        .children()
        .position(|row| row.bounds().contains(point))
}

/// The target index for a row dragged from `from` and dropped at `cursor`. The
/// raw insertion index counts the rows whose midpoint sits above the cursor,
/// then shifts down by one when it lands past the moved row so the app can
/// `remove(from)` then `insert(to, item)`.
fn drop_index(layout: Layout<'_>, cursor: Point, from: usize) -> usize {
    let ins = layout
        .children()
        .filter(|row| row.bounds().center_y() < cursor.y)
        .count();
    if ins > from { ins - 1 } else { ins }
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

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::flex::resolve(
            layout::flex::Axis::Vertical,
            renderer,
            limits,
            self.width,
            Length::Shrink,
            self.padding,
            self.spacing,
            Alignment::Start,
            &mut self.children,
            &mut tree.children,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            self.children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
                .for_each(|((child, state), layout)| {
                    child
                        .as_widget_mut()
                        .operate(state, layout, renderer, operation);
                });
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
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
        // release without movement still fires the row's own click.
        if !dragging {
            for ((child, tree), layout) in self
                .children
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                child
                    .as_widget_mut()
                    .update(tree, event, layout, cursor, renderer, shell, viewport);
            }
        }

        if self.on_reorder.is_none() {
            return;
        }

        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if let Some(origin) = cursor.position()
                    && let Some(index) = row_at(layout, origin)
                {
                    tree.state.downcast_mut::<State>().action = Action::Pending { index, origin };
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. })
            | Event::Touch(touch::Event::FingerMoved { .. }) => {
                let Some(position) = cursor.position() else {
                    return;
                };
                let state = tree.state.downcast_mut::<State>();
                match state.action {
                    Action::Pending { index, origin } if position.distance(origin) > DEADBAND => {
                        state.action = Action::Dragging {
                            index,
                            origin,
                            cursor: position,
                        };
                        shell.capture_event();
                        shell.request_redraw();
                    }
                    Action::Dragging { index, origin, .. } => {
                        state.action = Action::Dragging {
                            index,
                            origin,
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
                    // its pressed state. Its bounds are not under the cursor at
                    // a real drop, so it publishes nothing.
                    for ((child, tree), layout) in self
                        .children
                        .iter_mut()
                        .zip(&mut tree.children)
                        .zip(layout.children())
                    {
                        child
                            .as_widget_mut()
                            .update(tree, event, layout, cursor, renderer, shell, viewport);
                    }

                    let to = drop_index(layout, drop_cursor, index);
                    if to != index
                        && let Some(on_reorder) = &self.on_reorder
                    {
                        shell.publish(on_reorder(index, to));
                    }
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
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
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((child, tree), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(tree, layout, cursor, viewport, renderer)
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
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((child, tree), layout) in self
            .children
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
        {
            child
                .as_widget()
                .draw(tree, renderer, theme, style, layout, cursor, viewport);
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
        let rows: Vec<Rectangle> = layout.children().map(|l| l.bounds()).collect();

        // Dim the source slot, since the row itself is floating in the overlay.
        if let Some(bounds) = rows.get(index) {
            renderer.fill_quad(
                Quad {
                    bounds: *bounds,
                    ..Quad::default()
                },
                appearance.dragged_overlay,
            );
        }

        // The insertion line sits at the gap the row would drop into.
        let ins = rows.iter().filter(|b| b.center_y() < drop_cursor.y).count();
        let content = layout.bounds();
        let x = content.x + self.padding.left;
        let width = (content.width - self.padding.left - self.padding.right).max(0.0);
        let y = if rows.is_empty() {
            content.y
        } else if ins == 0 {
            rows[0].y
        } else if ins >= rows.len() {
            rows[rows.len() - 1].y + rows[rows.len() - 1].height
        } else {
            (rows[ins - 1].y + rows[ins - 1].height + rows[ins].y) / 2.0
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
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        if let Action::Dragging { index, origin, .. } = tree.state.downcast_ref::<State>().action
            && index < self.children.len()
            && let Some(child_layout) = layout.children().nth(index)
        {
            return Some(overlay::Element::new(Box::new(PickedRow {
                content: &self.children[index],
                tree: &tree.children[index],
                layout: child_layout,
                origin,
            })));
        }

        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

/// The floating copy of the row being dragged, drawn translated to the cursor.
struct PickedRow<'a, 'b, Message, Theme, Renderer> {
    content: &'b Element<'a, Message, Theme, Renderer>,
    tree: &'b Tree,
    layout: Layout<'b>,
    origin: Point,
}

impl<'a, 'b, Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for PickedRow<'a, 'b, Message, Theme, Renderer>
where
    Renderer: iced::advanced::Renderer,
{
    fn layout(&mut self, _renderer: &Renderer, _bounds: Size) -> layout::Node {
        layout::Node::new(self.layout.bounds().size()).move_to(self.origin)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        _layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let translation = cursor
            .position()
            .map(|position| position - self.origin)
            .unwrap_or(Vector::ZERO);

        renderer.with_translation(translation, |renderer| {
            self.content.as_widget().draw(
                self.tree,
                renderer,
                theme,
                style,
                self.layout,
                mouse::Cursor::Unavailable,
                &Rectangle::INFINITE,
            );
        });
    }
}

impl<'a, Message, Theme, Renderer> From<Reorder<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: Catalog + 'a,
    Renderer: iced::advanced::Renderer + 'a,
{
    fn from(reorder: Reorder<'a, Message, Theme, Renderer>) -> Self {
        Self::new(reorder)
    }
}

/// Creates a [`Reorder`] from the given rows.
pub fn reorder<'a, Message, Theme, Renderer>(
    children: impl IntoIterator<Item = Element<'a, Message, Theme, Renderer>>,
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
    /// A translucent tint drawn over the dragged row's original slot.
    pub dragged_overlay: Color,
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
        dragged_overlay: {
            let mut color = palette.background.base.color;
            color.a = 0.6;
            color
        },
    }
}
