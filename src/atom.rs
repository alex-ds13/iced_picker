use crate::core::layout::{self, Layout};
use crate::core::mouse;
use crate::core::overlay;
use crate::core::renderer;
use crate::core::widget;
use crate::core::widget::tree::{self, Tree};
use crate::core::{
    self, Clipboard, Element, Event, Length, Rectangle, Shell, Size, Vector, Widget,
};
use iced_widget::space;

use std::marker::PhantomData;

pub trait Component<'a, Message: 'a, Theme = iced::Theme, Renderer = iced::Renderer> {
    /// The internal state of this [`Component`].
    type State: Default;

    /// The type of message this [`Component`] handles internally.
    type InternalMessage;

    /// Processes an [`InternalMessage`](Component::InternalMessage) and updates the [`Component`] state accordingly.
    ///
    /// It can produce a `Message` for the parent application.
    fn update(
        &mut self,
        state: &mut Self::State,
        message: Self::InternalMessage,
    ) -> Option<Message>;

    /// Produces the widgets of the [`Component`], which may trigger an [`InternalMessage`](Component::InternalMessage)
    /// on user interaction.
    fn view(&self, state: &Self::State) -> Element<'a, Self::InternalMessage, Theme, Renderer>;

    /// Update the [`Component`] state based on the provided [`Operation`](widget::Operation)
    ///
    /// By default, it does nothing.
    fn operate(
        &self,
        _bounds: Rectangle,
        _state: &mut Self::State,
        _operation: &mut dyn widget::Operation,
    ) {
    }

    /// Returns a [`Size`] hint for laying out the [`Component`].
    ///
    /// This hint may be used by some widget containers to adjust their sizing strategy
    /// during construction.
    fn size_hint(&self) -> Size<Length> {
        Size {
            width: Length::Shrink,
            height: Length::Shrink,
        }
    }

    /// Update the [`Component`] state based on the provided [`diff`] function.
    ///
    /// By default, it does nothing.
    fn diff(&self, _state: &mut Self::State) {}
}

/// A widget that encapsulates a [`Component`]
pub struct Atom<
    'a,
    T: Component<'a, Message, Theme, Renderer>,
    State: Default,
    InternalMessage,
    Message: 'a,
    Theme = iced::Theme,
    Renderer = iced::Renderer,
> {
    pub component: T,
    pub width: Length,
    pub height: Length,
    pub content: Element<'a, InternalMessage, Theme, Renderer>,
    pub internal_message: std::marker::PhantomData<InternalMessage>,
    pub message: std::marker::PhantomData<Message>,
    pub state: std::marker::PhantomData<State>,
}

impl<'a, T, State: Default, InternalMessage, Message, Theme, Renderer>
    Atom<'a, T, State, InternalMessage, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer>,
    Renderer: core::Renderer,
{
    /// Creates a new [`Atom`] widget that encapsulates a [`Component`]
    pub fn new(component: T) -> Self {
        Self {
            component,
            width: Length::Shrink,
            height: Length::Shrink,
            content: Element::new(space()),
            internal_message: std::marker::PhantomData,
            message: std::marker::PhantomData,
            state: std::marker::PhantomData,
        }
    }

    /// Sets the width of the [`Atom`].
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`Atom`].
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

impl<'a, T, State, InternalMessage, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Atom<'a, T, State, InternalMessage, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer, State = State, InternalMessage = InternalMessage>,
    State: Default + 'static,
    <T as Component<'a, Message, Theme, Renderer>>::State: Default + 'static,
    Renderer: core::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn diff(&self, tree: &mut Tree) {
        // The Tree diff is deferred to layout, here we simply call the component diff function to
        // diff the `State`
        self.component.diff(tree.state.downcast_mut());
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::empty()]
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.component.size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        self.content = self.component.view(state);
        tree.diff_children(std::slice::from_ref(&self.content));

        let node = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        let size = limits.resolve(self.width, self.height, node.size());

        layout::Node::with_children(size, vec![node])
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let mut local_messages = Vec::new();
        let mut local_shell = Shell::new(&mut local_messages);
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            &mut local_shell,
            viewport,
        );

        if local_shell.is_event_captured() {
            shell.capture_event();
        }

        local_shell.revalidate_layout(|| shell.invalidate_layout());
        shell.request_redraw_at(local_shell.redraw_request());
        shell.request_input_method(local_shell.input_method());

        if !local_messages.is_empty() {
            let state = tree.state.downcast_mut::<State>();
            for message in local_messages
                .into_iter()
                .filter_map(|message| self.component.update(state, message))
            {
                shell.publish(message);
            }

            shell.request_redraw();
            shell.invalidate_layout();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        self.component.operate(layout.bounds(), state, operation);

        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(
                &mut tree.children[0],
                layout.children().next().unwrap(),
                renderer,
                viewport,
                translation,
            )
            .map(|element| {
                let state = tree.state.downcast_mut::<State>();
                Overlay::overlay(element, &mut self.component, state)
            })
    }
}

pub struct Overlay<
    'a,
    'b,
    T,
    State,
    InternalMessage,
    Message,
    Theme = iced::Theme,
    Renderer = iced::Renderer,
> where
    T: Component<'a, Message, Theme, Renderer, State = State, InternalMessage = InternalMessage>,
    State: Default,
    Message: 'a,
{
    element: overlay::Element<'b, InternalMessage, Theme, Renderer>,
    component: &'b mut T,
    state: &'b mut State,
    phantom: PhantomData<overlay::Element<'a, Message, Theme, Renderer>>,
}

impl<'a, 'b, T, State, InternalMessage, Message, Theme, Renderer>
    Overlay<'a, 'b, T, State, InternalMessage, Message, Theme, Renderer>
where
    'a: 'b,
    T: Component<'a, Message, Theme, Renderer, State = State, InternalMessage = InternalMessage>,
    State: Default + 'static,
    Message: 'a,
    InternalMessage: 'b,
    Theme: 'b,
    Renderer: core::Renderer + 'b,
{
    pub fn overlay(
        element: overlay::Element<'b, InternalMessage, Theme, Renderer>,
        component: &'b mut T,
        state: &'b mut State,
    ) -> overlay::Element<'b, Message, Theme, Renderer> {
        overlay::Element::new(Box::new(Overlay {
            element,
            component,
            state,
            phantom: PhantomData,
        }))
    }
}

impl<'a, 'b, T, State, InternalMessage, Message, Theme, Renderer>
    core::overlay::Overlay<Message, Theme, Renderer>
    for Overlay<'a, 'b, T, State, InternalMessage, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer, State = State, InternalMessage = InternalMessage>,
    State: Default + 'static,
    Renderer: core::Renderer,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.element.as_overlay_mut().layout(renderer, bounds)
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.element
            .as_overlay()
            .draw(renderer, theme, style, layout, cursor);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let mut local_messages = Vec::new();
        let mut local_shell = Shell::new(&mut local_messages);
        self.element.as_overlay_mut().update(
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            &mut local_shell,
        );

        if local_shell.is_event_captured() {
            shell.capture_event();
        }

        local_shell.revalidate_layout(|| shell.invalidate_layout());
        shell.request_redraw_at(local_shell.redraw_request());
        shell.request_input_method(local_shell.input_method());

        if !local_messages.is_empty() {
            for message in local_messages
                .into_iter()
                .filter_map(|message| self.component.update(self.state, message))
            {
                shell.publish(message);
            }

            shell.request_redraw();
            shell.invalidate_layout();
        }
    }
}

impl<'a, T, State, InternalMessage, Message, Theme, Renderer>
    From<Atom<'a, T, State, InternalMessage, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer, State = State, InternalMessage = InternalMessage>
        + 'a,
    State: Default + 'static,
    InternalMessage: 'a,
    Message: 'a,
    Theme: 'a,
    Renderer: core::Renderer + 'a,
{
    fn from(atom: Atom<'a, T, State, InternalMessage, Message, Theme, Renderer>) -> Self {
        Self::new(atom)
    }
}

pub fn atom<'a, T, State, InternalMessage, Message, Theme, Renderer>(
    component: T,
) -> Element<'a, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer, State = State, InternalMessage = InternalMessage>
        + 'a,
    State: Default + 'static,
    InternalMessage: 'a,
    Message: 'a,
    Theme: 'a,
    Renderer: core::Renderer + 'a,
{
    Element::from(Atom::new(component))
}
