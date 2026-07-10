use iced::{
    Center, Color, Element, Fill, Length, Shrink, Theme,
    widget::{Button, Column, Text, column, container, pick_list, row, space},
};
use iced_picker::color_picker::color_picker;

fn main() -> iced::Result {
    iced::application(
        ColorPickerExample::default,
        ColorPickerExample::update,
        ColorPickerExample::view,
    )
    .title("Color Picker - iced_picker")
    .theme(ColorPickerExample::theme)
    .run()
}

#[derive(Clone, Debug)]
#[allow(clippy::enum_variant_names)]
enum Message {
    ChooseColor,
    ChooseColor2,
    SubmitColor(Color),
    CancelColor,
    Theme(Theme),
}

#[derive(Debug)]
struct ColorPickerExample {
    color: Color,
    show_picker: bool,
    show_picker2: bool,
    theme: Theme,
}

impl Default for ColorPickerExample {
    fn default() -> Self {
        Self {
            color: Color::from_rgba8(100, 149, 237, 1.0),
            show_picker: false,
            show_picker2: false,
            theme: Theme::TokyoNightStorm,
        }
    }
}

impl ColorPickerExample {
    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::ChooseColor => self.show_picker = true,
            Message::ChooseColor2 => self.show_picker2 = true,
            Message::SubmitColor(color) => {
                self.color = color;
                self.show_picker = false;
                self.show_picker2 = false;
            }
            Message::CancelColor => {
                self.show_picker = false;
                self.show_picker2 = false;
            }
            Message::Theme(theme) => self.theme = theme,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let theme_picker = pick_list(Some(self.theme.clone()), Theme::ALL, |t: &Theme| {
            t.to_string()
        })
        .on_select(Message::Theme);

        let picker1 = color_picker(
            self.show_picker,
            self.color,
            Button::new(Text::new("Set Color")).on_press(Message::ChooseColor),
            Message::CancelColor,
            Message::SubmitColor,
        );

        let picker2 = color_picker(
            self.show_picker2,
            self.color,
            Button::new(Text::new("Set Color 2")).on_press(Message::ChooseColor2),
            Message::CancelColor,
            Message::SubmitColor,
        );

        let color_swatch = container(space::horizontal())
            .width(40)
            .height(40)
            .style(move |_| container::Style {
                background: Some(self.color.into()),
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let color_info = column![
            row![
                Text::new("RGBA:"),
                Text::new(format!(
                    "({:.3}, {:.3}, {:.3}, {:.3})",
                    self.color.r, self.color.g, self.color.b, self.color.a
                )),
            ]
            .spacing(5)
            .align_y(Center),
            row![
                Text::new("Hex:"),
                Text::new(format!(
                    "#{:02X}{:02X}{:02X}{:02X}",
                    (self.color.r * 255.0) as u8,
                    (self.color.g * 255.0) as u8,
                    (self.color.b * 255.0) as u8,
                    (self.color.a * 255.0) as u8,
                )),
            ]
            .spacing(5)
            .align_y(Center),
        ]
        .spacing(5);

        let pickers_row = row![picker1, picker2].spacing(10).align_y(Center);
        let color_display = row![color_swatch, color_info].spacing(15).align_y(Center);

        let content: Column<Message> = column![theme_picker, pickers_row, color_display]
            .spacing(20)
            .width(Shrink);

        container(content)
            .width(Fill)
            .height(Length::Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into()
    }
}
