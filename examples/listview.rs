use iced::{
    Alignment, Fill, Length, Shrink, Task, Theme, Widget,
    widget::{
        button, center, checkbox, column, component, container,
        operation::{
            Animation,
            scrollable::{AbsoluteOffset, scroll_to},
        },
        pick_list, row, scrollable, space, text, text_input,
    },
};
use iced_picker::listview::listview;
use std::collections::HashSet;

fn main() -> iced::Result {
    iced::application(List::default, List::update, List::view)
        .title("ListView - iced_picker")
        .theme(List::theme)
        .run()
}

struct List {
    content: Vec<(usize, ItemState)>,
    filter: usize,
    selection: HashSet<usize>,
    single_selection: bool,
    scroll_to_selected: bool,
    theme: Theme,
}

#[derive(Debug, Clone)]
enum Message {
    Toggle(usize),
    Remove(usize),
    Selected(usize, f32),
    Deselected(usize),
    FilterChanged(String),
    SingleSelectionToggled(bool),
    ScrollToSelectedToggled(bool),
    ClearSelection,
    ThemeChanged(Theme),
}

impl List {
    fn theme(&self) -> Theme {
        self.theme.clone()
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
                self.selection = self
                    .selection
                    .iter()
                    .filter(|&&i| i != pos)
                    .map(|&i| if i > pos { i - 1 } else { i })
                    .collect();
            }
            Message::Selected(pos, offset) => {
                if self.single_selection {
                    self.selection = HashSet::from([pos]);
                } else {
                    self.selection.insert(pos);
                }
                if self.scroll_to_selected {
                    return scroll_to(
                        "SCROLLABLE",
                        AbsoluteOffset { x: 0.0, y: offset },
                        Animation::Auto,
                    );
                }
            }
            Message::Deselected(pos) => {
                self.selection.remove(&pos);
            }
            Message::FilterChanged(s) => {
                self.filter = s.parse().unwrap_or_default();
                self.selection.clear();
            }
            Message::SingleSelectionToggled(v) => {
                self.single_selection = v;
                self.selection.clear();
            }
            Message::ScrollToSelectedToggled(v) => {
                self.scroll_to_selected = v;
            }
            Message::ClearSelection => {
                self.selection.clear();
            }
            Message::ThemeChanged(theme) => self.theme = theme,
        }
        Task::none()
    }

    fn view(&self) -> impl Widget<Message> {
        let theme_picker = pick_list(Some(self.theme.clone()), Theme::ALL, |t: &Theme| {
            t.to_string()
        })
        .on_select(Message::ThemeChanged);

        let options = row![
            checkbox(self.single_selection)
                .label("Single selection")
                .on_toggle(Message::SingleSelectionToggled),
            checkbox(self.scroll_to_selected)
                .label("Scroll to selected")
                .on_toggle(Message::ScrollToSelectedToggled),
            space::horizontal(),
            text("Theme:"),
            theme_picker,
        ]
        .spacing(20)
        .align_y(Alignment::Center);

        let filter_input = row![
            text("Show IDs ≥"),
            text_input("0", self.filter.to_string())
                .on_input(Message::FilterChanged)
                .width(80),
        ]
        .spacing(5)
        .align_y(Alignment::Center);

        let filtered_positions = self
            .content
            .iter()
            .enumerate()
            .filter_map(|(pos, (id, _))| (*id >= self.filter).then_some(pos));

        let list = {
            let base = listview(&self.content, |pos, (id, state), _selected| {
                row![
                    match state {
                        ItemState::Closed => text(format!("Item {id}")).boxed(),
                        ItemState::Opened => center(
                            column![
                                text(format!("Item {id} (expanded)")),
                                text("Extra detail row"),
                                text("Another detail row"),
                            ]
                            .spacing(8),
                        )
                        .height(120)
                        .boxed(),
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
                .boxed()
            })
            .on_selected(Message::Selected)
            .on_deselected(Message::Deselected)
            .filter(filtered_positions);

            let base = if self.single_selection {
                base.single_selection()
            } else {
                base.multiple_selection()
            };

            component(self.selection.iter().fold(base, |lv, &idx| lv.select(idx)))
        };

        let visible_count = self
            .content
            .iter()
            .filter(|(id, _)| *id >= self.filter)
            .count();

        let status = text(format!(
            "{} of {} items  ·  {} selected",
            visible_count,
            self.content.len(),
            self.selection.len(),
        ));

        let clear_btn = {
            let btn = button("Clear Selection").style(button::secondary);
            if self.selection.is_empty() {
                btn
            } else {
                btn.on_press(Message::ClearSelection)
            }
        };

        let status_row = row![status, space::horizontal(), clear_btn].align_y(Alignment::Center);

        center(
            column![
                options,
                filter_input,
                status_row,
                container(
                    scrollable(container(list).width(Fill).padding(10))
                        .id("SCROLLABLE")
                        .height(Fill),
                )
                .style(container::bordered_box)
                .height(Fill),
            ]
            .spacing(10)
            .padding(10)
            .width(Fill)
            .height(Fill),
        )
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
            selection: HashSet::new(),
            single_selection: true,
            scroll_to_selected: false,
            theme: Theme::TokyoNight,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemState {
    Closed,
    Opened,
}
