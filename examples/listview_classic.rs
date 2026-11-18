//! The first list view example, ported to the current widget: 500 rows of different
//! heights in a bordered scrollable, with an application-owned filter and selection.
//!
//! The old widget wrapped every item in a button: a click selected an unselected item
//! and deselected a selected one, in single and multiple selection. The update below
//! gives mouse clicks those semantics. Ctrl and Shift clicks and the keyboard keep the
//! list's own.

use iced::widget::operation::Animation;
use iced::widget::{
    Id, button, center, checkbox, column, container, pick_list, row, scrollable, space, text,
    text_input,
};
use iced::{Alignment, Fill, Shrink, Task, Theme, Widget};
use iced_picker::listview::{self, Action, Align, Change, Selection, list_view};

const SCROLL: Id = Id::new("SCROLLABLE");
const LIST: Id = Id::new("LIST");

fn main() -> iced::Result {
    iced::application(List::default, List::update, List::view)
        .title("ListView classic - iced_picker")
        .theme(List::theme)
        .run()
}

struct List {
    content: Vec<(usize, ItemState)>,
    /// The positions in `content` of the items the filter lets through.
    visible: Vec<usize>,
    filter: usize,
    selection: Selection<usize>,
    single_selection: bool,
    scroll_to_selected: bool,
    theme: Theme,
}

#[derive(Debug, Clone)]
enum Message {
    Toggle(usize),
    Remove(usize),
    Select(Action<usize>),
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

    fn refilter(&mut self) {
        self.visible = self
            .content
            .iter()
            .enumerate()
            .filter_map(|(position, (id, _))| (*id >= self.filter).then_some(position))
            .collect();
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Toggle(id) => {
                if let Some((_, state)) = self.content.iter_mut().find(|(item, _)| *item == id) {
                    *state = match state {
                        ItemState::Closed => ItemState::Opened,
                        ItemState::Opened => ItemState::Closed,
                    };
                }
            }
            Message::Remove(id) => {
                self.content.retain(|(item, _)| *item != id);
                self.selection.retain(|&key| key != id);
                self.refilter();
            }
            Message::Select(action) => return self.select(action),
            Message::FilterChanged(s) => {
                self.filter = s.parse().unwrap_or_default();
                self.selection.clear();
                self.refilter();
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

    fn select(&mut self, action: Action<usize>) -> Task<Message> {
        let mut task = Task::none();

        match (action.reveal().copied(), &action.change) {
            // A plain mouse click toggles, as the old items, which were buttons, did.
            (None, &Change::Select(id)) => {
                let selected = self.selection.contains(&id);

                if selected || !self.single_selection {
                    self.selection.perform(Change::Toggle(id).into());
                } else {
                    self.selection.select(id);
                }

                if !selected && self.scroll_to_selected {
                    task = listview::scroll_to(LIST, id, Align::Start, Animation::Auto);
                }
            }
            (reveal, _) => {
                self.selection.perform(action);

                if let Some(id) = reveal {
                    task = listview::scroll_to(LIST, id, Align::Nearest, Animation::Instant);
                }
            }
        }

        // An action from an older view can carry ids that were removed since. The ids
        // stay sorted, as removing keeps the order.
        self.selection.retain(|id| {
            self.content
                .binary_search_by_key(id, |(item, _)| *item)
                .is_ok()
        });

        task
    }

    fn view(&self) -> impl Widget<Message> + '_ {
        let theme_picker = pick_list(Some(self.theme.clone()), Theme::ALL, Theme::to_string)
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

        let list = list_view(&self.visible, |_, &position| {
            let (id, state) = &self.content[position];
            let id = *id;

            // The old items were buttons, whose default padding surrounded the row.
            container(
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
                    .on_press(Message::Toggle(id)),
                    button("Remove")
                        .on_press(Message::Remove(id))
                        .style(button::danger),
                ]
                .spacing(10)
                .padding(5)
                .align_y(Alignment::Center)
                .height(Shrink)
                .width(Fill),
            )
            .padding([5, 10])
        })
        .key(|&position| self.content[position].0)
        .id(LIST)
        .scroller(SCROLL)
        .selection(&self.selection, Message::Select)
        .multiple(!self.single_selection);

        let status = text(format!(
            "{} of {} items  ·  {} selected",
            self.visible.len(),
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
                        .id(SCROLL)
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
        let mut list = Self {
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
            visible: Vec::new(),
            filter: 0,
            selection: Selection::new(),
            single_selection: true,
            scroll_to_selected: false,
            theme: Theme::TokyoNight,
        };

        list.refilter();
        list
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemState {
    Closed,
    Opened,
}
