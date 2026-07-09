use iced::{
    Element, Fill, Theme,
    widget::{button, center, column, container, scrollable, text},
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
}

impl Default for App {
    fn default() -> Self {
        Self {
            items: (1..=8).map(|n| format!("Item {n}")).collect(),
            last: None,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Clicked(usize),
    Reordered(usize, usize),
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
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let rows = self.items.iter().enumerate().map(|(index, label)| {
            button(text(label).size(16))
                .on_press(Message::Clicked(index))
                .width(Fill)
                .padding(12)
                .into()
        });

        let list = reorder(rows).spacing(4).on_reorder(Message::Reordered);

        let clicked = text(match &self.last {
            Some(label) => format!("Last clicked: {label}"),
            None => "Drag a row to reorder, or click one.".to_string(),
        })
        .size(13);

        let content = column![clicked, scrollable(list).height(Fill)]
            .spacing(12)
            .width(360);

        center(container(content).padding(20)).into()
    }
}
