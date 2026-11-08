// This is a reworked version of the iced_aw's ColorPicker ported to iced version
// 0.14+ and with some changes/fixes that I wanted myself.
//
// Copyright (c) 2020 Kaiden42
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

use std::{cell::Cell, collections::HashMap, rc::Rc, str::FromStr};

use iced::{
    Alignment, Background, Border, Color, Element, Event, Font, Length, Padding, Pixels, Point,
    Rectangle, Renderer, Shadow, Size, Theme, Vector, Widget as _,
    advanced::{
        Layout, Overlay, Renderer as _, Shell, Text, Widget,
        graphics::geometry::Renderer as _,
        layout::Limits,
        overlay, renderer, shell,
        text::Renderer as _,
        widget::{
            Id, Meta, Operation, operation,
            tree::{self, Tag, Tree},
        },
    },
    alignment::{Horizontal, Vertical},
    event, keyboard,
    mouse::{self, Cursor},
    touch,
    widget::{
        Button, Column, Row, TextInput, button,
        canvas::{self, LineCap, Path, Stroke},
        container, row, text,
        text::Wrapping,
        text_input,
    },
};

/// An input element for picking colors.
///
/// # Example
/// ```rust
/// # use iced_picker::color_picker::ColorPicker;
/// # use iced::{Color, widget::{button, Button, Text}};
/// #
/// #[derive(Clone, Debug)]
/// enum Message {
///     Open,
///     Cancel,
///     Submit(Color),
/// }
///
/// let color_picker: ColorPicker<Message> = ColorPicker::new(
///     true,
///     Color::default(),
///     Button::new(Text::new("Pick color"))
///         .on_press(Message::Open),
///     Message::Cancel,
///     Message::Submit,
/// );
/// ```
#[allow(missing_debug_implementations)]
pub struct ColorPicker<'a, Message, Theme = iced::Theme>
where
    Message: Clone,
    Theme: Catalog + iced::widget::button::Catalog + text_input::Catalog + Clone,
{
    /// Show the picker.
    show_picker: bool,
    /// The color to show.
    color: Color,
    /// The underlying element.
    underlay: Element<'a, Message, Theme, Renderer>,
    /// The message that is sent if the cancel button of the [`ColorPickerOverlay`] is pressed.
    on_cancel: Message,
    /// The function that produces a message when the submit button of the [`ColorPickerOverlay`] is pressed.
    on_submit: Box<dyn Fn(Color) -> Message + 'a>,
    /// The style of the [`ColorPickerOverlay`].
    class: <Theme as Catalog>::Class<'a>,
    appearance: Appearance<'a, Theme>,
    /// The cancel button of the [`ColorPickerOverlay`].
    cancel_button: PickerButton<'a, Message, Theme>,
    /// The submit button of the [`ColorPickerOverlay`].
    submit_button: PickerButton<'a, Message, Theme>,
    /// The hex input of the [`ColorPickerOverlay`].
    hex_input: TextInput<'a, InternalMessage, InternalTheme<Theme>>,
}

type ButtonStyleFn<'a, Theme> = Rc<dyn Fn(&Theme, ButtonStatus) -> button::Style + 'a>;
type HexStyleFn<'a, Theme> = Rc<dyn Fn(&Theme, HexInputStatus) -> text_input::Style + 'a>;
type PickerButton<'a, Message, Theme> =
    Button<'a, ButtonAction<Message>, ButtonContent<'a, Message, Theme>, ButtonTheme<Theme>>;

#[derive(Clone, Debug)]
enum ButtonAction<Message> {
    Content(Message),
    Press,
}

struct Appearance<'a, Theme> {
    font: Option<Font>,
    text_size: Option<Pixels>,
    channel_labels: [String; 4],
    cancel_label: String,
    submit_label: String,
    cancel_style: Option<ButtonStyleFn<'a, Theme>>,
    submit_style: Option<ButtonStyleFn<'a, Theme>>,
    hex_style: Option<HexStyleFn<'a, Theme>>,
}

impl<Theme> Default for Appearance<'_, Theme> {
    fn default() -> Self {
        Self {
            font: None,
            text_size: None,
            channel_labels: ["R:", "G:", "B:", "A:"].map(String::from),
            cancel_label: "Cancel".into(),
            submit_label: "Submit".into(),
            cancel_style: None,
            submit_style: None,
            hex_style: None,
        }
    }
}

/// The interaction state of a picker button. Pressed takes priority over focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStatus {
    /// The button is idle.
    Active,
    /// The pointer is over the button.
    Hovered,
    /// The button is being pressed.
    Pressed,
    /// The button has keyboard focus.
    Focused {
        /// Whether the pointer is also over the button.
        is_hovered: bool,
    },
}

/// The interaction state and working color of the hex input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HexInputStatus {
    /// The iced text input's interaction state.
    pub status: text_input::Status,
    /// The current working color (the last valid edit).
    pub color: Color,
    /// Whether the text currently parses as a color.
    pub is_valid: bool,
}

struct ButtonTheme<Theme> {
    theme: Theme,
    focused: bool,
}

impl<Theme: button::Catalog> button::Catalog for ButtonTheme<Theme> {
    type Class<'a> = Option<ButtonStyleFn<'a, Theme>>;

    fn default<'a>() -> Self::Class<'a> {
        None
    }

    fn style(&self, class: &Self::Class<'_>, status: button::Status) -> button::Style {
        let picker_status = if status == button::Status::Pressed {
            ButtonStatus::Pressed
        } else if self.focused {
            ButtonStatus::Focused {
                is_hovered: status == button::Status::Hovered,
            }
        } else if status == button::Status::Hovered {
            ButtonStatus::Hovered
        } else {
            ButtonStatus::Active
        };
        if let Some(style) = class {
            style(&self.theme, picker_status)
        } else {
            <Theme as button::Catalog>::style(
                &self.theme,
                &<Theme as button::Catalog>::default(),
                match picker_status {
                    ButtonStatus::Focused { is_hovered: true } => button::Status::Hovered,
                    ButtonStatus::Focused { is_hovered: false } => button::Status::Active,
                    _ => status,
                },
            )
        }
    }
}

struct ButtonContent<'a, Message, Theme>(Element<'a, Message, Theme, Renderer>);

impl<Message, Theme> Meta for ButtonContent<'_, Message, Theme> {}

impl<'a, Message: 'a, Theme: 'a> Widget<ButtonAction<Message>, ButtonTheme<Theme>, Renderer>
    for ButtonContent<'a, Message, Theme>
{
    fn tag(&self) -> Tag {
        self.0.tag()
    }
    fn state(&self) -> tree::State {
        self.0.state()
    }
    fn diff(&mut self, tree: &mut Tree) {
        self.0.diff(tree);
    }
    fn size(&self) -> Size<Length> {
        self.0.size()
    }
    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) {
        self.0.layout(tree, renderer, limits);
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.0.operate(tree, layout, viewport, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: Cursor,
        renderer: &Renderer,
        shell: &mut Shell<ButtonAction<Message>>,
        viewport: &Rectangle,
    ) {
        let mut messages = shell::Bus::new();
        let mut local = shell.local(&mut messages);
        if shell.is_event_captured() {
            local.capture_event();
        }
        self.0
            .update(tree, event, layout, cursor, renderer, &mut local, viewport);
        shell.merge(local, ButtonAction::Content);
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.0
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &ButtonTheme<Theme>,
        style: &renderer::Style,
        layout: Layout,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        self.0.draw(
            tree,
            renderer,
            &theme.theme,
            style,
            layout,
            cursor,
            viewport,
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
    ) -> Vec<overlay::Element<'b, ButtonAction<Message>, ButtonTheme<Theme>, Renderer>> {
        self.0
            .overlay(tree, layout, renderer, viewport, translation, window)
            .into_iter()
            .map(|inner| {
                overlay::Element::new(Box::new(ButtonContentOverlay(
                    inner.map(&ButtonAction::Content),
                )))
            })
            .collect()
    }
}

struct ButtonContentOverlay<'a, Message, Theme>(overlay::Element<'a, Message, Theme, Renderer>);

impl<Message, Theme> Overlay<Message, ButtonTheme<Theme>, Renderer>
    for ButtonContentOverlay<'_, Message, Theme>
{
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &ButtonTheme<Theme>,
        style: &renderer::Style,
        cursor: Cursor,
    ) {
        self.0
            .as_overlay()
            .draw(renderer, &theme.theme, style, cursor);
    }
    fn operate(&mut self, renderer: &Renderer, operation: &mut dyn Operation) {
        self.0.as_overlay_mut().operate(renderer, operation);
    }
    fn update(
        &mut self,
        event: &Event,
        cursor: Cursor,
        renderer: &Renderer,
        shell: &mut Shell<Message>,
    ) {
        self.0
            .as_overlay_mut()
            .update(event, cursor, renderer, shell);
    }
    fn mouse_interaction(&self, cursor: Cursor, renderer: &Renderer) -> mouse::Interaction {
        self.0.as_overlay().mouse_interaction(cursor, renderer)
    }
    fn overlay<'a>(
        &'a mut self,
        renderer: &Renderer,
    ) -> Vec<overlay::Element<'a, Message, ButtonTheme<Theme>, Renderer>> {
        self.0
            .as_overlay_mut()
            .overlay(renderer)
            .into_iter()
            .map(|inner| overlay::Element::new(Box::new(ButtonContentOverlay(inner))))
            .collect()
    }
    fn index(&self) -> f32 {
        self.0.as_overlay().index()
    }
}

fn picker_button<'a, Message: Clone + 'a, Theme: button::Catalog + 'a>(
    content: impl Widget<Message, Theme, Renderer> + 'a,
    style: Option<ButtonStyleFn<'a, Theme>>,
) -> PickerButton<'a, Message, Theme> {
    Button::new(ButtonContent(content.boxed()))
        .width(Length::Fill)
        .on_press(ButtonAction::Press)
        .class(style)
}

#[derive(Debug, Clone)]
enum InternalMessage {
    ChangeInput(String),
}

/// Builds the hex input showing `text`, which may be a partial edit of `color`.
fn hex_text_input<'a, Theme>(
    color: Color,
    text: &str,
    appearance: &Appearance<'a, Theme>,
) -> TextInput<'a, InternalMessage, InternalTheme<Theme>>
where
    Theme: text_input::Catalog + Clone + 'a,
{
    let mut input = text_input(color.as_hex_string(), text.to_owned())
        .on_input(InternalMessage::ChangeInput)
        .padding(PADDING)
        .align_x(iced::Alignment::Center)
        .class(appearance.hex_style.clone());
    if let Some(font) = appearance.font {
        input = input.font(font);
    }
    if let Some(size) = appearance.text_size {
        input = input.size(size);
    }
    input
}

/// Parses a hex color typed into the hex input.
fn parse_hex(text: &str) -> Option<Color> {
    // `Color::from_str` slices by byte index, which panics on multibyte characters.
    text.is_ascii()
        .then(|| Color::from_str(text).ok())
        .flatten()
}

struct InternalTheme<Theme: text_input::Catalog + Clone> {
    theme: Theme,
    color: Color,
    is_valid: bool,
    invalid_hex: Color,
}
impl<Theme: text_input::Catalog + Clone> text_input::Catalog for InternalTheme<Theme> {
    type Class<'a> = Option<HexStyleFn<'a, Theme>>;

    fn default<'a>() -> Self::Class<'a> {
        None
    }

    fn style(&self, class: &Self::Class<'_>, status: text_input::Status) -> text_input::Style {
        if let Some(style) = class {
            return style(
                &self.theme,
                HexInputStatus {
                    status,
                    color: self.color,
                    is_valid: self.is_valid,
                },
            );
        }
        let hsv = Hsv::from(self.color);
        let default = <Theme as text_input::Catalog>::style(
            &self.theme,
            &<Theme as text_input::Catalog>::default(),
            status,
        );
        text_input::Style {
            background: self.color.into(),
            value: Color {
                a: 1.0,
                ..Hsv {
                    hue: 0,
                    saturation: 0.0,
                    value: if hsv.value < 0.5 { 1.0 } else { 0.0 },
                }
                .into()
            },
            border: Border {
                color: if self.is_valid {
                    default.border.color
                } else {
                    self.invalid_hex
                },
                width: 1.0,
                radius: 5.0.into(),
            },
            ..default
        }
    }
}

impl<'a, Message, Theme> ColorPicker<'a, Message, Theme>
where
    Message: 'a + Clone,
    Theme: 'a
        + Catalog
        + iced::widget::button::Catalog
        + iced::widget::text::Catalog
        + container::Catalog
        + text_input::Catalog
        + Clone,
{
    /// Creates a new [`ColorPicker`] wrapping around the given underlay.
    ///
    /// It expects:
    ///     * if the overlay of the color picker is visible.
    ///     * the initial color to show.
    ///     * the underlay [`Widget`] on which this [`ColorPicker`]
    ///         will be wrapped around.
    ///     * a message that will be send when the cancel button of the [`ColorPicker`]
    ///         is pressed.
    ///     * a function that will be called when the submit button of the [`ColorPicker`]
    ///         is pressed, which takes the picked [`Color`] value.
    pub fn new<U, F>(
        show_picker: bool,
        color: Color,
        underlay: U,
        on_cancel: Message,
        on_submit: F,
    ) -> Self
    where
        U: Widget<Message, Theme, Renderer> + 'a,
        F: Fn(Color) -> Message + 'a,
    {
        let appearance = Appearance::default();
        let cancel_button = picker_button(
            text("X")
                .align_x(Horizontal::Center)
                .font(iced::Font {
                    weight: iced::font::Weight::ExtraBold,
                    ..iced::Font::DEFAULT
                })
                .width(Length::Fill),
            None,
        );
        let submit_button = picker_button(
            text(Renderer::CHECKMARK_ICON)
                .font(Renderer::ICON_FONT)
                .align_x(Horizontal::Center)
                .width(Length::Fill),
            None,
        );
        let hex_input = hex_text_input(color, &color.as_hex_string(), &appearance);

        Self {
            show_picker,
            color,
            underlay: underlay.boxed(),
            on_cancel,
            on_submit: Box::new(on_submit),
            class: <Theme as Catalog>::default(),
            appearance,
            cancel_button,
            submit_button,
            hex_input,
        }
    }

    /// Sets the font of channel labels, values, and the hex input.
    /// Unset fonts retain each element's default font.
    #[must_use]
    pub fn font(mut self, font: impl Into<Font>) -> Self {
        self.appearance.font = Some(font.into());
        self.hex_input = hex_text_input(self.color, &self.color.as_hex_string(), &self.appearance);
        self
    }

    /// Sets the text size of channel labels, values, and the hex input.
    #[must_use]
    pub fn text_size(mut self, size: impl Into<Pixels>) -> Self {
        self.appearance.text_size = Some(size.into());
        self.hex_input = hex_text_input(self.color, &self.color.as_hex_string(), &self.appearance);
        self
    }

    /// Sets the red, green, blue, and alpha channel labels.
    #[must_use]
    pub fn channel_labels(
        mut self,
        red: impl Into<String>,
        green: impl Into<String>,
        blue: impl Into<String>,
        alpha: impl Into<String>,
    ) -> Self {
        self.appearance.channel_labels = [red.into(), green.into(), blue.into(), alpha.into()];
        self
    }

    /// Replaces the cancel button's content without changing its action.
    #[must_use]
    pub fn cancel_content(mut self, content: impl Widget<Message, Theme, Renderer> + 'a) -> Self {
        self.cancel_button = picker_button(content, self.appearance.cancel_style.clone());
        self
    }

    /// Replaces the submit button's content without changing its action.
    #[must_use]
    pub fn submit_content(mut self, content: impl Widget<Message, Theme, Renderer> + 'a) -> Self {
        self.submit_button = picker_button(content, self.appearance.submit_style.clone());
        self
    }

    /// Sets the cancel label exposed through widget operations (for example iced_test selectors).
    /// This is a hook for screen readers once iced supports them, not screen-reader support.
    #[must_use]
    pub fn cancel_label(mut self, label: impl Into<String>) -> Self {
        self.appearance.cancel_label = label.into();
        self
    }

    /// Sets the submit label exposed through widget operations (for example iced_test selectors).
    /// This is a hook for screen readers once iced supports them, not screen-reader support.
    #[must_use]
    pub fn submit_label(mut self, label: impl Into<String>) -> Self {
        self.appearance.submit_label = label.into();
        self
    }

    /// Sets the cancel button style, including keyboard focus.
    #[must_use]
    pub fn cancel_button_style(
        mut self,
        style: impl Fn(&Theme, ButtonStatus) -> button::Style + 'a,
    ) -> Self {
        self.appearance.cancel_style = Some(Rc::new(style));
        self.cancel_button = self
            .cancel_button
            .class(self.appearance.cancel_style.clone());
        self
    }

    /// Sets the submit button style, including keyboard focus.
    #[must_use]
    pub fn submit_button_style(
        mut self,
        style: impl Fn(&Theme, ButtonStatus) -> button::Style + 'a,
    ) -> Self {
        self.appearance.submit_style = Some(Rc::new(style));
        self.submit_button = self
            .submit_button
            .class(self.appearance.submit_style.clone());
        self
    }

    /// Sets a shared style for both buttons, including keyboard focus.
    #[must_use]
    pub fn button_style(
        mut self,
        style: impl Fn(&Theme, ButtonStatus) -> button::Style + 'a,
    ) -> Self {
        let style: ButtonStyleFn<'a, Theme> = Rc::new(style);
        self.appearance.cancel_style = Some(Rc::clone(&style));
        self.appearance.submit_style = Some(style);
        self.cancel_button = self
            .cancel_button
            .class(self.appearance.cancel_style.clone());
        self.submit_button = self
            .submit_button
            .class(self.appearance.submit_style.clone());
        self
    }

    /// Sets the hex input style using its interaction state, color, and draft validity.
    #[must_use]
    pub fn hex_input_style(
        mut self,
        style: impl Fn(&Theme, HexInputStatus) -> text_input::Style + 'a,
    ) -> Self {
        self.appearance.hex_style = Some(Rc::new(style));
        self.hex_input = hex_text_input(self.color, &self.color.as_hex_string(), &self.appearance);
        self
    }

    /// Sets the style of the [`ColorPicker`].
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        <Theme as Catalog>::Class<'a>: From<StyleFn<'a, Theme, Style>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme, Style>).into();
        self
    }

    /// Sets the class of the input of the [`ColorPicker`].
    #[must_use]
    pub fn class(mut self, class: impl Into<<Theme as Catalog>::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }
}

/// The state of the [`ColorPicker`].
#[derive(Debug, Default)]
pub struct State {
    /// The state of the overlay.
    pub(crate) overlay_state: OverlayState,
    overlay_layout: Option<(Point, Size, Tree)>,
}

impl State {
    /// Creates a new [`State`].
    #[must_use]
    pub fn new(color: Color) -> Self
    where
        Theme: text::Catalog + container::Catalog + iced::widget::button::Catalog,
    {
        Self {
            overlay_state: OverlayState::new(color),
            overlay_layout: None,
        }
    }

    /// Resets the color of the state.
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.overlay_state.color = Color::from_rgb(0.5, 0.25, 0.25);
        self.overlay_state.hex_text = self.overlay_state.color.as_hex_string();
        self.overlay_state.color_bar_dragged = ColorBarDragged::None;
    }
}

impl<'a, Message, Theme> Widget<Message, Theme, Renderer> for ColorPicker<'a, Message, Theme>
where
    Message: 'a + Clone + std::fmt::Debug,
    Theme: 'a
        + Catalog
        + iced::widget::button::Catalog
        + iced::widget::text::Catalog
        + container::Catalog
        + text_input::Catalog
        + Clone,
{
    fn tag(&self) -> Tag {
        Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new(self.color))
    }

    fn diff(&mut self, tree: &mut Tree) {
        let State {
            overlay_state: color_picker_state,
            overlay_layout,
        } = tree.state.downcast_mut::<State>();
        *overlay_layout = None;
        let drawn = (
            color_picker_state.color,
            color_picker_state.current_hue_degrees,
        );

        // Rebuilding the view with an unchanged color must not discard an edit in progress,
        // so the working color is only replaced by a new color or once the picker is closed.
        if color_picker_state.external_color != self.color
            || (!self.show_picker && color_picker_state.color != self.color)
        {
            color_picker_state.external_color = self.color;
            color_picker_state.color = self.color;
            color_picker_state.hex_text = self.color.as_hex_string();
            if !matches!(self.color, Color::BLACK | Color::WHITE) {
                // Black (0x000000) and White (0xFFFFFF) colors don't have hue, so we keep it as it
                // was on those cases
                let hue = Hsv::from(self.color).hue;
                color_picker_state.previous_hue_degrees = hue;
                color_picker_state.current_hue_degrees = hue;
            }
        }

        if !self.show_picker {
            color_picker_state.hex_text = color_picker_state.color.as_hex_string();
            if color_picker_state.current_hue_degrees != color_picker_state.previous_hue_degrees {
                color_picker_state.current_hue_degrees = color_picker_state.previous_hue_degrees;
            }
        }

        // Style changes clear the canvases while drawing, so only a new color needs it here.
        if drawn
            != (
                color_picker_state.color,
                color_picker_state.current_hue_degrees,
            )
        {
            color_picker_state.clear();
        }
        self.hex_input = hex_text_input(
            color_picker_state.color,
            &color_picker_state.hex_text,
            &self.appearance,
        );

        // The measured geometry is separate from the retained button and input trees.
        // Diff real content so custom widgets keep their state across view rebuilds.
        let mut diff_underlay = |tree: &mut Tree| tree.diff(&mut self.underlay);
        let mut diff_overlay = |tree: &mut Tree| {
            let mut children: [&mut dyn FnMut(&mut Tree); 4] = [
                &mut |tree| tree.diff(&mut self.cancel_button),
                &mut |tree| tree.diff(&mut self.submit_button),
                // Reserve the old stand-in slot to keep retained input state at index 3.
                &mut |_| {},
                &mut |tree| tree.diff::<_, _, Renderer>(&mut self.hex_input),
            ];
            tree.diff_children_custom(&mut children, |tree, diff| diff(tree), |_| Tree::empty());
        };
        let mut children: [&mut dyn FnMut(&mut Tree); 2] = [&mut diff_underlay, &mut diff_overlay];
        tree.diff_children_custom(&mut children, |tree, diff| diff(tree), |_| Tree::empty());
    }

    fn size(&self) -> iced::Size<Length> {
        self.underlay.size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &Limits) {
        self.underlay
            .layout(&mut tree.children[0], renderer, limits);
        tree.size = tree.children[0].size;
        tree.children[0].translation = Vector::ZERO;
    }

    fn update(
        &mut self,
        state: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.underlay.update(
            &mut state.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        )
    }

    fn mouse_interaction(
        &self,
        state: &Tree,
        layout: Layout,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.underlay
            .mouse_interaction(&state.children[0], layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        state: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: Cursor,
        viewport: &Rectangle,
    ) {
        self.underlay.draw(
            &state.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        state: &'b mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        let (underlay_tree, picker_tree) = state.children.split_at_mut(1);

        let underlays = self.underlay.overlay(
            &mut underlay_tree[0],
            layout,
            renderer,
            viewport,
            translation,
            window,
        );

        let picker = self.show_picker.then(|| {
            let State {
                overlay_state,
                overlay_layout,
            } = state.state.downcast_mut::<State>();
            let bounds = layout.bounds();
            let position = bounds.position() + translation;
            let underlay_height = bounds.height;

            if overlay_state.layout_stale
                || !matches!(
                    overlay_layout,
                    Some((cached_position, cached_window, _))
                        if *cached_position == position && *cached_window == window
                )
            {
                overlay_state.layout_stale = false;
                *overlay_layout = Some((
                    position,
                    window,
                    ColorPickerOverlay::compute_layout(
                        &mut picker_tree[0],
                        &mut overlay_state.rgba_tree,
                        renderer,
                        position,
                        window,
                        underlay_height,
                        &mut self.cancel_button,
                        &mut self.submit_button,
                        &mut self.hex_input,
                        &self.appearance,
                    ),
                ));
            }

            let (_, _, node) = overlay_layout
                .as_ref()
                .expect("color picker overlay layout should be cached");

            ColorPickerOverlay::new(
                overlay_state,
                &mut picker_tree[0],
                self.on_cancel.clone(),
                &self.on_submit,
                GeometryLayout::new(node),
                window,
                &self.class,
                &mut self.cancel_button,
                &mut self.submit_button,
                &mut self.hex_input,
                &mut self.appearance,
            )
            .overlay()
        });

        underlays.into_iter().chain(picker).collect()
    }
}

impl<Message, Theme> Meta for ColorPicker<'_, Message, Theme>
where
    Message: Clone,
    Theme: Catalog + iced::widget::button::Catalog + text_input::Catalog + Clone,
{
}

/// The padding around the elements.
const PADDING: Padding = Padding::new(10.0);
/// The spacing between the element.
const SPACING: Pixels = Pixels(15.0);
/// The spacing between the buttons.
const BUTTON_SPACING: Pixels = Pixels(5.0);
const MARKER_STROKE_WIDTH: f32 = 3.0;
const MAX_SIZE: Size = Size::new(600.0, 300.0);
/// The offset to move the overlay above/below the underlay element.
const OFFSET: f32 = 10.0;

/// The step value of the keyboard change of the sat/value color values.
const SAT_VALUE_STEP: f32 = 0.005;
/// The step value of the keyboard change of the hue color value.
const HUE_STEP: i32 = 1;
/// The step value of the keyboard change of the RGBA color values.
const RGBA_STEP: i16 = 1;

/// The overlay of the [`ColorPicker`](crate::widget::ColorPicker).
#[allow(missing_debug_implementations)]
pub struct ColorPickerOverlay<'a, 'b, Message, Theme>
where
    Message: Clone + 'a,
    Theme: Catalog + iced::widget::button::Catalog + text_input::Catalog + Clone,
    'b: 'a,
{
    /// The state of the [`ColorPickerOverlay`].
    state: &'a mut OverlayState,
    /// The cancel button of the [`ColorPickerOverlay`].
    cancel_button: &'a mut PickerButton<'b, Message, Theme>,
    /// The submit button of the [`ColorPickerOverlay`].
    submit_button: &'a mut PickerButton<'b, Message, Theme>,
    /// The hex input of the [`ColorPickerOverlay`].
    hex_input: &'a mut TextInput<'b, InternalMessage, InternalTheme<Theme>>,
    appearance: &'a mut Appearance<'b, Theme>,
    /// The message that is sent if the cancel button of the [`ColorPickerOverlay`] is pressed or
    /// when pressed outside its bounds.
    on_cancel: Message,
    /// The function that produces a message when the submit button of the [`ColorPickerOverlay`]
    /// is pressed.
    on_submit: &'a dyn Fn(Color) -> Message,
    /// The layout of the [`ColorPickerOverlay`].
    layout: GeometryLayout<'a>,
    /// The window containing the [`ColorPickerOverlay`].
    window: Size,
    /// The style of the [`ColorPickerOverlay`].
    class: &'a <Theme as Catalog>::Class<'b>,
    /// The reference to the tree holding the state of this overlay.
    tree: &'a mut Tree,
}

// Geometry nodes are stateless copies, separate from the widgets' retained trees.
#[derive(Clone, Copy)]
struct GeometryLayout<'a> {
    layout: Layout,
    tree: &'a Tree,
}

impl<'a> GeometryLayout<'a> {
    fn new(tree: &'a Tree) -> Self {
        Self {
            layout: Layout::new(tree.size).move_to(Point::ORIGIN + tree.translation),
            tree,
        }
    }

    fn bounds(self) -> Rectangle {
        self.layout.bounds()
    }

    fn children(self) -> impl Iterator<Item = Self> {
        self.layout
            .iter(&self.tree.children)
            .map(|(layout, tree)| Self { layout, tree })
    }

    fn for_widget(self, tree: &Tree) -> Layout {
        Layout::new(tree.size).move_to(self.layout.position())
    }
}

fn geometry_tree(tree: &Tree) -> Tree {
    Tree {
        size: tree.size,
        translation: tree.translation,
        children: tree.children.iter().map(geometry_tree).collect(),
        ..Tree::empty()
    }
}

fn layout_geometry<Message, Theme>(
    mut widget: impl Widget<Message, Theme, Renderer>,
    renderer: &Renderer,
    limits: &Limits,
) -> Tree {
    let mut tree = Tree::new(&widget);
    tree.diff(&mut widget);
    widget.layout(&mut tree, renderer, limits);
    geometry_tree(&tree)
}

struct ColorPickerLayout<'a, 'b, Message, Theme>
where
    Message: Clone + 'a,
    Theme: Catalog + iced::widget::button::Catalog + text_input::Catalog + Clone,
    'b: 'a,
{
    cancel_button: &'a mut PickerButton<'b, Message, Theme>,
    submit_button: &'a mut PickerButton<'b, Message, Theme>,
    hex_input: &'a mut TextInput<'b, InternalMessage, InternalTheme<Theme>>,
    tree: &'a mut Tree,
    rgba_tree: &'a mut Tree,
    appearance: &'a Appearance<'b, Theme>,
}

impl<'a, 'b, Message, Theme> ColorPickerOverlay<'a, 'b, Message, Theme>
where
    Message: 'b + Clone + std::fmt::Debug,
    Theme: 'b
        + Catalog
        + iced::widget::button::Catalog
        + iced::widget::text::Catalog
        + container::Catalog
        + text_input::Catalog
        + Clone,
    'b: 'a,
{
    /// Creates a new [`ColorPickerOverlay`] on the given position.
    #[allow(clippy::too_many_arguments)]
    pub(self) fn new(
        state: &'a mut OverlayState,
        tree: &'a mut Tree,
        on_cancel: Message,
        on_submit: &'a dyn Fn(Color) -> Message,
        layout: GeometryLayout<'a>,
        window: Size,
        class: &'a <Theme as Catalog>::Class<'b>,
        cancel_button: &'a mut PickerButton<'b, Message, Theme>,
        submit_button: &'a mut PickerButton<'b, Message, Theme>,
        hex_input: &'a mut TextInput<'b, InternalMessage, InternalTheme<Theme>>,
        appearance: &'a mut Appearance<'b, Theme>,
    ) -> Self {
        ColorPickerOverlay {
            state,
            cancel_button,
            submit_button,
            hex_input,
            appearance,
            on_cancel,
            on_submit,
            layout,
            window,
            class,
            tree,
        }
    }

    fn is_hex_input_focused(&mut self, layout: Layout, renderer: &Renderer) -> bool {
        struct IsFocused(bool);

        impl Operation for IsFocused {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }

            fn focusable(
                &mut self,
                _id: Option<&Id>,
                _bounds: Rectangle,
                state: &mut dyn operation::Focusable,
            ) {
                self.0 = state.is_focused();
            }
        }

        let mut is_focused = IsFocused(false);
        self.hex_input.operate(
            &mut self.tree.children[3],
            layout,
            &layout.bounds(),
            renderer,
            &mut is_focused,
        );
        is_focused.0
    }

    fn hex_input_layout(&self) -> Layout {
        self.layout
            .children()
            .nth(1)
            .and_then(|block2| block2.children().nth(1))
            .expect("widget: Layout should have a hex text layout")
            .for_widget(&self.tree.children[3])
    }

    fn submit(&mut self, shell: &mut Shell<Message>) {
        shell.publish((self.on_submit)(self.state.color));
        self.state.previous_hue_degrees = self.state.current_hue_degrees;
    }

    /// Handles Enter and Escape, returning whether the key was consumed.
    ///
    /// Enter submits while any part of the picker is focused, except the cancel button, where
    /// it cancels. Escape always cancels. Handled keys are captured so they cannot also reach
    /// the application below the picker.
    fn on_confirm_key(
        &mut self,
        event: &Event,
        renderer: &Renderer,
        shell: &mut Shell<Message>,
    ) -> bool {
        let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event else {
            return false;
        };

        let submit = match key {
            keyboard::Key::Named(keyboard::key::Named::Escape) => false,
            keyboard::Key::Named(keyboard::key::Named::Enter) => {
                if self.is_hex_input_focused(self.hex_input_layout(), renderer) {
                    if parse_hex(&self.state.hex_text).is_none() {
                        // Submitting would not use what was typed.
                        shell.capture_event();
                        return true;
                    }
                    true
                } else {
                    match self.state.focus {
                        Focus::None => return false,
                        Focus::Cancel => false,
                        _ => true,
                    }
                }
            }
            _ => return false,
        };

        if submit {
            self.submit(shell);
        } else {
            shell.publish(self.on_cancel.clone());
        }
        shell.capture_event();
        shell.request_redraw();
        true
    }

    /// Replaces the hex text with the formatted current color.
    fn sync_hex_text(&mut self) {
        self.state.hex_text = self.state.color.as_hex_string();
        self.rebuild_hex_input();
        // Upstream's text input only copies its value into the tree state during
        // layout, and the overlay layout is cached, so force a recompute.
        self.state.layout_stale = true;
    }

    fn rebuild_hex_input(&mut self) {
        let mut hex_input = hex_text_input(self.state.color, &self.state.hex_text, self.appearance);
        self.tree.children[3].diff::<_, _, Renderer>(&mut hex_input);
        *self.hex_input = hex_input;
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_layout(
        tree: &mut Tree,
        rgba_tree: &mut Tree,
        renderer: &Renderer,
        position: Point,
        window: Size,
        underlay_height: f32,
        cancel_button: &mut PickerButton<'b, Message, Theme>,
        submit_button: &mut PickerButton<'b, Message, Theme>,
        hex_input: &mut TextInput<'b, InternalMessage, InternalTheme<Theme>>,
        appearance: &Appearance<'b, Theme>,
    ) -> Tree {
        let space_below = window.height - (position.y + underlay_height + OFFSET);
        let space_above = position.y - OFFSET;

        let mut show_below = false;
        let bounds = Size::new(
            window.width,
            if space_below > space_above {
                show_below = true;
                space_below
            } else {
                space_above
            },
        );

        let position = if show_below {
            position + Vector::new(0.0, underlay_height + OFFSET)
        } else {
            position - Vector::new(0.0, OFFSET)
        };

        let (max_width, max_height) = if bounds.width > bounds.height {
            (MAX_SIZE.width, MAX_SIZE.height)
        } else {
            (MAX_SIZE.height, MAX_SIZE.width)
        };

        let limits = Limits::new(Size::ZERO, bounds)
            .shrink(PADDING)
            .width(Length::Fill.max(max_width))
            .height(Length::Fill.max(max_height));

        let mut picker = ColorPickerLayout {
            cancel_button,
            submit_button,
            hex_input,
            tree,
            rgba_tree,
            appearance,
        };
        let first = Row::<Element<'_, (), Theme, Renderer>>::new()
            .width(Length::Fill)
            .height(Length::Fill);
        let second = Row::<Element<'_, (), Theme, Renderer>>::new()
            .width(Length::Fill)
            .height(Length::Fill);
        let divider = if bounds.width > bounds.height {
            layout_geometry(
                Row::new().spacing(SPACING).push(first).push(second),
                renderer,
                &limits,
            )
        } else {
            layout_geometry(
                Column::new().spacing(SPACING).push(first).push(second),
                renderer,
                &limits,
            )
        };

        let mut divider_children = GeometryLayout::new(&divider).children();
        let block1_bounds = divider_children
            .next()
            .expect("Divider should have a first child")
            .bounds();
        let block2_bounds = divider_children
            .next()
            .expect("Divider should have a second child")
            .bounds();
        let block1_node = block1_layout::<Theme>(renderer, block1_bounds);
        let block2_node = block2_layout(&mut picker, renderer, block2_bounds);

        let (width, height) = if bounds.width > bounds.height {
            (
                block1_node.size.width + block2_node.size.width + SPACING.0,
                block2_node.size.height,
            )
        } else {
            (
                block2_node.size.width,
                block1_node.size.height + block2_node.size.height + SPACING.0,
            )
        };

        let mut node = Tree {
            size: Size::new(width, height),
            children: vec![block1_node, block2_node],
            ..Tree::empty()
        };
        node.position_and_bounce(position, window, show_below);
        node
    }

    /// Turn this [`ColorPickerOverlay`] into an overlay [`Element`](overlay::Element).
    #[must_use]
    pub fn overlay(self) -> overlay::Element<'a, Message, Theme, Renderer> {
        overlay::Element::new(Box::new(self))
    }

    /// The event handling for the HSV color area.
    fn update_hsv_color(
        &mut self,
        event: &Event,
        layout: GeometryLayout<'_>,
        cursor: Cursor,
        shell: &mut Shell<Message>,
    ) {
        let mut hsv_color_children = layout.children();

        let hsv_color: Hsv = self.state.color.into();
        let mut color_changed = false;

        let sat_value_bounds = hsv_color_children
            .next()
            .expect("widget: Layout should have a sat/value layout")
            .bounds();
        let hue_bounds = hsv_color_children
            .next()
            .expect("widget: Layout should have a hue layout")
            .bounds();

        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => match delta {
                mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => {
                    let move_value =
                        |value: u16, y: f32| ((i32::from(value) + y as i32).rem_euclid(360)) as u16;

                    if cursor.is_over(hue_bounds) {
                        self.state.current_hue_degrees = move_value(hsv_color.hue, *y);
                        self.state.color = Color {
                            a: self.state.color.a,
                            ..Hsv {
                                hue: self.state.current_hue_degrees,
                                ..hsv_color
                            }
                            .into()
                        };
                        color_changed = true;
                    }
                }
            },
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if cursor.is_over(sat_value_bounds) {
                    self.state.color_bar_dragged = ColorBarDragged::SatValue;
                    self.state.focus = Focus::SatValue;
                }
                if cursor.is_over(hue_bounds) {
                    self.state.color_bar_dragged = ColorBarDragged::Hue;
                    self.state.focus = Focus::Hue;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. } | touch::Event::FingerLost { .. }) => {
                if matches!(
                    self.state.color_bar_dragged,
                    ColorBarDragged::SatValue | ColorBarDragged::Hue
                ) {
                    shell.capture_event();
                    self.state.color_bar_dragged = ColorBarDragged::None;
                }
            }
            _ => {}
        }

        let calc_percentage_sat =
            |cursor_position: Point| (cursor_position.x.max(0.0) / sat_value_bounds.width).min(1.0);

        let calc_percentage_value = |cursor_position: Point| {
            (cursor_position.y.max(0.0) / sat_value_bounds.height).min(1.0)
        };

        let calc_hue = |cursor_position: Point| {
            ((cursor_position.x.max(0.0) / hue_bounds.width).min(1.0) * 360.0) as u16
        };

        match self.state.color_bar_dragged {
            ColorBarDragged::SatValue => {
                self.state.color = Color {
                    a: self.state.color.a,
                    ..Hsv {
                        saturation: cursor
                            .position()
                            .map(|position| {
                                calc_percentage_sat(Point {
                                    x: (position.x - sat_value_bounds.x)
                                        .max(0.0)
                                        .min(sat_value_bounds.width),
                                    y: (position.y - sat_value_bounds.y)
                                        .max(0.0)
                                        .min(sat_value_bounds.height),
                                })
                            })
                            .unwrap_or(hsv_color.saturation),
                        value: cursor
                            .position()
                            .map(|position| {
                                calc_percentage_value(Point {
                                    x: (position.x - sat_value_bounds.x)
                                        .max(0.0)
                                        .min(sat_value_bounds.width),
                                    y: (position.y - sat_value_bounds.y)
                                        .max(0.0)
                                        .min(sat_value_bounds.height),
                                })
                            })
                            .unwrap_or(hsv_color.value),
                        hue: self.state.current_hue_degrees,
                    }
                    .into()
                };
                color_changed = true;
            }
            ColorBarDragged::Hue => {
                self.state.current_hue_degrees = cursor
                    .position()
                    .map(|position| {
                        calc_hue(Point {
                            x: (position.x - hue_bounds.x).max(0.0).min(hue_bounds.width),
                            y: (position.y - hue_bounds.y).max(0.0).min(hue_bounds.height),
                        })
                    })
                    .unwrap_or(self.state.current_hue_degrees);

                self.state.color = Color {
                    a: self.state.color.a,
                    ..Hsv {
                        hue: self.state.current_hue_degrees,
                        ..hsv_color
                    }
                    .into()
                };
                color_changed = true;
            }
            _ => {}
        }

        if color_changed {
            shell.capture_event();
            shell.request_redraw();
        }
    }

    /// The event handling for the RGBA color area.
    #[allow(clippy::too_many_lines)]
    fn update_rgba_color(
        &mut self,
        event: &Event,
        layout: GeometryLayout<'_>,
        hex_input_layout: GeometryLayout<'_>,
        cursor: Cursor,
        shell: &mut Shell<Message>,
    ) {
        let mut rgba_color_children = layout.children();
        let mut color_changed = false;

        let mut red_row_children = rgba_color_children
            .next()
            .expect("widget: Layout should have a red row layout")
            .children();
        let _ = red_row_children.next();
        let red_bar_bounds = red_row_children
            .next()
            .expect("widget: Layout should have a red bar layout")
            .bounds();

        let mut green_row_children = rgba_color_children
            .next()
            .expect("widget: Layout should have a green row layout")
            .children();
        let _ = green_row_children.next();
        let green_bar_bounds = green_row_children
            .next()
            .expect("widget: Layout should have a green bar layout")
            .bounds();

        let mut blue_row_children = rgba_color_children
            .next()
            .expect("widget: Layout should have a blue row layout")
            .children();
        let _ = blue_row_children.next();
        let blue_bar_bounds = blue_row_children
            .next()
            .expect("widget: Layout should have a blue bar layout")
            .bounds();

        let mut alpha_row_children = rgba_color_children
            .next()
            .expect("widget: Layout should have an alpha row layout")
            .children();
        let _ = alpha_row_children.next();
        let alpha_bar_bounds = alpha_row_children
            .next()
            .expect("widget: Layout should have an alpha bar layout")
            .bounds();

        let hex_input_bounds = hex_input_layout.bounds();

        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => match delta {
                mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => {
                    let move_value =
                        //|value: f32, y: f32| (value * 255.0 + y).clamp(0.0, 255.0) / 255.0;
                        |value: f32, y: f32| value.mul_add(255.0, y).clamp(0.0, 255.0) / 255.0;

                    if cursor.is_over(red_bar_bounds) {
                        self.state.color = Color {
                            r: move_value(self.state.color.r, *y),
                            ..self.state.color
                        };
                        color_changed = true;
                    }
                    if cursor.is_over(green_bar_bounds) {
                        self.state.color = Color {
                            g: move_value(self.state.color.g, *y),
                            ..self.state.color
                        };
                        color_changed = true;
                    }
                    if cursor.is_over(blue_bar_bounds) {
                        self.state.color = Color {
                            b: move_value(self.state.color.b, *y),
                            ..self.state.color
                        };
                        color_changed = true;
                    }
                    if cursor.is_over(alpha_bar_bounds) {
                        self.state.color = Color {
                            a: move_value(self.state.color.a, *y),
                            ..self.state.color
                        };
                        color_changed = true;
                    }
                }
            },
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if cursor.is_over(red_bar_bounds) {
                    self.state.color_bar_dragged = ColorBarDragged::Red;
                    self.state.focus = Focus::Red;
                }
                if cursor.is_over(green_bar_bounds) {
                    self.state.color_bar_dragged = ColorBarDragged::Green;
                    self.state.focus = Focus::Green;
                }
                if cursor.is_over(blue_bar_bounds) {
                    self.state.color_bar_dragged = ColorBarDragged::Blue;
                    self.state.focus = Focus::Blue;
                }
                if cursor.is_over(alpha_bar_bounds) {
                    self.state.color_bar_dragged = ColorBarDragged::Alpha;
                    self.state.focus = Focus::Alpha;
                }
                if cursor.is_over(hex_input_bounds) {
                    self.state.focus = Focus::None;
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerLifted { .. } | touch::Event::FingerLost { .. }) => {
                if matches!(
                    self.state.color_bar_dragged,
                    ColorBarDragged::Red
                        | ColorBarDragged::Green
                        | ColorBarDragged::Blue
                        | ColorBarDragged::Alpha
                ) {
                    shell.capture_event();
                    self.state.color_bar_dragged = ColorBarDragged::None;
                }
            }
            _ => {}
        }

        let calc_percentage = |bounds: Rectangle, cursor_position: Point| {
            (cursor_position.x.max(0.0) / bounds.width).min(1.0)
        };

        match self.state.color_bar_dragged {
            ColorBarDragged::Red => {
                self.state.color = Color {
                    r: cursor
                        .position()
                        .map(|position| {
                            calc_percentage(
                                red_bar_bounds,
                                Point {
                                    x: (position.x - red_bar_bounds.x)
                                        .max(0.0)
                                        .min(red_bar_bounds.width),
                                    y: (position.y - red_bar_bounds.y)
                                        .max(0.0)
                                        .min(red_bar_bounds.height),
                                },
                            )
                        })
                        .unwrap_or(self.state.color.r),
                    ..self.state.color
                };
                color_changed = true;
            }
            ColorBarDragged::Green => {
                self.state.color = Color {
                    g: cursor
                        .position()
                        .map(|position| {
                            calc_percentage(
                                green_bar_bounds,
                                Point {
                                    x: (position.x - green_bar_bounds.x)
                                        .max(0.0)
                                        .min(green_bar_bounds.width),
                                    y: (position.y - green_bar_bounds.y)
                                        .max(0.0)
                                        .min(green_bar_bounds.height),
                                },
                            )
                        })
                        .unwrap_or(self.state.color.g),
                    ..self.state.color
                };
                color_changed = true;
            }
            ColorBarDragged::Blue => {
                self.state.color = Color {
                    b: cursor
                        .position()
                        .map(|position| {
                            calc_percentage(
                                blue_bar_bounds,
                                Point {
                                    x: (position.x - blue_bar_bounds.x)
                                        .max(0.0)
                                        .min(blue_bar_bounds.width),
                                    y: (position.y - blue_bar_bounds.y)
                                        .max(0.0)
                                        .min(blue_bar_bounds.height),
                                },
                            )
                        })
                        .unwrap_or(self.state.color.b),
                    ..self.state.color
                };
                color_changed = true;
            }
            ColorBarDragged::Alpha => {
                self.state.color = Color {
                    a: cursor
                        .position()
                        .map(|position| {
                            calc_percentage(
                                alpha_bar_bounds,
                                Point {
                                    x: (position.x - alpha_bar_bounds.x)
                                        .max(0.0)
                                        .min(alpha_bar_bounds.width),
                                    y: (position.y - alpha_bar_bounds.y)
                                        .max(0.0)
                                        .min(alpha_bar_bounds.height),
                                },
                            )
                        })
                        .unwrap_or(self.state.color.a),
                    ..self.state.color
                };
                color_changed = true;
            }
            _ => {}
        }

        if color_changed {
            if !matches!(self.state.color, Color::BLACK | Color::WHITE) {
                // Black (0x000000) and White (0xFFFFFF) colors don't have hue, so we keep it as it
                // was on those cases
                let hue = Hsv::from(self.state.color).hue;
                self.state.current_hue_degrees = hue;
            }
            shell.capture_event();
            shell.request_redraw();
        }
    }

    /// The even handling for the keyboard input.
    fn on_event_keyboard(&mut self, event: &Event) -> event::Status {
        if let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event {
            let mut status = event::Status::Ignored;

            if matches!(key, keyboard::Key::Named(keyboard::key::Named::Tab)) {
                if self.state.keyboard_modifiers.shift() {
                    self.state.focus = self.state.focus.previous();
                } else {
                    self.state.focus = self.state.focus.next();
                }
                status = event::Status::Captured;
            } else {
                let sat_value_handle = |key_code: &keyboard::Key, color: &mut Color, hue: u16| {
                    let mut hsv_color: Hsv = (*color).into();
                    let mut status = event::Status::Ignored;

                    match key_code {
                        keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => {
                            hsv_color.saturation -= SAT_VALUE_STEP;
                            status = event::Status::Captured;
                        }
                        keyboard::Key::Named(keyboard::key::Named::ArrowRight) => {
                            hsv_color.saturation += SAT_VALUE_STEP;
                            status = event::Status::Captured;
                        }
                        keyboard::Key::Named(keyboard::key::Named::ArrowUp) => {
                            hsv_color.value -= SAT_VALUE_STEP;
                            status = event::Status::Captured;
                        }
                        keyboard::Key::Named(keyboard::key::Named::ArrowDown) => {
                            hsv_color.value += SAT_VALUE_STEP;
                            status = event::Status::Captured;
                        }
                        _ => {}
                    }

                    hsv_color.saturation = hsv_color.saturation.clamp(0.0, 1.0);
                    hsv_color.value = hsv_color.value.clamp(0.0, 1.0);
                    hsv_color.hue = hue;

                    *color = Color {
                        a: color.a,
                        ..hsv_color.into()
                    };
                    status
                };

                let mut hue_handle = |key_code: &keyboard::Key, color: &mut Color| {
                    let mut hsv_color: Hsv = (*color).into();
                    let mut status = event::Status::Ignored;

                    let mut value = i32::from(self.state.current_hue_degrees);

                    match key_code {
                        keyboard::Key::Named(
                            keyboard::key::Named::ArrowLeft | keyboard::key::Named::ArrowDown,
                        ) => {
                            value -= HUE_STEP;
                            status = event::Status::Captured;
                        }
                        keyboard::Key::Named(
                            keyboard::key::Named::ArrowRight | keyboard::key::Named::ArrowUp,
                        ) => {
                            value += HUE_STEP;
                            status = event::Status::Captured;
                        }
                        _ => {}
                    }

                    hsv_color.hue = value.rem_euclid(360) as u16;
                    self.state.current_hue_degrees = hsv_color.hue;

                    *color = Color {
                        a: color.a,
                        ..hsv_color.into()
                    };

                    status
                };

                let rgba_bar_handle = |key_code: &keyboard::Key, value: &mut f32| {
                    let mut byte_value = (*value * 255.0) as i16;
                    let mut status = event::Status::Ignored;

                    match key_code {
                        keyboard::Key::Named(
                            keyboard::key::Named::ArrowLeft | keyboard::key::Named::ArrowDown,
                        ) => {
                            byte_value -= RGBA_STEP;
                            status = event::Status::Captured;
                        }
                        keyboard::Key::Named(
                            keyboard::key::Named::ArrowRight | keyboard::key::Named::ArrowUp,
                        ) => {
                            byte_value += RGBA_STEP;
                            status = event::Status::Captured;
                        }
                        _ => {}
                    }
                    *value = f32::from(byte_value.clamp(0, 255)) / 255.0;

                    status
                };

                match self.state.focus {
                    Focus::SatValue => {
                        status = sat_value_handle(
                            key,
                            &mut self.state.color,
                            self.state.current_hue_degrees,
                        )
                    }
                    Focus::Hue => status = hue_handle(key, &mut self.state.color),
                    Focus::Red => status = rgba_bar_handle(key, &mut self.state.color.r),
                    Focus::Green => status = rgba_bar_handle(key, &mut self.state.color.g),
                    Focus::Blue => status = rgba_bar_handle(key, &mut self.state.color.b),
                    Focus::Alpha => status = rgba_bar_handle(key, &mut self.state.color.a),
                    _ => {}
                }
            }

            if matches!(status, event::Status::Captured)
                && !matches!(self.state.color, Color::BLACK | Color::WHITE)
            {
                // Black (0x000000) and White (0xFFFFFF) colors don't have hue, so we keep it as it
                // was on those cases
                let hue = Hsv::from(self.state.color).hue;
                self.state.current_hue_degrees = hue;
            }

            status
        } else if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            self.state.keyboard_modifiers = *modifiers;
            event::Status::Ignored
        } else {
            event::Status::Ignored
        }
    }

    fn update_mouse(
        &self,
        layout: GeometryLayout<'_>,
        cursor: Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let mut children = layout.children();

        let mouse_interaction = if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Idle
        } else {
            mouse::Interaction::None
        };

        if !matches!(self.state.color_bar_dragged, ColorBarDragged::None) {
            // If a bar is being dragged keep the mouse interaction the same as the one from the
            // dragged bar
            return match self.state.color_bar_dragged {
                ColorBarDragged::None => mouse::Interaction::default(),
                ColorBarDragged::SatValue | ColorBarDragged::Hue => mouse::Interaction::Pointer,
                ColorBarDragged::Red
                | ColorBarDragged::Green
                | ColorBarDragged::Blue
                | ColorBarDragged::Alpha => mouse::Interaction::ResizingHorizontally,
            };
        }

        // Block 1
        let block1_layout = children
            .next()
            .expect("Graphics: Layout should have a 1. block layout");
        let mut block1_mouse_interaction = mouse::Interaction::default();
        // HSV color
        let mut hsv_color_children = block1_layout.children();
        let sat_value_layout = hsv_color_children
            .next()
            .expect("Graphics: Layout should have a sat/value layout");
        if cursor.is_over(sat_value_layout.bounds()) {
            block1_mouse_interaction = block1_mouse_interaction.max(mouse::Interaction::Pointer);
        }
        let hue_layout = hsv_color_children
            .next()
            .expect("Graphics: Layout should have a hue layout");
        if cursor.is_over(hue_layout.bounds()) {
            block1_mouse_interaction = block1_mouse_interaction.max(mouse::Interaction::Pointer);
        }

        // Block 2
        let block2_layout = children
            .next()
            .expect("Graphics: Layout should have a 2. block layout");
        let mut block2_mouse_interaction = mouse::Interaction::default();
        let mut block2_children = block2_layout.children();
        // RGBA color
        let rgba_color_layout = block2_children
            .next()
            .expect("Graphics: Layout should have a RGBA color layout");
        let mut rgba_color_children = rgba_color_layout.children();

        let f = |layout: GeometryLayout<'_>, cursor: Cursor| {
            let mut children = layout.children();

            let _label_layout = children.next();
            let bar_layout = children
                .next()
                .expect("Graphics: Layout should have a bar layout");

            if cursor.is_over(bar_layout.bounds()) {
                mouse::Interaction::ResizingHorizontally
            } else {
                mouse::Interaction::default()
            }
        };
        let red_row_layout = rgba_color_children
            .next()
            .expect("Graphics: Layout should have a red row layout");
        block2_mouse_interaction = block2_mouse_interaction.max(f(red_row_layout, cursor));
        let green_row_layout = rgba_color_children
            .next()
            .expect("Graphics: Layout should have a green row layout");
        block2_mouse_interaction = block2_mouse_interaction.max(f(green_row_layout, cursor));
        let blue_row_layout = rgba_color_children
            .next()
            .expect("Graphics: Layout should have a blue row layout");
        block2_mouse_interaction = block2_mouse_interaction.max(f(blue_row_layout, cursor));
        let alpha_row_layout = rgba_color_children
            .next()
            .expect("Graphics: Layout should have an alpha row layout");
        block2_mouse_interaction = block2_mouse_interaction.max(f(alpha_row_layout, cursor));

        let hex_input_layout = block2_children
            .next()
            .expect("Graphics: Layout should have an hex input layout for a ColorPicker");
        let hex_input_interaction = self.hex_input.mouse_interaction(
            &self.tree.children[3],
            hex_input_layout.for_widget(&self.tree.children[3]),
            cursor,
            viewport,
            renderer,
        );

        // Buttons
        let cancel_button_layout = block2_children
            .next()
            .expect("Graphics: Layout should have a cancel button layout for a ColorPicker");
        let cancel_mouse_interaction = self.cancel_button.mouse_interaction(
            &self.tree.children[0],
            cancel_button_layout.for_widget(&self.tree.children[0]),
            cursor,
            viewport,
            renderer,
        );

        let submit_button_layout = block2_children
            .next()
            .expect("Graphics: Layout should have a submit button layout for a ColorPicker");
        let submit_mouse_interaction = self.submit_button.mouse_interaction(
            &self.tree.children[1],
            submit_button_layout.for_widget(&self.tree.children[1]),
            cursor,
            viewport,
            renderer,
        );

        mouse_interaction
            .max(block1_mouse_interaction)
            .max(block2_mouse_interaction)
            .max(hex_input_interaction)
            .max(cancel_mouse_interaction)
            .max(submit_mouse_interaction)
    }
}

impl<'a, 'b, Message, Theme> Overlay<Message, Theme, Renderer>
    for ColorPickerOverlay<'a, 'b, Message, Theme>
where
    Message: 'b + Clone + std::fmt::Debug,
    Theme: 'b
        + Catalog
        + iced::widget::button::Catalog
        + iced::widget::text::Catalog
        + container::Catalog
        + text_input::Catalog
        + Clone,
    'b: 'a,
{
    fn operate(&mut self, renderer: &Renderer, operation: &mut dyn Operation) {
        let viewport = Rectangle::with_size(self.window);
        operation.container(None, self.layout.bounds(), &viewport);
        operation.traverse(&mut |operation| {
            let hex_layout = self.hex_input_layout();
            self.hex_input.operate(
                &mut self.tree.children[3],
                hex_layout,
                &viewport,
                renderer,
                operation,
            );
            let controls = self.layout.children().nth(1).expect("picker controls");
            let cancel = controls.children().nth(2).expect("cancel button");
            let submit = controls.children().nth(3).expect("submit button");
            // String implements operation::Text with no-op selection in this iced revision.
            operation.text(None, cancel.bounds(), &mut self.appearance.cancel_label);
            let cancel_layout = cancel.for_widget(&self.tree.children[0]);
            self.cancel_button.operate(
                &mut self.tree.children[0],
                cancel_layout,
                &viewport,
                renderer,
                operation,
            );
            operation.text(None, submit.bounds(), &mut self.appearance.submit_label);
            let submit_layout = submit.for_widget(&self.tree.children[1]);
            self.submit_button.operate(
                &mut self.tree.children[1],
                submit_layout,
                &viewport,
                renderer,
                operation,
            );
        });
    }

    fn update(
        &mut self,
        event: &Event,
        cursor: Cursor,
        renderer: &Renderer,
        shell: &mut Shell<Message>,
    ) {
        // The winit runtime moves the cursor to the finger, but other callers (such as headless
        // simulators) may not, so touch hit tests use the finger position directly.
        let cursor = match event {
            Event::Touch(
                touch::Event::FingerPressed { position, .. }
                | touch::Event::FingerMoved { position, .. }
                | touch::Event::FingerLifted { position, .. }
                | touch::Event::FingerLost { position, .. },
            ) => Cursor::Available(*position),
            _ => cursor,
        };
        // Hover styles are resolved from the cursor while drawing, and a plain move between
        // parts changes nothing else that would request a frame.
        if Hover::at(self.layout, cursor) != self.state.drawn_hover.get() {
            shell.request_redraw();
        }
        let layout = self.layout;
        if self.on_confirm_key(event, renderer, shell) {
            return;
        }

        let color_before = self.state.color;
        if event::Status::Captured == self.on_event_keyboard(event) {
            if self.state.color != color_before {
                self.sync_hex_text();
            }
            self.state.clear();
            shell.capture_event();
            shell.request_redraw();
            return;
        }

        let mut children = layout.children();

        // ----------- Block 1 ----------------------
        let block1_layout = children
            .next()
            .expect("widget: Layout should have a 1. block layout");
        self.update_hsv_color(event, block1_layout, cursor, shell);
        // ----------- Block 1 end ------------------

        // ----------- Block 2 ----------------------
        let mut block2_children = children
            .next()
            .expect("widget: Layout should have a 2. block layout")
            .children();

        let rgba_color_layout = block2_children
            .next()
            .expect("widget: Layout should have a RGBA color layout");
        let hex_input_layout = block2_children
            .next()
            .expect("widget: Layout should have a hex text layout");
        // ----------- RGB Color -----------------------
        self.update_rgba_color(event, rgba_color_layout, hex_input_layout, cursor, shell);

        // ----------- Hex input ----------------------
        let hex_input_layout = hex_input_layout.for_widget(&self.tree.children[3]);
        let was_editing_hex = self.is_hex_input_focused(hex_input_layout, renderer);
        let mut fake_input_messages = shell::Bus::new();
        let mut fake_input_shell = shell.local(&mut fake_input_messages);

        // if event was already captured, mark this fake shell with a captured_event as well
        if shell.event_status() == event::Status::Captured {
            fake_input_shell.capture_event();
        }

        self.hex_input.update(
            &mut self.tree.children[3],
            event,
            hex_input_layout,
            cursor,
            renderer,
            &mut fake_input_shell,
            &layout.bounds(),
        );

        // Reconcile redraws and captures
        match fake_input_shell.redraw_request() {
            iced::window::RedrawRequest::NextFrame => shell.request_redraw(),
            iced::window::RedrawRequest::At(instant) => shell.request_redraw_at(instant),
            iced::window::RedrawRequest::Wait => {}
        }
        if fake_input_shell.event_status() == event::Status::Captured {
            shell.capture_event();
        }
        drop(fake_input_shell);

        let mut hex_edited = false;
        for InternalMessage::ChangeInput(text) in fake_input_messages {
            if let Some(color) = parse_hex(&text) {
                self.state.color = color;
                if !matches!(self.state.color, Color::BLACK | Color::WHITE) {
                    // Black (0x000000) and White (0xFFFFFF) colors don't have hue, so we keep it as it
                    // was on those cases
                    let hue = Hsv::from(self.state.color).hue;
                    self.state.current_hue_degrees = hue;
                }
            }
            self.state.hex_text = text;
            hex_edited = true;
            shell.capture_event();
            shell.request_redraw();
        }
        let hex_edit_ended =
            was_editing_hex && !self.is_hex_input_focused(hex_input_layout, renderer);
        // ----------- Buttons -------------------------
        let cancel_button_layout = block2_children
            .next()
            .expect("widget: Layout should have a cancel button layout for a ColorPicker")
            .for_widget(&self.tree.children[0]);

        let mut cancel_messages = shell::Bus::new();
        let mut cancel_shell = shell.local(&mut cancel_messages);
        if shell.is_event_captured() {
            cancel_shell.capture_event();
        }
        self.cancel_button.update(
            &mut self.tree.children[0],
            event,
            cancel_button_layout,
            cursor,
            renderer,
            &mut cancel_shell,
            &layout.bounds(),
        );
        shell.merge(cancel_shell, |action| match action {
            ButtonAction::Content(message) => message,
            ButtonAction::Press => self.on_cancel.clone(),
        });

        let mut fake_messages = shell::Bus::new();
        let mut fake_submit_shell = shell.local(&mut fake_messages);

        // if event was already captured, mark this fake shell with a captured_event as well
        if shell.event_status() == event::Status::Captured {
            fake_submit_shell.capture_event();
        }

        let submit_button_layout = block2_children
            .next()
            .expect("widget: Layout should have a submit button layout for a ColorPicker")
            .for_widget(&self.tree.children[1]);
        self.submit_button.update(
            &mut self.tree.children[1],
            event,
            submit_button_layout,
            cursor,
            renderer,
            &mut fake_submit_shell,
            &layout.bounds(),
        );

        // Content messages must not be mistaken for a submit button press.
        let submitted = Cell::new(false);
        shell.merge(fake_submit_shell, |action| match action {
            ButtonAction::Content(message) => message,
            ButtonAction::Press => {
                submitted.set(true);
                (self.on_submit)(self.state.color)
            }
        });
        if submitted.get() {
            self.state.previous_hue_degrees = self.state.current_hue_degrees;
            shell.capture_event();
            shell.request_redraw();
        }
        // ----------- Block 2 end ------------------

        // Rewriting the hex text on every parseable keystroke would mix the formatted color
        // into what the user is typing, so it is only re-synced once the edit ends or when
        // another control changed the color.
        if hex_edit_ended || (!hex_edited && self.state.color != color_before) {
            self.sync_hex_text();
        } else if hex_edited {
            // The input already shows the typed text. Its value must match, or the next
            // layout would overwrite the edit.
            self.rebuild_hex_input();
        }

        if shell.is_event_captured() {
            self.state.clear();
        } else if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                | Event::Touch(touch::Event::FingerLifted { .. })
                | Event::Touch(touch::Event::FingerLost { .. })
                | Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                | Event::Touch(touch::Event::FingerPressed { .. })
        ) && !cursor.is_over(layout.bounds())
        {
            // Clicked outside of bounds so lets send the `on_cancel` message to close the picker
            shell.publish(self.on_cancel.clone());
            shell.capture_event();
            self.state.clear();
        }
    }

    fn mouse_interaction(&self, cursor: Cursor, renderer: &Renderer) -> mouse::Interaction {
        self.update_mouse(
            self.layout,
            cursor,
            &Rectangle::with_size(self.window),
            renderer,
        )
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        cursor: Cursor,
    ) {
        let layout = self.layout;
        self.state.drawn_hover.set(Hover::at(layout, cursor));
        renderer.with_layer(Rectangle::with_size(self.window), |renderer| {
            let bounds = layout.bounds();
            let mut children = layout.children();

            let mut style_sheet: HashMap<StyleState, Style> = HashMap::new();
            let _ = style_sheet.insert(
                StyleState::Active,
                Catalog::style(theme, self.class, Status::Active),
            );
            let _ = style_sheet.insert(
                StyleState::Selected,
                Catalog::style(theme, self.class, Status::Selected),
            );
            let _ = style_sheet.insert(
                StyleState::Hovered,
                Catalog::style(theme, self.class, Status::Hovered),
            );
            let _ = style_sheet.insert(
                StyleState::Focused,
                Catalog::style(theme, self.class, Status::Focused),
            );

            let mut style_state = StyleState::Active;
            if self.state.focus == Focus::Overlay {
                style_state = style_state.max(StyleState::Focused);
            }
            if cursor.is_over(bounds) {
                style_state = style_state.max(StyleState::Hovered);
            }

            if (bounds.width > 0.) && (bounds.height > 0.) {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: style_sheet[&style_state].border,
                        shadow: style_sheet[&style_state].shadow,
                        snap: true,
                    },
                    style_sheet[&style_state].background,
                );
            }

            // ----------- Block 1 ----------------------
            let block1_layout = children
                .next()
                .expect("Graphics: Layout should have a 1. block layout");
            block1(renderer, self, block1_layout, cursor, &style_sheet);

            // ----------- Block 2 ----------------------
            let block2_layout = children
                .next()
                .expect("Graphics: Layout should have a 2. block layout");
            block2(
                renderer,
                self,
                block2_layout,
                cursor,
                theme,
                style,
                &bounds,
                &style_sheet,
            );
        });
    }
}

fn rgba_elements<'a, Message, Theme>(
    renderer: &Renderer,
    appearance: &Appearance<'_, Theme>,
) -> Column<Element<'a, Message, Theme, Renderer>>
where
    Message: 'a,
    Theme: text::Catalog + 'a,
{
    let default_labels = ["R:", "G:", "B:", "A:"];
    let custom_labels = appearance
        .channel_labels
        .iter()
        .map(String::as_str)
        .ne(default_labels);
    let font = appearance.font.unwrap_or_else(|| {
        if custom_labels || appearance.text_size.is_some() {
            Font::default()
        } else {
            renderer.font()
        }
    });
    let size = appearance.text_size.unwrap_or_else(|| renderer.text_size());
    let label_width =
        if custom_labels || appearance.font.is_some() || appearance.text_size.is_some() {
            appearance
                .channel_labels
                .iter()
                .map(|label| {
                    layout_geometry::<(), Theme>(
                        text::<Theme>(label.clone())
                            .font(font)
                            .size(size)
                            .shaping(text::Shaping::Advanced),
                        renderer,
                        &Limits::new(Size::ZERO, Size::INFINITE),
                    )
                    .size
                    .width
                })
                .fold(0.0, f32::max)
        } else {
            // Preserve the original stand-in's spacing when no text options are set.
            layout_geometry::<(), Theme>(
                text::<Theme>("X: "),
                renderer,
                &Limits::new(Size::ZERO, Size::INFINITE),
            )
            .size
            .width
        };
    let mut rgba_colors = Column::new();
    for label in &appearance.channel_labels {
        rgba_colors = rgba_colors.push(
            row![
                text(label.clone())
                    .font(font)
                    .size(size)
                    .width(label_width)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center),
                Row::<Element<'a, Message, Theme, Renderer>>::new()
                    .width(Length::FillPortion(5))
                    .height(Length::Fill),
                text("XXX")
                    .font(appearance.font.unwrap_or_else(|| renderer.font()))
                    .size(size)
                    .align_x(Horizontal::Center)
                    .align_y(Vertical::Center),
            ]
            .align_y(Alignment::Center)
            .spacing(SPACING)
            .padding(PADDING)
            .height(Length::Fill)
            .boxed(),
        );
    }
    rgba_colors
}

/// Defines the layout of the 1. block of the color picker containing the HSV part.
fn block1_layout<Theme>(renderer: &Renderer, bounds: Rectangle) -> Tree {
    let block1_limits = Limits::new(Size::ZERO, bounds.size())
        .width(Length::Fill)
        .height(Length::Fill);

    let mut block1_node = layout_geometry(
        Column::new()
            .spacing(PADDING.y() / 2.)
            .push(
                Row::<Element<'_, (), Theme, Renderer>>::new()
                    .width(Length::Fill)
                    .height(Length::FillPortion(7)),
            )
            .push(
                Row::<Element<'_, (), Theme, Renderer>>::new()
                    .width(Length::Fill)
                    .height(Length::FillPortion(1)),
            ),
        renderer,
        &block1_limits,
    );

    block1_node.translation = Vector::new(bounds.x + PADDING.left, bounds.y + PADDING.top);
    block1_node
}

/// Defines the layout of the 2. block of the color picker containing the RGBA part, Hex and buttons.
fn block2_layout<'a, 'b: 'a, Message, Theme>(
    color_picker: &mut ColorPickerLayout<'a, 'b, Message, Theme>,
    renderer: &Renderer,
    bounds: Rectangle,
) -> Tree
where
    Message: 'b + Clone,
    Theme: 'b
        + Catalog
        + iced::widget::button::Catalog
        + iced::widget::text::Catalog
        + text_input::Catalog
        + Clone,
{
    let block2_limits = Limits::new(Size::ZERO, bounds.size())
        .width(Length::Fill)
        .height(Length::Fill);

    // Measure buttons at their final width so wrapped content reserves enough height.
    let button_limits =
        block2_limits.width(Length::Fill.max(((bounds.width / 2.0) - BUTTON_SPACING.0).max(0.0)));
    color_picker
        .cancel_button
        .layout(&mut color_picker.tree.children[0], renderer, &button_limits);
    color_picker
        .submit_button
        .layout(&mut color_picker.tree.children[1], renderer, &button_limits);
    let button_height = color_picker.tree.children[0]
        .size
        .height
        .max(color_picker.tree.children[1].size.height);
    color_picker
        .hex_input
        .layout(&mut color_picker.tree.children[3], renderer, &block2_limits);
    let mut hex_text_layout = Tree {
        size: color_picker.tree.children[3].size,
        ..Tree::empty()
    };

    let block2_limits = block2_limits.shrink(Size::new(
        0.0,
        button_height + hex_text_layout.size.height + 2.0 * SPACING.0,
    ));

    let mut rgba_elements = rgba_elements::<Message, Theme>(renderer, color_picker.appearance);
    color_picker.rgba_tree.diff(&mut rgba_elements);
    rgba_elements.layout(color_picker.rgba_tree, renderer, &block2_limits);
    let mut rgba_colors = geometry_tree(color_picker.rgba_tree);
    rgba_colors.translation = Vector::new(PADDING.left, PADDING.top);
    let rgba_size = rgba_colors.size;

    hex_text_layout.translation =
        Vector::new(PADDING.left, rgba_size.height + PADDING.top + SPACING.0);
    let hex_size = hex_text_layout.size;

    let button_limits = Limits::new(Size::ZERO, bounds.size())
        .width(Length::Fill.max(((rgba_size.width / 2.0) - BUTTON_SPACING.0).max(0.0)));
    color_picker
        .cancel_button
        .layout(&mut color_picker.tree.children[0], renderer, &button_limits);
    color_picker
        .submit_button
        .layout(&mut color_picker.tree.children[1], renderer, &button_limits);

    let buttons_y = rgba_size.height + hex_size.height + PADDING.top + 2.0 * SPACING.0;
    let cancel_button = Tree {
        size: color_picker.tree.children[0].size,
        translation: Vector::new(PADDING.left, buttons_y),
        ..Tree::empty()
    };
    let submit_button = Tree {
        size: color_picker.tree.children[1].size,
        translation: Vector::new(
            rgba_size.width - color_picker.tree.children[1].size.width + PADDING.left,
            buttons_y,
        ),
        ..Tree::empty()
    };

    Tree {
        size: Size::new(
            rgba_size.width + PADDING.x(),
            rgba_size.height
                + hex_size.height
                + cancel_button.size.height.max(submit_button.size.height)
                + PADDING.y()
                + (2.0 * SPACING.0),
        ),
        translation: Vector::new(bounds.x, bounds.y),
        children: vec![rgba_colors, hex_text_layout, cancel_button, submit_button],
        ..Tree::empty()
    }
}

/// Draws the 1. block of the color picker containing the HSV part.
fn block1<Message, Theme>(
    renderer: &mut Renderer,
    color_picker: &ColorPickerOverlay<'_, '_, Message, Theme>,
    layout: GeometryLayout<'_>,
    cursor: Cursor,
    style_sheet: &HashMap<StyleState, Style>,
) where
    Message: Clone,
    Theme: Catalog
        + iced::widget::button::Catalog
        + iced::widget::text::Catalog
        + text_input::Catalog
        + Clone,
{
    // ----------- Block 1 ----------------------
    let hsv_color_layout = layout;

    // ----------- HSV Color ----------------------
    //let hsv_color_layout = block1_children.next().unwrap();
    hsv_color(
        renderer,
        color_picker,
        hsv_color_layout,
        cursor,
        style_sheet,
    );

    // ----------- Block 1 end ------------------
}

/// Draws the 2. block of the color picker containing the RGBA part, Hex and buttons.
#[allow(clippy::too_many_arguments)]
fn block2<'b, Message, Theme>(
    renderer: &mut Renderer,
    color_picker: &ColorPickerOverlay<'_, 'b, Message, Theme>,
    layout: GeometryLayout<'_>,
    cursor: Cursor,
    theme: &Theme,
    style: &renderer::Style,
    viewport: &Rectangle,
    style_sheet: &HashMap<StyleState, Style>,
) where
    Message: Clone + 'b,
    Theme: 'b
        + Catalog
        + iced::widget::button::Catalog
        + iced::widget::text::Catalog
        + text_input::Catalog
        + Clone,
{
    // ----------- Block 2 ----------------------
    let mut block2_children = layout.children();

    // ----------- RGBA Color ----------------------
    let rgba_color_layout = block2_children
        .next()
        .expect("Graphics: Layout should have a RGBA color layout");
    rgba_color(
        renderer,
        rgba_color_layout,
        &color_picker.state.color,
        cursor,
        color_picker.appearance,
        style_sheet,
        color_picker.state.focus,
    );

    // ----------- Hex text ----------------------
    let hex_text_layout = block2_children
        .next()
        .expect("Graphics: Layout should have a hex text layout");
    let hex_input_theme = InternalTheme {
        theme: theme.clone(),
        color: color_picker.state.color,
        is_valid: parse_hex(&color_picker.state.hex_text).is_some(),
        invalid_hex: style_sheet[&StyleState::Active].invalid_hex,
    };
    color_picker.hex_input.draw(
        &color_picker.tree.children[3],
        renderer,
        &hex_input_theme,
        style,
        hex_text_layout.for_widget(&color_picker.tree.children[3]),
        cursor,
        viewport,
    );

    // ----------- Buttons -------------------------
    let cancel_button_layout = block2_children
        .next()
        .expect("Graphics: Layout should have a cancel button layout for a ColorPicker");

    color_picker.cancel_button.draw(
        &color_picker.tree.children[0],
        renderer,
        &ButtonTheme {
            theme: theme.clone(),
            focused: color_picker.state.focus == Focus::Cancel,
        },
        style,
        cancel_button_layout.for_widget(&color_picker.tree.children[0]),
        cursor,
        viewport,
    );

    let submit_button_layout = block2_children
        .next()
        .expect("Graphics: Layout should have a submit button layout for a ColorPicker");

    color_picker.submit_button.draw(
        &color_picker.tree.children[1],
        renderer,
        &ButtonTheme {
            theme: theme.clone(),
            focused: color_picker.state.focus == Focus::Submit,
        },
        style,
        submit_button_layout.for_widget(&color_picker.tree.children[1]),
        cursor,
        viewport,
    );

    // Buttons are not focusable right now...
    if color_picker.state.focus == Focus::Cancel {
        let bounds = cancel_button_layout.bounds();
        if (bounds.width > 0.) && (bounds.height > 0.) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: style_sheet[&StyleState::Focused].focus_border,
                    shadow: Shadow::default(),
                    snap: true,
                },
                Color::TRANSPARENT,
            );
        }
    }

    if color_picker.state.focus == Focus::Submit {
        let bounds = submit_button_layout.bounds();
        if (bounds.width > 0.) && (bounds.height > 0.) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: style_sheet[&StyleState::Focused].focus_border,
                    shadow: Shadow::default(),
                    snap: true,
                },
                Color::TRANSPARENT,
            );
        }
    }
    // ----------- Block 2 end ------------------
}

/// Draws the HSV color area.
#[allow(clippy::too_many_lines)]
fn hsv_color<Message, Theme>(
    renderer: &mut Renderer,
    color_picker: &ColorPickerOverlay<'_, '_, Message, Theme>,
    layout: GeometryLayout<'_>,
    cursor: Cursor,
    style_sheet: &HashMap<StyleState, Style>,
) where
    Message: Clone,
    Theme: Catalog
        + iced::widget::button::Catalog
        + iced::widget::text::Catalog
        + text_input::Catalog
        + Clone,
{
    let mut hsv_color_children = layout.children();
    let hsv_color: Hsv = color_picker.state.color.into();

    let sat_value_layout = hsv_color_children
        .next()
        .expect("Graphics: Layout should have a sat/value layout");
    let mut sat_value_style_state = StyleState::Active;
    if color_picker.state.focus == Focus::SatValue {
        sat_value_style_state = sat_value_style_state.max(StyleState::Focused);
    }
    if cursor.is_over(sat_value_layout.bounds()) {
        sat_value_style_state = sat_value_style_state.max(StyleState::Hovered);
    }

    let sat_style = style_sheet[&sat_value_style_state];
    if color_picker.state.sat_value_style.replace(Some(sat_style)) != Some(sat_style) {
        color_picker.state.sat_value_canvas_cache.clear();
    }
    let geometry = color_picker.state.sat_value_canvas_cache.draw(
        renderer,
        sat_value_layout.bounds().size(),
        |frame| {
            let column_count = frame.width() as u16;
            let row_count = frame.height() as u16;

            for column in 0..column_count {
                for row in 0..row_count {
                    let saturation = f32::from(column) / frame.width();
                    let value = f32::from(row) / frame.height();

                    frame.fill_rectangle(
                        Point::new(f32::from(column), f32::from(row)),
                        Size::new(1.0, 1.0),
                        Color::from(Hsv::from_hsv(
                            color_picker.state.current_hue_degrees,
                            saturation,
                            value,
                        )),
                    );
                }
            }

            let stroke = Stroke {
                style: canvas::Style::Solid(sat_style.sat_value_marker.unwrap_or_else(|| {
                    Hsv {
                        hue: 0,
                        saturation: 0.0,
                        value: 1.0 - hsv_color.value,
                    }
                    .into()
                })),
                width: MARKER_STROKE_WIDTH,
                line_cap: LineCap::Round,
                ..Stroke::default()
            };

            let saturation = hsv_color.saturation * frame.width();
            let value = hsv_color.value * frame.height();

            frame.stroke(
                &Path::line(
                    Point::new(saturation, 0.0),
                    Point::new(saturation, frame.height()),
                ),
                stroke,
            );

            frame.stroke(
                &Path::line(Point::new(0.0, value), Point::new(frame.width(), value)),
                stroke,
            );

            let stroke = Stroke {
                style: canvas::Style::Solid(sat_style.area_border.color),
                width: sat_style.area_border.width,
                line_cap: LineCap::Round,
                ..Stroke::default()
            };

            frame.stroke(&area_outline(frame.size(), sat_style.area_border), stroke);
        },
    );

    let translation = Vector::new(sat_value_layout.bounds().x, sat_value_layout.bounds().y);
    renderer.with_translation(translation, |renderer| {
        renderer.draw_geometry(geometry);
    });

    let hue_layout = hsv_color_children
        .next()
        .expect("Graphics: Layout should have a hue layout");
    let mut hue_style_state = StyleState::Active;
    if color_picker.state.focus == Focus::Hue {
        hue_style_state = hue_style_state.max(StyleState::Focused);
    }
    if cursor.is_over(hue_layout.bounds()) {
        hue_style_state = hue_style_state.max(StyleState::Hovered);
    }

    let hue_style = style_sheet[&hue_style_state];
    if color_picker.state.hue_style.replace(Some(hue_style)) != Some(hue_style) {
        color_picker.state.hue_canvas_cache.clear();
    }
    let geometry =
        color_picker
            .state
            .hue_canvas_cache
            .draw(renderer, hue_layout.bounds().size(), |frame| {
                let column_count = frame.width() as u16;

                for column in 0..column_count {
                    let hue = (f32::from(column) * 360.0 / frame.width()) as u16;

                    let hsv_color = Hsv::from_hsv(hue, 1.0, 1.0);
                    let stroke = Stroke {
                        style: canvas::Style::Solid(hsv_color.into()),
                        width: 1.0,
                        line_cap: LineCap::Round,
                        ..Stroke::default()
                    };

                    frame.stroke(
                        &Path::line(
                            Point::new(f32::from(column), 0.0),
                            Point::new(f32::from(column), frame.height()),
                        ),
                        stroke,
                    );
                }

                let stroke = Stroke {
                    style: canvas::Style::Solid(hue_style.hue_marker),
                    width: MARKER_STROKE_WIDTH,
                    line_cap: LineCap::Round,
                    ..Stroke::default()
                };

                let column =
                    f32::from(color_picker.state.current_hue_degrees) * frame.width() / 360.0;

                frame.stroke(
                    &Path::line(Point::new(column, 0.0), Point::new(column, frame.height())),
                    stroke,
                );

                let stroke = Stroke {
                    style: canvas::Style::Solid(hue_style.area_border.color),
                    width: hue_style.area_border.width,
                    line_cap: LineCap::Round,
                    ..Stroke::default()
                };

                frame.stroke(&area_outline(frame.size(), hue_style.area_border), stroke);
            });

    let translation = Vector::new(hue_layout.bounds().x, hue_layout.bounds().y);
    renderer.with_translation(translation, |renderer| {
        renderer.draw_geometry(geometry);
    });
}

fn area_outline(size: Size, border: Border) -> Path {
    if border.radius == iced::border::Radius::default() {
        Path::rectangle(Point::ORIGIN, size)
    } else {
        Path::rounded_rectangle(Point::ORIGIN, size, border.radius)
    }
}

/// Draws the RGBA color area.
#[allow(clippy::too_many_lines)]
fn rgba_color<Theme>(
    renderer: &mut Renderer,
    layout: GeometryLayout<'_>,
    color: &Color,
    cursor: Cursor,
    appearance: &Appearance<'_, Theme>,
    style_sheet: &HashMap<StyleState, Style>,
    focus: Focus,
) {
    let mut rgba_color_children = layout.children();

    let f = |renderer: &mut Renderer,
             layout: GeometryLayout<'_>,
             label: &str,
             color: Color,
             value: f32,
             cursor: Cursor,
             target: Focus| {
        let mut children = layout.children();

        let label_layout = children
            .next()
            .expect("Graphics: Layout should have a label layout");
        let bar_layout = children
            .next()
            .expect("Graphics: Layout should have a bar layout");
        let value_layout = children
            .next()
            .expect("Graphics: Layout should have a value layout");

        let bar_bounds = bar_layout.bounds();
        let bar_style_state = if cursor.is_over(bar_bounds) {
            StyleState::Hovered
        } else {
            StyleState::Active
        };
        let part_style = style_sheet[&bar_style_state];

        // Label
        renderer.fill_text(
            Text {
                content: label.to_owned(),
                bounds: Size::new(label_layout.bounds().width, label_layout.bounds().height),
                size: appearance.text_size.unwrap_or_else(|| renderer.text_size()),
                font: appearance.font.unwrap_or_default(),
                align_x: Horizontal::Center.into(),
                align_y: Vertical::Center,
                line_height: text::LineHeight::Relative(1.3),
                shaping: text::Shaping::Advanced,
                wrapping: Wrapping::default(),
                ellipsis: text::Ellipsis::None,
                hint_factor: renderer.hint_factor(),
            },
            Point::new(
                label_layout.bounds().center_x(),
                label_layout.bounds().center_y(),
            ),
            part_style.text_color,
            label_layout.bounds(),
        );

        if bar_bounds.width > 0.0 && bar_bounds.height > 0.0 {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: bar_bounds,
                    border: Border {
                        radius: part_style.bar_border.radius,
                        ..Border::default()
                    },
                    ..renderer::Quad::default()
                },
                part_style.bar_background,
            );
        }

        // Bar background
        let background_bounds = Rectangle {
            x: bar_bounds.x,
            y: bar_bounds.y,
            width: bar_bounds.width * value,
            height: bar_bounds.height,
        };
        if (background_bounds.width > 0.) && (background_bounds.height > 0.) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: background_bounds,
                    border: Border {
                        radius: part_style.bar_border.radius,
                        width: part_style.bar_border.width,
                        color: Color::TRANSPARENT,
                    },
                    shadow: Shadow::default(),
                    snap: true,
                },
                color,
            );
        }

        // Bar
        if (bar_bounds.width > 0.) && (bar_bounds.height > 0.) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: bar_bounds,
                    border: part_style.bar_border,
                    shadow: Shadow::default(),
                    snap: true,
                },
                Color::TRANSPARENT,
            );
        }

        // Value
        renderer.fill_text(
            Text {
                content: format!("{}", (255.0 * value) as u8),
                bounds: Size::new(value_layout.bounds().width, value_layout.bounds().height),
                size: appearance.text_size.unwrap_or_else(|| renderer.text_size()),
                font: appearance.font.unwrap_or_else(|| renderer.font()),
                align_x: Horizontal::Center.into(),
                align_y: Vertical::Center,
                line_height: iced::widget::text::LineHeight::Relative(1.3),
                shaping: iced::widget::text::Shaping::Advanced,
                wrapping: Wrapping::default(),
                ellipsis: text::Ellipsis::None,
                hint_factor: renderer.hint_factor(),
            },
            Point::new(
                value_layout.bounds().center_x(),
                value_layout.bounds().center_y(),
            ),
            part_style.text_color,
            value_layout.bounds(),
        );

        let bounds = layout.bounds();
        if (focus == target) && (bounds.width > 0.) && (bounds.height > 0.) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: style_sheet[&StyleState::Focused].focus_border,
                    shadow: Shadow::default(),
                    snap: true,
                },
                Color::TRANSPARENT,
            );
        }
    };

    // Red
    let red_row_layout = rgba_color_children
        .next()
        .expect("Graphics: Layout should have a red row layout");

    f(
        renderer,
        red_row_layout,
        &appearance.channel_labels[0],
        Color::from_rgb(color.r, 0.0, 0.0),
        color.r,
        cursor,
        Focus::Red,
    );

    // Green
    let green_row_layout = rgba_color_children
        .next()
        .expect("Graphics: Layout should have a green row layout");

    f(
        renderer,
        green_row_layout,
        &appearance.channel_labels[1],
        Color::from_rgb(0.0, color.g, 0.0),
        color.g,
        cursor,
        Focus::Green,
    );

    // Blue
    let blue_row_layout = rgba_color_children
        .next()
        .expect("Graphics: Layout should have a blue row layout");

    f(
        renderer,
        blue_row_layout,
        &appearance.channel_labels[2],
        Color::from_rgb(0.0, 0.0, color.b),
        color.b,
        cursor,
        Focus::Blue,
    );

    // Alpha
    let alpha_row_layout = rgba_color_children
        .next()
        .expect("Graphics: Layout should have an alpha row layout");

    f(
        renderer,
        alpha_row_layout,
        &appearance.channel_labels[3],
        Color::from_rgba(0.0, 0.0, 0.0, color.a),
        color.a,
        cursor,
        Focus::Alpha,
    );
}

/// The state of the [`ColorPickerOverlay`].
#[derive(Debug)]
pub struct OverlayState {
    /// The selected color of the [`ColorPickerOverlay`].
    pub(crate) color: Color,
    /// The cache of the sat/value canvas of the [`ColorPickerOverlay`].
    pub(crate) sat_value_canvas_cache: canvas::Cache,
    /// The cache of the hue canvas of the [`ColorPickerOverlay`].
    pub(crate) hue_canvas_cache: canvas::Cache,
    /// The previous hue degrees of the [`ColorPickerOverlay`].
    pub(crate) previous_hue_degrees: u16,
    /// The current hue degrees of the [`ColorPickerOverlay`].
    pub(crate) current_hue_degrees: u16,
    /// The dragged color bar of the [`ColorPickerOverlay`].
    pub(crate) color_bar_dragged: ColorBarDragged,
    /// the focus of the [`ColorPickerOverlay`].
    pub(crate) focus: Focus,
    /// The previously pressed keyboard modifiers.
    pub(crate) keyboard_modifiers: keyboard::Modifiers,
    /// Whether the cached overlay layout must be recomputed before the next use.
    pub(crate) layout_stale: bool,
    /// The text of the hex input, kept as typed while it is being edited.
    hex_text: String,
    /// The last color given to the [`ColorPicker`] by the application.
    external_color: Color,
    rgba_tree: Tree,
    sat_value_style: Cell<Option<Style>>,
    hue_style: Cell<Option<Style>>,
    /// The hover the last frame was drawn with. It outlives a close and reopen, and the
    /// layout can move under a still cursor, so it tracks what is on screen instead of the
    /// last cursor seen by `update`.
    drawn_hover: Cell<Hover>,
}

/// The parts of the [`ColorPickerOverlay`] whose style depends on the cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Hover {
    overlay: bool,
    area: HoveredArea,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum HoveredArea {
    #[default]
    None,
    SatValue,
    Hue,
    Red,
    Green,
    Blue,
    Alpha,
}

impl Hover {
    /// Mirrors the hover checks made while drawing the overlay.
    fn at(layout: GeometryLayout<'_>, cursor: Cursor) -> Self {
        let mut blocks = layout.children();
        let hsv_areas = blocks
            .next()
            .expect("Hover: Layout should have a 1. block layout")
            .children()
            .map(GeometryLayout::bounds);
        let bars = blocks
            .next()
            .expect("Hover: Layout should have a 2. block layout")
            .children()
            .next()
            .expect("Hover: Layout should have a RGBA color layout")
            .children()
            .map(|row| {
                row.children()
                    .nth(1)
                    .expect("Hover: Layout should have a bar layout")
                    .bounds()
            });
        let area = [
            HoveredArea::SatValue,
            HoveredArea::Hue,
            HoveredArea::Red,
            HoveredArea::Green,
            HoveredArea::Blue,
            HoveredArea::Alpha,
        ]
        .into_iter()
        .zip(hsv_areas.chain(bars))
        .find_map(|(area, bounds)| cursor.is_over(bounds).then_some(area))
        .unwrap_or_default();

        Self {
            overlay: cursor.is_over(layout.bounds()),
            area,
        }
    }
}

impl OverlayState {
    /// Creates a new State with the given color.
    #[must_use]
    pub fn new(color: Color) -> Self {
        let hue = Hsv::from(color).hue;
        Self {
            color,
            hex_text: color.as_hex_string(),
            external_color: color,
            previous_hue_degrees: hue,
            current_hue_degrees: hue,
            ..Self::default()
        }
    }

    /// Clears the state cache
    pub fn clear(&mut self) {
        self.sat_value_canvas_cache.clear();
        self.hue_canvas_cache.clear();
    }
}

impl Default for OverlayState {
    fn default() -> Self {
        let color = Color::from_rgb(0.5, 0.25, 0.25);
        Self {
            color,
            hex_text: color.as_hex_string(),
            external_color: color,
            sat_value_canvas_cache: canvas::Cache::default(),
            hue_canvas_cache: canvas::Cache::default(),
            previous_hue_degrees: 0,
            current_hue_degrees: 0,
            color_bar_dragged: ColorBarDragged::None,
            focus: Focus::default(),
            keyboard_modifiers: keyboard::Modifiers::default(),
            layout_stale: false,
            rgba_tree: Tree::empty(),
            sat_value_style: Cell::new(None),
            hue_style: Cell::new(None),
            drawn_hover: Cell::new(Hover::default()),
        }
    }
}

/// The state of the currently dragged area.
#[derive(Copy, Clone, Debug, Default)]
pub enum ColorBarDragged {
    /// No area is focussed.
    #[default]
    None,

    /// The saturation/value area is focussed.
    SatValue,

    /// The hue area is focussed.
    Hue,

    /// The red area is focussed.
    Red,

    /// The green area is focussed.
    Green,

    /// The blue area is focussed.
    Blue,

    /// The alpha area is focussed.
    Alpha,
}

/// An enumeration of all focusable element of the [`ColorPickerOverlay`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// Nothing is in focus.
    #[default]
    None,

    /// The overlay itself is in focus.
    Overlay,

    /// The saturation and value area is in focus.
    SatValue,

    /// The hue bar is in focus.
    Hue,

    /// The red bar is in focus.
    Red,

    /// The green bar is in focus.
    Green,

    /// The blue bar is in focus.
    Blue,

    /// The alpha bar is in focus.
    Alpha,

    /// The cancel button is in focus.
    Cancel,

    /// The submit button is in focus.
    Submit,
}

impl Focus {
    /// Gets the next focusable element.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Overlay => Self::SatValue,
            Self::SatValue => Self::Hue,
            Self::Hue => Self::Red,
            Self::Red => Self::Green,
            Self::Green => Self::Blue,
            Self::Blue => Self::Alpha,
            Self::Alpha => Self::Cancel,
            Self::Cancel => Self::Submit,
            Self::Submit | Self::None => Self::Overlay,
        }
    }

    /// Gets the previous focusable element.
    #[must_use]
    pub const fn previous(self) -> Self {
        match self {
            Self::None => Self::None,
            Self::Overlay => Self::Submit,
            Self::SatValue => Self::Overlay,
            Self::Hue => Self::SatValue,
            Self::Red => Self::Hue,
            Self::Green => Self::Red,
            Self::Blue => Self::Green,
            Self::Alpha => Self::Blue,
            Self::Cancel => Self::Alpha,
            Self::Submit => Self::Cancel,
        }
    }
}

/// The appearance of a [`ColorPicker`](crate::widget::ColorPicker).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// The overlay background.
    pub background: Background,
    /// The overlay border.
    pub border: Border,
    /// The overlay shadow (none by default).
    pub shadow: Shadow,
    /// The color of channel labels and values.
    pub text_color: Color,
    /// The saturation/value and hue outlines. Radius rounds the outline, not the gradient.
    pub area_border: Border,
    /// The RGBA bar outlines.
    pub bar_border: Border,
    /// The track behind each RGBA bar's filled portion.
    pub bar_background: Background,
    /// The ring drawn around a focused RGBA bar or button, read from the Focused style. The
    /// saturation/value area and hue bar show focus through `area_border` instead.
    pub focus_border: Border,
    /// The saturation/value crosshair color. None uses automatic contrast gray.
    pub sat_value_marker: Option<Color>,
    /// The hue marker color.
    pub hue_marker: Color,
    /// The default hex input's border color when its text is invalid.
    pub invalid_hex: Color,
}

impl Default for Style {
    fn default() -> Self {
        primary(&Theme::Light, Status::Active)
    }
}

/// The Catalog of a [`ColorPicker`](crate::widget::ColorPicker).
pub trait Catalog {
    ///Style for the trait to use.
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self, Style>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(primary)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

/// The primary theme of a [`ColorPicker`](crate::widget::color_picker::ColorPicker).
#[must_use]
pub fn primary(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();

    let border_color = if status == Status::Focused {
        palette.background.strong.color
    } else {
        palette.background.base.text
    };
    Style {
        background: palette.background.base.color.into(),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: 15.0.into(),
        },
        shadow: Shadow::default(),
        text_color: palette.background.base.text,
        area_border: Border {
            color: border_color,
            width: 2.0,
            radius: 0.0.into(),
        },
        bar_border: Border {
            color: border_color,
            width: 1.0,
            radius: 5.0.into(),
        },
        bar_background: Color::TRANSPARENT.into(),
        focus_border: Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 15.0.into(),
        },
        sat_value_marker: None,
        hue_marker: Color::BLACK,
        invalid_hex: palette.danger.base.color,
    }
}

/// Status Enum of an mouse Event.
///
/// The Status of a widget event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// can be pressed.
    Active,
    /// can be pressed and it is being hovered.
    Hovered,
    /// is being pressed.
    Pressed,
    /// cannot be pressed.
    Disabled,
    /// is focused.
    Focused,
    /// is Selected.
    Selected,
}

/// The style function of widget.
pub type StyleFn<'a, Theme, Style> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

/// The state of the style
#[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StyleState {
    /// Use the active style
    Active,
    /// Use the selected style
    Selected,
    /// Use the hovered style
    Hovered,
    /// Use the focused style
    Focused,
}

/// A color in the HSV color space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    /// HSV hue.
    pub(crate) hue: u16,
    /// HSV Saturation.
    pub(crate) saturation: f32,
    /// HSV value.
    pub(crate) value: f32,
}

impl Hsv {
    /// Creates a [`Hsv`] from its HSV components.
    #[must_use]
    pub const fn from_hsv(hue: u16, saturation: f32, value: f32) -> Self {
        Self {
            hue,
            saturation,
            value,
        }
    }
}

/// Creates a string of hexadecimal characters.
pub trait HexString {
    /// Turns self into a string of hexadecimal characters.
    fn as_hex_string(&self) -> String;
}

impl HexString for Color {
    fn as_hex_string(&self) -> String {
        format!(
            "#{:02X?}{:02X?}{:02X?}{:02X?}",
            (self.r * 255.0) as u8,
            (self.g * 255.0) as u8,
            (self.b * 255.0) as u8,
            (self.a * 255.0) as u8,
        )
    }
}

impl From<Color> for Hsv {
    // https://de.wikipedia.org/wiki/HSV-Farbraum#Umrechnung_RGB_in_HSV/HSL
    fn from(color: Color) -> Self {
        let max = color.r.max(color.g.max(color.b));
        let min = color.r.min(color.g.min(color.b));

        let hue = if (max - min).abs() < f32::EPSILON {
            0.0
        } else if (max - color.r).abs() < f32::EPSILON {
            60.0 * (0.0 + (color.g - color.b) / (max - min))
        } else if (max - color.g).abs() < f32::EPSILON {
            60.0 * (2.0 + (color.b - color.r) / (max - min))
        } else {
            60.0 * (4.0 + (color.r - color.g) / (max - min))
        };

        let hue = if hue < 0.0 { hue + 360.0 } else { hue } as u16 % 360;

        let saturation = if max == 0.0 { 0.0 } else { (max - min) / max };

        let value = max;

        Self {
            hue,
            saturation,
            value,
        }
    }
}

impl From<Hsv> for Color {
    fn from(hsv: Hsv) -> Self {
        // https://de.wikipedia.org/wiki/HSV-Farbraum#Umrechnung_HSV_in_RGB
        let h_i = (f32::from(hsv.hue) / 60.0).floor();
        let f = (f32::from(hsv.hue) / 60.0) - h_i;

        let p = hsv.value * (1.0 - hsv.saturation);
        let q = hsv.value * (1.0 - hsv.saturation * f);
        let t = hsv.value * (1.0 - hsv.saturation * (1.0 - f));

        let h_i = h_i as u8;
        let (red, green, blue) = match h_i {
            1 => (q, hsv.value, p),
            2 => (p, hsv.value, t),
            3 => (p, q, hsv.value),
            4 => (t, p, hsv.value),
            5 => (hsv.value, p, q),
            _ => (hsv.value, t, p),
        };

        Self::from_rgb(red, green, blue)
    }
}

/// Trait containing functions for positioning of nodes.
pub trait Position {
    /// Position this node on the given position. If the node is over the
    /// specified bounds it's bouncing back to be fully visible on screen.
    /// specified bounds it's bouncing back to be fully visible on screen.
    /// The `top` specifies if the position should be considered the top left
    /// corner of the node or the bottom left corner.
    fn position_and_bounce(&mut self, position: Point, bounds: Size, top: bool);
}

impl Position for Tree {
    fn position_and_bounce(&mut self, position: Point, bounds: Size, top: bool) {
        self.translation = Vector::new(position.x.max(0.0), position.y.max(0.0));

        let new_self_bounds = Rectangle::new(Point::ORIGIN + self.translation, self.size);

        self.translation = Vector::new(
            if new_self_bounds.x + new_self_bounds.width > bounds.width {
                (new_self_bounds.x - (new_self_bounds.width - (bounds.width - new_self_bounds.x)))
                    .max(0.0)
            } else {
                new_self_bounds.x
            },
            if top && (new_self_bounds.y + new_self_bounds.height > bounds.height) {
                (new_self_bounds.y - (new_self_bounds.height - (bounds.height - new_self_bounds.y)))
                    .max(0.0)
            } else if !top && (new_self_bounds.y - new_self_bounds.height < 0.0) {
                (new_self_bounds.y + (new_self_bounds.height - new_self_bounds.y)).max(0.0)
            } else if top {
                new_self_bounds.y
            } else {
                new_self_bounds.y - new_self_bounds.height
            },
        );
    }
}

/// Shortcut helper to create a [`ColorPicker`] Widget.
///
/// [`ColorPicker`]: crate::ColorPicker
pub fn color_picker<'a, Message, Theme, F>(
    show_picker: bool,
    color: Color,
    underlay: impl Widget<Message, Theme, Renderer> + 'a,
    on_cancel: Message,
    on_submit: F,
) -> ColorPicker<'a, Message, Theme>
where
    Message: 'a + Clone,
    Theme: 'a
        + Catalog
        + iced::widget::button::Catalog
        + iced::widget::text::Catalog
        + container::Catalog
        + text_input::Catalog
        + Clone,
    F: 'a + Fn(Color) -> Message,
{
    ColorPicker::new(show_picker, color, underlay, on_cancel, on_submit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    enum Message {
        Cancel,
        Submit(Color),
    }

    fn renderer() -> Renderer {
        smol::block_on(<Renderer as renderer::Headless>::new(
            renderer::Settings::default(),
            Some("tiny-skia"),
        ))
        .expect("the software renderer should be available")
    }

    fn picker() -> ColorPicker<'static, Message> {
        color_picker(
            true,
            Color::from_rgb(0.5, 0.25, 0.25),
            iced::widget::Space::new().width(80).height(30),
            Message::Cancel,
            Message::Submit,
        )
    }

    #[test]
    fn geometry_copies_layout_without_widget_state() {
        let tree = Tree {
            tag: Tag::of::<u32>(),
            state: tree::State::new(7_u32),
            size: Size::new(100.0, 80.0),
            translation: Vector::new(40.0, 50.0),
            children: vec![Tree {
                tag: Tag::of::<u32>(),
                state: tree::State::new(9_u32),
                size: Size::new(20.0, 10.0),
                translation: Vector::new(5.0, 6.0),
                ..Tree::empty()
            }],
        };
        let geometry = geometry_tree(&tree);
        assert_eq!(geometry.tag, Tag::stateless());
        assert!(matches!(geometry.state, tree::State::None));
        let child = GeometryLayout::new(&geometry).children().next().unwrap();
        assert_eq!(
            child.bounds(),
            Rectangle::new(Point::new(45.0, 56.0), tree.children[0].size)
        );
        assert_eq!(child.for_widget(&tree.children[0]).bounds(), child.bounds());
        assert_eq!(*tree.state.downcast_ref::<u32>(), 7);
        assert_eq!(*tree.children[0].state.downcast_ref::<u32>(), 9);
    }

    #[test]
    fn placement_preserves_above_below_and_bounce() {
        let size = Size::new(100.0, 60.0);
        let window = Size::new(400.0, 300.0);
        for (position, top, expected) in [
            (Point::new(25.0, 30.0), true, Vector::new(25.0, 30.0)),
            (Point::new(350.0, 280.0), true, Vector::new(300.0, 240.0)),
            (Point::new(25.0, 260.0), false, Vector::new(25.0, 200.0)),
            (Point::new(-5.0, 30.0), false, Vector::new(0.0, 60.0)),
        ] {
            let mut tree = Tree {
                size,
                ..Tree::empty()
            };
            tree.position_and_bounce(position, window, top);
            assert_eq!(tree.translation, expected);
        }
    }

    #[test]
    fn overlay_keeps_geometry_and_real_widget_hierarchies_separate() {
        let renderer = renderer();
        let mut picker = picker();
        let mut tree = Tree::new(&picker);
        tree.diff(&mut picker);
        picker.layout(
            &mut tree,
            &renderer,
            &Limits::new(Size::ZERO, Size::new(80.0, 30.0)),
        );
        assert_eq!(tree.size, tree.children[0].size);

        for (window, position, below, horizontal) in [
            (Size::new(900.0, 700.0), Point::new(100.0, 40.0), true, true),
            (
                Size::new(900.0, 700.0),
                Point::new(100.0, 650.0),
                false,
                true,
            ),
            (Size::new(320.0, 900.0), Point::new(10.0, 30.0), true, false),
        ] {
            let layout = Layout::new(tree.size).move_to(position);
            let viewport = Rectangle::with_size(window);
            let overlays = picker.overlay(
                &mut tree,
                layout,
                &renderer,
                &viewport,
                Vector::ZERO,
                window,
            );
            assert_eq!(overlays.len(), 1);
            drop(overlays);

            let state = tree.state.downcast_ref::<State>();
            let (_, _, geometry) = state.overlay_layout.as_ref().unwrap();
            assert_eq!(geometry.children.len(), 2);
            let mut blocks = GeometryLayout::new(geometry).children();
            let hsv = blocks.next().unwrap();
            let controls = blocks.next().unwrap();
            assert_eq!(hsv.tree.children.len(), 2);
            assert_eq!(controls.tree.children.len(), 4);
            assert_eq!(controls.tree.children[0].children.len(), 4);
            for row in &controls.tree.children[0].children {
                assert_eq!(row.children.len(), 3);
            }
            assert!(controls.tree.children[1].children.is_empty());
            assert!(controls.tree.children[2].children.is_empty());
            assert_eq!(state.overlay_state.rgba_tree.children.len(), 4);
            assert_eq!(tree.children[1].children[0].children.len(), 1);
            assert!(tree.children[1].children[3].children.is_empty());
            assert_eq!(tree.children[1].children[2].size, Size::ZERO);
            assert_eq!(hsv.bounds().x < controls.bounds().x, horizontal);
            if below {
                assert_eq!(
                    GeometryLayout::new(geometry).bounds().y,
                    position.y + tree.size.height + OFFSET
                );
            } else {
                assert_eq!(
                    GeometryLayout::new(geometry).bounds().y + geometry.size.height,
                    position.y - OFFSET
                );
            }
        }
    }

    struct InputText(String);

    impl Operation for InputText {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }

        fn text_input(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn operation::TextInput,
        ) {
            self.0 = state.text().into_owned();
        }
    }

    #[test]
    fn changing_color_relayouts_the_leaf_hex_input() {
        let mut renderer = renderer();
        let mut picker = picker();
        let mut tree = Tree::new(&picker);
        tree.diff(&mut picker);
        let window = Size::new(900.0, 700.0);
        let viewport = Rectangle::with_size(window);
        picker.layout(&mut tree, &renderer, &Limits::new(Size::ZERO, window));
        let layout = Layout::new(tree.size).move_to(Point::new(40.0, 30.0));
        drop(picker.overlay(
            &mut tree,
            layout,
            &renderer,
            &viewport,
            Vector::ZERO,
            window,
        ));

        let hue_bounds = {
            let state = tree.state.downcast_ref::<State>();
            let (_, _, geometry) = state.overlay_layout.as_ref().unwrap();
            GeometryLayout::new(geometry)
                .children()
                .next()
                .unwrap()
                .children()
                .nth(1)
                .unwrap()
                .bounds()
        };
        let cursor = Cursor::Available(hue_bounds.center());
        let mut messages = shell::Bus::new();
        let mut shell = Shell::new(
            &iced_core::window::Headless,
            shell::Waker::noop(),
            &mut messages,
        );
        let mut overlays = picker.overlay(
            &mut tree,
            layout,
            &renderer,
            &viewport,
            Vector::ZERO,
            window,
        );
        let overlay = overlays[0].as_overlay_mut();
        assert_eq!(
            overlay.mouse_interaction(cursor, &renderer),
            mouse::Interaction::Pointer
        );
        overlay.draw(
            &mut renderer,
            &Theme::Dark,
            &renderer::Style::default(),
            cursor,
        );
        overlay.update(
            &Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: 20.0 },
            }),
            cursor,
            &renderer,
            &mut shell,
        );
        assert!(shell.is_event_captured());
        drop(overlays);
        let state = tree.state.downcast_ref::<State>();
        assert!(state.overlay_state.layout_stale);
        let expected = state.overlay_state.color.as_hex_string();
        assert_ne!(expected, picker.color.as_hex_string());

        drop(picker.overlay(
            &mut tree,
            layout,
            &renderer,
            &viewport,
            Vector::ZERO,
            window,
        ));
        assert!(
            !tree
                .state
                .downcast_ref::<State>()
                .overlay_state
                .layout_stale
        );
        let hex_tree = &mut tree.children[1].children[3];
        let mut input_text = InputText(String::new());
        picker.hex_input.operate(
            hex_tree,
            Layout::new(hex_tree.size),
            &viewport,
            &renderer,
            &mut input_text,
        );
        assert_eq!(input_text.0, expected);
    }

    const WINDOW: Size = Size::new(900.0, 700.0);
    const ORIGINAL_HEX: &str = "#112233FF";

    fn original() -> Color {
        Color::from_rgb8(0x11, 0x22, 0x33)
    }

    fn picker_with(color: Color) -> ColorPicker<'static, Message> {
        color_picker(
            true,
            color,
            iced::widget::Space::new().width(80).height(30),
            Message::Cancel,
            Message::Submit,
        )
    }

    /// Drives a picker the way the runtime does: the overlay is rebuilt for
    /// every event and every redraw, and `rebuild` stands in for `view`.
    struct Harness {
        renderer: Renderer,
        picker: ColorPicker<'static, Message>,
        tree: Tree,
        cursor: Cursor,
    }

    impl Harness {
        fn new(color: Color) -> Self {
            Self::with_picker(picker_with(color))
        }

        fn with_picker(mut picker: ColorPicker<'static, Message>) -> Self {
            let mut tree = Tree::new(&picker);
            tree.diff(&mut picker);
            let mut harness = Self {
                renderer: renderer(),
                picker,
                tree,
                cursor: Cursor::Unavailable,
            };
            harness.relayout();
            harness.redraw();
            harness
        }

        fn layout(&self) -> Layout {
            Layout::new(self.tree.size).move_to(Point::new(40.0, 30.0))
        }

        fn relayout(&mut self) {
            self.picker.layout(
                &mut self.tree,
                &self.renderer,
                &Limits::new(Size::ZERO, WINDOW),
            );
        }

        fn rebuild(&mut self, color: Color) {
            self.picker = picker_with(color);
            self.tree.diff(&mut self.picker);
            self.relayout();
            self.redraw();
        }

        fn control_bounds(&mut self, index: usize) -> Rectangle {
            self.redraw();
            let state = self.tree.state.downcast_ref::<State>();
            let (_, _, geometry) = state.overlay_layout.as_ref().unwrap();
            GeometryLayout::new(geometry)
                .children()
                .nth(1)
                .unwrap()
                .children()
                .nth(index)
                .unwrap()
                .bounds()
        }

        fn send(&mut self, event: Event) -> Vec<Message> {
            self.send_with_status(event).0
        }

        fn send_with_status(&mut self, event: Event) -> (Vec<Message>, event::Status) {
            let (messages, status, _) = self.dispatch(event);
            (messages, status)
        }

        /// Moves the cursor without drawing, returning the redraw the overlay requested.
        fn move_cursor(&mut self, cursor: Cursor) -> iced::window::RedrawRequest {
            self.cursor = cursor;
            let event = cursor
                .position()
                .map_or(mouse::Event::CursorLeft, |position| {
                    mouse::Event::CursorMoved { position }
                });
            self.dispatch(Event::Mouse(event)).2
        }

        fn dispatch(
            &mut self,
            event: Event,
        ) -> (Vec<Message>, event::Status, iced::window::RedrawRequest) {
            let mut messages = shell::Bus::new();
            let mut shell = Shell::new(
                &iced_core::window::Headless,
                shell::Waker::noop(),
                &mut messages,
            );
            let layout = self.layout();
            let mut overlays = self.picker.overlay(
                &mut self.tree,
                layout,
                &self.renderer,
                &Rectangle::with_size(WINDOW),
                Vector::ZERO,
                WINDOW,
            );
            overlays[0]
                .as_overlay_mut()
                .update(&event, self.cursor, &self.renderer, &mut shell);
            drop(overlays);
            let status = shell.event_status();
            let redraw = shell.redraw_request();
            drop(shell);
            (messages.into_iter().collect(), status, redraw)
        }

        /// Presses a named key, returning the messages and whether the press was captured.
        fn press(&mut self, key: keyboard::key::Named) -> (Vec<Message>, bool) {
            let key = keyboard::Key::Named(key);
            let (messages, status) =
                self.send_with_status(Event::Keyboard(keyboard::Event::KeyPressed {
                    key: key.clone(),
                    modified_key: key,
                    physical_key: keyboard::key::Physical::Unidentified(
                        keyboard::key::NativeCode::Unidentified,
                    ),
                    location: keyboard::Location::Standard,
                    modifiers: keyboard::Modifiers::empty(),
                    text: None,
                    repeat: false,
                }));
            self.redraw();
            (messages, status == event::Status::Captured)
        }

        fn redraw(&mut self) {
            let _ = self.send(Event::Window(iced::window::Event::RedrawRequested(
                iced::time::Instant::now(),
            )));
            let layout = self.layout();
            let mut overlays = self.picker.overlay(
                &mut self.tree,
                layout,
                &self.renderer,
                &Rectangle::with_size(WINDOW),
                Vector::ZERO,
                WINDOW,
            );
            overlays[0].as_overlay_mut().draw(
                &mut self.renderer,
                &Theme::Dark,
                &renderer::Style::default(),
                self.cursor,
            );
        }

        fn click(&mut self, bounds: Rectangle) -> Vec<Message> {
            self.cursor = Cursor::Available(bounds.center());
            let mut messages = self.send(Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            )));
            messages.extend(self.send(Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left,
            ))));
            self.redraw();
            messages
        }

        fn key(&mut self, key: keyboard::Key, modifiers: keyboard::Modifiers, text: Option<&str>) {
            let physical_key =
                keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified);
            let _ = self.send(Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(),
                modified_key: key.clone(),
                physical_key,
                location: keyboard::Location::Standard,
                modifiers,
                text: text.map(Into::into),
                repeat: false,
            }));
            let _ = self.send(Event::Keyboard(keyboard::Event::KeyReleased {
                key: key.clone(),
                modified_key: key,
                physical_key,
                location: keyboard::Location::Standard,
                modifiers,
            }));
        }

        fn select_all(&mut self) {
            let _ = self.send(Event::Keyboard(keyboard::Event::ModifiersChanged(
                keyboard::Modifiers::CTRL,
            )));
            self.key(
                keyboard::Key::Character("a".into()),
                keyboard::Modifiers::CTRL,
                None,
            );
            let _ = self.send(Event::Keyboard(keyboard::Event::ModifiersChanged(
                keyboard::Modifiers::empty(),
            )));
        }

        fn type_text(&mut self, text: &str) {
            for character in text.chars() {
                let character = character.to_string();
                self.key(
                    keyboard::Key::Character(character.as_str().into()),
                    keyboard::Modifiers::empty(),
                    Some(&character),
                );
                self.redraw();
            }
        }

        fn hex_text(&mut self) -> String {
            let hex_tree = &mut self.tree.children[1].children[3];
            let mut input_text = InputText(String::new());
            self.picker.hex_input.operate(
                hex_tree,
                Layout::new(hex_tree.size),
                &Rectangle::with_size(WINDOW),
                &self.renderer,
                &mut input_text,
            );
            input_text.0
        }

        fn color(&self) -> Color {
            self.tree.state.downcast_ref::<State>().overlay_state.color
        }
    }

    #[test]
    fn default_body_style_supports_partial_customization() {
        assert_eq!(Style::default(), primary(&Theme::Light, Status::Active));
        let customized = Style {
            background: Color::BLACK.into(),
            ..Style::default()
        };
        assert_eq!(customized.background, Color::BLACK.into());
        assert_eq!(customized.border, Style::default().border);
        for theme in [Theme::Light, Theme::Dark] {
            let palette = theme.palette();
            let active = primary(&theme, Status::Active);
            assert_eq!(active.text_color, palette.background.base.text);
            assert_eq!(active.area_border.width, 2.0);
            assert_eq!(active.bar_background, Color::TRANSPARENT.into());
            assert_eq!(active.shadow, Shadow::default());
            assert_eq!(active.sat_value_marker, None);
            assert_eq!(active.hue_marker, Color::BLACK);
            assert_eq!(
                primary(&theme, Status::Focused).focus_border.color,
                palette.background.strong.color
            );
        }
    }

    fn channel_bounds(ui: &Harness) -> Vec<Vec<Rectangle>> {
        let state = ui.tree.state.downcast_ref::<State>();
        let geometry = &state.overlay_layout.as_ref().unwrap().2;
        GeometryLayout::new(geometry)
            .children()
            .nth(1)
            .unwrap()
            .children()
            .next()
            .unwrap()
            .children()
            .map(|row| row.children().map(GeometryLayout::bounds).collect())
            .collect()
    }

    #[test]
    fn custom_labels_and_text_size_reserve_space_before_bars() {
        let default = Harness::new(original());
        let ui = Harness::with_picker(
            picker_with(original())
                .channel_labels("Red channel", "Green", "Blue", "Alpha")
                .text_size(24)
                .font(Font::MONOSPACE),
        );
        let defaults = channel_bounds(&default);
        let custom = channel_bounds(&ui);
        for (row, default_row) in custom.iter().zip(&defaults) {
            assert!(row[0].width > default_row[0].width);
            assert!(row[1].x >= row[0].x + row[0].width + SPACING.0);
            assert!(row[1].width > 0.0);
            assert!(row[2].x >= row[1].x + row[1].width);
            assert_eq!(row[0].width, custom[0][0].width);
        }
        assert!(
            ui.tree.children[1].children[3].size.height
                > default.tree.children[1].children[3].size.height
        );
    }

    #[test]
    fn default_button_styles_keep_the_theme_catalog_appearance() {
        for theme in [Theme::Light, Theme::Dark] {
            for focused in [false, true] {
                let wrapper = ButtonTheme {
                    theme: theme.clone(),
                    focused,
                };
                for status in [
                    button::Status::Active,
                    button::Status::Hovered,
                    button::Status::Pressed,
                ] {
                    assert_eq!(
                        button::Catalog::style(&wrapper, &None, status),
                        button::Catalog::style(
                            &theme,
                            &<Theme as button::Catalog>::default(),
                            status
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn custom_button_styles_receive_hover_press_and_keyboard_focus() {
        use keyboard::key::Named;
        use std::cell::RefCell;

        let statuses = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&statuses);
        let mut ui =
            Harness::with_picker(picker_with(original()).button_style(move |theme, status| {
                record.borrow_mut().push(status);
                button::primary(theme, button::Status::Active)
            }));
        let cancel = ui.control_bounds(2);
        ui.cursor = Cursor::Available(cancel.center());
        ui.redraw();
        assert!(statuses.borrow().contains(&ButtonStatus::Hovered));
        for _ in 0..8 {
            ui.press(Named::Tab);
        }
        assert_eq!(
            ui.tree.state.downcast_ref::<State>().overlay_state.focus,
            Focus::Cancel
        );
        assert!(
            statuses
                .borrow()
                .contains(&ButtonStatus::Focused { is_hovered: true })
        );
        statuses.borrow_mut().clear();
        ui.send(Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        )));
        ui.redraw();
        assert!(statuses.borrow().contains(&ButtonStatus::Pressed));
        assert!(
            !statuses
                .borrow()
                .contains(&ButtonStatus::Focused { is_hovered: true })
        );
        ui.cursor = Cursor::Unavailable;
        ui.send(Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        )));
        ui.press(Named::Tab);
        assert_eq!(
            ui.tree.state.downcast_ref::<State>().overlay_state.focus,
            Focus::Submit
        );
        assert!(
            statuses
                .borrow()
                .contains(&ButtonStatus::Focused { is_hovered: false })
        );
    }

    #[test]
    fn custom_button_contents_keep_their_actions_and_independent_styles() {
        use std::cell::RefCell;
        let cancel_statuses = Rc::new(RefCell::new(Vec::new()));
        let submit_statuses = Rc::new(RefCell::new(Vec::new()));
        let cancel = Rc::clone(&cancel_statuses);
        let submit = Rc::clone(&submit_statuses);
        let mut ui = Harness::with_picker(
            picker_with(original())
                .cancel_button_style(move |theme, status| {
                    cancel.borrow_mut().push(status);
                    button::danger(theme, button::Status::Active)
                })
                .cancel_content(text("Discard"))
                .submit_content(text("Apply"))
                .submit_button_style(move |theme, status| {
                    submit.borrow_mut().push(status);
                    button::success(theme, button::Status::Active)
                }),
        );
        let submit = ui.control_bounds(3);
        assert_eq!(ui.click(submit), vec![Message::Submit(original())]);
        let cancel = ui.control_bounds(2);
        assert_eq!(ui.click(cancel), vec![Message::Cancel]);
        assert!(!cancel_statuses.borrow().is_empty());
        assert!(!submit_statuses.borrow().is_empty());
    }

    #[test]
    fn tall_submit_content_reserves_its_real_height() {
        let mut ui = Harness::with_picker(
            picker_with(original())
                .submit_content(iced::widget::Space::new().width(20).height(120)),
        );
        let hex = ui.control_bounds(1);
        let submit = ui.control_bounds(3);
        assert_eq!(submit.height, 120.0 + button::DEFAULT_PADDING.y());
        assert!(submit.y >= hex.y + hex.height + SPACING.0);
        assert_eq!(ui.click(submit), vec![Message::Submit(original())]);
    }

    #[test]
    fn custom_buttons_follow_release_inside_and_touch_semantics() {
        let mut ui = Harness::with_picker(
            picker_with(original())
                .cancel_content(text("Discard"))
                .submit_content(text("Apply")),
        );
        let submit = ui.control_bounds(3);
        ui.cursor = Cursor::Available(submit.center());
        assert!(
            ui.send(Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left
            )))
            .is_empty()
        );
        ui.cursor = Cursor::Available(Point::ORIGIN);
        assert!(
            ui.send(Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left
            )))
            .is_empty()
        );
        ui.cursor = Cursor::Unavailable;
        let id = touch::Finger(1);
        assert!(
            ui.send(Event::Touch(touch::Event::FingerPressed {
                id,
                position: submit.center()
            }))
            .is_empty()
        );
        assert_eq!(
            ui.send(Event::Touch(touch::Event::FingerLifted {
                id,
                position: submit.center()
            })),
            vec![Message::Submit(original())]
        );
        let cancel = ui.control_bounds(2);
        assert!(
            ui.send(Event::Touch(touch::Event::FingerPressed {
                id,
                position: cancel.center()
            }))
            .is_empty()
        );
        assert_eq!(
            ui.send(Event::Touch(touch::Event::FingerLifted {
                id,
                position: cancel.center()
            })),
            vec![Message::Cancel]
        );
    }

    #[test]
    fn custom_content_messages_are_not_converted_to_submit() {
        let mut ui = Harness::with_picker(
            picker_with(original())
                .submit_content(Button::new(text("Inner action")).on_press(Message::Cancel)),
        );
        let submit = ui.control_bounds(3);
        assert_eq!(ui.click(submit), vec![Message::Cancel]);
    }

    #[derive(Default)]
    struct Controls {
        labels: Vec<(String, Rectangle)>,
        inputs: Vec<String>,
        focusables: usize,
    }

    impl Operation for Controls {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }
        fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, state: &mut dyn operation::Text) {
            self.labels.push((state.text().to_string(), bounds));
            state.select_all();
            assert_eq!(state.copy(), None);
        }
        fn text_input(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn operation::TextInput,
        ) {
            self.inputs.push(state.text().into_owned());
        }
        fn focusable(
            &mut self,
            _id: Option<&Id>,
            _bounds: Rectangle,
            _state: &mut dyn operation::Focusable,
        ) {
            self.focusables += 1;
        }
    }

    fn operate(ui: &mut Harness) -> Controls {
        let mut operation = Controls::default();
        let layout = ui.layout();
        let mut overlays = ui.picker.overlay(
            &mut ui.tree,
            layout,
            &ui.renderer,
            &Rectangle::with_size(WINDOW),
            Vector::ZERO,
            WINDOW,
        );
        overlays[0]
            .as_overlay_mut()
            .operate(&ui.renderer, &mut operation);
        operation
    }

    #[test]
    fn overlay_operations_expose_button_labels_contents_and_hex_input() {
        let mut ui = Harness::new(original());
        let cancel = ui.control_bounds(2);
        let submit = ui.control_bounds(3);
        let controls = operate(&mut ui);
        assert!(controls.labels.contains(&("Cancel".into(), cancel)));
        assert!(controls.labels.contains(&("Submit".into(), submit)));
        assert_eq!(controls.inputs, [ORIGINAL_HEX]);
        assert_eq!(controls.focusables, 1);

        let mut ui = Harness::with_picker(
            picker_with(original())
                .cancel_label("Discard changes")
                .submit_label("Apply color")
                .cancel_content(text("Keep old"))
                .submit_content(text_input("", "nested input").on_input(|_| Message::Cancel)),
        );
        let controls = operate(&mut ui);
        assert!(
            controls
                .labels
                .iter()
                .any(|(text, _)| text == "Discard changes")
        );
        assert!(
            controls
                .labels
                .iter()
                .any(|(text, _)| text == "Apply color")
        );
        assert!(controls.labels.iter().any(|(text, _)| text == "Keep old"));
        assert_eq!(controls.inputs, [ORIGINAL_HEX, "nested input"]);
        assert_eq!(controls.focusables, 2);
    }

    #[test]
    fn hex_style_receives_draft_validity_and_current_color() {
        use std::cell::RefCell;
        let states = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&states);
        let mut ui = Harness::with_picker(
            picker_with(original())
                .font(Font::MONOSPACE)
                .text_size(22)
                .hex_input_style(move |theme, status| {
                    record.borrow_mut().push(status);
                    text_input::default(theme, status.status)
                }),
        );
        let hex = ui.control_bounds(1);
        ui.click(hex);
        ui.select_all();
        ui.type_text("#12z");
        assert_eq!(ui.hex_text(), "#12z");
        assert!(!states.borrow().last().unwrap().is_valid);
        assert!(matches!(
            states.borrow().last().unwrap().status,
            text_input::Status::Focused { .. }
        ));
        ui.select_all();
        ui.type_text("#123");
        let state = *states.borrow().last().unwrap();
        assert!(state.is_valid);
        assert_eq!(state.color, Color::from_rgb8(0x11, 0x22, 0x33));
    }

    #[test]
    fn default_hex_style_uses_active_invalid_hex_color() {
        let invalid_hex = Color::from_rgb8(234, 12, 56);
        let mut ui =
            Harness::with_picker(picker_with(original()).style(move |theme, status| Style {
                invalid_hex: if status == Status::Active {
                    invalid_hex
                } else {
                    Color::BLACK
                },
                ..primary(theme, status)
            }));
        let hex = ui.control_bounds(1);
        ui.click(hex);
        ui.select_all();
        ui.type_text("#12z");
        let state = &ui.tree.state.downcast_ref::<State>().overlay_state;
        let active = Catalog::style(&Theme::Dark, &ui.picker.class, Status::Active);
        let theme = InternalTheme {
            theme: Theme::Dark,
            color: state.color,
            is_valid: parse_hex(&state.hex_text).is_some(),
            invalid_hex: active.invalid_hex,
        };
        let status = text_input::Status::Focused { is_hovered: true };
        let style = text_input::Catalog::style(&theme, &None, status);
        assert_eq!(style.border.color, invalid_hex);
        assert_eq!(style.border.width, 1.0);
        assert_eq!(style.border.radius, 5.0.into());
        let valid = InternalTheme {
            is_valid: true,
            ..theme
        };
        assert_eq!(
            text_input::Catalog::style(&valid, &None, status)
                .border
                .color,
            text_input::default(&Theme::Dark, status).border.color
        );
    }

    #[test]
    fn typing_a_full_hex_value_keeps_the_typed_text_and_submits_it() {
        let mut ui = Harness::new(original());
        assert_eq!(ui.hex_text(), ORIGINAL_HEX);

        let hex = ui.control_bounds(1);
        assert!(ui.click(hex).is_empty());
        ui.select_all();

        let mut typed = String::new();
        for character in ORIGINAL_HEX.chars() {
            ui.type_text(&character.to_string());
            typed.push(character);
            assert_eq!(ui.hex_text(), typed, "the field rewrote the typed text");
        }
        assert_eq!(ui.color(), original());

        let submit = ui.control_bounds(3);
        assert_eq!(ui.click(submit), vec![Message::Submit(original())]);
        assert_eq!(ui.hex_text(), ORIGINAL_HEX);
    }

    #[test]
    fn hex_text_is_formatted_when_focus_is_lost() {
        let mut ui = Harness::new(original());
        let hex = ui.control_bounds(1);
        ui.click(hex);
        ui.select_all();
        ui.type_text("#abc");
        assert_eq!(ui.hex_text(), "#abc");
        assert_eq!(ui.color(), Color::from_rgb8(0xAA, 0xBB, 0xCC));

        // An unparseable draft keeps the last valid color. Multibyte text must not panic.
        ui.select_all();
        ui.type_text("#abé");
        assert_eq!(ui.hex_text(), "#abé");
        assert_eq!(ui.color(), Color::from_rgb8(0xAA, 0xBB, 0xCC));
        let submit = ui.control_bounds(3);
        assert_eq!(
            ui.click(submit),
            vec![Message::Submit(Color::from_rgb8(0xAA, 0xBB, 0xCC))]
        );
        assert_eq!(ui.hex_text(), "#AABBCCFF");
    }

    #[test]
    fn rebuilding_the_view_keeps_the_draft_until_the_color_changes() {
        let mut ui = Harness::new(original());
        let hex = ui.control_bounds(1);
        ui.click(hex);
        ui.select_all();
        ui.type_text("#4455");
        ui.rebuild(original());
        assert_eq!(ui.hex_text(), "#4455");
        assert_eq!(
            ui.color(),
            Color::from_rgba8(0x44, 0x44, 0x55, 85.0 / 255.0)
        );

        ui.type_text("66");
        assert_eq!(ui.hex_text(), "#445566");

        let external = Color::from_rgb8(0x01, 0x02, 0x03);
        ui.rebuild(external);
        assert_eq!(ui.hex_text(), "#010203FF");
        assert_eq!(ui.color(), external);
    }

    #[test]
    fn enter_in_the_hex_input_submits_only_valid_text() {
        use keyboard::key::Named;

        let mut ui = Harness::new(original());
        let hex = ui.control_bounds(1);
        ui.click(hex);
        ui.select_all();
        ui.type_text("#abc");
        let abc = Color::from_rgb8(0xAA, 0xBB, 0xCC);
        assert_eq!(ui.press(Named::Enter), (vec![Message::Submit(abc)], true));

        ui.type_text("z");
        assert_eq!(ui.hex_text(), "#abcz");
        assert_eq!(ui.press(Named::Enter), (vec![], true));
    }

    #[test]
    fn enter_submits_or_cancels_from_the_focused_control() {
        use keyboard::key::Named;

        let mut ui = Harness::new(original());
        assert_eq!(ui.press(Named::Enter), (vec![], false));

        assert!(ui.press(Named::Tab).1);
        assert_eq!(
            ui.press(Named::Enter),
            (vec![Message::Submit(original())], true)
        );

        assert!(ui.press(Named::Tab).1);
        assert!(ui.press(Named::ArrowDown).1);
        let changed = ui.color();
        assert_ne!(changed, original());
        assert_eq!(ui.hex_text(), changed.as_hex_string());
        assert_eq!(
            ui.press(Named::Enter),
            (vec![Message::Submit(changed)], true)
        );

        // Hue, red, green, blue, alpha, then cancel.
        for _ in 0..6 {
            assert!(ui.press(Named::Tab).1);
        }
        assert_eq!(ui.press(Named::Enter), (vec![Message::Cancel], true));
    }

    #[test]
    fn escape_cancels_and_is_consumed() {
        let mut ui = Harness::new(original());
        assert_eq!(
            ui.press(keyboard::key::Named::Escape),
            (vec![Message::Cancel], true)
        );

        let hex = ui.control_bounds(1);
        ui.click(hex);
        ui.type_text("#1");
        assert_eq!(
            ui.press(keyboard::key::Named::Escape),
            (vec![Message::Cancel], true)
        );
    }

    fn hsv_area_bounds(ui: &Harness) -> Vec<Rectangle> {
        let state = ui.tree.state.downcast_ref::<State>();
        let geometry = &state.overlay_layout.as_ref().unwrap().2;
        GeometryLayout::new(geometry)
            .children()
            .next()
            .unwrap()
            .children()
            .map(GeometryLayout::bounds)
            .collect()
    }

    // The hex input stays unfocused in these tests: its caret blink schedules redraws,
    // which used to hide the missing hover redraws.
    #[test]
    fn moving_between_rgba_bars_requests_a_redraw() {
        use iced::window::RedrawRequest;

        let mut ui = Harness::new(original());
        let bars: Vec<_> = channel_bounds(&ui).iter().map(|row| row[1]).collect();
        let _ = ui.move_cursor(Cursor::Available(bars[0].center()));
        ui.redraw();

        for bar in &bars[1..] {
            assert_eq!(
                ui.move_cursor(Cursor::Available(bar.center())),
                RedrawRequest::NextFrame
            );
            ui.redraw();
        }
    }

    #[test]
    fn moving_between_hsv_areas_and_out_requests_a_redraw() {
        use iced::window::RedrawRequest;

        let mut ui = Harness::new(original());
        let areas = hsv_area_bounds(&ui);
        let label = channel_bounds(&ui)[0][0];
        assert_eq!(
            ui.move_cursor(Cursor::Available(areas[0].center())),
            RedrawRequest::NextFrame
        );
        ui.redraw();

        for cursor in [
            Cursor::Available(areas[1].center()),
            Cursor::Available(label.center()),
            Cursor::Unavailable,
        ] {
            assert_eq!(ui.move_cursor(cursor), RedrawRequest::NextFrame);
            ui.redraw();
        }
    }

    #[test]
    fn moving_without_changing_the_hovered_part_does_not_request_a_redraw() {
        use iced::window::RedrawRequest;

        let mut ui = Harness::new(original());
        let channels = channel_bounds(&ui);
        let red = channels[0][1];
        let hue = hsv_area_bounds(&ui)[1];
        for (from, to) in [
            (
                red.center(),
                red.center() + Vector::new(red.width / 4.0, 0.0),
            ),
            (
                hue.center(),
                hue.center() + Vector::new(hue.width / 4.0, 0.0),
            ),
            (channels[0][0].center(), channels[1][0].center()),
        ] {
            let _ = ui.move_cursor(Cursor::Available(from));
            ui.redraw();
            assert_eq!(ui.move_cursor(Cursor::Available(to)), RedrawRequest::Wait);
        }
    }
}
