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
use iced_core as core;

use core::layout::{self, Layout};
use core::mouse;
use core::overlay;
use core::renderer;
use core::text;
use core::time::{Duration, Instant};
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
        if let Event::Mouse(_) | Event::Window(window::Event::RedrawRequested(_)) = event {
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
                            if let Event::Mouse(mouse::Event::ButtonPressed(_)) = event {
                                Mode::default()
                            } else {
                                state.mode
                            }
                        }
                        Open::RightPointer => {
                            if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) =
                                event
                            {
                                cursor
                                    .position_over(layout.bounds())
                                    .map(|cursor_position| Mode::Opened {
                                        cursor_position,
                                        over_overlay: false,
                                    })
                                    .unwrap_or_default()
                            } else if let Event::Mouse(mouse::Event::ButtonPressed(
                                mouse::Button::Left,
                            )) = event
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
            } else if let Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) = event
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
            } else if let Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)) = event
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
            } else if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
                && self.open == Open::LeftPointer
            {
                cursor
                    .position_over(layout.bounds())
                    .map(|_| Mode::Idle {
                        pressed: true,
                        hovered_at: None,
                    })
                    .unwrap_or_default()
            } else if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) = event
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
            let position = layout.position() + translation;
            let content_bounds = layout.bounds();
            let content_size = tooltip_tree.size;
            let tooltip_size = content_size.expand(Padding::new(self.padding));
            let viewport = Rectangle::with_size(window);
            let x_center = position.x + (content_bounds.width - content_size.width) / 2.0;
            let y_center = position.y + (content_bounds.height - content_size.height) / 2.0;

            let offset = match self.position {
                Position::Top => Point::new(
                    x_center,
                    position.y - content_size.height - self.gap - self.padding,
                ),
                Position::Bottom => Point::new(
                    x_center,
                    position.y + content_bounds.height + self.gap + self.padding,
                ),
                Position::Left => Point::new(
                    position.x - content_size.width - self.gap - self.padding,
                    y_center,
                ),
                Position::Right => Point::new(
                    position.x + content_bounds.width + self.gap + self.padding,
                    y_center,
                ),
                Position::FollowCursor => {
                    let translation = position - content_bounds.position();

                    Point::new(cursor_position.x, cursor_position.y - content_size.height)
                        + translation
                }
                Position::TopRight => Point::new(
                    position.x + content_bounds.width - content_size.width,
                    position.y - content_size.height - self.gap - self.padding,
                ),
                Position::TopLeft => Point::new(
                    position.x,
                    position.y - content_size.height - self.gap - self.padding,
                ),
                Position::BottomRight => Point::new(
                    position.x + content_bounds.width - content_size.width,
                    position.y + content_bounds.height + self.gap + self.padding,
                ),
                Position::BottomLeft => Point::new(
                    position.x,
                    position.y + content_bounds.height + self.gap + self.padding,
                ),
            };

            let mut tooltip_bounds = Rectangle {
                x: offset.x - self.padding,
                y: offset.y - self.padding,
                width: tooltip_size.width,
                height: tooltip_size.height,
            };

            if self.snap_within_viewport {
                if tooltip_bounds.x < viewport.x {
                    tooltip_bounds.x = viewport.x;
                } else if viewport.x + viewport.width < tooltip_bounds.x + tooltip_bounds.width {
                    tooltip_bounds.x = viewport.x + viewport.width - tooltip_bounds.width;
                }

                if tooltip_bounds.y < viewport.y {
                    tooltip_bounds.y = viewport.y;
                } else if viewport.y + viewport.height < tooltip_bounds.y + tooltip_bounds.height {
                    tooltip_bounds.y = viewport.y + viewport.height - tooltip_bounds.height;
                }
            }

            Some(overlay::Element::new(Box::new(Overlay {
                layout: Layout::new(tooltip_tree.size)
                    .move_to(tooltip_bounds.position() + Vector::new(self.padding, self.padding)),
                padding: self.padding,
                tooltip: &mut self.tooltip,
                state: tooltip_tree,
                tooltip_state: mode,
                positioning: self.position,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
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
        if let Event::Mouse(_) | Event::Window(window::Event::RedrawRequested(_)) = event {
            let bounds = self.bounds();
            let state = &mut *self.tooltip_state;
            let previous_state = *state;
            let was_idle = matches!(*state, Mode::Idle { .. });

            *state = cursor
                .position_over(bounds)
                .map(|_| Mode::Opened {
                    cursor_position: if let Mode::Opened {
                        cursor_position, ..
                    } = *state
                    {
                        cursor_position
                    } else {
                        Point::default()
                    },
                    over_overlay: true,
                })
                .unwrap_or(Mode::Opened {
                    cursor_position: if let Mode::Opened {
                        cursor_position, ..
                    } = *state
                    {
                        cursor_position
                    } else {
                        Point::default()
                    },
                    over_overlay: false,
                });

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

        if cursor.is_over(self.bounds()) && matches!(event, Event::Mouse(_)) {
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
