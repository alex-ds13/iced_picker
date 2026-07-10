use iced::{
    Element, Fill, Theme,
    widget::{button, center, checkbox, column, container, row, scrollable, space, text},
};
use iced_picker::reorder::reorder;

fn main() -> iced::Result {
    iced::application(App::default, App::update, App::view)
        .title("Reorder - iced_picker")
        .theme(App::theme)
        .run()
}

struct App {
    items: Vec<String>,
    last: Option<String>,
    live: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            items: (1..=8).map(|n| format!("Item {n}")).collect(),
            last: None,
            live: false,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Clicked(usize),
    Reordered(usize, usize),
    ToggleLive(bool),
}

impl App {
    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Clicked(index) => {
                self.last = self.items.get(index).cloned();
            }
            Message::Reordered(from, to) => {
                let item = self.items.remove(from);
                self.items.insert(to, item);
            }
            Message::ToggleLive(live) => {
                self.live = live;
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let rows = self.items.iter().enumerate().map(|(index, label)| {
            button(text(label).size(16))
                .on_press(Message::Clicked(index))
                .width(Fill)
                .padding(12)
                .style(button::subtle)
                .into()
        });

        let list = reorder(rows)
            .spacing(0)
            .live(self.live)
            .on_reorder(Message::Reordered);

        let clicked = text(match &self.last {
            Some(label) => format!("Last clicked: {label}"),
            None => "Drag a row to reorder, or click one.".to_string(),
        })
        .size(13);

        let live_toggle = checkbox(self.live)
            .label("Live reorder")
            .text_size(13)
            .on_toggle(Message::ToggleLive);

        let content = column![
            row![clicked, space::horizontal(), live_toggle],
            scrollable(list).height(Fill)
        ]
        .spacing(12)
        .width(iced::Fit.max(360));

        center(container(content).padding(20)).into()
    }
}
