use crate::opaque_maybe::opaque_maybe;
use iced::widget::{center, container, mouse_area, opaque, stack};
use iced::{Border, Color, Theme, Widget};

pub fn modal<'a, Message>(
    base: impl Widget<Message> + 'a,
    content: Option<impl Widget<Message> + 'a>,
    on_close: Message,
) -> impl Widget<Message> + 'a
where
    Message: Clone + 'a,
{
    stack![opaque_maybe(base, content.is_some())].push(
        content
            .map(|content| {
                mouse_area(center(opaque(content)).style(|_theme| {
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
