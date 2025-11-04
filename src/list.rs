use iced::widget::{
    button, center, column, container, space, row, scrollable,
    text,
};
use iced::{Alignment, Element, Length, Theme};

pub fn main() -> iced::Result {
    iced::application(List::default, List::update, List::view)
        .title("List - Iced")
        .theme(List::theme)
        .run()
}

struct List {
    content: Vec<(usize, State)>,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Update(usize),
    Remove(usize),
}

impl List {
    fn theme(&self) -> Theme {
        Theme::TokyoNight
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Update(index) => {
                if let Some((_id, state)) = self.content.get_mut(index) {
                    *state = State::Updated;
                }
            }
            Message::Remove(index) => {
                let _ = self.content.remove(index);
            }
        }
    }

    fn view(&self) -> Element<Message> {
        center(
            scrollable(
                container(listview::listview(&self.content, |index, (id, state), selected| {
                    row![
                        match state {
                            State::Idle =>
                                Element::from(text(format!("I am item {id}!"))),
                            State::Updated => center(
                                column![
                                    text(format!("I am item {id}!")),
                                    text("... but different!")
                                ]
                                .spacing(20)
                            )
                            .height(300)
                            .into(),
                        },
                        space::horizontal(),
                        button("Update").on_press_maybe(
                            matches!(state, State::Idle)
                                .then_some(Message::Update(index))
                        ),
                        button("Remove")
                            .on_press(Message::Remove(index))
                            .style(button::danger)
                    ]
                    .spacing(10)
                    .padding(5)
                    .align_items(Alignment::Center)
                    .into()
                }))
                .padding(10),
            )
            .width(Length::Fill),
        )
        .padding(10)
        .into()
    }
}

impl Default for List {
    fn default() -> Self {
        Self {
            content: (0..1_000).map(|id| (id, State::Idle)).to_vec(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Updated,
}

mod listview {
    use std::{
        any::Any,
        collections::{HashMap, HashSet},
    };

    use iced::{
        Element, Renderer, Task, Theme,
        advanced::widget::{Operation, operate},
        widget::{Component, Id, button, column, component, container, sensor, space},
    };

    pub static ITEM_HEIGHT: f32 = 40.8;

    pub struct ListView<'a, 'b, T, Message> {
        id: Option<Id>,
        items: Vec<T>,
        single_selection: bool,
        filtered_ids: Option<HashSet<usize>>,
        view_item: Box<dyn Fn(usize, T, bool) -> Element<'a, Message> + 'b>,
        on_selected: Option<Box<dyn Fn(usize, f32) -> Message + 'b>>,
        on_deselected: Option<Box<dyn Fn(usize) -> Message + 'b>>,
    }

    impl<'a, 'b, T, Message> ListView<'a, 'b, T, Message> {
        pub fn new<F, E>(items: impl IntoIterator<Item = T>, view_item: F) -> Self
        where
            F: Fn(usize, T, bool) -> E + 'b,
            E: Into<Element<'a, Message>>,
        {
            Self {
                id: None,
                items: items.into_iter().collect(),
                single_selection: false,
                filtered_ids: None,
                view_item: Box::new(move |idx, item, selected| view_item(idx, item, selected).into()),
                on_selected: None,
                on_deselected: None,
            }
        }

        pub fn id(mut self, id: impl Into<Id>) -> Self {
            self.id = Some(id.into());
            self
        }

        pub fn single_selection(mut self) -> Self {
            self.single_selection = true;
            self
        }

        pub fn multiple_selection(mut self) -> Self {
            self.single_selection = false;
            self
        }

        pub fn filter(mut self, filtered_ids: impl IntoIterator<Item = usize>) -> Self {
            self.filtered_ids = Some(HashSet::from_iter(filtered_ids));
            self
        }

        pub fn filter_maybe(mut self, filtered_ids: Option<impl IntoIterator<Item = usize>>) -> Self {
            self.filtered_ids = filtered_ids.map(HashSet::from_iter);
            self
        }

        pub fn on_selected(mut self, on_selected: impl Fn(usize, f32) -> Message + 'b) -> Self {
            self.on_selected = Some(Box::new(on_selected));
            self
        }

        pub fn on_deselected(mut self, on_deselected: impl Fn(usize) -> Message + 'b) -> Self {
            self.on_deselected = Some(Box::new(on_deselected));
            self
        }

        fn make_hash(&self, state: &mut State) {
            use std::hash::{DefaultHasher, Hash, Hasher};

            let mut hasher = DefaultHasher::new();
            if let Some(filtered_ids) = &self.filtered_ids {
                filtered_ids.iter().for_each(|item| item.hash(&mut hasher));
            } else {
                self.items
                    .iter()
                    .enumerate()
                    .for_each(|(idx, _item)| idx.hash(&mut hasher));
            }
            state.hash = hasher.finish();
        }

        pub fn offset(&self, state: &State, idx: usize) -> f32 {
            let filtered_heights_until_idx = state.heights.iter().filter(|(i, _)| {
                self.filtered_ids.as_ref().is_none_or(|fi| fi.contains(i)) && **i < idx
            });

            filtered_heights_until_idx.fold(0.0, |offset, (i, eh)| {
                let h = eh.height(state.selected.contains(i));
                let height = match h {
                    Height::Unknown => ITEM_HEIGHT,
                    Height::Known(height) => height,
                };
                offset + height
            })
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct State {
        initialized: bool,
        selected: HashSet<usize>,
        visible: HashSet<usize>,
        hash: u64,
        heights: HashMap<usize, ElementHeight>,
        filtered_ids: Option<HashSet<usize>>,
    }
    impl State {
        fn clear_selection(&mut self) {
            self.selected.clear();
        }
    }

    pub fn clear_selection<T>(target: Id) -> impl Operation<T> {
        struct Cleareable {
            target: Id,
        }

        impl<T> Operation<T> for Cleareable {
            fn custom(&mut self, id: Option<&Id>, _bounds: iced::Rectangle, state: &mut dyn Any) {
                if Some(&self.target) == id {
                    let state = state
                        .downcast_mut::<State>()
                        .expect("Downcast ListView state");
                    state.clear_selection();
                }
            }

            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
                operate(self)
            }
        }

        Cleareable { target }
    }

    pub fn clear_selection_task<T: Send + 'static>(id: impl Into<Id>) -> Task<T> {
        operate(clear_selection(id.into()))
    }

    #[derive(Clone, Debug)]
    pub enum Event<Message> {
        Select(usize),
        Deselect(usize),
        ShowItem(usize, f32),
        HideItem(usize),
        ItemResize(usize, f32),
        Content(Message),
    }

    impl<'a, T: Clone, Message: Clone + 'a> Component<'a, Message> for ListView<'a, '_, T, Message> {
        type State = State;

        type Event = Event<Message>;

        fn update(
            &mut self,
            state: &mut Self::State,
            event: Self::Event,
            _renderer: &Renderer,
        ) -> Option<Message> {
            match event {
                Event::Select(idx) => {
                    if self.single_selection && !state.selected.is_empty() {
                        state.selected.clear();
                        state.selected.insert(idx);
                    } else {
                        state.selected.insert(idx);
                    }
                    let offset = self.offset(state, idx);
                    return self.on_selected.as_ref().map(|f| f(idx, offset));
                }
                Event::Deselect(idx) => {
                    state.selected.remove(&idx);
                    return self.on_deselected.as_ref().map(|f| f(idx));
                }
                Event::ShowItem(idx, height) => {
                    state.visible.insert(idx);
                    if height > 0.0001 {
                        if let Some(h) = state.heights.get_mut(&idx) {
                            if state.selected.contains(&idx) {
                                h.selected(height);
                            } else {
                                h.unselected(height);
                            }
                        } else {
                            let mut h = ElementHeight::unknown();
                            if state.selected.contains(&idx) {
                                h.selected(height);
                            } else {
                                h.unselected(height);
                            }
                            state.heights.insert(idx, h);
                        }
                    }
                }
                Event::HideItem(idx) => {
                    state.visible.remove(&idx);
                }
                Event::ItemResize(idx, height) => {
                    if let Some(h) = state.heights.get_mut(&idx) {
                        if state.selected.contains(&idx) {
                            h.selected(height);
                        } else {
                            h.unselected(height);
                        }
                    } else {
                        let mut h = ElementHeight::unknown();
                        if state.selected.contains(&idx) {
                            h.selected(height);
                        } else {
                            h.unselected(height);
                        }
                        state.heights.insert(idx, h);
                    }
                }
                Event::Content(message) => {
                    return Some(message);
                }
            };
            None
        }

        fn view(&self, state: &Self::State) -> Element<'a, Self::Event, Theme, Renderer> {
            let content = self
                .items
                .iter()
                .enumerate()
                .fold(column![], |col, (idx, item)| {
                    if self
                        .filtered_ids
                        .as_ref()
                        .is_none_or(|fi| fi.contains(&idx))
                    {
                        let selected = state.selected.contains(&idx);
                        let el_height = state
                            .heights
                            .get(&idx)
                            .map(|eh| match eh.height(selected) {
                                Height::Unknown => ITEM_HEIGHT,
                                Height::Known(height) => height,
                            })
                            .unwrap_or(ITEM_HEIGHT);
                        let el: Element<_> = if state.visible.contains(&idx) {
                            let selected = state.selected.contains(&idx);
                            let item = container(
                                (self.view_item)(idx, item.clone(), selected).map(Event::Content),
                            );
                            button(item)
                                .on_press(if !selected {
                                    Event::Select(idx)
                                } else {
                                    Event::Deselect(idx)
                                })
                                .style(if selected {
                                    button::success
                                } else {
                                    button::primary
                                })
                                .into()
                        } else {
                            space::horizontal().height(el_height).into()
                        };
                        let s = sensor(el)
                            .on_show(move |size| Event::ShowItem(idx, size.height))
                            .on_hide(Event::HideItem(idx))
                            .on_resize(move |size| Event::ItemResize(idx, size.height))
                            .key(state.hash)
                            .anticipate(ITEM_HEIGHT * 5.0);
                        col.push(s)
                    } else {
                        col
                    }
                });

            content.into()
        }

        fn operate(
            &self,
            state: &mut Self::State,
            bounds: iced::Rectangle,
            operation: &mut dyn Operation,
        ) {
            operation.custom(self.id.as_ref(), bounds, state);
        }

        fn diff(&self, state: &mut Self::State) {
            if !state.initialized {
                state.heights = self
                    .items
                    .iter()
                    .enumerate()
                    .map(|(idx, _)| (idx, ElementHeight::unknown()))
                    .collect();
                state.initialized = true;
            }

            if state.heights.len() != self.items.len() {
                // The items probably changed so we need to recreate the heights map
                state.heights = self
                    .items
                    .iter()
                    .enumerate()
                    .map(|(idx, _)| (idx, ElementHeight::unknown()))
                    .collect();
            }

            if self.filtered_ids != state.filtered_ids {
                state.filtered_ids = self.filtered_ids.clone();
                self.make_hash(state);
            }
        }
    }

    #[derive(Clone, Debug)]
    struct ElementHeight {
        selected: Height,
        unselected: Height,
    }
    impl ElementHeight {
        fn unknown() -> Self {
            Self {
                selected: Height::Unknown,
                unselected: Height::Unknown,
            }
        }

        fn selected(&mut self, height: impl Into<Height>) {
            self.selected = height.into();
        }

        fn unselected(&mut self, height: impl Into<Height>) {
            self.unselected = height.into();
        }

        fn height(&self, selected: bool) -> Height {
            if selected {
                self.selected
            } else {
                self.unselected
            }
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Height {
        Unknown,
        Known(f32),
    }
    impl From<f32> for Height {
        fn from(value: f32) -> Self {
            Height::Known(value)
        }
    }

    impl<'a, 'b: 'a, T: Clone + 'a, Message: Clone + 'a> From<ListView<'a, 'b, T, Message>>
        for Element<'a, Message>
    {
        fn from(value: ListView<'a, 'b, T, Message>) -> Self {
            component(value)
        }
    }

    pub fn listview<'a, 'b, T: 'a, Message, F, E>(
        items: impl IntoIterator<Item = T>,
        view_item: F,
    ) -> ListView<'a, 'b, T, Message>
    where
        F: Fn(usize, T, bool) -> E + 'b,
        E: Into<Element<'a, Message>>,
    {
        ListView::new(items, view_item)
    }
}
