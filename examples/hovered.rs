use iced_picker::hovered::hovered;
use iced::{
    Element, Theme,
    widget::{center, column, container, row, text},
};

fn main() -> iced::Result {
    iced::application(App::default, App::update, App::view)
        .title("Hovered - iced_picker")
        .theme(App::theme)
        .run()
}

#[derive(Default, Debug)]
struct App;

#[derive(Clone, Debug)]
enum Message {}

impl App {
    fn theme(&self) -> Theme {
        Theme::TokyoNightStorm
    }

    fn update(&mut self, _: Message) {}

    fn view(&self) -> Element<'_, Message> {
        let label_style = hovered(|is_hovered| {
            container(
                row!["foo", if is_hovered { "bar" } else { "baz" }, "qux"].spacing(10),
            )
            .padding(10)
            .style(move |t| {
                if is_hovered {
                    container::dark(t)
                } else {
                    container::rounded_box(t)
                }
            })
        });

        let text_size = hovered(|is_hovered| {
            container(
                text(if is_hovered { "Hovered!" } else { "Hover over me" })
                    .size(if is_hovered { 28.0 } else { 16.0 }),
            )
            .padding(20)
            .style(move |t| {
                if is_hovered {
                    container::rounded_box(t)
                } else {
                    container::Style::default()
                }
            })
        });

        let nested = hovered(|outer| {
            column![
                text(if outer { "Outer hovered" } else { "Hover anywhere" }),
                hovered(|inner| {
                    container(text(if inner { "Inner hovered!" } else { "Inner area" }))
                        .padding(10)
                        .style(move |t| {
                            if inner {
                                container::dark(t)
                            } else {
                                container::rounded_box(t)
                            }
                        })
                }),
            ]
            .spacing(5)
            .padding(10)
        });

        center(column![label_style, text_size, nested].spacing(30)).into()
    }
}
