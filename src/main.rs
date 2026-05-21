use iced_picker::hovered::hovered;

use iced::{Color, Element, Length, Theme, widget::{column, container, row}};

fn main() -> iced::Result {
    iced::application(
        ColorPickerExample::default,
        ColorPickerExample::update,
        ColorPickerExample::view,
    )
    .title("iced_picker Example")
    .theme(ColorPickerExample::theme)
    .run()
}

#[derive(Clone, Debug)]
enum Message {
    Cancel,
    Theme(Theme),
}

#[derive(Debug)]
struct ColorPickerExample {
    color: Color,
    theme: Theme,
}

impl Default for ColorPickerExample {
    fn default() -> Self {
        Self {
            color: Color::from_rgba8(0, 0, 0, 1.0),
            theme: Theme::TokyoNightStorm,
        }
    }
}

impl ColorPickerExample {
    fn update(&mut self, message: Message) {
        match message {
            Message::Cancel => {}
            Message::Theme(theme) => self.theme = theme,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let hovered_example = hovered(|is_hovered| {
            container(row!["foo", if is_hovered { "bar" } else { "baz" }, "qux"].spacing(10))
                .style(move |t| {
                    if is_hovered {
                        container::dark(t)
                    } else {
                        container::rounded_box(t)
                    }
                })
        });

        container(column![hovered_example].spacing(20))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }
}
