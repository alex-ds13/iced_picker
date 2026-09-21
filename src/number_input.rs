// This widget is a modification of the original `TextInput` widget from [`iced`]
//
// [`iced`]: https://github.com/iced-rs/iced
//
// Copyright 2019 Héctor Ramón, Iced contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of
// this software and associated documentation files (the "Software"), to deal in
// the Software without restriction, including without limitation the rights to
// use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software is furnished to do so,
// subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
// FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
// COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
// IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use std::fmt::Display;
use std::str::FromStr;

use iced::advanced::layout::{self, Limits};
use iced::advanced::mouse;
use iced::advanced::renderer;
use iced::advanced::text::paragraph::{self, Paragraph as _};
use iced::advanced::text::{self, Text};
use iced::advanced::widget;
use iced::advanced::widget::operation::{Focusable, Operation, TextInput as _};
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Layout, Shell, Widget, shell};
use iced::alignment;
use iced::keyboard;
use iced::keyboard::key;
use iced::widget::{Button, button};
use iced::window;
use iced::{
    Alignment, Background, Color, Element, Event, Font, Length, Padding, Pixels, Point, Rectangle,
    Size, Theme,
    border::{self, Border},
};
use num_traits::{Bounded, Num, NumAssignOps};

/// A field that can be filled with numbers.
///
/// # Example
/// ```no_run
/// use iced::Element;
/// use iced_picker::number_input::NumberInput;
///
/// struct State {
///    content: i32,
/// }
///
/// #[derive(Debug, Clone)]
/// enum Message {
///     ContentChanged(i32)
/// }
///
/// fn view(state: &State) -> Element<'_, Message> {
///     NumberInput::new("Type some number here...", state.content)
///         .on_input(Message::ContentChanged)
///         .into()
/// }
///
/// fn update(state: &mut State, message: Message) {
///     match message {
///         Message::ContentChanged(content) => {
///             state.content = content;
///         }
///     }
/// }
/// ```
#[allow(missing_debug_implementations)]
pub struct NumberInput<'a, T, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Theme: Catalog + button::Catalog + iced::widget::text::Catalog,
    Renderer: text::Renderer,
{
    id: Option<widget::Id>,
    placeholder: String,
    value: T,
    max: T,
    min: T,
    is_secure: bool,
    font: Option<Font>,
    width: Length,
    padding: Padding,
    size: Option<Pixels>,
    line_height: text::LineHeight,
    alignment: alignment::Horizontal,
    on_input: Option<Box<dyn Fn(T) -> Message + 'a>>,
    on_paste: Option<Box<dyn Fn(T) -> Message + 'a>>,
    on_submit: Option<Message>,
    icon: Option<Icon>,
    class: <Theme as Catalog>::Class<'a>,
    last_status: Option<Status>,
    increment_button: Button<'a, ButtonMessage, Theme, Renderer>,
    decrement_button: Button<'a, ButtonMessage, Theme, Renderer>,
    buttons_width: Pixels,
}

/// The default [`Padding`] of a [`NumberInput`].
pub const DEFAULT_PADDING: Padding = Padding::new(5.0);

/// The default [`Pixels`] width of a [`NumberInput`] increment/decrement buttons.
pub const DEFAULT_BUTTON_WIDTH: Pixels = Pixels(16.0);

impl<'a, T, Message, Theme, Renderer> NumberInput<'a, T, Message, Theme, Renderer>
where
    Message: Clone,
    Theme: Catalog + button::Catalog + widget::text::Catalog + 'a,
    Renderer: text::Renderer + 'static,
    T: Num + NumAssignOps + PartialOrd + Display + FromStr + Clone + Default + Bounded + 'a,
{
    /// Creates a new [`NumberInput`] with the given placeholder and
    /// its current value.
    pub fn new(placeholder: &str, value: T) -> Self {
        let text_size = Pixels(16.0);
        let line_height = text::LineHeight::default();
        let height = line_height.to_absolute(text_size) + DEFAULT_PADDING.y();
        let increment_button: Button<ButtonMessage, Theme, Renderer> =
            button(iced::widget::text("▲").size(8.0).center())
                .width(DEFAULT_BUTTON_WIDTH)
                .height(height)
                .class(<Theme as Catalog>::default_increment_button())
                .padding(0);
        let decrement_button: Button<ButtonMessage, Theme, Renderer> =
            button(iced::widget::text("▼").size(8.0).center())
                .width(DEFAULT_BUTTON_WIDTH)
                .height(height)
                .class(<Theme as Catalog>::default_decrement_button())
                .padding(0);

        NumberInput {
            id: None,
            placeholder: String::from(placeholder),
            value,
            max: T::max_value(),
            min: T::min_value(),
            is_secure: false,
            font: None,
            width: Length::Fixed(125.0),
            padding: DEFAULT_PADDING,
            size: None,
            line_height,
            alignment: alignment::Horizontal::Left,
            on_input: None,
            on_paste: None,
            on_submit: None,
            icon: None,
            class: <Theme as Catalog>::default(),
            last_status: None,
            increment_button,
            decrement_button,
            buttons_width: DEFAULT_BUTTON_WIDTH,
        }
    }

    /// Sets the [`Id`] of the [`NumberInput`].
    pub fn id(mut self, id: impl Into<widget::Id>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Converts the [`NumberInput`] into a secure password input.
    pub fn secure(mut self, is_secure: bool) -> Self {
        self.is_secure = is_secure;
        self
    }

    /// Sets the [`NumberInput`] max bound.
    pub fn max(mut self, max: T) -> Self {
        self.max = max;
        self
    }

    /// Sets the [`NumberInput`] min bound.
    pub fn min(mut self, min: T) -> Self {
        self.min = min;
        self
    }

    /// Sets the message that should be produced when some numbers are typed into
    /// the [`NumberInput`].
    ///
    /// If this method is not called, the [`NumberInput`] will be disabled.
    pub fn on_input(mut self, on_input: impl Fn(T) -> Message + 'a) -> Self {
        self.on_input = Some(Box::new(on_input));
        self.increment_button = self.increment_button.on_press(ButtonMessage::Increment);
        self.decrement_button = self.decrement_button.on_press(ButtonMessage::Decrement);
        self
    }

    /// Sets the message that should be produced when some numbers are typed into
    /// the [`NumberInput`], if `Some`.
    ///
    /// If `None`, the [`NumberInput`] will be disabled.
    pub fn on_input_maybe(mut self, on_input: Option<impl Fn(T) -> Message + 'a>) -> Self {
        self.on_input = on_input.map(|f| Box::new(f) as _);
        if self.on_input.is_some() {
            self.increment_button = self.increment_button.on_press(ButtonMessage::Increment);
            self.decrement_button = self.decrement_button.on_press(ButtonMessage::Decrement);
        }
        self
    }

    /// Sets the message that should be produced when the [`NumberInput`] is
    /// focused and the enter key is pressed.
    pub fn on_submit(mut self, message: Message) -> Self {
        self.on_submit = Some(message);
        self
    }

    /// Sets the message that should be produced when the [`NumberInput`] is
    /// focused and the enter key is pressed, if `Some`.
    pub fn on_submit_maybe(mut self, on_submit: Option<Message>) -> Self {
        self.on_submit = on_submit;
        self
    }

    /// Sets the message that should be produced when some numbers are pasted into
    /// the [`NumberInput`].
    pub fn on_paste(mut self, on_paste: impl Fn(T) -> Message + 'a) -> Self {
        self.on_paste = Some(Box::new(on_paste));
        self
    }

    /// Sets the message that should be produced when some numbers are pasted into
    /// the [`NumberInput`], if `Some`.
    pub fn on_paste_maybe(mut self, on_paste: Option<impl Fn(T) -> Message + 'a>) -> Self {
        self.on_paste = on_paste.map(|f| Box::new(f) as _);
        self
    }

    /// Sets the [`Font`] of the [`NumberInput`].
    pub fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    /// Sets the [`Icon`] of the [`NumberInput`].
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Sets the width of the [`NumberInput`].
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the [`Padding`] of the [`NumberInput`].
    pub fn padding<P: Into<Padding>>(mut self, padding: P) -> Self {
        self.padding = padding.into();
        self
    }

    /// Sets the [`Pixels`] width of the [`NumberInput`] increment/decrement buttons.
    pub fn buttons_width<P: Into<Pixels>>(mut self, width: P) -> Self {
        self.buttons_width = width.into();
        self.increment_button = self.increment_button.width(self.buttons_width);
        self.decrement_button = self.decrement_button.width(self.buttons_width);
        self
    }

    /// Sets the text size of the [`NumberInput`].
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    /// Sets the [`text::LineHeight`] of the [`NumberInput`].
    pub fn line_height(mut self, line_height: impl Into<text::LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    /// Sets the horizontal alignment of the [`NumberInput`].
    pub fn align_x(mut self, alignment: impl Into<alignment::Horizontal>) -> Self {
        self.alignment = alignment.into();
        self
    }

    /// Sets the style of the [`NumberInput`].
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        <Theme as Catalog>::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the style class of the [`NumberInput`].
    #[must_use]
    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    /// Sets the style for the increment button of the [`NumberInput`].
    #[must_use]
    pub fn style_increment(
        mut self,
        style: impl Fn(&Theme, button::Status) -> button::Style + 'a,
    ) -> Self
    where
        <Theme as button::Catalog>::Class<'a>: From<button::StyleFn<'a, Theme>>,
    {
        self.increment_button = self
            .increment_button
            .class(Box::new(style) as button::StyleFn<'a, Theme>);
        self
    }

    /// Sets the style class for the increment button of the [`NumberInput`].
    #[must_use]
    pub fn class_increment(
        mut self,
        class: impl Into<<Theme as button::Catalog>::Class<'a>>,
    ) -> Self {
        self.increment_button = self.increment_button.class(class);
        self
    }

    /// Sets the style for the increment button of the [`NumberInput`].
    #[must_use]
    pub fn style_decrement(
        mut self,
        style: impl Fn(&Theme, button::Status) -> button::Style + 'a,
    ) -> Self
    where
        <Theme as button::Catalog>::Class<'a>: From<button::StyleFn<'a, Theme>>,
    {
        self.decrement_button = self
            .decrement_button
            .class(Box::new(style) as button::StyleFn<'a, Theme>);
        self
    }

    /// Sets the style class for the decrement button of the [`NumberInput`].
    #[must_use]
    pub fn class_decrement(
        mut self,
        class: impl Into<<Theme as button::Catalog>::Class<'a>>,
    ) -> Self {
        self.decrement_button = self.decrement_button.class(class);
        self
    }

    /// Returns `value` brought inside the `min` and `max` bounds.
    fn clamp(&self, value: T) -> T {
        if value > self.max {
            self.max.clone()
        } else if value < self.min {
            self.min.clone()
        } else {
            value
        }
    }

    /// Builds the message for a single step up or down, or `None` when the widget is
    /// disabled or the value already sits on the bound it would move towards.
    fn step(&self, increment: bool) -> Option<Message> {
        let on_input = self.on_input.as_ref()?;
        let mut value = self.value.clone();

        if increment {
            if value >= self.max {
                return None;
            }

            value += T::one();
        } else {
            if value <= self.min {
                return None;
            }

            value -= T::one();
        }

        Some(on_input(self.clamp(value)))
    }

    /// The layouts of the increment and decrement buttons.
    fn button_layouts<'b>(&self, layout: Layout<'b>) -> (Layout<'b>, Layout<'b>) {
        let mut children = layout
            .children()
            .skip(if self.icon.is_some() { 2 } else { 1 });

        (children.next().unwrap(), children.next().unwrap())
    }

    /// Runs the cleanup that must happen whenever the widget loses focus, regardless
    /// of whether focus was lost via a mouse click, the Escape key, or a programmatic
    /// operation (Tab navigation etc.).
    ///
    /// Publishes the clamped value of whatever partial text was being edited (trailing
    /// dot, trailing zeros, empty field) and collapses the display to the canonical
    /// form of that value.
    fn on_unfocus(&self, state: &mut State<Renderer>, shell: &mut Shell<'_, Message>) {
        let Some(on_input) = &self.on_input else {
            return;
        };

        let text = state.input.value();
        let value = self.clamp(parse_number(&text).unwrap_or_else(|| T::zero()));
        let has_changed = value != self.value;
        let canonical = value.to_string();

        if text != canonical {
            state.input.overwrite(&canonical);
            shell.request_redraw();
        }

        if has_changed {
            state.transaction = Some(shell.publish_and_track(on_input(value)));
        }
    }
}

impl<'a, T, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for NumberInput<'a, T, Message, Theme, Renderer>
where
    Message: Clone,
    Theme: Catalog + button::Catalog + widget::text::Catalog + 'a,
    Renderer: text::Renderer + 'static,
    T: Num + NumAssignOps + PartialOrd + Display + FromStr + Clone + Default + Bounded + 'a,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Renderer>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Renderer>::new())
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [
            &mut self.increment_button as &mut dyn Widget<_, _, _>,
            &mut self.decrement_button as &mut dyn Widget<_, _, _>,
        ]);
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: Length::Fit,
        }
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State<Renderer>>();

        if state
            .transaction
            .as_ref()
            .is_none_or(shell::Tracking::is_processed)
        {
            let text = state.input.value();
            let canonical = self.value.to_string();

            // While editing, text that still denotes `self.value` is left as typed, so
            // the partial states ("", "-", "5.", "5.0") survive the re-render that our
            // own publish triggers.
            let is_current = if state.input.is_focused() {
                parse_number::<T>(&text).is_some_and(|parsed| {
                    parsed == self.value
                        // An unfinished number keeps its text when only the bounds make
                        // it differ, e.g. an empty field with a `min` above zero.
                        || (is_partial_number(&text) && self.clamp(parsed) == self.value)
                })
            } else {
                text == canonical
            };

            if !is_current {
                state.input.overwrite(&canonical);
            }
        }

        let text_size = self.size.unwrap_or_else(|| renderer.text_size());
        let padding = self.padding.fit(Size::ZERO, limits.bounds());
        let height = self.line_height.to_absolute(text_size);
        let buttons_width = f32::from(self.buttons_width);

        let text_bounds = limits
            .width(self.width)
            .shrink(padding)
            .shrink(Size::new(buttons_width, 0.0))
            .resolve(self.width, height, Size::ZERO);

        let icon_width = self.icon.as_ref().map(|icon| {
            let mut content = [0; 4];

            state.icon.update(Text {
                line_height: self.line_height,
                content: icon.code_point.encode_utf8(&mut content) as &_,
                font: icon.font,
                size: icon.size.unwrap_or_else(|| renderer.text_size()),
                bounds: Size::new(f32::INFINITY, text_bounds.height),
                align_x: text::Alignment::Center,
                align_y: alignment::Vertical::Center,
                shaping: text::Shaping::Advanced,
                wrapping: text::Wrapping::default(),
                ellipsis: text::Ellipsis::None,
                hint_factor: renderer.hint_factor(),
            });

            state.icon.min_width()
        });

        let reserved = self
            .icon
            .as_ref()
            .zip(icon_width)
            .map_or(0.0, |(icon, width)| width + icon.spacing);

        let input_size = Size::new(
            text_bounds.width - reserved + padding.x(),
            text_bounds.height + padding.y(),
        );

        let input_x = match self.icon.as_ref().map(|icon| &icon.side) {
            Some(Side::Left) => reserved,
            _ => 0.0,
        };

        let input_node = state
            .input
            .layout(
                renderer,
                &Limits::new(Size::ZERO, input_size),
                text::input::Layout {
                    width: Length::Fixed(input_size.width),
                    height: Length::Fixed(input_size.height),
                    padding,
                    placeholder: self.placeholder.as_str(),
                    font: self.font,
                    size: self.size,
                    line_height: Some(self.line_height),
                    alignment: self.alignment.into(),
                    multiline: None,
                    is_secure: self.is_secure,
                },
            )
            .move_to(Point::new(input_x, 0.0));

        let button_padding = 1.0;
        let button_limits = Limits::new(
            Size::ZERO,
            Size::new(
                buttons_width - 2.0 * button_padding,
                (f32::from(height) + padding.y()) / 2.0 - button_padding,
            ),
        );

        let buttons_x = text_bounds.width + padding.x() + button_padding;

        let increment_node = self
            .increment_button
            .layout(&mut tree.children[0], renderer, &button_limits)
            .move_to(Point::new(buttons_x, button_padding));

        let decrement_node = self
            .decrement_button
            .layout(&mut tree.children[1], renderer, &button_limits)
            .move_to(Point::new(
                buttons_x,
                button_limits.max.height + button_padding,
            ));

        let children = match (&self.icon, icon_width) {
            (Some(icon), Some(icon_width)) => {
                let icon_x = match icon.side {
                    Side::Left => padding.left,
                    Side::Right => padding.left + text_bounds.width - icon_width,
                };

                let icon_node = layout::Node::new(Size::new(icon_width, text_bounds.height))
                    .move_to(Point::new(icon_x, padding.top));

                vec![input_node, icon_node, increment_node, decrement_node]
            }
            _ => vec![input_node, increment_node, decrement_node],
        };

        layout::Node::with_children(
            text_bounds
                .expand(padding)
                .expand(Size::new(buttons_width, 0.0)),
            children,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State<Renderer>>();
        let was_focused = state.input.is_focused();

        operation.text_input(self.id.as_ref(), layout.bounds(), &mut state.input);
        operation.focusable(self.id.as_ref(), layout.bounds(), &mut state.input);

        // Focus granted by an operation (Tab navigation) selects the value, just like
        // the first click on the field does.
        if !was_focused && state.input.is_focused() {
            state.input.select_all();
        }

        let (increment_layout, decrement_layout) = self.button_layouts(layout);

        self.increment_button.operate(
            &mut tree.children[0],
            increment_layout,
            viewport,
            renderer,
            operation,
        );
        self.decrement_button.operate(
            &mut tree.children[1],
            decrement_layout,
            viewport,
            renderer,
            operation,
        );
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
        let (increment_layout, decrement_layout) = self.button_layouts(layout);
        let mut buttons_messages = shell::Bus::<ButtonMessage>::new();
        {
            let mut buttons_shell = shell.local(&mut buttons_messages);

            if shell.is_event_captured() {
                buttons_shell.capture_event();
            }
            buttons_shell.request_redraw_at(shell.redraw_request());
            buttons_shell.request_input_method(shell.input_method());

            self.increment_button.update(
                &mut tree.children[0],
                event,
                increment_layout,
                cursor,
                renderer,
                &mut buttons_shell,
                viewport,
            );
            self.decrement_button.update(
                &mut tree.children[1],
                event,
                decrement_layout,
                cursor,
                renderer,
                &mut buttons_shell,
                viewport,
            );

            shell.invalidate(buttons_shell.invalidation());
            shell.request_redraw_at(buttons_shell.redraw_request());
            shell.request_input_method(buttons_shell.input_method());
            shell.clipboard_mut().merge(buttons_shell.clipboard_mut());
            if buttons_shell.is_event_captured() {
                shell.capture_event();
            }
        }

        for (message, receipt) in buttons_messages.drain() {
            if let Some(message) = self.step(matches!(message, ButtonMessage::Increment)) {
                shell.forward(message, receipt);
            }

            shell.request_redraw();
            shell.capture_event();
        }

        if shell.is_event_captured() {
            // if the buttons handled the event we can simply return
            return;
        }

        let input_bounds = layout.children().next().unwrap().bounds();
        let state = tree.state.downcast_mut::<State<Renderer>>();

        // Operations mutate focus directly instead of going through update(), so the
        // focus recorded at the end of the previous call is what reveals them.
        if state.was_focused && !state.input.is_focused() {
            self.on_unfocus(state, shell);
            state.was_focused = false;
        }

        let was_focused = state.was_focused;

        if let Some(on_input) = &self.on_input {
            let stepped = self.wheel_step(event, layout, cursor, shell);

            if !stepped {
                let previous_text = state.input.value();
                let had_focus = state.input.is_focused();

                let edit = state
                    .input
                    .update(event, input_bounds, cursor, shell, |key_press| {
                        if key_press.is_focused {
                            match key_press.modified_key.as_ref() {
                                keyboard::Key::Named(key::Named::Enter) => {
                                    if let Some(on_submit) = &self.on_submit {
                                        return Some(text::editor::Binding::Custom(
                                            on_submit.clone(),
                                        ));
                                    }
                                }
                                keyboard::Key::Named(key::Named::ArrowUp) => {
                                    return self.step(true).map(text::editor::Binding::Custom);
                                }
                                keyboard::Key::Named(key::Named::ArrowDown) => {
                                    return self.step(false).map(text::editor::Binding::Custom);
                                }
                                _ => {}
                            }
                        }

                        text::editor::Binding::from_key_press(key_press)
                    });

                // The click that brings focus to the field selects the whole value, so
                // typing replaces it.
                if !had_focus && state.input.is_focused() {
                    state.input.select_all();
                    shell.request_redraw();
                }

                if let Some(edit) = edit {
                    let text = state.input.value();

                    if is_number_text::<T>(&text) {
                        let value = self.clamp(parse_number(&text).unwrap_or_else(|| T::zero()));

                        let on_edit = match &self.on_paste {
                            Some(on_paste) if edit.has_pasted => on_paste,
                            _ => on_input,
                        };

                        state.transaction = Some(shell.publish_and_track(on_edit(value)));
                    } else {
                        // `Input` applies the edit before handing it over, so a result
                        // that is not a number is undone by restoring the old text.
                        state.input.overwrite(&previous_text);
                        shell.request_redraw();
                    }
                }
            }
        }

        let is_focused = state.input.is_focused();

        if was_focused && !is_focused {
            self.on_unfocus(state, shell);
        }

        state.was_focused = is_focused;

        let status = if self.on_input.is_none() {
            Status::Disabled
        } else if is_focused {
            Status::Focused {
                is_hovered: cursor.is_over(layout.bounds()),
            }
        } else if cursor.is_over(layout.bounds()) {
            Status::Hovered
        } else {
            Status::Active
        };

        if let Event::Window(window::Event::RedrawRequested(_now)) = event {
            self.last_status = Some(status);

            shell.request_input_method(
                &state
                    .input
                    .input_method(input_bounds.shrink(self.padding).position()),
            );
        } else if self
            .last_status
            .is_some_and(|last_status| status != last_status)
        {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        renderer_style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer>>();

        let style = <Theme as Catalog>::style(
            theme,
            &self.class,
            self.last_status.unwrap_or(Status::Disabled),
        );

        let mut children_layout = layout.children();
        let input_layout = children_layout.next().unwrap();

        renderer.fill_quad(
            renderer::Quad {
                bounds: layout.bounds(),
                border: style.border,
                ..renderer::Quad::default()
            },
            style.background,
        );

        if self.icon.is_some() {
            let icon_layout = children_layout.next().unwrap();
            let icon = state.icon.raw();

            renderer.fill_paragraph(
                icon,
                icon_layout.bounds().anchor(
                    icon.min_bounds(),
                    Alignment::Center,
                    Alignment::Center,
                ),
                style.icon,
                *viewport,
            );
        }

        let increment_layout = children_layout.next().unwrap();
        self.increment_button.draw(
            &tree.children[0],
            renderer,
            theme,
            renderer_style,
            increment_layout,
            cursor,
            viewport,
        );

        let decrement_layout = children_layout.next().unwrap();
        self.decrement_button.draw(
            &tree.children[1],
            renderer,
            theme,
            renderer_style,
            decrement_layout,
            cursor,
            viewport,
        );

        state.input.draw(
            renderer,
            input_layout.bounds(),
            *viewport,
            text::input::Style {
                value: style.value,
                selection: style.selection,
                placeholder: style.placeholder,
            },
        );
    }

    fn mouse_interaction(
        &self,
        _state: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            if self.on_input.is_none() {
                mouse::Interaction::Idle
            } else {
                let (increment_layout, decrement_layout) = self.button_layouts(layout);

                if cursor.is_over(increment_layout.bounds())
                    || cursor.is_over(decrement_layout.bounds())
                {
                    mouse::Interaction::Pointer
                } else {
                    mouse::Interaction::Text
                }
            }
        } else {
            mouse::Interaction::default()
        }
    }
}

impl<'a, T, Message, Theme, Renderer> NumberInput<'a, T, Message, Theme, Renderer>
where
    Message: Clone,
    Theme: Catalog + button::Catalog + widget::text::Catalog + 'a,
    Renderer: text::Renderer + 'static,
    T: Num + NumAssignOps + PartialOrd + Display + FromStr + Clone + Default + Bounded + 'a,
{
    /// Steps the value when the wheel is scrolled over the widget, returning whether
    /// the event was consumed.
    fn wheel_step(
        &self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) -> bool {
        let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event else {
            return false;
        };

        if !cursor.is_over(layout.bounds()) {
            return false;
        }

        let y = match delta {
            mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => *y,
        };

        if y == 0.0 {
            return false;
        }

        if let Some(message) = self.step(y > 0.0) {
            shell.publish(message);
            shell.request_redraw();
        }

        shell.capture_event();

        true
    }
}

impl<'a, T, Message, Theme, Renderer> From<NumberInput<'a, T, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: Catalog + button::Catalog + widget::text::Catalog + 'a,
    Renderer: text::Renderer + 'static,
    T: Num + NumAssignOps + PartialOrd + Display + FromStr + Clone + Default + Bounded + 'a,
{
    fn from(
        number_input: NumberInput<'a, T, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(number_input)
    }
}

/// The content of the [`Icon`].
#[derive(Debug, Clone)]
pub struct Icon {
    /// The font that will be used to display the `code_point`.
    pub font: Font,
    /// The unicode code point that will be used as the icon.
    pub code_point: char,
    /// The font size of the content.
    pub size: Option<Pixels>,
    /// The spacing between the [`Icon`] and the numbers in a [`NumberInput`].
    pub spacing: f32,
    /// The side of a [`NumberInput`] where to display the [`Icon`].
    pub side: Side,
}

/// The side of a [`NumberInput`].
#[derive(Debug, Clone)]
pub enum Side {
    /// The left side of a [`NumberInput`].
    Left,
    /// The right side of a [`NumberInput`].
    Right,
}

/// The state of a [`NumberInput`].
struct State<R: text::Renderer> {
    input: text::Input<R>,
    icon: paragraph::Plain<R::Paragraph>,
    transaction: Option<shell::Tracking>,
    /// Mirrors `input.is_focused()` as of the previous `update()` call, so focus
    /// revoked by an operation between frames can be told apart from focus that is
    /// still held.
    was_focused: bool,
}

impl<R: text::Renderer> State<R> {
    /// Creates a new [`State`], representing an unfocused [`NumberInput`].
    fn new() -> Self {
        Self {
            input: text::Input::new(),
            icon: paragraph::Plain::default(),
            transaction: None,
            was_focused: false,
        }
    }
}

/// Parses the text an editing session can hold, including the partial states `""`,
/// `"-"`, `"5."` and `".5"`, into a `T`.
fn parse_number<T>(text: &str) -> Option<T>
where
    T: Num + FromStr,
{
    if text.is_empty() || text == "-" {
        return Some(T::zero());
    }

    let mut probe = String::with_capacity(text.len() + 2);
    probe.push_str(text);

    if probe.starts_with('.') {
        probe.insert(0, '0');
    } else if probe.starts_with("-.") {
        probe.insert(1, '0');
    }

    if probe.ends_with('.') {
        probe.push('0');
    }

    probe.parse().ok()
}

/// Returns whether `text` is a number still being typed rather than a finished one.
fn is_partial_number(text: &str) -> bool {
    text.is_empty() || text == "-" || text.ends_with('.')
}

/// Returns whether `text` is something the user can keep typing towards a `T`.
///
/// A decimal point is rejected on integer types because the probe parse in
/// [`parse_number`] fails for them.
fn is_number_text<T>(text: &str) -> bool
where
    T: Num + FromStr + Bounded + PartialOrd,
{
    let body = match text.strip_prefix('-') {
        Some(body) => {
            if T::min_value() >= T::zero() {
                return false;
            }

            body
        }
        None => text,
    };

    if !body.chars().all(|c| c.is_ascii_digit() || c == '.')
        || body.chars().filter(|c| *c == '.').count() > 1
    {
        return false;
    }

    body.is_empty() || parse_number::<T>(text).is_some()
}

/// The possible status of a [`NumberInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The [`NumberInput`] can be interacted with.
    Active,
    /// The [`NumberInput`] is being hovered.
    Hovered,
    /// The [`NumberInput`] is focused.
    Focused {
        /// Whether the [`NumberInput`] is hovered, while focused.
        is_hovered: bool,
    },
    /// The [`NumberInput`] cannot be interacted with.
    Disabled,
}

/// The appearance of a number input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The [`Background`] of the number input.
    pub background: Background,
    /// The [`Border`] of the number input.
    pub border: Border,
    /// The [`Color`] of the icon of the number input.
    pub icon: Color,
    /// The [`Color`] of the placeholder of the number input.
    pub placeholder: Color,
    /// The [`Color`] of the value of the number input.
    pub value: Color,
    /// The [`Color`] of the selection of the number input.
    pub selection: Color,
}

/// The theme catalog of a [`NumberInput`].
pub trait Catalog: Sized + button::Catalog {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> <Self as Catalog>::Class<'a>;

    /// The default class for the increment buttons of the [`NumberInput`].
    fn default_increment_button<'a>() -> <Self as button::Catalog>::Class<'a> {
        <Self as button::Catalog>::default()
    }

    /// The default class for the decrement buttons of the [`NumberInput`].
    fn default_decrement_button<'a>() -> <Self as button::Catalog>::Class<'a> {
        <Self as button::Catalog>::default()
    }

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &<Self as Catalog>::Class<'_>, status: Status) -> Style;

    /// The button [`button::Style`] of a button class with the given button status.
    fn style_button(
        &self,
        class: &<Self as button::Catalog>::Class<'_>,
        status: button::Status,
    ) -> button::Style;
}

/// A styling function for a [`NumberInput`].
///
/// This is just a boxed closure: `Fn(&Theme, Status) -> Style`.
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> <Self as Catalog>::Class<'a> {
        Box::new(default)
    }

    fn default_increment_button<'a>() -> <Self as button::Catalog>::Class<'a> {
        Box::new(top_button) as button::StyleFn<'_, Theme>
    }

    fn default_decrement_button<'a>() -> <Self as button::Catalog>::Class<'a> {
        Box::new(bottom_button) as button::StyleFn<'_, Theme>
    }

    fn style(&self, class: &<Self as Catalog>::Class<'_>, status: Status) -> Style {
        class(self, status)
    }

    fn style_button(
        &self,
        class: &<Self as button::Catalog>::Class<'_>,
        status: button::Status,
    ) -> button::Style {
        class(self, status)
    }
}

/// The default style of a [`NumberInput`].
pub fn default(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();

    let active = Style {
        background: Background::Color(palette.background.base.color),
        border: Border {
            radius: 2.0.into(),
            width: 1.0,
            color: palette.background.strongest.color,
        },
        icon: palette.background.weak.text,
        placeholder: palette.background.strongest.color,
        value: palette.background.base.text,
        selection: palette.primary.weak.color,
    };

    match status {
        Status::Active => active,
        Status::Hovered => Style {
            border: Border {
                color: palette.background.base.text,
                ..active.border
            },
            ..active
        },
        Status::Focused { .. } => Style {
            border: Border {
                color: palette.primary.strong.color,
                ..active.border
            },
            ..active
        },
        Status::Disabled => Style {
            background: Background::Color(palette.background.weak.color),
            value: active.placeholder,
            ..active
        },
    }
}

/// The default style for the top button of a [`NumberInput`].
pub fn top_button(theme: &Theme, status: button::Status) -> button::Style {
    let base = button::secondary(theme, status);
    button::Style {
        border: border::rounded(border::top(2)),
        ..base
    }
}

/// The default style for the bottom button of a [`NumberInput`].
pub fn bottom_button(theme: &Theme, status: button::Status) -> button::Style {
    let base = button::secondary(theme, status);
    button::Style {
        border: border::rounded(border::bottom(2)),
        ..base
    }
}

#[derive(Debug, Clone)]
enum ButtonMessage {
    Increment,
    Decrement,
}
