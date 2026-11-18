//! A contact list with thousands of rows: keyed items of different heights, an
//! application-owned filter and selection, groups whose header sticks while their
//! phone numbers scroll by, and the list's tasks.
//!
//! Try the keyboard after clicking a row: the arrows, Page Up/Down, Home/End,
//! Space, Ctrl+A, Escape and Enter. Moves that leave the view scroll it, through
//! the task the update returns.

use std::collections::HashSet;

use iced::widget::operation::Animation;
use iced::widget::{
    Id, button, checkbox, column, container, pick_list, row, scrollable, space, sticky, text,
    text_input,
};
use iced::{Alignment, Element, Fill, Task, Theme, Widget};
use iced_picker::listview::{self, Action, Align, Selection, list_view};

const SCROLL: Id = Id::new("contacts-scroll");
const LIST: Id = Id::new("contacts");

fn main() -> iced::Result {
    iced::application(Contacts::default, Contacts::update, Contacts::view)
        .title("ListView - iced_picker")
        .theme(Contacts::theme)
        .run()
}

struct Contact {
    id: u64,
    name: String,
    note: String,
    phones: Vec<String>,
}

impl Contact {
    fn new(id: u64) -> Self {
        Self {
            id,
            name: format!("Contact {id}"),
            note: String::new(),
            phones: (0..3 + id % 12)
                .map(|n| format!("+1 555 {:04}", (id * 7 + n) % 10_000))
                .collect(),
        }
    }
}

/// The id of the note input inside the item of contact `id`.
fn note_input(id: u64) -> Id {
    Id::from(format!("note-{id}"))
}

struct Contacts {
    contacts: Vec<Contact>,
    /// The ids of `contacts`.
    ids: HashSet<u64>,
    /// The positions in `contacts` that match the query, which the list shows.
    visible: Vec<usize>,
    query: String,
    selection: Selection<u64>,
    expanded: HashSet<u64>,
    multiple: bool,
    smooth: bool,
    target: String,
    next_id: u64,
    theme: Theme,
}

#[derive(Debug, Clone)]
enum Message {
    QueryChanged(String),
    Select(Action<u64>),
    Toggle(u64),
    Remove(u64),
    NoteChanged(u64, String),
    Edit(u64),
    AddAtTop,
    Settle,
    TargetChanged(String),
    ScrollTo(Align),
    EditTarget,
    MultipleToggled(bool),
    SmoothToggled(bool),
    ThemeChanged(Theme),
}

impl Default for Contacts {
    fn default() -> Self {
        let mut contacts = Self {
            contacts: (0..5_000).map(Contact::new).collect(),
            ids: (0..5_000).collect(),
            visible: Vec::new(),
            query: String::new(),
            selection: Selection::new(),
            expanded: (0..5_000).step_by(40).collect(),
            multiple: true,
            smooth: false,
            target: String::from("2500"),
            next_id: 5_000,
            theme: Theme::TokyoNight,
        };

        contacts.filter();
        contacts
    }
}

impl Contacts {
    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    /// The filter is the application's: it passes the matching positions.
    fn filter(&mut self) {
        self.visible = self
            .contacts
            .iter()
            .enumerate()
            .filter(|(_, contact)| contact.name.contains(self.query.as_str()))
            .map(|(position, _)| position)
            .collect();
    }

    fn target(&self) -> Option<u64> {
        self.target.trim().parse().ok()
    }

    fn animation(&self) -> Animation {
        if self.smooth {
            Animation::Smooth
        } else {
            Animation::Instant
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::QueryChanged(query) => {
                self.query = query;
                self.filter();
            }
            Message::Select(action) => {
                // Keyboard moves come with the key to bring into view. The list
                // cannot scroll its scrollable, so the update returns the task.
                let reveal = action.reveal().copied();
                self.selection.perform(action);

                // An action from an older view can carry keys that were removed
                // since. Contacts the filter hides stay selected.
                self.selection.retain(|id| self.ids.contains(id));

                if let Some(id) = reveal {
                    return listview::scroll_to(LIST, id, Align::Nearest, Animation::Instant);
                }
            }
            Message::Toggle(id) => {
                if !self.expanded.remove(&id) {
                    self.expanded.insert(id);
                }
            }
            Message::Remove(id) => {
                self.contacts.retain(|contact| contact.id != id);
                self.ids.remove(&id);
                self.expanded.remove(&id);
                self.selection.retain(|&key| key != id);
                self.filter();
            }
            Message::NoteChanged(id, note) => {
                if let Some(contact) = self.contacts.iter_mut().find(|contact| contact.id == id) {
                    contact.note = note;
                }
            }
            Message::Edit(id) => return listview::focus(LIST, id, note_input(id)),
            Message::AddAtTop => {
                // Items inserted above the viewport do not move what is on screen,
                // unless the list is at its start, where the new items appear.
                for _ in 0..50 {
                    self.contacts.insert(0, Contact::new(self.next_id));
                    self.ids.insert(self.next_id);
                    self.next_id += 1;
                }

                self.filter();
            }
            // Turns what the insertions added above the view into a scroll offset.
            Message::Settle => return listview::settle(LIST),
            Message::TargetChanged(target) => self.target = target,
            Message::ScrollTo(align) => {
                if let Some(id) = self.target() {
                    return listview::scroll_to(LIST, id, align, self.animation());
                }
            }
            // Focus works for items outside the window too: the list scrolls to the
            // item first, and focuses its input once it is laid out.
            Message::EditTarget => {
                if let Some(id) = self.target() {
                    return listview::focus(LIST, id, note_input(id));
                }
            }
            Message::MultipleToggled(multiple) => {
                self.multiple = multiple;

                // Single selection keeps one item: the cursor's if it is selected,
                // else the first selected one in the list's order.
                if !multiple && self.selection.len() > 1 {
                    let kept = self
                        .selection
                        .cursor()
                        .filter(|&id| self.selection.contains(id))
                        .copied()
                        .or_else(|| {
                            self.contacts
                                .iter()
                                .map(|contact| contact.id)
                                .find(|id| self.selection.contains(id))
                        });

                    if let Some(id) = kept {
                        self.selection.select(id);
                    }
                }
            }
            Message::SmoothToggled(smooth) => self.smooth = smooth,
            Message::ThemeChanged(theme) => self.theme = theme,
        }

        Task::none()
    }

    fn view(&self) -> impl Widget<Message> + '_ {
        let options = row![
            checkbox(self.multiple)
                .label("Multiple selection")
                .on_toggle(Message::MultipleToggled),
            checkbox(self.smooth)
                .label("Smooth scrolling")
                .on_toggle(Message::SmoothToggled),
            space::horizontal(),
            text("Theme:"),
            pick_list(Some(self.theme.clone()), Theme::ALL, Theme::to_string)
                .on_select(Message::ThemeChanged),
        ]
        .spacing(20)
        .align_y(Alignment::Center);

        let tasks = row![
            text("Contact"),
            text_input("id", &self.target)
                .on_input(Message::TargetChanged)
                .width(80),
            button("Start").on_press(Message::ScrollTo(Align::Start)),
            button("Center").on_press(Message::ScrollTo(Align::Center)),
            button("End").on_press(Message::ScrollTo(Align::End)),
            button("Nearest").on_press(Message::ScrollTo(Align::Nearest)),
            button("Edit note").on_press(Message::EditTarget),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let edits = row![
            text_input("Filter by name", &self.query)
                .on_input(Message::QueryChanged)
                .width(200),
            button("Add 50 at the top").on_press(Message::AddAtTop),
            button("Settle").on_press(Message::Settle),
            space::horizontal(),
            text(format!(
                "{} of {} contacts, {} selected",
                self.visible.len(),
                self.contacts.len(),
                self.selection.len()
            )),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let list = list_view(&self.visible, |_, &position| {
            self.item(&self.contacts[position])
        })
        .key(|&position| self.contacts[position].id)
        .id(LIST)
        .scroller(SCROLL)
        .estimated_height(44)
        .spacing(2)
        .selection(&self.selection, Message::Select)
        .multiple(self.multiple)
        .on_activate(Message::Toggle)
        .highlight(self.target());

        column![
            options,
            tasks,
            edits,
            // An embedded scrollbar keeps a stuck header, drawn as an overlay, from
            // covering it.
            container(scrollable(list).id(SCROLL).spacing(2).height(Fill))
                .style(container::bordered_box)
                .height(Fill),
        ]
        .spacing(10)
        .padding(10)
    }

    /// A contact: its header and note, and while it is expanded its phone numbers
    /// below a header that sticks to the top while they scroll by. The header toggles
    /// the group with its own button, which also works while it is stuck. The note
    /// stays out of the sticky header, so `listview::focus` always finds its input
    /// in the item and not in the stuck header's overlay.
    fn item<'a>(&self, contact: &'a Contact) -> Element<'a, Message> {
        let id = contact.id;
        let expanded = self.expanded.contains(&id);

        let header = row![
            text(&contact.name).width(Fill),
            button(if expanded { "Collapse" } else { "Expand" }).on_press(Message::Toggle(id)),
            button("Edit").on_press(Message::Edit(id)),
            button("Remove")
                .on_press(Message::Remove(id))
                .style(button::danger),
        ]
        .spacing(10)
        .padding(5)
        .align_y(Alignment::Center);

        let note = container(
            text_input("Note", &contact.note)
                .id(note_input(id))
                .on_input(move |note| Message::NoteChanged(id, note)),
        )
        .padding([0, 5]);

        if !expanded {
            return column![header, note]
                .padding(iced::Padding::ZERO.bottom(5))
                .boxed();
        }

        let header = container(header).style(container::rounded_box);
        let phones = contact
            .phones
            .iter()
            .map(|phone| container(text(phone)).padding([4, 40]).boxed());

        column![sticky(header), note].extend(phones).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use iced_picker::listview::Change;

    #[test]
    fn stale_actions_do_not_select_removed_contacts() {
        let mut app = Contacts::default();
        let before: Vec<u64> = app.contacts.iter().map(|contact| contact.id).collect();

        let _ = app.update(Message::Remove(40));

        // Actions resolved by a view from before the removal.
        let stale = [
            Change::All(before.clone()),
            Change::Range {
                anchor: 38,
                cursor: 42,
                keys: vec![38, 39, 40, 41, 42],
                additive: true,
            },
            Change::Select(40),
        ];

        for change in stale {
            let _ = app.update(Message::Select(change.into()));
            assert!(!app.selection.contains(&40));
            assert!(app.selection.len() <= app.contacts.len());
        }

        assert!(app.selection.is_empty());
    }

    #[test]
    fn single_selection_keeps_the_cursor_or_the_first_selected() {
        let mut app = Contacts::default();
        let _ = app.update(Message::Select(Change::All(vec![5, 7, 9]).into()));
        let _ = app.update(Message::Select(Change::Cursor(7).into()));
        let _ = app.update(Message::MultipleToggled(false));
        assert_eq!(app.selection.iter().copied().collect::<Vec<_>>(), [7]);

        let _ = app.update(Message::MultipleToggled(true));
        let _ = app.update(Message::Select(Change::All(vec![9, 5]).into()));
        let _ = app.update(Message::Select(Change::Cursor(1).into()));
        let _ = app.update(Message::MultipleToggled(false));
        assert_eq!(app.selection.iter().copied().collect::<Vec<_>>(), [5]);
    }
}
