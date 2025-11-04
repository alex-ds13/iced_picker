use std::time::{Duration, Instant};

use iced::widget::{Component, component};
use iced::{Element, widget::mouse_area};

const TIMEOUT: Duration = Duration::from_millis(250);

pub struct DoubleClick<'a, Message, F, I>
where
    F: Fn() -> I,
    I: Into<Element<'a, Message>>,
{
    builder: Box<F>,
    on_double_click: Message,
    _lifetime_marker: std::marker::PhantomData<&'a Message>,
}

impl<'a, Message, F, I> DoubleClick<'a, Message, F, I>
where
    F: Fn() -> I,
    I: Into<Element<'a, Message>>,
{
    pub fn new(f: F, message: Message) -> Self {
        Self {
            builder: Box::new(f),
            on_double_click: message,
            _lifetime_marker: std::marker::PhantomData,
        }
    }
}

#[derive(Debug)]
pub struct State {
    is_hovered: bool,
    instant: Instant,
}

impl Default for State {
    fn default() -> Self {
        Self {
            is_hovered: Default::default(),
            instant: Instant::now(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub enum InternalMessage<Message> {
    #[default]
    None,
    SetHovered(bool),
    Message(Message),
    Instant(Instant),
    DoubleClick,
}

impl<'a, Message, F, I> Component<'a, Message> for DoubleClick<'a, Message, F, I>
where
    Message: Clone + std::fmt::Debug + 'a,
    F: Fn() -> I,
    I: Into<Element<'a, Message>>,
{
    type State = State;

    type Event = InternalMessage<Message>;

    fn update(&mut self, state: &mut Self::State, event: Self::Event) -> Option<Message> {
        match event {
            InternalMessage::None => {}
            InternalMessage::SetHovered(hover) => state.is_hovered = hover,
            InternalMessage::Message(message) => return Some(message),
            InternalMessage::Instant(instant) => state.instant = instant,
            InternalMessage::DoubleClick => return Some(self.on_double_click.clone()),
        }
        None
    }

    fn view(&self, _state: &Self::State) -> Element<'a, Self::Event> {
        let content = (self.builder)().into().map(InternalMessage::Message);

        let area = mouse_area(content)
            // .interaction(iced::mouse::Interaction::Pointer)
            .on_enter(InternalMessage::SetHovered(true))
            .on_exit(InternalMessage::SetHovered(false));

        area.into()
    }

    fn listen(&self, state: &Self::State, event: &iced::Event) -> iced_widget::Action<Self::Event> {
        if !state.is_hovered {
            return iced_widget::Action::none();
        }

        match event {
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
            | iced::Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
                let now = Instant::now();
                let diff = now - state.instant;

                if diff <= TIMEOUT {
                    iced_widget::Action::publish(InternalMessage::Message(
                        self.on_double_click.clone(),
                    ))
                    .and_capture()
                } else {
                    iced_widget::Action::publish(InternalMessage::Instant(Instant::now()))
                }
            }
            _ => iced_widget::Action::none(),
        }
    }

    fn mouse_interaction(&self, state: &Self::State) -> iced_core::mouse::Interaction {
        if state.is_hovered {
            iced::mouse::Interaction::Pointer
        } else {
            iced::mouse::Interaction::None
        }
    }
}

impl<'a, Message, F, I> From<DoubleClick<'a, Message, F, I>> for Element<'a, Message>
where
    Message: Clone + std::fmt::Debug + 'a,
    F: Fn() -> I + 'a,
    I: Into<Element<'a, Message>> + 'a,
{
    fn from(value: DoubleClick<'a, Message, F, I>) -> Self {
        component(value)
    }
}

pub fn double_click<'a, Message, F, I>(f: F, message: Message) -> DoubleClick<'a, Message, F, I>
where
    Message: Clone + std::fmt::Debug + 'a,
    F: Fn() -> I + 'a,
    I: Into<Element<'a, Message>> + 'a,
{
    DoubleClick::new(f, message)
}
