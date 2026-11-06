use iced::{
    Border, Center, Color, Fill, Font, Length, Shrink, Theme, Widget,
    widget::{
        Button, Column, Text, button, checkbox, column, container, pick_list, row, space,
        text_input,
    },
};
use iced_picker::color_picker::{self, ButtonStatus, color_picker};

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
    CustomStyle(bool),
}

#[derive(Debug)]
struct ColorPickerExample {
    color: Color,
    show_picker: bool,
    show_picker2: bool,
    theme: Theme,
    custom_style: bool,
}

impl Default for ColorPickerExample {
    fn default() -> Self {
        Self {
            color: Color::from_rgba8(100, 149, 237, 1.0),
            show_picker: false,
            show_picker2: false,
            theme: Theme::TokyoNightStorm,
            custom_style: false,
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
            Message::CustomStyle(enabled) => self.custom_style = enabled,
        }
    }

    fn view(&self) -> impl Widget<Message> {
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

        let picker1 = if self.custom_style {
            picker1
                .style(|theme: &Theme, status| {
                    let palette = theme.palette();
                    color_picker::Style {
                        background: palette.background.weak.color.into(),
                        border: Border {
                            color: palette.primary.base.color,
                            width: 2.0,
                            radius: 10.0.into(),
                        },
                        shadow: iced::Shadow {
                            color: Color::BLACK.scale_alpha(0.35),
                            offset: iced::Vector::new(3.0, 4.0),
                            blur_radius: 12.0,
                        },
                        bar_background: palette.background.strong.color.into(),
                        sat_value_marker: Some(Color::WHITE),
                        hue_marker: Color::WHITE,
                        ..color_picker::primary(theme, status)
                    }
                })
                .font(Font::MONOSPACE)
                .channel_labels("Red", "Green", "Blue", "Alpha")
                .cancel_content(Text::new("Cancel").width(Fill).align_x(Center))
                .submit_content(Text::new("Apply").width(Fill).align_x(Center))
                .submit_label("Apply")
                .button_style(|theme, status| {
                    let palette = theme.palette();
                    let (background, text_color) = match status {
                        ButtonStatus::Active => {
                            (palette.secondary.base.color, palette.secondary.base.text)
                        }
                        ButtonStatus::Hovered => {
                            (palette.primary.strong.color, palette.primary.strong.text)
                        }
                        ButtonStatus::Pressed => {
                            (palette.success.strong.color, palette.success.strong.text)
                        }
                        ButtonStatus::Focused { is_hovered } => (
                            if is_hovered {
                                palette.warning.strong.color
                            } else {
                                palette.warning.base.color
                            },
                            palette.warning.base.text,
                        ),
                    };
                    button::Style {
                        background: Some(background.into()),
                        text_color,
                        border: Border {
                            radius: 8.0.into(),
                            ..Border::default()
                        },
                        ..button::Style::default()
                    }
                })
                .hex_input_style(|theme, state| {
                    let palette = theme.palette();
                    let base = text_input::default(theme, state.status);
                    text_input::Style {
                        background: palette.background.base.color.into(),
                        value: state.color,
                        border: Border {
                            color: if state.is_valid {
                                palette.primary.base.color
                            } else {
                                palette.danger.base.color
                            },
                            width: 2.0,
                            radius: 8.0.into(),
                        },
                        ..base
                    }
                })
        } else {
            picker1
        };
        let custom_style = checkbox(self.custom_style)
            .label("Custom style")
            .on_toggle(Message::CustomStyle);

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

        let content: Column<_> = column![theme_picker, custom_style, pickers_row, color_display]
            .spacing(20)
            .width(Shrink);

        container(content)
            .width(Fill)
            .height(Length::Fill)
            .center_x(Fill)
            .center_y(Fill)
    }
}
