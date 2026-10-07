use crate::atom::{Atom, Component};
use iced::{Widget, widget::mouse_area};

pub struct Hovered<'a, Message, F, I>
where
    F: Fn(bool) -> I,
    I: Widget<Message> + 'a,
{
    builder: Box<F>,
    message: std::marker::PhantomData<&'a Message>,
    on_press: Option<Message>,
}

impl<'a, Message, F, I> Hovered<'a, Message, F, I>
where
    F: Fn(bool) -> I,
    I: Widget<Message> + 'a,
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
    Message: Clone + std::fmt::Debug + 'static,
    F: Fn(bool) -> I,
    I: Widget<Message> + 'a,
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

    fn view(&self, state: &Self::State) -> impl Widget<Self::Event> + 'a {
        let content = (self.builder)(state.is_hovered).map(Event::Message);

        let mut area = mouse_area(content)
            .interaction(iced::mouse::Interaction::Pointer)
            .on_enter(Event::SetHovered(true))
            .on_exit(Event::SetHovered(false));

        if let Some(message) = &self.on_press {
            area = area.on_press(Event::Message(message.clone()));
        }

        area
    }
}

pub fn hovered<'a, Message, F, I>(f: F) -> Atom<'a, Hovered<'a, Message, F, I>, Message>
where
    Message: Clone + std::fmt::Debug + 'static,
    F: Fn(bool) -> I + 'a,
    I: Widget<Message> + 'a,
{
    Atom::new(Hovered::new(f))
}
