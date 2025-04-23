mod color_picker;

use color_picker::color_picker;

use iced::{
    Alignment, Color, Element, Length, Theme,
    widget::{Button, Container, Row, Text, column, pick_list},
};

fn main() -> iced::Result {
    iced::application(
        ColorPickerExample::default,
        ColorPickerExample::update,
        ColorPickerExample::view,
    )
    .title("Color Picker Example")
    .theme(ColorPickerExample::theme)
    // .font(iced_fonts::REQUIRED_FONT_BYTES)
    .run()
}

#[derive(Clone, Debug)]
#[allow(clippy::enum_variant_names)]
enum Message {
    ChooseColor,
    SubmitColor(Color),
    CancelColor,
    Theme(Theme),
}

#[derive(Debug)]
struct ColorPickerExample {
    color: Color,
    show_picker: bool,
    theme: Theme,
}

impl Default for ColorPickerExample {
    fn default() -> Self {
        Self {
            color: Color::from_rgba8(0, 0, 0, 1.0),
            show_picker: false,
            theme: Theme::TokyoNightStorm,
        }
    }
}
impl ColorPickerExample {
    fn update(&mut self, message: Message) {
        match message {
            Message::ChooseColor => {
                self.show_picker = true;
            }
            Message::SubmitColor(color) => {
                self.color = color;
                self.show_picker = false;
            }
            Message::CancelColor => {
                self.show_picker = false;
            }
            Message::Theme(theme) => {
                self.theme = theme;
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let theme_picker = pick_list(Theme::ALL, Some(&self.theme), Message::Theme);
        let but = Button::new(Text::new("Set Color")).on_press(Message::ChooseColor);

        let color_picker = color_picker(
            self.show_picker,
            self.color,
            but,
            Message::CancelColor,
            Message::SubmitColor,
        );

        let row = Row::new()
            .align_y(Alignment::Center)
            .spacing(10)
            .push(color_picker)
            .push(Text::new(format!("Color: {:?}", self.color)));

        let col = column![theme_picker, row,].spacing(20);

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
