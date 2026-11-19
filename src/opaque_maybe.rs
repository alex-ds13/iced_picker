use iced_core as core;

use core::layout::{self, Layout};
use core::mouse;
use core::renderer;
use core::touch;
use core::widget::tree::{self, Tree};
use core::{Element, Event, Length, Rectangle, Shell, Size, Widget, widget::operation};

use iced::Widget as _;

/// Wraps the given widget and captures any mouse button events inside the bounds of
/// the widget by lifting the mouse cursor as well — effectively making it _opaque_.
///
/// This helper is meant to be used to mark elements in a [`Stack`] to avoid mouse
/// events from passing through layers.
///
/// [`Stack`]: crate::Stack
pub fn opaque_maybe<'a, Message, Theme, Renderer>(
    content: impl Widget<Message, Theme, Renderer> + 'a,
    opaque: bool,
) -> impl Widget<Message, Theme, Renderer> + 'a
where
    Message: 'a,
    Theme: 'a,
    Renderer: core::Renderer + 'a,
{
    Opaque {
        content: content.boxed(),
        mode: if opaque {
            Mode::Opaque
        } else {
            Mode::Transparent
        },
    }
}

/// Like [`opaque_maybe`], but an opaque layer is also inert, as the background of a
/// modal dialog should be: keyboard, input method and clipboard events are not
/// forwarded to it, its input method requests are dropped and its overlays are hidden.
///
/// Clipboard events are dropped because a focused text input applies a clipboard read
/// result as a paste, even for a read it requested before the layer turned inert.
///
/// Window and other events still reach it, so animations and layout stay current.
/// Operations (focus traversal included) are still forwarded.
pub(crate) fn inert_maybe<'a, Message, Theme, Renderer>(
    content: impl Widget<Message, Theme, Renderer> + 'a,
    inert: bool,
) -> impl Widget<Message, Theme, Renderer> + 'a
where
    Message: 'a,
    Theme: 'a,
    Renderer: core::Renderer + 'a,
{
    Opaque {
        content: content.boxed(),
        mode: if inert {
            Mode::Inert
        } else {
            Mode::Transparent
        },
    }
}

/// Wraps the given widget, which keeps working normally, and captures the mouse
/// button and finger presses inside its bounds, so they do not reach the layers below.
///
/// Unlike iced's `opaque`, finger presses are captured too.
pub(crate) fn barrier<'a, Message, Theme, Renderer>(
    content: impl Widget<Message, Theme, Renderer> + 'a,
) -> impl Widget<Message, Theme, Renderer> + 'a
where
    Message: 'a,
    Theme: 'a,
    Renderer: core::Renderer + 'a,
{
    Opaque {
        content: content.boxed(),
        mode: Mode::Barrier,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Transparent,
    /// Mouse events over the layer are captured and the content never sees the cursor.
    Opaque,
    /// [`Mode::Opaque`], and isolated from keyboard, input method and clipboard input.
    Inert,
    /// The content sees every event, and presses inside it are captured afterwards.
    Barrier,
}

struct Opaque<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    mode: Mode,
}

impl<Message, Theme, Renderer> core::widget::Meta for Opaque<'_, Message, Theme, Renderer> {}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Opaque<'_, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    fn tag(&self) -> tree::Tag {
        self.content.tag()
    }

    fn state(&self) -> tree::State {
        self.content.state()
    }

    fn diff(&mut self, tree: &mut Tree) {
        self.content.diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.content.layout(tree, renderer, limits);
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
        self.content
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn operate(
        &mut self,
        state: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn operation::Operation,
    ) {
        self.content
            .operate(state, layout, viewport, renderer, operation);
    }

    fn update(
        &mut self,
        state: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();

        match self.mode {
            Mode::Transparent => {
                self.content
                    .update(state, event, layout, cursor, renderer, shell, viewport);
                return;
            }
            Mode::Barrier => {
                self.content
                    .update(state, event, layout, cursor, renderer, shell, viewport);

                let is_press_inside = match event {
                    Event::Mouse(mouse::Event::ButtonPressed(_)) => cursor.is_over(bounds),
                    Event::Touch(touch::Event::FingerPressed { position, .. }) => {
                        cursor.is_over(bounds) || bounds.contains(*position)
                    }
                    _ => false,
                };

                if is_press_inside {
                    shell.capture_event();
                }
                return;
            }
            Mode::Opaque | Mode::Inert => {}
        }

        if matches!(event, Event::Mouse(_)) && cursor.is_over(bounds) {
            shell.capture_event();
        }
        let cursor = cursor.levitate();

        if self.mode == Mode::Opaque {
            self.content
                .update(state, event, layout, cursor, renderer, shell, viewport);
            return;
        }

        if matches!(
            event,
            Event::Keyboard(_) | Event::InputMethod(_) | Event::Clipboard(_)
        ) {
            return;
        }

        // A text input focused before the layer turned inert keeps requesting the
        // input method on every redraw, which would take it from the dialog.
        let input_method = shell.input_method().clone();
        self.content
            .update(state, event, layout, cursor, renderer, shell, viewport);
        *shell.input_method_mut() = input_method;
    }

    fn mouse_interaction(
        &self,
        state: &core::widget::Tree,
        layout: core::Layout,
        cursor: core::mouse::Cursor,
        viewport: &core::Rectangle,
        renderer: &Renderer,
    ) -> core::mouse::Interaction {
        match self.mode {
            Mode::Opaque | Mode::Inert => mouse::Interaction::Idle,
            Mode::Transparent | Mode::Barrier => self
                .content
                .mouse_interaction(state, layout, cursor, viewport, renderer),
        }
    }

    fn overlay<'b>(
        &'b mut self,
        state: &'b mut core::widget::Tree,
        layout: core::Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: core::Vector,
        window: Size,
    ) -> Vec<core::overlay::Element<'b, Message, Theme, Renderer>> {
        if self.mode == Mode::Inert {
            return Vec::new();
        }

        self.content
            .overlay(state, layout, renderer, viewport, translation, window)
    }
}
