use iced_picker::listview::listview;
use iced::{
    Alignment, Element, Fill, Length, Shrink, Task, Theme,
    widget::{
        button, center, column, container, operation::AbsoluteOffset, row, scrollable, space,
        text, text_input,
    },
};

fn main() -> iced::Result {
    iced::application(List::default, List::update, List::view)
        .title("ListView - iced_picker")
        .theme(List::theme)
        .run()
}

struct List {
    content: Vec<(usize, ItemState)>,
    filter: usize,
    selected: Option<usize>,
}

#[derive(Debug, Clone)]
enum Message {
    Toggle(usize),
    Remove(usize),
    Selected(usize, f32),
    Deselected(usize),
    FilterChanged(String),
}

impl List {
    fn theme(&self) -> Theme {
        Theme::TokyoNight
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Toggle(pos) => {
                if let Some((_, state)) = self.content.get_mut(pos) {
                    *state = match state {
                        ItemState::Closed => ItemState::Opened,
                        ItemState::Opened => ItemState::Closed,
                    };
                }
            }
            Message::Remove(pos) => {
                self.content.remove(pos);
                self.selected = match self.selected {
                    Some(sel) if sel == pos => None,
                    Some(sel) if sel > pos => Some(sel - 1),
                    other => other,
                };
            }
            Message::Selected(pos, offset) => {
                self.selected = Some(pos);
                return iced::widget::operation::scroll_to(
                    "SCROLLABLE",
                    AbsoluteOffset { x: 0.0, y: offset },
                );
            }
            Message::Deselected(_) => {
                self.selected = None;
            }
            Message::FilterChanged(s) => {
                self.filter = s.parse().unwrap_or_default();
                self.selected = None;
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let filter_input = row![
            text("Show IDs ≥ "),
            text_input("0", &self.filter.to_string()).on_input(Message::FilterChanged),
        ]
        .spacing(5)
        .align_y(Alignment::Center);

        let filtered_positions = self
            .content
            .iter()
            .enumerate()
            .filter_map(|(pos, (id, _))| (*id >= self.filter).then_some(pos));

        let list = listview(
            &self.content,
            |pos, (id, state), _selected| -> Element<Message> {
                row![
                    match state {
                        ItemState::Closed => Element::from(text(format!("Item {id}"))),
                        ItemState::Opened => center(
                            column![
                                text(format!("Item {id} (expanded)")),
                                text("Extra detail row"),
                                text("Another detail row"),
                            ]
                            .spacing(8)
                        )
                        .height(120)
                        .into(),
                    },
                    space::horizontal().width(Fill),
                    button(match state {
                        ItemState::Closed => "Expand",
                        ItemState::Opened => "Collapse",
                    })
                    .on_press(Message::Toggle(pos)),
                    button("Remove")
                        .on_press(Message::Remove(pos))
                        .style(button::danger),
                ]
                .spacing(10)
                .padding(5)
                .align_y(Alignment::Center)
                .height(Shrink)
                .width(Length::Fill)
                .into()
            },
        )
        .on_selected(Message::Selected)
        .on_deselected(Message::Deselected)
        .select_maybe(self.selected)
        .single_selection()
        .filter(filtered_positions);

        let count = self
            .content
            .iter()
            .filter(|(id, _)| *id >= self.filter)
            .count();

        let status = text(format!(
            "{} of {} items shown{}",
            count,
            self.content.len(),
            self.selected
                .map(|pos| format!(
                    " · selected item {}",
                    self.content.get(pos).map(|(id, _)| *id).unwrap_or(pos)
                ))
                .unwrap_or_default()
        ));

        center(
            column![
                filter_input,
                status,
                scrollable(container(list).width(Fill).padding(10))
                    .id("SCROLLABLE")
                    .height(Fill),
            ]
            .spacing(10)
            .padding(10)
            .width(Fill)
            .height(Fill),
        )
        .into()
    }
}

impl Default for List {
    fn default() -> Self {
        Self {
            content: (0..500)
                .map(|id| {
                    (
                        id,
                        if id % 50 == 0 {
                            ItemState::Opened
                        } else {
                            ItemState::Closed
                        },
                    )
                })
                .collect(),
            filter: 0,
            selected: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemState {
    Closed,
    Opened,
}
