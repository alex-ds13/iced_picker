use std::collections::HashMap;

use iced::{
    Alignment, Border, Color, Element, Event, Length, Padding, Pixels, Point, Rectangle, Renderer,
    Shadow, Size, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget,
        layout::{Limits, Node},
        renderer,
        widget::tree::{self, Tag, Tree},
    },
    advanced::{Renderer as _, Text, graphics::geometry::Renderer as _, text::Renderer as _},
    alignment::{Horizontal, Vertical},
    event, keyboard,
    mouse::{self, Cursor},
    touch,
    widget::{
        Button, Column, Component, Row,
        canvas::{self, LineCap, Path, Stroke},
        container, text,
        text::Wrapping,
    },
};

#[derive(Clone, Debug)]
pub enum Message {
    None
}

pub struct Expander<'a> {
    pub name: &'a str,
    pub description: Option<Element<'a, Message>>,
    pub inner_element: Option<Element<'a, Message>>,
}

#[derive(Debug, Default)]
pub struct State {
    is_hovered: bool,
    is_expanded: bool,
}

#[derive(Clone, Debug, Default)]
pub enum InternalMessage {
    #[default]
    None,
    ToggleHovered,
    ToggleExpanded,
    Message(Message),
}

impl Component<Message> for Expander<'_> {
    type State = State;

    type Event = InternalMessage;

    fn update(&mut self, state: &mut Self::State, event: Self::Event) -> Option<Message> {
        match event {
            InternalMessage::None => {}
            InternalMessage::ToggleHovered => state.is_hovered = !state.is_hovered,
            InternalMessage::ToggleExpanded => state.is_expanded = !state.is_expanded,
            InternalMessage::Message(message) => return Some(message),
        }
        None
    }

    fn view(
        &self,
        state: &Self::State,
    ) -> Element<'_, Self::Event> {
        todo!()
    }
}

impl<'a> From<Expander<'a>> for Element<'a, Message> {
    fn from(value: Expander<'a>) -> Self {
        iced::widget::component(value)
    }
}
