#![allow(dead_code)]
//! Tooltips display a hint of information over some element when hovered.
//!
//! # Example
//! ```no_run
//! # pub type State = ();
//! use iced::Widget;
//! use iced::widget::container;
//! use iced_picker::tooltip::{self, tooltip};
//!
//! enum Message {
//!     // ...
//! }
//!
//! fn view(_state: &State) -> impl Widget<Message> {
//!     tooltip(
//!         "Hover me to display the tooltip!",
//!         container("This is the tooltip contents!")
//!             .padding(10)
//!             .style(container::rounded_box),
//!     )
//!     .position(tooltip::Position::Bottom)
//! }
//! ```
use core::layout::{self, Layout};
use core::mouse;
use core::overlay;
use core::renderer;
use core::text;
use core::time::{Duration, Instant};
use core::touch;
use core::widget::{self, Id, Widget};
use core::window;
use core::{
    Background, Color, Element, Event, Length, Padding, Pixels, Point, Rectangle, Shadow, Shell,
    Size, Theme, Vector,
    border::{self, Border},
    theme::palette,
};
use iced::widget::container;
use iced::{Task, Widget as _};
use iced_core as core;

/// An element to display a widget over another.
///
/// # Example
/// ```no_run
/// # pub type State = ();
/// use iced::Widget;
/// use iced::widget::container;
/// use iced_picker::tooltip::{self, tooltip};
///
/// enum Message {
///     // ...
/// }
///
/// fn view(_state: &State) -> impl Widget<Message> {
///     tooltip(
///         "Hover me to display the tooltip!",
///         container("This is the tooltip contents!")
///             .padding(10)
///             .style(container::rounded_box),
///     )
///     .position(tooltip::Position::Bottom)
/// }
/// ```
pub struct Tooltip<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Theme: container::Catalog + Catalog,
    Renderer: text::Renderer,
{
    id: Option<Id>,
    content: Element<'a, Message, Theme, Renderer>,
    tooltip: Element<'a, Message, Theme, Renderer>,
    position: Position,
    open: Open,
    disabled: bool,
    gap: f32,
    content_padding: Padding,
    padding: f32,
    snap_within_viewport: bool,
    delay: Duration,
    content_class: <Theme as Catalog>::Class<'a>,
    tooltip_class: <Theme as container::Catalog>::Class<'a>,
    status: Status,
}

/// The default [`Padding`] of a [`Tooltip`] content.
pub(crate) const DEFAULT_CONTENT_PADDING: Padding = Padding {
    top: 5.0,
    bottom: 5.0,
    right: 5.0,
    left: 5.0,
};

impl<'a, Message, Theme, Renderer> Tooltip<'a, Message, Theme, Renderer>
where
    Theme: container::Catalog + Catalog,
    Renderer: text::Renderer,
{
    /// The default padding of a [`Tooltip`] drawn by this renderer.
    const DEFAULT_PADDING: f32 = 0.0;

    /// Creates a new [`Tooltip`].
    ///
    /// [`Tooltip`]: struct.Tooltip.html
    pub fn new(
        content: impl Widget<Message, Theme, Renderer> + 'a,
        tooltip: impl Widget<Message, Theme, Renderer> + 'a,
    ) -> Self {
        Tooltip {
            id: None,
            content: content.boxed(),
            tooltip: tooltip.boxed(),
            position: Default::default(),
            open: Default::default(),
            disabled: false,
            gap: 0.0,
            content_padding: DEFAULT_CONTENT_PADDING,
            padding: Self::DEFAULT_PADDING,
            snap_within_viewport: true,
            delay: Duration::ZERO,
            content_class: <Theme as Catalog>::default(),
            tooltip_class: <Theme as container::Catalog>::default(),
            status: Default::default(),
        }
    }

    /// Sets the id for the [`Tooltip`].
    pub fn id(mut self, id: impl Into<Id>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets the gap between the content and its [`Tooltip`].
    pub fn gap(mut self, gap: impl Into<Pixels>) -> Self {
        self.gap = gap.into().0;
        self
    }

    /// Sets the padding of the [`Tooltip`] content.
    pub fn content_padding(mut self, padding: impl Into<Padding>) -> Self {
        self.content_padding = padding.into();
        self
    }

    /// Sets the padding of the [`Tooltip`].
    pub fn padding(mut self, padding: impl Into<Pixels>) -> Self {
        self.padding = padding.into().0;
        self
    }

    /// Sets the delay before the [`Tooltip`] is shown.
    ///
    /// Set to [`Duration::ZERO`] to be shown immediately.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Sets whether the [`Tooltip`] is snapped within the viewport.
    pub fn snap_within_viewport(mut self, snap: bool) -> Self {
        self.snap_within_viewport = snap;
        self
    }

    /// Sets how the [`Tooltip`] is opened.
    pub fn open(mut self, open: Open) -> Self {
        self.open = open;
        self
    }

    /// Sets how the [`Tooltip`] is positioned.
    pub fn position(mut self, position: Position) -> Self {
        self.position = position;
        self
    }

    /// Disables/enables this [`Tooltip`] to toggle it from showing the tooltip or not.
    pub fn disable(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Sets the style of the [`Tooltip`] content.
    #[must_use]
    pub fn content_style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        <Theme as Catalog>::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.content_class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the style of the [`Tooltip`].
    #[must_use]
    pub fn tooltip_style(mut self, style: impl Fn(&Theme) -> container::Style + 'a) -> Self
    where
        <Theme as container::Catalog>::Class<'a>: From<container::StyleFn<'a, Theme>>,
    {
        self.tooltip_class = (Box::new(style) as container::StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the style class of the [`Tooltip`] content.
    #[must_use]
    pub fn content_class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.content_class = class.into();
        self
    }

    /// Sets the style class of the [`Tooltip`].
    #[must_use]
    pub fn tooltip_class(
        mut self,
        class: impl Into<<Theme as container::Catalog>::Class<'a>>,
    ) -> Self {
        self.tooltip_class = class.into();
        self
    }
}

impl<Message, Theme, Renderer> widget::Meta for Tooltip<'_, Message, Theme, Renderer>
where
    Theme: container::Catalog + Catalog,
    Renderer: text::Renderer,
{
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Tooltip<'_, Message, Theme, Renderer>
where
    Theme: container::Catalog + Catalog,
    Renderer: text::Renderer,
{
    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.state.downcast_mut::<State>().needs_relayout = true;

        if tree.children.len() != 2 {
            tree.children = vec![
                widget::Tree::new(&self.content),
                widget::Tree::new(&self.tooltip),
            ];
        }

        tree.children[0].diff(&mut self.content);
        tree.children[1].diff(&mut self.tooltip);
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State::default())
    }

    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn size(&self) -> Size<Length> {
        self.content.size()
    }

    fn layout(&mut self, tree: &mut widget::Tree, renderer: &Renderer, limits: &layout::Limits) {
        let size = self.content.size();
        layout::padded(
            tree,
            limits,
            size.width,
            size.height,
            self.content_padding,
            |tree, limits| {
                self.content.layout(tree, renderer, limits);
                tree.size
            },
        );
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Mouse(_)
        | Event::Touch(_)
        | Event::Window(window::Event::RedrawRequested(_)) = event
        {
            let pointer_event = pointer_event(event);
            let state = tree.state.downcast_mut::<State>();
            let now = Instant::now();
            let previous_mode = state.mode;
            let was_idle = matches!(state.mode, Mode::Idle { .. });
            let is_over = cursor.is_over(layout.bounds());

            state.mode = if self.disabled {
                Mode::default()
            } else if let Mode::Opened {
                cursor_position,
                over_overlay,
            } = state.mode
            {
                if over_overlay {
                    state.mode
                } else {
                    match self.open {
                        Open::Hovered => cursor
                            .position_over(layout.bounds())
                            .map(|position| Mode::Opened {
                                cursor_position: if self.position == Position::FollowCursor {
                                    position
                                } else {
                                    cursor_position
                                },
                                over_overlay: false,
                            })
                            .unwrap_or_default(),
                        Open::LeftPointer => {
                            if let Some(mouse::Event::ButtonPressed(_)) = pointer_event {
                                Mode::default()
                            } else {
                                state.mode
                            }
                        }
                        Open::RightPointer => {
                            if let Some(mouse::Event::ButtonPressed(mouse::Button::Right)) =
                                pointer_event
                            {
                                cursor
                                    .position_over(layout.bounds())
                                    .map(|cursor_position| Mode::Opened {
                                        cursor_position,
                                        over_overlay: false,
                                    })
                                    .unwrap_or_default()
                            } else if let Some(mouse::Event::ButtonPressed(mouse::Button::Left)) =
                                pointer_event
                            {
                                Mode::default()
                            } else {
                                state.mode
                            }
                        }
                    }
                }
            } else if self.open == Open::Hovered {
                let cursor_position = cursor.position_over(layout.bounds());

                match (state.mode, cursor_position) {
                    (
                        Mode::Idle {
                            pressed,
                            hovered_at: None,
                        },
                        Some(cursor_position),
                    ) => {
                        if self.delay == Duration::ZERO {
                            Mode::Opened {
                                cursor_position,
                                over_overlay: false,
                            }
                        } else {
                            shell.request_redraw_at(now + self.delay);

                            Mode::Idle {
                                pressed,
                                hovered_at: Some(now),
                            }
                        }
                    }
                    (
                        Mode::Idle {
                            hovered_at: Some(_),
                            ..
                        },
                        None,
                    ) => Mode::default(),
                    (
                        Mode::Idle {
                            hovered_at: Some(at),
                            ..
                        },
                        _,
                    ) if at.elapsed() < self.delay => {
                        shell.request_redraw_at(now + self.delay - at.elapsed());
                        state.mode
                    }
                    (
                        Mode::Idle {
                            hovered_at: Some(_),
                            ..
                        },
                        Some(cursor_position),
                    ) => Mode::Opened {
                        cursor_position,
                        over_overlay: false,
                    },
                    _ => state.mode,
                }
            } else if let Some(mouse::Event::ButtonReleased(mouse::Button::Left)) = pointer_event
                && self.open == Open::LeftPointer
                && matches!(state.mode, Mode::Idle { pressed, .. } if pressed)
            {
                cursor
                    .position_over(layout.bounds())
                    .map(|cursor_position| Mode::Opened {
                        cursor_position,
                        over_overlay: false,
                    })
                    .unwrap_or_default()
            } else if let Some(mouse::Event::ButtonReleased(mouse::Button::Right)) = pointer_event
                && self.open == Open::RightPointer
                && matches!(state.mode, Mode::Idle { pressed, .. } if pressed)
            {
                cursor
                    .position_over(layout.bounds())
                    .map(|cursor_position| Mode::Opened {
                        cursor_position,
                        over_overlay: false,
                    })
                    .unwrap_or_default()
            } else if let Some(mouse::Event::ButtonPressed(mouse::Button::Left)) = pointer_event
                && self.open == Open::LeftPointer
            {
                cursor
                    .position_over(layout.bounds())
                    .map(|_| Mode::Idle {
                        pressed: true,
                        hovered_at: None,
                    })
                    .unwrap_or_default()
            } else if let Some(mouse::Event::ButtonPressed(mouse::Button::Right)) = pointer_event
                && self.open == Open::RightPointer
            {
                cursor
                    .position_over(layout.bounds())
                    .map(|_| Mode::Idle {
                        pressed: true,
                        hovered_at: None,
                    })
                    .unwrap_or_default()
            } else {
                state.mode
            };

            let is_idle = matches!(state.mode, Mode::Idle { .. });

            if was_idle != is_idle {
                if was_idle {
                    state.needs_relayout = true;
                }

                shell.invalidate_overlay();
                shell.request_redraw();
            } else if self.position == Position::FollowCursor && state.mode != previous_mode {
                shell.request_redraw();
            }

            let previous_status = self.status;
            if self.disabled {
                self.status = Status::Disabled;
            } else {
                match state.mode {
                    Mode::Idle { pressed, .. } => {
                        self.status = if pressed {
                            match self.open {
                                Open::Hovered => Status::Idle,
                                Open::LeftPointer => Status::LeftPressed,
                                Open::RightPointer => Status::RightPressed,
                            }
                        } else if is_over {
                            Status::Hovered
                        } else {
                            Status::Idle
                        };
                    }
                    Mode::Opened { .. } => {
                        self.status = Status::Opened;
                    }
                }
            }
            if !matches!(event, Event::Window(window::Event::RedrawRequested(_)))
                && shell.redraw_request() != window::RedrawRequest::NextFrame
                && previous_status != self.status
            {
                shell.request_redraw();
            }
        }

        let (content_layout, content_tree) = layout.iter_mut(&mut tree.children).next().unwrap();

        self.content.update(
            content_tree,
            event,
            content_layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let (content_layout, content_tree) = layout.iter(&tree.children).next().unwrap();
        let interaction = self.content.mouse_interaction(
            content_tree,
            content_layout,
            cursor,
            viewport,
            renderer,
        );

        if cursor.is_over(layout.bounds())
            && matches!(self.open, Open::LeftPointer)
            && self.status != Status::Disabled
            && interaction == mouse::Interaction::None
        {
            mouse::Interaction::Pointer
        } else {
            interaction
        }
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let style = Catalog::style(theme, &self.content_class, self.status);
        let (content_layout, content_tree) = layout.iter(&tree.children).next().unwrap();

        let bounds = layout.bounds();

        if style.background.is_some() || style.border.width > 0.0 || style.shadow.color.a > 0.0 {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: style.border,
                    shadow: style.shadow,
                    snap: style.snap,
                },
                style
                    .background
                    .unwrap_or(Background::Color(Color::TRANSPARENT)),
            );
        }

        self.content.draw(
            content_tree,
            renderer,
            theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            content_layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();

        operation.container(self.id.as_ref(), layout.bounds(), viewport);
        operation.traverse(&mut |operation| {
            let (content_layout, content_tree) =
                layout.iter_mut(&mut tree.children).next().unwrap();
            self.content
                .operate(content_tree, content_layout, viewport, renderer, operation);
        });
        operation.custom(self.id.as_ref(), layout.bounds(), state);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        let state = tree.state.downcast_mut::<State>();
        let State {
            mode,
            needs_relayout,
        } = state;

        let mut children = layout.iter_mut(&mut tree.children);
        let (content_layout, content_tree) = children.next().unwrap();

        let content = self.content.overlay(
            content_tree,
            content_layout,
            renderer,
            viewport,
            translation,
            window,
        );

        let (_, tooltip_tree) = children.next().unwrap();

        if matches!(mode, Mode::Opened { .. }) && *needs_relayout {
            self.tooltip.layout(
                tooltip_tree,
                renderer,
                &layout::Limits::new(
                    Size::ZERO,
                    if self.snap_within_viewport {
                        window
                    } else {
                        Size::INFINITE
                    },
                )
                .shrink(Padding::new(self.padding)),
            );
            *needs_relayout = false;
        }

        let tooltip = if let Mode::Opened {
            cursor_position,
            over_overlay: _,
        } = *mode
        {
            let tooltip_bounds = resolve_tooltip_bounds(
                self.position,
                layout.bounds() + translation,
                tooltip_tree.size,
                self.padding,
                self.gap,
                cursor_position + translation,
                Rectangle::with_size(window),
                self.snap_within_viewport,
            );

            Some(overlay::Element::new(Box::new(Overlay {
                layout: Layout::new(tooltip_tree.size)
                    .move_to(tooltip_bounds.position() + Vector::new(self.padding, self.padding)),
                padding: self.padding,
                tooltip: &mut self.tooltip,
                state: tooltip_tree,
                tooltip_state: mode,
                positioning: self.position,
                open: self.open,
                class: &self.tooltip_class,
                window,
            })))
        } else {
            None
        };

        content.into_iter().chain(tooltip).collect()
    }
}

/// The position of the tooltip. Defaults to following the cursor.
///
/// When snapping within the viewport, fixed directional positions flip to the
/// opposite side if the padded tooltip does not fit on the requested side and
/// the opposite side has strictly more room. Corner positions retain their
/// left or right content alignment. [`Position::FollowCursor`] never flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    /// The tooltip will appear on the preferred side if the padded tooltip
    /// fits there, or on the side with the most available space otherwise.
    /// This side selection also applies when viewport snapping is disabled.
    Auto {
        /// The side to prefer when the padded tooltip fits there.
        preference: Side,
    },
    /// The tooltip will appear on the top of the widget.
    Top,
    /// The tooltip will appear on the bottom of the widget.
    Bottom,
    /// The tooltip will appear on the left of the widget.
    Left,
    /// The tooltip will appear on the right of the widget.
    Right,
    /// The tooltip will follow the cursor.
    #[default]
    FollowCursor,
    /// The tooltip will appear aligned to the top right of the widget.
    TopRight,
    /// The tooltip will appear aligned to the top left of the widget.
    TopLeft,
    /// The tooltip will appear aligned to the bottom right of the widget.
    BottomRight,
    /// The tooltip will appear aligned to the bottom left of the widget.
    BottomLeft,
}

/// A side that a tooltip can appear on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    /// The top side.
    Top,
    /// The bottom side.
    #[default]
    Bottom,
    /// The left side.
    Left,
    /// The right side.
    Right,
}

impl Side {
    fn available_space(self, anchor: Rectangle, viewport: Rectangle, popup: Size, gap: f32) -> f32 {
        match self {
            Self::Top => anchor.y - viewport.y - popup.height - gap,
            Self::Bottom => {
                viewport.y + viewport.height - (anchor.y + anchor.height) - popup.height - gap
            }
            Self::Left => anchor.x - viewport.x - popup.width - gap,
            Self::Right => {
                viewport.x + viewport.width - (anchor.x + anchor.width) - popup.width - gap
            }
        }
    }

    fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    fn position(self) -> Position {
        match self {
            Self::Top => Position::Top,
            Self::Bottom => Position::Bottom,
            Self::Left => Position::Left,
            Self::Right => Position::Right,
        }
    }
}

/// Resolves the padded tooltip rectangle. Anchor, cursor, and viewport are in
/// screen coordinates. The caller translates the cursor stored in [`Mode`].
#[allow(clippy::too_many_arguments)]
fn resolve_tooltip_bounds(
    position: Position,
    anchor: Rectangle,
    content: Size,
    padding: f32,
    gap: f32,
    cursor_position: Point,
    viewport: Rectangle,
    snap_within_viewport: bool,
) -> Rectangle {
    let popup = content.expand(Padding::new(padding));
    let available = |side: Side| side.available_space(anchor, viewport, popup, gap);
    let position = match position {
        Position::Auto { preference } => {
            let side = if available(preference) >= 0.0 {
                preference
            } else {
                [Side::Top, Side::Left, Side::Right, Side::Bottom]
                    .into_iter()
                    .max_by(|a, b| available(*a).total_cmp(&available(*b)))
                    .unwrap_or_default()
            };
            side.position()
        }
        position if snap_within_viewport => {
            let side = match position {
                Position::Top | Position::TopLeft | Position::TopRight => Some(Side::Top),
                Position::Bottom | Position::BottomLeft | Position::BottomRight => {
                    Some(Side::Bottom)
                }
                Position::Left => Some(Side::Left),
                Position::Right => Some(Side::Right),
                _ => None,
            };

            if side.is_some_and(|side| {
                available(side) < 0.0 && available(side.opposite()) > available(side)
            }) {
                match position {
                    Position::Top => Position::Bottom,
                    Position::Bottom => Position::Top,
                    Position::Left => Position::Right,
                    Position::Right => Position::Left,
                    Position::TopLeft => Position::BottomLeft,
                    Position::TopRight => Position::BottomRight,
                    Position::BottomLeft => Position::TopLeft,
                    Position::BottomRight => Position::TopRight,
                    _ => position,
                }
            } else {
                position
            }
        }
        position => position,
    };

    let x_center = anchor.x + (anchor.width - content.width) / 2.0;
    let y_center = anchor.y + (anchor.height - content.height) / 2.0;
    // Corners align the content edge, not the outer padded box.
    let offset = match position {
        Position::Top => Point::new(x_center, anchor.y - content.height - gap - padding),
        Position::Bottom => Point::new(x_center, anchor.y + anchor.height + gap + padding),
        Position::Left => Point::new(anchor.x - content.width - gap - padding, y_center),
        Position::Right => Point::new(anchor.x + anchor.width + gap + padding, y_center),
        Position::FollowCursor => Point::new(cursor_position.x, cursor_position.y - content.height),
        Position::TopRight => Point::new(
            anchor.x + anchor.width - content.width,
            anchor.y - content.height - gap - padding,
        ),
        Position::TopLeft => Point::new(anchor.x, anchor.y - content.height - gap - padding),
        Position::BottomRight => Point::new(
            anchor.x + anchor.width - content.width,
            anchor.y + anchor.height + gap + padding,
        ),
        Position::BottomLeft => Point::new(anchor.x, anchor.y + anchor.height + gap + padding),
        Position::Auto { .. } => unreachable!(),
    };

    let mut bounds = Rectangle {
        x: offset.x - padding,
        y: offset.y - padding,
        width: popup.width,
        height: popup.height,
    };

    if snap_within_viewport {
        if bounds.x < viewport.x {
            bounds.x = viewport.x;
        } else if viewport.x + viewport.width < bounds.x + bounds.width {
            bounds.x = viewport.x + viewport.width - bounds.width;
        }

        if bounds.y < viewport.y {
            bounds.y = viewport.y;
        } else if viewport.y + viewport.height < bounds.y + bounds.height {
            bounds.y = viewport.y + viewport.height - bounds.height;
        }
    }

    bounds
}

fn pointer_event(event: &Event) -> Option<mouse::Event> {
    match event {
        Event::Mouse(event) => Some(*event),
        Event::Touch(touch::Event::FingerPressed { .. }) => {
            Some(mouse::Event::ButtonPressed(mouse::Button::Left))
        }
        Event::Touch(touch::Event::FingerLifted { .. } | touch::Event::FingerLost { .. }) => {
            Some(mouse::Event::ButtonReleased(mouse::Button::Left))
        }
        Event::Touch(touch::Event::FingerMoved { position, .. }) => {
            Some(mouse::Event::CursorMoved {
                position: *position,
            })
        }
        _ => None,
    }
}

/// How should the tooltip open. Defaults to hovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Open {
    /// The tooltip will appear when hovered.
    #[default]
    Hovered,
    /// The tooltip will appear when pressing left pointer on it.
    LeftPointer,
    /// The tooltip will appear when pressing right pointer on it.
    RightPointer,
}

#[derive(Debug, Default)]
struct State {
    mode: Mode,
    needs_relayout: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Idle {
        pressed: bool,
        hovered_at: Option<Instant>,
    },
    Opened {
        cursor_position: Point,
        over_overlay: bool,
    },
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Idle {
            pressed: false,
            hovered_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Status {
    #[default]
    /// The [`Tooltip`] content can be hovered/pressed to show the tooltip.
    Idle,
    /// The [`Tooltip`] content is being hovered.
    Hovered,
    /// The [`Tooltip`] content is being pressed with left pointer.
    LeftPressed,
    /// The [`Tooltip`] content is being pressed with right pointer.
    RightPressed,
    /// The [`Tooltip`] is opened.
    Opened,
    /// The [`Tooltip`] won't show.
    Disabled,
}

/// The style of the [`Tooltip`] content.
///
/// If not specified with [`Tooltip::content_style`]
/// the theme will provide the style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The [`Background`] of the button.
    pub background: Option<Background>,
    /// The text [`Color`] of the button.
    pub text_color: Color,
    /// The [`Border`] of the button.
    pub border: Border,
    /// The [`Shadow`] of the button.
    pub shadow: Shadow,
    /// Whether the tooltip content should be snapped to the pixel grid.
    pub snap: bool,
}

impl Style {
    /// Updates the [`Style`] with the given [`Background`].
    pub fn with_background(self, background: impl Into<Background>) -> Self {
        Self {
            background: Some(background.into()),
            ..self
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: None,
            text_color: Color::BLACK,
            border: Border::default(),
            shadow: Shadow::default(),
            snap: true,
        }
    }
}

/// The theme catalog of a [`Tooltip`] content.
///
/// All themes that can be used with [`Tooltip`]
/// must implement this trait.
pub trait Catalog {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

/// A styling function for a [`Tooltip`] content.
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(text)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

/// A primary button; denoting a main action.
pub fn primary(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();
    let base = styled(palette.primary.base);

    match status {
        Status::Idle | Status::LeftPressed | Status::RightPressed => base,
        Status::Hovered | Status::Opened => Style {
            background: Some(Background::Color(palette.primary.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A secondary button; denoting a complementary action.
pub fn secondary(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();
    let base = styled(palette.secondary.base);

    match status {
        Status::Idle | Status::LeftPressed | Status::RightPressed => base,
        Status::Hovered | Status::Opened => Style {
            background: Some(Background::Color(palette.secondary.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A success button; denoting a good outcome.
pub fn success(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();
    let base = styled(palette.success.base);

    match status {
        Status::Idle | Status::LeftPressed | Status::RightPressed => base,
        Status::Hovered | Status::Opened => Style {
            background: Some(Background::Color(palette.success.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A warning button; denoting a risky action.
pub fn warning(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();
    let base = styled(palette.warning.base);

    match status {
        Status::Idle | Status::LeftPressed | Status::RightPressed => base,
        Status::Hovered | Status::Opened => Style {
            background: Some(Background::Color(palette.warning.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A danger button; denoting a destructive action.
pub fn danger(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();
    let base = styled(palette.danger.base);

    match status {
        Status::Idle | Status::LeftPressed | Status::RightPressed => base,
        Status::Hovered | Status::Opened => Style {
            background: Some(Background::Color(palette.danger.strong.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A text button; useful for links.
pub fn text(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();

    let base = Style {
        text_color: palette.background.base.text,
        ..Style::default()
    };

    match status {
        Status::Idle | Status::LeftPressed | Status::RightPressed => base,
        Status::Hovered | Status::Opened => Style {
            text_color: palette.background.base.text.scale_alpha(0.8),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A button using background shades.
pub fn background(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();
    let base = styled(palette.background.base);

    match status {
        Status::Idle => base,
        Status::LeftPressed | Status::RightPressed => Style {
            background: Some(Background::Color(palette.background.strong.color)),
            ..base
        },
        Status::Hovered | Status::Opened => Style {
            background: Some(Background::Color(palette.background.weak.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

/// A subtle button using weak background shades.
pub fn subtle(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();
    let base = styled(palette.background.weakest);

    match status {
        Status::Idle => base,
        Status::LeftPressed | Status::RightPressed => Style {
            background: Some(Background::Color(palette.background.strong.color)),
            ..base
        },
        Status::Hovered | Status::Opened => Style {
            background: Some(Background::Color(palette.background.weaker.color)),
            ..base
        },
        Status::Disabled => disabled(base),
    }
}

fn styled(pair: palette::Pair) -> Style {
    Style {
        background: Some(Background::Color(pair.color)),
        text_color: pair.text,
        border: border::rounded(2),
        ..Style::default()
    }
}

fn disabled(style: Style) -> Style {
    Style {
        background: style
            .background
            .map(|background| background.scale_alpha(0.5)),
        text_color: style.text_color.scale_alpha(0.5),
        ..style
    }
}

struct Overlay<'a, 'b, Message, Theme, Renderer>
where
    Theme: container::Catalog,
    Renderer: text::Renderer,
{
    layout: Layout,
    padding: f32,
    tooltip: &'b mut Element<'a, Message, Theme, Renderer>,
    state: &'b mut widget::Tree,
    tooltip_state: &'b mut Mode,
    positioning: Position,
    open: Open,
    class: &'b Theme::Class<'a>,
    window: Size,
}

impl<Message, Theme, Renderer> Overlay<'_, '_, Message, Theme, Renderer>
where
    Theme: container::Catalog,
    Renderer: text::Renderer,
{
    fn bounds(&self) -> Rectangle {
        self.layout.bounds().expand(self.padding)
    }
}

impl<Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for Overlay<'_, '_, Message, Theme, Renderer>
where
    Theme: container::Catalog,
    Renderer: text::Renderer,
{
    fn operate(&mut self, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        let bounds = self.bounds();
        let viewport = Rectangle::with_size(self.window);

        operation.container(None, bounds, &bounds);
        operation.traverse(&mut |operation| {
            self.tooltip
                .operate(self.state, self.layout, &viewport, renderer, operation);
        });
    }

    fn update(
        &mut self,
        event: &Event,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        if let Event::Mouse(_)
        | Event::Touch(_)
        | Event::Window(window::Event::RedrawRequested(_)) = event
        {
            let bounds = self.bounds();
            // Group hides the cursor from a parent while it is over a nested overlay.
            let preserve_hover =
                matches!(cursor, mouse::Cursor::Unavailable) && !self.overlay(renderer).is_empty();
            let state = &mut *self.tooltip_state;
            let previous_state = *state;
            let was_idle = matches!(*state, Mode::Idle { .. });

            let (cursor_position, over_overlay) = match *state {
                Mode::Opened {
                    cursor_position,
                    over_overlay,
                } => (cursor_position, over_overlay),
                Mode::Idle { .. } => (Point::default(), false),
            };
            *state = Mode::Opened {
                cursor_position,
                over_overlay: if preserve_hover {
                    over_overlay
                } else {
                    cursor.is_over(bounds)
                },
            };

            let is_idle = matches!(*state, Mode::Idle { .. });

            if was_idle != is_idle {
                shell.invalidate_overlay();
                shell.request_redraw();
            } else if self.positioning == Position::FollowCursor && *state != previous_state {
                shell.request_redraw();
            }
        }

        let viewport = Rectangle::with_size(self.window);

        self.tooltip.update(
            self.state,
            event,
            self.layout,
            cursor,
            renderer,
            shell,
            &viewport,
        );

        if cursor.is_over(self.bounds()) && matches!(event, Event::Mouse(_) | Event::Touch(_)) {
            shell.capture_event();
        }
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        inherited_style: &renderer::Style,
        cursor: mouse::Cursor,
    ) {
        let viewport = Rectangle::with_size(self.window);
        let bounds = self.bounds();
        let style = theme.style(self.class);

        renderer.with_layer(viewport, |renderer| {
            container::draw_background(renderer, &style, bounds);

            let defaults = renderer::Style {
                text_color: style.text_color.unwrap_or(inherited_style.text_color),
            };

            self.tooltip.draw(
                self.state,
                renderer,
                theme,
                &defaults,
                self.layout,
                cursor,
                &viewport,
            );
        });
    }

    fn mouse_interaction(&self, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        let viewport = Rectangle::with_size(self.window);
        let interaction =
            self.tooltip
                .mouse_interaction(self.state, self.layout, cursor, &viewport, renderer);

        if cursor.is_over(self.bounds()) && interaction == mouse::Interaction::None {
            mouse::Interaction::Idle
        } else {
            interaction
        }
    }

    fn overlay<'c>(
        &'c mut self,
        renderer: &Renderer,
    ) -> Vec<overlay::Element<'c, Message, Theme, Renderer>> {
        self.tooltip.overlay(
            self.state,
            self.layout,
            renderer,
            &Rectangle::with_size(self.window),
            Vector::ZERO,
            self.window,
        )
    }

    fn index(&self) -> f32 {
        match self.open {
            Open::Hovered => 1.0,
            Open::LeftPointer | Open::RightPointer => 2.0,
        }
    }
}

fn close_all_operation<T>() -> impl widget::Operation<T> {
    struct Close;

    impl<T> widget::Operation<T> for Close {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation<T>)) {
            operate(self)
        }

        fn custom(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn std::any::Any) {
            if let Some(state) = state.downcast_mut::<State>() {
                state.mode = Mode::default();
            }
        }
    }

    Close
}

fn close_operation<T>(id: Id) -> impl widget::Operation<T> {
    struct Close {
        target: Id,
    }

    impl<T> widget::Operation<T> for Close {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation<T>)) {
            operate(self)
        }

        fn custom(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn std::any::Any) {
            if id == Some(&self.target)
                && let Some(state) = state.downcast_mut::<State>()
            {
                state.mode = Mode::default();
            }
        }
    }

    Close { target: id }
}

pub fn close_all<T: Send + 'static>() -> Task<T> {
    iced::advanced::widget::operate(close_all_operation::<T>()).discard()
}

pub fn close<T: Send + 'static>(id: impl Into<Id>) -> Task<T> {
    iced::advanced::widget::operate(close_operation::<T>(id.into())).discard()
}

pub fn tooltip<'a, Message, Theme, Renderer>(
    content: impl Widget<Message, Theme, Renderer> + 'a,
    tooltip: impl Widget<Message, Theme, Renderer> + 'a,
) -> Tooltip<'a, Message, Theme, Renderer>
where
    Theme: container::Catalog + Catalog,
    Renderer: text::Renderer,
{
    Tooltip::new(content, tooltip)
}

// iced's null renderer is only available in debug builds.
#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    use iced::widget::Space;
    use iced_core::event;
    use iced_core::shell::{Bus, Invalidation, Waker};
    use iced_core::widget::Operation as _;

    fn test_widget() -> Tooltip<'static, (), Theme, ()> {
        Tooltip::new(
            Space::new().width(80).height(30),
            Space::new().width(40).height(20),
        )
    }

    fn tree_for(tooltip: &mut Tooltip<'_, (), Theme, ()>) -> widget::Tree {
        let mut tree = widget::Tree::new(tooltip);
        tree.diff(tooltip);
        tooltip.layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(200.0, 100.0)),
        );
        tree
    }

    fn move_cursor(
        tooltip: &mut Tooltip<'_, (), Theme, ()>,
        tree: &mut widget::Tree,
        position: Option<Point>,
    ) -> (Invalidation, window::RedrawRequest) {
        let mut bus = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
        let layout = Layout::new(tree.size);
        tooltip.update(
            tree,
            &Event::Mouse(mouse::Event::CursorMoved {
                position: position.unwrap_or(Point::ORIGIN),
            }),
            layout,
            position.map(mouse::Cursor::Available).unwrap_or_default(),
            &(),
            &mut shell,
            &Rectangle::with_size(Size::new(200.0, 100.0)),
        );
        (shell.invalidation(), shell.redraw_request())
    }

    struct NestedContent {
        has_nested: bool,
    }

    impl widget::Meta for NestedContent {}

    impl Widget<(), Theme, ()> for NestedContent {
        fn tag(&self) -> widget::tree::Tag {
            widget::tree::Tag::of::<Vec<Event>>()
        }

        fn state(&self) -> widget::tree::State {
            widget::tree::State::new(Vec::<Event>::new())
        }

        fn size(&self) -> Size<Length> {
            Size::new(Length::Fixed(40.0), Length::Fixed(20.0))
        }

        fn layout(&mut self, tree: &mut widget::Tree, _renderer: &(), _limits: &layout::Limits) {
            tree.size = Size::new(40.0, 20.0);
        }

        fn update(
            &mut self,
            tree: &mut widget::Tree,
            event: &Event,
            _layout: Layout,
            _cursor: mouse::Cursor,
            _renderer: &(),
            _shell: &mut Shell<'_, ()>,
            _viewport: &Rectangle,
        ) {
            tree.state.downcast_mut::<Vec<Event>>().push(event.clone());
        }

        fn draw(
            &self,
            _tree: &widget::Tree,
            _renderer: &mut (),
            _theme: &Theme,
            _style: &renderer::Style,
            _layout: Layout,
            _cursor: mouse::Cursor,
            _viewport: &Rectangle,
        ) {
        }

        fn overlay<'a>(
            &'a mut self,
            _tree: &'a mut widget::Tree,
            layout: Layout,
            _renderer: &(),
            viewport: &Rectangle,
            translation: Vector,
            window: Size,
        ) -> Vec<overlay::Element<'a, (), Theme, ()>> {
            assert_eq!(*viewport, Rectangle::with_size(window));
            assert_eq!(translation, Vector::ZERO);
            if !self.has_nested {
                return Vec::new();
            }
            vec![overlay::Element::new(Box::new(NestedOverlay {
                bounds: Rectangle::new(
                    layout.position() + Vector::new(0.0, 20.0),
                    Size::new(80.0, 40.0),
                ),
            }))]
        }
    }

    struct NestedOverlay {
        bounds: Rectangle,
    }

    impl overlay::Overlay<(), Theme, ()> for NestedOverlay {
        fn update(
            &mut self,
            event: &Event,
            cursor: mouse::Cursor,
            _renderer: &(),
            shell: &mut Shell<'_, ()>,
        ) {
            if cursor.is_over(self.bounds)
                && matches!(pointer_event(event), Some(mouse::Event::ButtonPressed(_)))
            {
                shell.capture_event();
            }
        }

        fn mouse_interaction(&self, cursor: mouse::Cursor, _renderer: &()) -> mouse::Interaction {
            if cursor.is_over(self.bounds) {
                mouse::Interaction::Idle
            } else {
                mouse::Interaction::None
            }
        }

        fn draw(
            &self,
            _renderer: &mut (),
            _theme: &Theme,
            _style: &renderer::Style,
            _cursor: mouse::Cursor,
        ) {
        }
    }

    fn dispatch_event(
        tooltip: &mut Tooltip<'_, (), Theme, ()>,
        tree: &mut widget::Tree,
        event: &Event,
        cursor: mouse::Cursor,
    ) -> event::Status {
        let window = Size::new(200.0, 100.0);
        let viewport = Rectangle::with_size(window);
        let layout = Layout::new(tree.size);
        let mut bus = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
        let overlays = tooltip.overlay(tree, layout, &(), &viewport, Vector::ZERO, window);
        let mut group = overlay::Group::new(overlays);
        group.update(event, cursor, &(), &mut shell);
        let base_cursor = if group.mouse_interaction(cursor, &()) == mouse::Interaction::None {
            cursor
        } else {
            cursor.levitate()
        };
        drop(group);
        if shell.event_status() == event::Status::Ignored {
            tooltip.update(tree, event, layout, base_cursor, &(), &mut shell, &viewport);
        }
        shell.event_status()
    }

    #[test]
    fn existing_positions_preserve_content_alignment_and_outer_padding() {
        let anchor = Rectangle::new(Point::new(100.0, 100.0), Size::new(80.0, 30.0));
        let content = Size::new(40.0, 20.0);
        let viewport = Rectangle::with_size(Size::new(400.0, 300.0));
        let cases = [
            (Position::Top, Point::new(116.0, 65.0)),
            (Position::Bottom, Point::new(116.0, 137.0)),
            (Position::Left, Point::new(45.0, 101.0)),
            (Position::Right, Point::new(187.0, 101.0)),
            (Position::FollowCursor, Point::new(126.0, 96.0)),
            (Position::TopRight, Point::new(136.0, 65.0)),
            (Position::TopLeft, Point::new(96.0, 65.0)),
            (Position::BottomRight, Point::new(136.0, 137.0)),
            (Position::BottomLeft, Point::new(96.0, 137.0)),
        ];

        for snap in [false, true] {
            for (position, origin) in cases {
                assert_eq!(
                    resolve_tooltip_bounds(
                        position,
                        anchor,
                        content,
                        4.0,
                        7.0,
                        Point::new(130.0, 120.0),
                        viewport,
                        snap,
                    ),
                    Rectangle::new(origin, Size::new(48.0, 28.0)),
                    "{position:?}, snap = {snap}",
                );
            }
        }
        assert_eq!(Position::default(), Position::FollowCursor);
        assert_eq!(Side::default(), Side::Bottom);
    }

    #[test]
    fn auto_keeps_a_fitting_preference_even_when_another_side_has_more_room() {
        let bounds = resolve_tooltip_bounds(
            Position::Auto {
                preference: Side::Bottom,
            },
            Rectangle::new(Point::new(100.0, 180.0), Size::new(80.0, 30.0)),
            Size::new(40.0, 20.0),
            4.0,
            7.0,
            Point::ORIGIN,
            Rectangle::with_size(Size::new(400.0, 300.0)),
            true,
        );
        assert_eq!(
            bounds,
            Rectangle::new(Point::new(116.0, 217.0), Size::new(48.0, 28.0)),
        );
    }

    #[test]
    fn auto_uses_padded_size_gap_and_viewport_origin_to_check_a_preference() {
        let anchor = Rectangle::new(Point::new(150.0, 302.0), Size::new(80.0, 30.0));
        let viewport = Rectangle::new(Point::new(50.0, 100.0), Size::new(400.0, 260.0));
        let position = Position::Auto {
            preference: Side::Bottom,
        };
        for (padding, gap, expected) in [
            (4.0, 0.0, Point::new(166.0, 332.0)),
            (4.0, 1.0, Point::new(166.0, 273.0)),
            (5.0, 0.0, Point::new(165.0, 272.0)),
        ] {
            assert_eq!(
                resolve_tooltip_bounds(
                    position,
                    anchor,
                    Size::new(40.0, 20.0),
                    padding,
                    gap,
                    Point::ORIGIN,
                    viewport,
                    false,
                )
                .position(),
                expected,
            );
        }
    }

    #[test]
    fn auto_falls_back_to_the_side_with_the_most_space_without_snapping() {
        let bounds = resolve_tooltip_bounds(
            Position::Auto {
                preference: Side::Bottom,
            },
            Rectangle::new(Point::new(0.0, 0.0), Size::new(50.0, 50.0)),
            Size::new(80.0, 960.0),
            4.0,
            7.0,
            Point::ORIGIN,
            Rectangle::with_size(Size::new(1000.0, 1000.0)),
            false,
        );
        assert_eq!(
            bounds,
            Rectangle::new(Point::new(57.0, -459.0), Size::new(88.0, 968.0)),
        );
    }

    #[test]
    fn auto_ties_choose_the_later_side_in_popover_order() {
        let viewport = Rectangle::with_size(Size::new(220.0, 220.0));
        let bounds = resolve_tooltip_bounds(
            Position::Auto {
                preference: Side::Top,
            },
            Rectangle::new(Point::new(100.0, 100.0), Size::new(20.0, 20.0)),
            Size::new(240.0, 240.0),
            4.0,
            7.0,
            Point::ORIGIN,
            viewport,
            false,
        );
        assert_eq!(
            bounds,
            Rectangle::new(Point::new(-14.0, 127.0), Size::new(248.0, 248.0)),
        );

        let bounds = resolve_tooltip_bounds(
            Position::Auto {
                preference: Side::Top,
            },
            Rectangle::new(Point::new(100.0, 10.0), Size::new(20.0, 200.0)),
            Size::new(240.0, 240.0),
            4.0,
            7.0,
            Point::ORIGIN,
            viewport,
            false,
        );
        assert_eq!(bounds.position(), Point::new(127.0, -14.0));
    }

    #[test]
    fn fixed_directions_flip_and_preserve_cross_axis_alignment() {
        let viewport = Rectangle::with_size(Size::new(200.0, 100.0));
        let cases = [
            (Position::Top, Point::new(80.0, 0.0), Point::new(96.0, 37.0)),
            (
                Position::TopLeft,
                Point::new(80.0, 0.0),
                Point::new(76.0, 37.0),
            ),
            (
                Position::TopRight,
                Point::new(80.0, 0.0),
                Point::new(116.0, 37.0),
            ),
            (
                Position::Bottom,
                Point::new(80.0, 70.0),
                Point::new(96.0, 35.0),
            ),
            (
                Position::BottomLeft,
                Point::new(80.0, 70.0),
                Point::new(76.0, 35.0),
            ),
            (
                Position::BottomRight,
                Point::new(80.0, 70.0),
                Point::new(116.0, 35.0),
            ),
            (
                Position::Left,
                Point::new(0.0, 30.0),
                Point::new(87.0, 31.0),
            ),
            (
                Position::Right,
                Point::new(120.0, 30.0),
                Point::new(65.0, 31.0),
            ),
        ];
        for (position, anchor, expected) in cases {
            let bounds = resolve_tooltip_bounds(
                position,
                Rectangle::new(anchor, Size::new(80.0, 30.0)),
                Size::new(40.0, 20.0),
                4.0,
                7.0,
                Point::ORIGIN,
                viewport,
                true,
            );
            assert_eq!(bounds.position(), expected, "{position:?}");
            assert_eq!(bounds.size(), Size::new(48.0, 28.0));
        }
    }

    #[test]
    fn bottom_right_dropdown_flips_above_its_anchor_instead_of_over_it() {
        let anchor = Rectangle::new(Point::new(50.0, 60.0), Size::new(30.0, 10.0));
        let viewport = Rectangle::with_size(Size::new(100.0, 80.0));
        for (snap, expected) in [
            (true, Point::new(36.0, 29.0)),
            (false, Point::new(36.0, 73.0)),
        ] {
            assert_eq!(
                resolve_tooltip_bounds(
                    Position::BottomRight,
                    anchor,
                    Size::new(40.0, 20.0),
                    4.0,
                    3.0,
                    Point::ORIGIN,
                    viewport,
                    snap,
                )
                .position(),
                expected,
            );
        }
    }

    #[test]
    fn a_non_fitting_side_does_not_flip_when_the_opposite_has_less_or_equal_room() {
        for y in [10.0, 35.0] {
            let bounds = resolve_tooltip_bounds(
                Position::Bottom,
                Rectangle::new(Point::new(50.0, y), Size::new(30.0, 30.0)),
                Size::new(40.0, 80.0),
                4.0,
                3.0,
                Point::ORIGIN,
                Rectangle::with_size(Size::new(200.0, 100.0)),
                true,
            );
            assert_eq!(bounds.position(), Point::new(41.0, 12.0));
        }
    }

    #[test]
    fn clamp_applies_after_flipping_and_follow_cursor_never_flips() {
        let viewport = Rectangle::new(Point::new(10.0, 20.0), Size::new(100.0, 80.0));
        let anchor = Rectangle::new(Point::new(100.0, 80.0), Size::new(30.0, 10.0));
        assert_eq!(
            resolve_tooltip_bounds(
                Position::BottomRight,
                anchor,
                Size::new(40.0, 20.0),
                4.0,
                3.0,
                Point::ORIGIN,
                viewport,
                true,
            ),
            Rectangle::new(Point::new(62.0, 49.0), Size::new(48.0, 28.0)),
        );
        assert_eq!(
            resolve_tooltip_bounds(
                Position::FollowCursor,
                anchor,
                Size::new(40.0, 20.0),
                4.0,
                3.0,
                Point::new(12.0, 22.0),
                viewport,
                true,
            ),
            Rectangle::new(Point::new(10.0, 20.0), Size::new(48.0, 28.0)),
        );
    }

    #[test]
    fn oversized_tooltip_retains_the_original_per_axis_clamp() {
        let viewport = Rectangle::new(Point::new(10.0, 20.0), Size::new(100.0, 80.0));
        for (cursor, expected) in [
            (Point::new(0.0, 0.0), Point::new(10.0, 20.0)),
            (Point::new(100.0, 200.0), Point::new(-98.0, -8.0)),
        ] {
            assert_eq!(
                resolve_tooltip_bounds(
                    Position::FollowCursor,
                    Rectangle::with_size(Size::new(30.0, 10.0)),
                    Size::new(200.0, 100.0),
                    4.0,
                    3.0,
                    cursor,
                    viewport,
                    true,
                )
                .position(),
                expected,
            );
        }
    }

    #[test]
    fn touch_tap_opens_left_pointer_and_an_outside_tap_closes_it() {
        let mut tooltip = test_widget()
            .open(Open::LeftPointer)
            .position(Position::Bottom);
        let mut tree = tree_for(&mut tooltip);
        let position = Point::new(10.0, 10.0);
        for event in [
            touch::Event::FingerPressed {
                id: touch::Finger(1),
                position,
            },
            touch::Event::FingerLifted {
                id: touch::Finger(1),
                position,
            },
        ] {
            dispatch_event(
                &mut tooltip,
                &mut tree,
                &Event::Touch(event),
                mouse::Cursor::Available(position),
            );
        }
        assert_eq!(
            tree.state.downcast_ref::<State>().mode,
            Mode::Opened {
                cursor_position: position,
                over_overlay: false
            },
        );

        let outside = Point::new(190.0, 90.0);
        for event in [
            touch::Event::FingerPressed {
                id: touch::Finger(1),
                position: outside,
            },
            touch::Event::FingerLifted {
                id: touch::Finger(1),
                position: outside,
            },
        ] {
            dispatch_event(
                &mut tooltip,
                &mut tree,
                &Event::Touch(event),
                mouse::Cursor::Available(outside),
            );
            assert_eq!(tree.state.downcast_ref::<State>().mode, Mode::default());
        }
    }

    #[test]
    fn touch_cannot_open_right_pointer_but_an_outside_press_closes_it() {
        let mut tooltip = test_widget()
            .open(Open::RightPointer)
            .position(Position::Bottom);
        let mut tree = tree_for(&mut tooltip);
        let position = Point::new(10.0, 10.0);
        for event in [
            touch::Event::FingerPressed {
                id: touch::Finger(1),
                position,
            },
            touch::Event::FingerLifted {
                id: touch::Finger(1),
                position,
            },
        ] {
            dispatch_event(
                &mut tooltip,
                &mut tree,
                &Event::Touch(event),
                mouse::Cursor::Available(position),
            );
            assert_eq!(tree.state.downcast_ref::<State>().mode, Mode::default());
        }
        for event in [
            mouse::Event::ButtonPressed(mouse::Button::Right),
            mouse::Event::ButtonReleased(mouse::Button::Right),
        ] {
            dispatch_event(
                &mut tooltip,
                &mut tree,
                &Event::Mouse(event),
                mouse::Cursor::Available(position),
            );
        }
        assert!(matches!(
            tree.state.downcast_ref::<State>().mode,
            Mode::Opened { .. }
        ));
        let outside = Point::new(190.0, 90.0);
        dispatch_event(
            &mut tooltip,
            &mut tree,
            &Event::Touch(touch::Event::FingerPressed {
                id: touch::Finger(1),
                position: outside,
            }),
            mouse::Cursor::Available(outside),
        );
        assert_eq!(tree.state.downcast_ref::<State>().mode, Mode::default());
    }

    #[test]
    fn touch_normalization_preserves_mouse_events_and_maps_lost_to_release() {
        let position = Point::new(10.0, 10.0);
        let events = [
            (
                touch::Event::FingerPressed {
                    id: touch::Finger(1),
                    position,
                },
                mouse::Event::ButtonPressed(mouse::Button::Left),
            ),
            (
                touch::Event::FingerLifted {
                    id: touch::Finger(1),
                    position,
                },
                mouse::Event::ButtonReleased(mouse::Button::Left),
            ),
            (
                touch::Event::FingerLost {
                    id: touch::Finger(1),
                    position,
                },
                mouse::Event::ButtonReleased(mouse::Button::Left),
            ),
            (
                touch::Event::FingerMoved {
                    id: touch::Finger(1),
                    position,
                },
                mouse::Event::CursorMoved { position },
            ),
        ];
        for (touch, mouse) in events {
            assert_eq!(pointer_event(&Event::Touch(touch)), Some(mouse));
            assert_eq!(pointer_event(&Event::Mouse(mouse)), Some(mouse));
        }
    }

    #[test]
    fn tooltip_overlay_captures_touch_and_delivers_the_original_event_to_content() {
        let mut tooltip = Tooltip::new(
            Space::new().width(80).height(30),
            NestedContent { has_nested: false },
        )
        .open(Open::LeftPointer)
        .position(Position::Bottom);
        let mut tree = tree_for(&mut tooltip);
        tree.state.downcast_mut::<State>().mode = Mode::Opened {
            cursor_position: Point::new(10.0, 10.0),
            over_overlay: false,
        };
        let position = Point::new(30.0, 40.0);
        for touch in [
            touch::Event::FingerPressed {
                id: touch::Finger(1),
                position,
            },
            touch::Event::FingerMoved {
                id: touch::Finger(1),
                position,
            },
            touch::Event::FingerLifted {
                id: touch::Finger(1),
                position,
            },
            touch::Event::FingerLost {
                id: touch::Finger(1),
                position,
            },
        ] {
            let event = Event::Touch(touch);
            assert_eq!(
                dispatch_event(
                    &mut tooltip,
                    &mut tree,
                    &event,
                    mouse::Cursor::Available(position)
                ),
                event::Status::Captured,
            );
            assert_eq!(
                tree.children[1].state.downcast_ref::<Vec<Event>>().last(),
                Some(&event)
            );
            assert!(matches!(
                tree.state.downcast_ref::<State>().mode,
                Mode::Opened {
                    over_overlay: true,
                    ..
                }
            ));
        }
    }

    #[test]
    fn unavailable_cursor_only_preserves_hover_when_content_has_nested_overlays() {
        for has_nested in [false, true] {
            for over_overlay in [false, true] {
                let mut tooltip = Tooltip::new(
                    Space::new().width(80).height(30),
                    NestedContent { has_nested },
                )
                .position(Position::Bottom);
                let mut tree = tree_for(&mut tooltip);
                let cursor_position = Point::new(10.0, 10.0);
                tree.state.downcast_mut::<State>().mode = Mode::Opened {
                    cursor_position,
                    over_overlay,
                };
                let mut bus = Bus::new();
                let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
                let window = Size::new(200.0, 100.0);
                let mut overlays = tooltip.overlay(
                    &mut tree,
                    Layout::new(Size::new(80.0, 30.0)),
                    &(),
                    &Rectangle::with_size(window),
                    Vector::ZERO,
                    window,
                );
                overlays[0].as_overlay_mut().update(
                    &Event::Mouse(mouse::Event::CursorMoved {
                        position: Point::new(70.0, 65.0),
                    }),
                    mouse::Cursor::Unavailable,
                    &(),
                    &mut shell,
                );
                drop(overlays);
                assert_eq!(
                    tree.state.downcast_ref::<State>().mode,
                    Mode::Opened {
                        cursor_position,
                        over_overlay: has_nested && over_overlay,
                    }
                );
            }
        }
    }

    #[test]
    fn moving_onto_a_nested_overlay_outside_the_tooltip_keeps_hovered_open() {
        let mut tooltip = Tooltip::new(
            Space::new().width(80).height(30),
            NestedContent { has_nested: true },
        )
        .position(Position::Bottom);
        let mut tree = tree_for(&mut tooltip);
        for position in [
            Point::new(10.0, 10.0),
            Point::new(30.0, 40.0),
            Point::new(70.0, 65.0),
        ] {
            dispatch_event(
                &mut tooltip,
                &mut tree,
                &Event::Mouse(mouse::Event::CursorMoved { position }),
                mouse::Cursor::Available(position),
            );
            assert!(matches!(
                tree.state.downcast_ref::<State>().mode,
                Mode::Opened { .. }
            ));
        }
        assert!(matches!(
            tree.state.downcast_ref::<State>().mode,
            Mode::Opened {
                over_overlay: true,
                ..
            }
        ));
        let outside = Point::new(190.0, 90.0);
        dispatch_event(
            &mut tooltip,
            &mut tree,
            &Event::Mouse(mouse::Event::CursorMoved { position: outside }),
            mouse::Cursor::Available(outside),
        );
        assert_eq!(tree.state.downcast_ref::<State>().mode, Mode::default());
    }

    #[test]
    fn a_captured_nested_option_press_does_not_close_left_pointer() {
        let mut tooltip = Tooltip::new(
            Space::new().width(80).height(30),
            NestedContent { has_nested: true },
        )
        .open(Open::LeftPointer)
        .position(Position::Bottom);
        let mut tree = tree_for(&mut tooltip);
        let mode = Mode::Opened {
            cursor_position: Point::new(10.0, 10.0),
            over_overlay: true,
        };
        tree.state.downcast_mut::<State>().mode = mode;
        assert_eq!(
            dispatch_event(
                &mut tooltip,
                &mut tree,
                &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                mouse::Cursor::Available(Point::new(70.0, 65.0)),
            ),
            event::Status::Captured,
        );
        assert_eq!(tree.state.downcast_ref::<State>().mode, mode);
        assert!(
            tree.children[1]
                .state
                .downcast_ref::<Vec<Event>>()
                .is_empty()
        );
    }

    #[test]
    fn pointer_overlays_draw_at_index_two_and_hovered_at_one() {
        for (open, expected) in [
            (Open::Hovered, 1.0),
            (Open::LeftPointer, 2.0),
            (Open::RightPointer, 2.0),
        ] {
            let mut tooltip = test_widget().open(open);
            let mut tree = tree_for(&mut tooltip);
            tree.state.downcast_mut::<State>().mode = Mode::Opened {
                cursor_position: Point::new(10.0, 10.0),
                over_overlay: false,
            };
            let window = Size::new(200.0, 100.0);
            let overlays = tooltip.overlay(
                &mut tree,
                Layout::new(Size::new(80.0, 30.0)),
                &(),
                &Rectangle::with_size(window),
                Vector::ZERO,
                window,
            );
            assert_eq!(overlays[0].as_overlay().index(), expected);
        }
    }

    #[test]
    fn follow_cursor_translates_the_recorded_cursor_into_screen_coordinates() {
        #[derive(Default)]
        struct Bounds(Option<Rectangle>);

        impl widget::Operation for Bounds {
            fn traverse(&mut self, _operate: &mut dyn FnMut(&mut dyn widget::Operation)) {}

            fn container(&mut self, _id: Option<&Id>, bounds: Rectangle, _viewport: &Rectangle) {
                self.0 = Some(bounds);
            }
        }

        let mut tooltip = test_widget().padding(4).snap_within_viewport(false);
        let mut tree = tree_for(&mut tooltip);
        move_cursor(&mut tooltip, &mut tree, Some(Point::new(10.0, 10.0)));
        let layout = Layout::new(tree.size).move_to(Point::new(50.0, 50.0));
        let window = Size::new(200.0, 100.0);
        let mut overlays = tooltip.overlay(
            &mut tree,
            layout,
            &(),
            &Rectangle::with_size(window),
            Vector::new(20.0, 30.0),
            window,
        );
        let mut bounds = Bounds::default();
        overlays[0].as_overlay_mut().operate(&(), &mut bounds);
        assert_eq!(
            bounds.0,
            Some(Rectangle::new(
                Point::new(26.0, 16.0),
                Size::new(48.0, 28.0)
            ))
        );
    }

    #[test]
    fn padded_content_geometry_is_stored_in_the_tree() {
        let mut tooltip = test_widget();
        let tree = tree_for(&mut tooltip);

        assert_eq!(tree.size, Size::new(80.0, 30.0));
        assert_eq!(tree.children[0].size, Size::new(70.0, 20.0));
        assert_eq!(tree.children[0].translation, Vector::new(5.0, 5.0));
        assert_eq!(tree.children[1].size, Size::ZERO);
    }

    #[test]
    fn tooltip_layout_is_reused_until_diff_or_reopen() {
        let mut tooltip = test_widget().padding(4);
        let mut tree = tree_for(&mut tooltip);
        let position = Point::new(10.0, 10.0);
        let window = Size::new(200.0, 100.0);
        let viewport = Rectangle::with_size(window);

        assert_eq!(
            move_cursor(&mut tooltip, &mut tree, Some(position)).0,
            Invalidation::Overlay
        );
        let layout = Layout::new(tree.size);
        assert_eq!(
            tooltip
                .overlay(&mut tree, layout, &(), &viewport, Vector::ZERO, window)
                .len(),
            1
        );
        assert_eq!(tree.children[1].size, Size::new(40.0, 20.0));
        assert!(!tree.state.downcast_ref::<State>().needs_relayout);

        tree.children[1].size = Size::new(1.0, 1.0);
        drop(tooltip.overlay(&mut tree, layout, &(), &viewport, Vector::ZERO, window));
        assert_eq!(tree.children[1].size, Size::new(1.0, 1.0));

        tree.diff(&mut tooltip);
        assert!(tree.state.downcast_ref::<State>().needs_relayout);
        drop(tooltip.overlay(&mut tree, layout, &(), &viewport, Vector::ZERO, window));
        assert_eq!(tree.children[1].size, Size::new(40.0, 20.0));

        close_all_operation::<()>().custom(
            None,
            layout.bounds(),
            tree.state.downcast_mut::<State>(),
        );
        assert!(!tree.state.downcast_ref::<State>().needs_relayout);
        move_cursor(&mut tooltip, &mut tree, Some(position));
        assert!(tree.state.downcast_ref::<State>().needs_relayout);
    }

    #[test]
    fn padded_tooltip_snaps_to_the_window_not_the_parent_viewport() {
        #[derive(Default)]
        struct Bounds(Vec<Rectangle>);

        impl widget::Operation for Bounds {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn widget::Operation)) {
                operate(self);
            }

            fn container(&mut self, _id: Option<&Id>, bounds: Rectangle, _viewport: &Rectangle) {
                self.0.push(bounds);
            }
        }

        let mut tooltip: Tooltip<'_, (), Theme, ()> = Tooltip::new(
            Space::new().width(80).height(30),
            Space::new().width(300).height(200),
        )
        .padding(4)
        .position(Position::BottomRight);
        let mut tree = tree_for(&mut tooltip);
        move_cursor(&mut tooltip, &mut tree, Some(Point::new(10.0, 10.0)));
        let window = Size::new(100.0, 80.0);
        let layout = Layout::new(tree.size).move_to(Point::new(50.0, 50.0));
        let mut overlays = tooltip.overlay(
            &mut tree,
            layout,
            &(),
            &Rectangle::with_size(Size::new(20.0, 10.0)),
            Vector::new(20.0, 30.0),
            window,
        );
        let mut bounds = Bounds::default();
        overlays[0].as_overlay_mut().operate(&(), &mut bounds);
        assert_eq!(bounds.0, vec![Rectangle::with_size(window)]);
        drop(overlays);
        assert_eq!(tree.children[1].size, Size::new(92.0, 72.0));
    }

    #[test]
    fn follow_cursor_updates_the_open_position_without_relayout() {
        let mut tooltip = test_widget();
        let mut tree = tree_for(&mut tooltip);
        move_cursor(&mut tooltip, &mut tree, Some(Point::new(10.0, 10.0)));
        tree.state.downcast_mut::<State>().needs_relayout = false;
        let position = Point::new(30.0, 20.0);

        assert_eq!(
            move_cursor(&mut tooltip, &mut tree, Some(position)),
            (Invalidation::None, window::RedrawRequest::NextFrame),
        );
        assert_eq!(
            tree.state.downcast_ref::<State>().mode,
            Mode::Opened {
                cursor_position: position,
                over_overlay: false,
            }
        );
        assert!(!tree.state.downcast_ref::<State>().needs_relayout);
    }

    #[test]
    fn leaving_before_the_delay_cancels_the_pending_hover() {
        let mut tooltip = test_widget().delay(Duration::from_secs(60));
        let mut tree = tree_for(&mut tooltip);
        move_cursor(&mut tooltip, &mut tree, Some(Point::new(10.0, 10.0)));
        assert!(matches!(
            tree.state.downcast_ref::<State>().mode,
            Mode::Idle {
                hovered_at: Some(_),
                ..
            }
        ));

        move_cursor(&mut tooltip, &mut tree, None);
        assert_eq!(tree.state.downcast_ref::<State>().mode, Mode::default());
    }

    #[test]
    fn close_operations_reset_only_the_matching_mode() {
        let target = Id::new("target-tooltip");
        let other = Id::new("other-tooltip");
        let mode = Mode::Opened {
            cursor_position: Point::new(10.0, 20.0),
            over_overlay: true,
        };
        let mut state = State {
            mode,
            needs_relayout: true,
        };
        let bounds = Rectangle::with_size(Size::new(40.0, 20.0));
        let mut close = close_operation::<()>(target.clone());
        close.custom(Some(&other), bounds, &mut state);
        assert_eq!(state.mode, mode);
        close.custom(Some(&target), bounds, &mut state);
        assert_eq!(state.mode, Mode::default());
        assert!(state.needs_relayout);

        state.mode = mode;
        state.needs_relayout = false;
        close_all_operation::<()>().custom(None, bounds, &mut state);
        assert_eq!(state.mode, Mode::default());
        assert!(!state.needs_relayout);
    }
}
