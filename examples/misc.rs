use iced_picker::{
    helpers::button_separator,
    modal::{self, modal},
    number_input::NumberInput,
    text_input::TextInput,
    tooltip::{Open, Position, tooltip},
};
use iced::{
    Border, Center, Element, Fill, Shrink, Theme,
    widget::{button, center, column, container, row, text},
};

fn main() -> iced::Result {
    iced::application(App::default, App::update, App::view)
        .title("Misc Widgets - iced_picker")
        .theme(App::theme)
        .run()
}

#[derive(Debug)]
struct App {
    amount: f64,
    show_modal: bool,
    last_action: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            amount: 42.50,
            show_modal: false,
            last_action: None,
        }
    }
}

#[derive(Clone, Debug)]
enum Message {
    AmountChanged(f64),
    OpenModal,
    CloseModal,
    Confirm,
    Save,
    SaveAs,
    Backup,
}

impl App {
    fn theme(&self) -> Theme {
        Theme::TokyoNightStorm
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::AmountChanged(v) => self.amount = v,
            Message::OpenModal => self.show_modal = true,
            Message::CloseModal => self.show_modal = false,
            Message::Confirm => {
                self.show_modal = false;
                self.last_action = Some(format!("Confirmed: €{:.2}", self.amount));
            }
            Message::Save => self.last_action = Some("Saved".into()),
            Message::SaveAs => self.last_action = Some("Save As...".into()),
            Message::Backup => self.last_action = Some("Backup created".into()),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let amount_row = row![
            text("Amount:").width(100),
            NumberInput::new("0.00", self.amount)
                .on_input(Message::AmountChanged)
                .min(0.0_f64)
                .max(999_999.99_f64),
        ]
        .spacing(10)
        .align_y(Center);

        let formatted_row = row![
            text("Formatted:").width(100),
            TextInput::new("€0.00", format!("€{:.2}", self.amount))
                .padding([0, 5])
                .line_height(1.85),
        ]
        .spacing(10)
        .align_y(Center);

        let modal_btn = button("Open Confirmation Modal").on_press(Message::OpenModal);

        let last_action = text(match &self.last_action {
            Some(s) => format!("Last action: {s}"),
            None => "No action yet".into(),
        });

        let content = column![
            text("Number Input + Formatted TextInput").size(18),
            amount_row,
            formatted_row,
            iced::widget::rule::horizontal(1.0),
            text("Modal").size(18),
            modal_btn,
            last_action,
            iced::widget::rule::horizontal(1.0),
            text("Save with Dropdown").size(18),
            save_buttons_row(),
        ]
        .spacing(12)
        .padding(30)
        .width(Shrink);

        let modal_content = container(
            column![
                text("Are you sure?").size(20),
                text(format!("Save with amount: €{:.2}", self.amount)),
                row![
                    button("Cancel")
                        .on_press(Message::CloseModal)
                        .style(button::secondary),
                    button("Confirm")
                        .on_press(Message::Confirm)
                        .style(button::primary),
                ]
                .spacing(10),
            ]
            .spacing(15),
        )
        .padding(25)
        .style(modal::default);

        modal(
            center(content),
            self.show_modal.then_some(modal_content),
            Message::CloseModal,
        )
    }
}

fn save_buttons_row() -> Element<'static, Message> {
    let save_btn = button("Save")
        .on_press(Message::Save)
        .style(|t: &Theme, s| iced::widget::button::Style {
            border: Border {
                radius: iced::border::left(4.0),
                ..button::primary(t, s).border
            },
            ..button::primary(t, s)
        });

    let dropdown_items = container(
        column![
            button("Save As...")
                .width(Fill)
                .on_press(Message::SaveAs)
                .style(|t: &Theme, s| match s {
                    button::Status::Active => button::text(t, s),
                    _ => button::background(t, s),
                }),
            button("Backup")
                .width(Fill)
                .on_press(Message::Backup)
                .style(|t: &Theme, s| match s {
                    button::Status::Active => button::text(t, s),
                    _ => button::background(t, s),
                }),
        ]
        .spacing(2)
        .width(Shrink),
    )
    .padding(5)
    .style(container::bordered_box);

    let dropdown_trigger = button(text("▾").size(12).center())
        .style(|t: &Theme, _| {
            let s = button::Status::Active;
            iced::widget::button::Style {
                border: Border {
                    radius: iced::border::right(4.0),
                    ..button::primary(t, s).border
                },
                ..button::primary(t, s)
            }
        });

    let dropdown = tooltip(dropdown_trigger, dropdown_items)
        .open(Open::LeftPointer)
        .position(Position::BottomLeft)
        .gap(2.0);

    row![save_btn, button_separator(), dropdown]
        .align_y(Center)
        .into()
}
