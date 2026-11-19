use crate::opaque_maybe::{barrier, inert_maybe};
use iced::widget::{center, container, mouse_area, stack};
use iced::{Border, Color, Theme, Widget};

pub fn modal<'a, Message>(
    base: impl Widget<Message> + 'a,
    content: Option<impl Widget<Message> + 'a>,
    on_close: Message,
) -> impl Widget<Message> + 'a
where
    Message: Clone + 'a,
{
    stack![inert_maybe(base, content.is_some())].push(
        content
            .map(|content| {
                // A barrier rather than `opaque`: a finger press on static dialog
                // content must not reach the backdrop and close the dialog.
                mouse_area(center(barrier(content)).style(|_theme| {
                    container::Style {
                        background: Some(
                            Color {
                                a: 0.5,
                                ..Color::BLACK
                            }
                            .into(),
                        ),
                        ..container::Style::default()
                    }
                }))
                .on_press(on_close)
            })
            .boxed(),
    )
}

pub fn default(theme: &Theme) -> container::Style {
    let palette = theme.palette();

    let background = Some(
        Color {
            a: 0.95,
            ..palette.background.base.color
        }
        .into(),
    );

    container::Style {
        text_color: None,
        background,
        border: Border {
            color: palette.background.strong.color,
            width: 1.5,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn red(theme: &Theme) -> container::Style {
    let palette = theme.palette();

    let background = Some(
        Color {
            a: 0.975,
            ..palette.background.base.color
        }
        .into(),
    );

    container::Style {
        text_color: None,
        background,
        border: Border {
            color: palette.danger.strong.color,
            width: 1.5,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use iced::advanced::clipboard;
    use iced::advanced::layout::Limits;
    use iced::advanced::renderer::{self, Headless as _};
    use iced::advanced::shell::{Bus, Waker};
    use iced::advanced::widget::{Tree, operation};
    use iced::advanced::{InputMethod, Layout, Shell, Widget as _};
    use iced::widget::{Space, text_input};
    use iced::{
        Event, Fill, Point, Rectangle, Renderer, Size, Vector, keyboard, mouse, touch, window,
    };

    const WINDOW: Size = Size::new(900.0, 700.0);

    fn renderer() -> Renderer {
        smol::block_on(Renderer::new(
            renderer::Settings::default(),
            Some("tiny-skia"),
        ))
        .expect("the software renderer should be available")
    }

    fn dialog(is_shown: bool) -> Option<Space> {
        is_shown.then(|| Space::new().width(80).height(60))
    }

    fn lay_out(root: &mut impl Widget<u8>, tree: &mut Tree, renderer: &Renderer) -> Layout {
        tree.diff(root);
        root.layout(tree, renderer, &Limits::new(Size::ZERO, WINDOW));
        Layout::new(tree.size)
    }

    fn send(
        root: &mut impl Widget<u8>,
        tree: &mut Tree,
        layout: Layout,
        renderer: &Renderer,
        event: Event,
        cursor: mouse::Cursor,
    ) -> (Vec<u8>, InputMethod) {
        let mut bus = Bus::new();
        let mut shell = Shell::new(&window::Headless, Waker::noop(), &mut bus);
        root.update(
            tree,
            &event,
            layout,
            cursor,
            renderer,
            &mut shell,
            &Rectangle::with_size(WINDOW),
        );
        let input_method = shell.input_method().clone();
        drop(shell);
        (bus.into_iter().collect(), input_method)
    }

    fn overlays(
        root: &mut impl Widget<u8>,
        tree: &mut Tree,
        layout: Layout,
        renderer: &Renderer,
    ) -> usize {
        root.overlay(
            tree,
            layout,
            renderer,
            &Rectangle::with_size(WINDOW),
            Vector::ZERO,
            WINDOW,
        )
        .len()
    }

    #[test]
    fn dialog_hides_background_overlays() {
        let renderer = renderer();
        let base = || {
            crate::tooltip::tooltip(
                Space::new().width(200).height(40),
                Space::new().width(120).height(100),
            )
            .open(crate::tooltip::Open::LeftPointer)
        };

        let mut root = modal(base(), dialog(false), 0);
        let mut tree = Tree::new(&root);
        let layout = lay_out(&mut root, &mut tree, &renderer);
        for event in [
            mouse::Event::ButtonPressed(mouse::Button::Left),
            mouse::Event::ButtonReleased(mouse::Button::Left),
        ] {
            let cursor = mouse::Cursor::Available(Point::new(20.0, 20.0));
            send(
                &mut root,
                &mut tree,
                layout,
                &renderer,
                Event::Mouse(event),
                cursor,
            );
        }
        assert_eq!(overlays(&mut root, &mut tree, layout, &renderer), 1);
        drop(root);

        let mut root = modal(base(), dialog(true), 0);
        let layout = lay_out(&mut root, &mut tree, &renderer);
        assert_eq!(overlays(&mut root, &mut tree, layout, &renderer), 0);
        drop(root);

        let mut root = modal(base(), dialog(false), 0);
        let layout = lay_out(&mut root, &mut tree, &renderer);
        assert_eq!(overlays(&mut root, &mut tree, layout, &renderer), 1);
    }

    #[test]
    fn dialog_keeps_keyboard_and_input_method_from_focused_background() {
        let renderer = renderer();
        let key = keyboard::Key::Character("q".into());
        let typing = Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: Some("q".into()),
            repeat: false,
        });
        let redraw = Event::Window(window::Event::RedrawRequested(iced::time::Instant::now()));

        for is_shown in [false, true] {
            let mut root = modal(
                text_input("", "").id("base").on_input(|_| 1),
                dialog(is_shown),
                2,
            );
            let mut tree = Tree::new(&root);
            let layout = lay_out(&mut root, &mut tree, &renderer);
            root.operate(
                &mut tree,
                layout,
                &Rectangle::with_size(WINDOW),
                &renderer,
                &mut operation::focusable::focus(iced::widget::Id::new("base")),
            );
            let cursor = mouse::Cursor::Unavailable;

            let (messages, _) = send(
                &mut root,
                &mut tree,
                layout,
                &renderer,
                typing.clone(),
                cursor,
            );
            let (_, input_method) = send(
                &mut root,
                &mut tree,
                layout,
                &renderer,
                redraw.clone(),
                cursor,
            );

            if is_shown {
                assert!(messages.is_empty());
                assert_eq!(input_method, InputMethod::Disabled);
            } else {
                assert_eq!(messages, [1]);
                assert!(matches!(input_method, InputMethod::Enabled { .. }));
            }
        }
    }

    #[test]
    fn dialog_content_captures_mouse_and_finger_presses() {
        let renderer = renderer();
        let mut root = modal(Space::new().width(Fill).height(Fill), dialog(true), 9);
        let mut tree = Tree::new(&root);
        let layout = lay_out(&mut root, &mut tree, &renderer);

        for (position, expected) in [(layout.bounds().center(), &[][..]), (Point::ORIGIN, &[9])] {
            let cursor = mouse::Cursor::Available(position);
            let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
            let finger = Event::Touch(touch::Event::FingerPressed {
                id: touch::Finger(1),
                position,
            });

            for event in [press, finger] {
                let (messages, _) = send(&mut root, &mut tree, layout, &renderer, event, cursor);
                assert_eq!(messages, expected);
            }
        }
    }

    #[test]
    fn dialog_keeps_clipboard_reads_from_focused_background() {
        let renderer = renderer();
        let input = || text_input("", "").id("base").on_input(|_| 1);
        let paste = Event::Clipboard(clipboard::Event::Read(Ok(Arc::new(
            clipboard::Content::Text("synthetic".into()),
        ))));

        let mut root = modal(input(), dialog(false), 2);
        let mut tree = Tree::new(&root);
        let layout = lay_out(&mut root, &mut tree, &renderer);
        root.operate(
            &mut tree,
            layout,
            &Rectangle::with_size(WINDOW),
            &renderer,
            &mut operation::focusable::focus(iced::widget::Id::new("base")),
        );
        let cursor = mouse::Cursor::Unavailable;
        let (messages, _) = send(
            &mut root,
            &mut tree,
            layout,
            &renderer,
            paste.clone(),
            cursor,
        );
        assert_eq!(messages, [1]);
        drop(root);

        // A read requested before the dialog opened resolves while it is shown.
        let mut root = modal(input(), dialog(true), 2);
        let layout = lay_out(&mut root, &mut tree, &renderer);
        let (messages, _) = send(&mut root, &mut tree, layout, &renderer, paste, cursor);
        assert!(messages.is_empty());
    }
}
