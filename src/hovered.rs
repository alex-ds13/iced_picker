use crate::atom::{Component, atom};
use iced::{Element, widget::mouse_area};

pub struct Hovered<'a, Message, F, I>
where
    F: Fn(bool) -> I,
    I: Into<Element<'a, Message>>,
{
    builder: Box<F>,
    message: std::marker::PhantomData<&'a Message>,
    on_press: Option<Message>,
}

impl<'a, Message, F, I> Hovered<'a, Message, F, I>
where
    F: Fn(bool) -> I,
    I: Into<Element<'a, Message>>,
{
    pub fn new(f: F) -> Self {
        Self {
            builder: Box::new(f),
            message: std::marker::PhantomData,
            on_press: None,
        }
    }

    pub fn on_press(mut self, message: Message) -> Self {
        self.on_press = Some(message);
        self
    }
}

#[derive(Debug, Default)]
pub struct State {
    is_hovered: bool,
}

#[derive(Clone, Debug, Default)]
pub enum Event<Message> {
    #[default]
    None,
    SetHovered(bool),
    Message(Message),
}

impl<'a, Message, F, I> Component<'a, Message> for Hovered<'a, Message, F, I>
where
    Message: Clone + std::fmt::Debug + 'a,
    F: Fn(bool) -> I,
    I: Into<Element<'a, Message>>,
{
    type State = State;
    type Event = Event<Message>;

    fn update(&mut self, state: &mut Self::State, event: Self::Event) -> Option<Message> {
        match event {
            Event::None => {}
            Event::SetHovered(hover) => state.is_hovered = hover,
            Event::Message(message) => return Some(message),
        }
        None
    }

    fn view(&self, state: &Self::State) -> Element<'a, Self::Event> {
        let content = (self.builder)(state.is_hovered).into().map(Event::Message);

        let mut area = mouse_area(content)
            .interaction(iced::mouse::Interaction::Pointer)
            .on_enter(Event::SetHovered(true))
            .on_exit(Event::SetHovered(false));

        if let Some(message) = &self.on_press {
            area = area.on_press(Event::Message(message.clone()));
        }

        area.into()
    }
}

impl<'a, Message, F, I> From<Hovered<'a, Message, F, I>> for Element<'a, Message>
where
    Message: Clone + std::fmt::Debug + 'a,
    F: Fn(bool) -> I + 'a,
    I: Into<Element<'a, Message>> + 'a,
{
    fn from(value: Hovered<'a, Message, F, I>) -> Self {
        atom(value)
    }
}

pub fn hovered<'a, Message, F, I>(f: F) -> Element<'a, Message>
where
    Message: Clone + std::fmt::Debug + 'a,
    F: Fn(bool) -> I + 'a,
    I: Into<Element<'a, Message>> + 'a,
{
    Hovered::new(f).into()
}
