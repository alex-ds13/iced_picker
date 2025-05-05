mod color;
mod color_picker;
mod custom_widget;
mod hovered;
mod test_overlay;

use color_picker::color_picker;
use hovered::hovered;

use iced::{
    Alignment, Color, Element, Length, Theme,
    widget::{Button, Container, Row, Text, column, container, pick_list, row},
};

fn main() -> iced::Result {
    iced::application(
        ColorPickerExample::default,
        ColorPickerExample::update,
        ColorPickerExample::view,
    )
    .title("Color Picker Example")
    .theme(ColorPickerExample::theme)
    .run()
}

#[derive(Clone, Debug)]
#[allow(clippy::enum_variant_names)]
enum Message {
    ChooseColor,
    ChooseColor2,
    ShowTest,
    SubmitColor(Color),
    CancelColor,
    Theme(Theme),
}

#[derive(Debug)]
struct ColorPickerExample {
    color: Color,
    show_picker: bool,
    show_picker2: bool,
    show_test: bool,
    theme: Theme,
}

impl Default for ColorPickerExample {
    fn default() -> Self {
        Self {
            color: Color::from_rgba8(0, 0, 0, 1.0),
            show_picker: false,
            show_picker2: false,
            show_test: false,
            theme: Theme::TokyoNightStorm,
        }
    }
}
impl ColorPickerExample {
    fn update(&mut self, message: Message) {
        match message {
            Message::ShowTest => {
                self.show_test = true;
            }
            Message::ChooseColor => {
                self.show_picker = true;
            }
            Message::ChooseColor2 => {
                self.show_picker2 = true;
            }
            Message::SubmitColor(color) => {
                self.color = color;
                self.show_picker = false;
                self.show_picker2 = false;
            }
            Message::CancelColor => {
                self.show_picker = false;
                self.show_picker2 = false;
                self.show_test = false;
            }
            Message::Theme(theme) => {
                self.theme = theme;
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let theme_picker = pick_list(Theme::ALL, Some(&self.theme), Message::Theme);
        let but = Button::new(Text::new("Set Color")).on_press(Message::ChooseColor);

        let picker = color_picker(
            self.show_picker,
            self.color,
            but,
            Message::CancelColor,
            Message::SubmitColor,
        );

        let but = Button::new(Text::new("Set Color")).on_press(Message::ChooseColor2);

        let picker2 = color_picker(
            self.show_picker2,
            self.color,
            but,
            Message::CancelColor,
            Message::SubmitColor,
        );

        // let color = color::color(self.color, Message::CancelColor, Message::SubmitColor);
        let base = container(iced::widget::row![
            Button::new("Press Me1!").on_press(Message::CancelColor),
            iced::widget::horizontal_space(),
            Button::new("Press Me2!").on_press(Message::CancelColor),
            iced::widget::horizontal_space(),
            Button::new("Press Me3!").on_press(Message::CancelColor),
        ])
        .center_x(iced::Fill)
        .center_y(300)
        .style(container::dark);

        let but = Button::new(Text::new("Set Color")).on_press(Message::ShowTest);
        let bar = test_overlay::Bar::new(self.show_test, but.into(), Message::CancelColor);
        let row = Row::new()
            .align_y(Alignment::Center)
            .spacing(10)
            .push(picker)
            .push(Text::new(format!("Color: {:?}", self.color)));

        let hovered_example = hovered(|is_hovered| {
            println!("is_hovered: {}", is_hovered);
            container(row!["foo", if is_hovered { "bar" } else { "baz" }, "qux"].spacing(10))
                .style(container::dark)
        });
        let col = column![theme_picker, picker2, bar, base, row, hovered_example].spacing(20);

        Container::new(col)
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
