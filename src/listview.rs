use std::collections::{HashMap, HashSet};

use iced::{
    Element, Fill, Renderer, Shrink, Theme,
    widget::{Action, Component, Id, button, column, component, container, sensor, space},
};

fn listview_item_style(
    theme: &Theme,
    status: button::Status,
    selected: bool,
    highlighted: bool,
) -> button::Style {
    let palette = theme.palette();

    let text_color = match status {
        button::Status::Disabled => palette.secondary.strong.color,
        _ => palette.background.base.text,
    };

    let tint = |color: iced::Color, alpha: f32| iced::Color { a: alpha, ..color };

    let background = match (status, selected) {
        (button::Status::Disabled, _) => None,

        (button::Status::Active, true) => Some(tint(palette.primary.base.color, 0.25).into()),
        (button::Status::Hovered, true) => Some(tint(palette.primary.base.color, 0.35).into()),
        (button::Status::Pressed, true) => Some(tint(palette.primary.base.color, 0.45).into()),

        (button::Status::Active, false) => None,
        (button::Status::Hovered, false) => Some(tint(palette.background.base.text, 0.06).into()),
        (button::Status::Pressed, false) => Some(tint(palette.background.base.text, 0.12).into()),
    };

    let (border, shadow) = if highlighted {
        let accent = palette.warning.base.color;
        (
            iced::Border {
                color: accent,
                width: 1.5,
                radius: 0.0.into(),
            },
            iced::Shadow {
                color: iced::Color { a: 0.45, ..accent },
                offset: iced::Vector::new(0.0, 0.0),
                blur_radius: 6.0,
            },
        )
    } else {
        (
            iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            iced::Shadow::default(),
        )
    };

    button::Style {
        background,
        text_color,
        border,
        shadow,
        snap: false,
    }
}

pub static ITEM_HEIGHT: f32 = 40.8;

pub struct ListView<'a, 'b, T, Message> {
    id: Option<Id>,
    items: Vec<T>,
    single_selection: bool,
    selected: HashSet<usize>,
    filtered_ids: Option<HashSet<usize>>,
    highlights: Option<HashSet<&'a usize>>,
    view_item: Box<dyn Fn(usize, T, bool) -> Element<'a, Message> + 'b>,
    on_selected: Option<Box<dyn Fn(usize, f32) -> Message + 'b>>,
    on_deselected: Option<Box<dyn Fn(usize) -> Message + 'b>>,
    clear_selection_maybe: Option<Message>,
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
            selected: HashSet::new(),
            filtered_ids: None,
            highlights: None,
            view_item: Box::new(move |idx, item, selected| view_item(idx, item, selected).into()),
            on_selected: None,
            on_deselected: None,
            clear_selection_maybe: None,
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

    pub fn select(mut self, idx: usize) -> Self {
        self.selected.insert(idx);
        self
    }

    pub fn select_maybe(mut self, idx: Option<usize>) -> Self {
        if let Some(idx) = idx {
            self.selected.insert(idx);
        }
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

    pub fn clear_selection_maybe(mut self, on_selection_cleared: Option<Message>) -> Self {
        self.clear_selection_maybe = on_selection_cleared;
        self
    }

    pub fn highlight(mut self, highlights: impl IntoIterator<Item = &'a usize>) -> Self {
        self.highlights = Some(HashSet::from_iter(highlights));
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

#[derive(Clone, Debug)]
pub enum Event<Message> {
    Select(usize),
    Deselect(usize),
    ShowItem(usize, f32),
    HideItem(usize),
    ItemResize(usize, f32),
    Content(Message),
}

impl<'a, T: Clone, Message: Clone + 'static> Component<'a, Message>
    for ListView<'a, '_, T, Message>
{
    type State = State;

    type Event = Event<Message>;

    fn listen(
        &self,
        _state: &Self::State,
        _event: &iced::Event,
        _bounds: iced::Rectangle,
        _cursor: iced_core::mouse::Cursor,
    ) -> Action<Self::Event> {
        if let Some(message) = &self.clear_selection_maybe {
            return Action::publish(Event::Content(message.clone()));
        }
        Action::none()
    }

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
                log::trace!("ShowItem: idx:{idx}, h: {height}");
                state.visible.insert(idx);
                if let Some(h) = state.heights.get_mut(&idx) {
                    if self.selected.contains(&idx) {
                        h.selected(height);
                    } else {
                        h.unselected(height);
                    }
                } else {
                    let mut h = ElementHeight::unknown();
                    if self.selected.contains(&idx) {
                        h.selected(height);
                    } else {
                        h.unselected(height);
                    }
                    state.heights.insert(idx, h);
                }
            }
            Event::HideItem(idx) => {
                log::trace!("HideItem: idx:{idx}");
                state.visible.remove(&idx);
            }
            Event::ItemResize(idx, height) => {
                log::trace!("ItemResize: idx:{idx}, h: {height}");
                if let Some(h) = state.heights.get_mut(&idx) {
                    if self.selected.contains(&idx) {
                        h.selected(height);
                    } else {
                        h.unselected(height);
                    }
                } else {
                    let mut h = ElementHeight::unknown();
                    if self.selected.contains(&idx) {
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
                    let selected = self.selected.contains(&idx);
                    let highlighted = self.highlights.as_ref().is_some_and(|h| h.contains(&idx));
                    let el_height = state
                        .heights
                        .get(&idx)
                        .map(|eh| match eh.height(selected) {
                            Height::Unknown => ITEM_HEIGHT,
                            Height::Known(height) => height,
                        })
                        .unwrap_or(ITEM_HEIGHT);
                    let el: Element<_> = if state.visible.contains(&idx) {
                        let item = container(
                            (self.view_item)(idx, item.clone(), selected).map(Event::Content),
                        );
                        button(item)
                            .on_press(if !selected {
                                Event::Select(idx)
                            } else {
                                Event::Deselect(idx)
                            })
                            .style(move |theme: &Theme, status| {
                                listview_item_style(theme, status, selected, highlighted)
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

    fn size_hint(&self) -> iced::Size<iced::Length> {
        iced::Size {
            width: Fill,
            height: Shrink,
        }
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
            state.heights = self
                .items
                .iter()
                .enumerate()
                .map(|(idx, _)| (idx, ElementHeight::unknown()))
                .collect();
        }

        if self.filtered_ids != state.filtered_ids {
            state.filtered_ids = self.filtered_ids.clone();
            state.clear_selection();
            self.make_hash(state);
        }

        if self.clear_selection_maybe.is_some() {
            state.clear_selection();
        }

        if self.selected != state.selected {
            state.selected = self.selected.clone();
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

impl<'a, 'b: 'a, T: Clone + 'a, Message: Clone + 'static> From<ListView<'a, 'b, T, Message>>
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
