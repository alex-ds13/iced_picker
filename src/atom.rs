use crate::core::layout::{self, Layout};
use crate::core::mouse;
use crate::core::overlay;
use crate::core::renderer;
use crate::core::shell;
use crate::core::widget;
use crate::core::widget::tree::{self, Tree};
use crate::core::{self, Element, Event, Length, Rectangle, Shell, Size, Vector, Widget};

use std::cell::RefCell;

use iced::Widget as _;
use iced::widget::space;

pub trait Component<'a, Message: 'a, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Renderer: core::Renderer,
{
    /// The internal state of this [`Component`].
    type State: Default;

    /// The type of message this [`Component`] handles internally.
    type Event;

    /// Processes an [`Event`](Component::Event) and updates the [`Component`] state accordingly.
    ///
    /// It can produce a `Message` for the parent application.
    fn update(&mut self, state: &mut Self::State, event: Self::Event) -> Option<Message>;

    /// Produces the widgets of the [`Component`], which may trigger an [`InternalMessage`](Component::InternalMessage)
    /// on user interaction.
    fn view(&self, state: &Self::State) -> impl iced::Widget<Self::Event, Theme, Renderer> + 'a;

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
pub struct Atom<'a, T, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    T: Component<'a, Message, Theme, Renderer> + 'a,
    Renderer: core::Renderer,
{
    component: RefCell<T>,
    pub width: Length,
    pub height: Length,
    pub content: Element<'a, T::Event, Theme, Renderer>,
}

impl<'a, T, Message, Theme, Renderer> Atom<'a, T, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer>,
    Renderer: core::Renderer,
{
    /// Creates a new [`Atom`] widget that encapsulates a [`Component`]
    pub fn new(component: T) -> Self {
        Self {
            component: RefCell::new(component),
            width: Length::Shrink,
            height: Length::Shrink,
            content: space().boxed(),
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

impl<'a, T, Message, Theme, Renderer> widget::Meta for Atom<'a, T, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer>,
    Renderer: core::Renderer,
{
}

impl<'a, T, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Atom<'a, T, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer>,
    T::State: Default + 'static,
    Renderer: core::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<RefCell<T::State>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(RefCell::new(T::State::default()))
    }

    fn diff(&mut self, tree: &mut Tree) {
        let state = tree.state.downcast_ref::<RefCell<T::State>>();
        self.component.borrow().diff(&mut state.borrow_mut());
    }

    fn size(&self) -> Size<Length> {
        self.content.size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        let state = tree.state.downcast_ref::<RefCell<T::State>>().borrow();
        self.content = self.component.borrow().view(&state).boxed();
        drop(state);

        tree.diff_children(std::slice::from_mut(&mut self.content));

        let content = &mut tree.children[0];
        self.content.layout(content, renderer, limits);
        content.translation = Vector::ZERO;

        tree.size = limits.resolve(self.width, self.height, content.size);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let mut local_messages = shell::Bus::new();
        let mut local_shell = shell.local(&mut local_messages);
        let (layout, content) = layout.iter_mut(&mut tree.children).next().unwrap();
        self.content.update(
            content,
            event,
            layout,
            cursor,
            renderer,
            &mut local_shell,
            viewport,
        );

        if local_shell.is_event_captured() {
            shell.capture_event();
        }

        shell.invalidate(local_shell.invalidation());
        shell.request_redraw_at(local_shell.redraw_request());
        shell.request_input_method(local_shell.input_method());
        shell.clipboard_mut().merge(local_shell.clipboard_mut());
        drop(local_shell);

        if !local_messages.is_empty() {
            let mut state = tree.state.downcast_ref::<RefCell<T::State>>().borrow_mut();
            let mut component = self.component.borrow_mut();

            for (event, receipt) in local_messages.drain() {
                if let Some(message) = component.update(&mut state, event) {
                    shell.forward(message, receipt);
                }
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
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let (layout, content) = layout.iter(&tree.children).next().unwrap();
        self.content
            .draw(content, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let (layout, content) = layout.iter(&tree.children).next().unwrap();
        self.content
            .mouse_interaction(content, layout, cursor, viewport, renderer)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        let mut state = tree.state.downcast_ref::<RefCell<T::State>>().borrow_mut();
        self.component
            .borrow()
            .operate(layout.bounds(), &mut state, operation);
        drop(state);

        let (layout, content) = layout.iter_mut(&mut tree.children).next().unwrap();
        self.content
            .operate(content, layout, viewport, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
        window: Size,
    ) -> Vec<overlay::Element<'b, Message, Theme, Renderer>> {
        let (layout, content) = layout.iter_mut(&mut tree.children).next().unwrap();
        let overlays =
            self.content
                .overlay(content, layout, renderer, viewport, translation, window);
        let state = tree.state.downcast_ref::<RefCell<T::State>>();

        overlays
            .into_iter()
            .map(|element| Overlay::overlay(element, &self.component, state))
            .collect()
    }
}

pub struct Overlay<'a, 'b, T, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    T: Component<'a, Message, Theme, Renderer>,
    Renderer: core::Renderer,
{
    element: overlay::Element<'b, T::Event, Theme, Renderer>,
    component: &'b RefCell<T>,
    state: &'b RefCell<T::State>,
}

impl<'a, 'b, T, Message, Theme, Renderer> Overlay<'a, 'b, T, Message, Theme, Renderer>
where
    'a: 'b,
    T: Component<'a, Message, Theme, Renderer>,
    Theme: 'b,
    Renderer: core::Renderer + 'b,
{
    pub fn overlay(
        element: overlay::Element<'b, T::Event, Theme, Renderer>,
        component: &'b RefCell<T>,
        state: &'b RefCell<T::State>,
    ) -> overlay::Element<'b, Message, Theme, Renderer> {
        overlay::Element::new(Box::new(Overlay {
            element,
            component,
            state,
        }))
    }
}

impl<'a, 'b, T, Message, Theme, Renderer> core::overlay::Overlay<Message, Theme, Renderer>
    for Overlay<'a, 'b, T, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer>,
    Renderer: core::Renderer,
{
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        cursor: mouse::Cursor,
    ) {
        self.element
            .as_overlay()
            .draw(renderer, theme, style, cursor);
    }

    fn update(
        &mut self,
        event: &Event,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        shell: &mut Shell<'_, Message>,
    ) {
        let mut local_messages = shell::Bus::new();
        let mut local_shell = shell.local(&mut local_messages);
        self.element
            .as_overlay_mut()
            .update(event, cursor, renderer, &mut local_shell);

        if local_shell.is_event_captured() {
            shell.capture_event();
        }

        shell.invalidate(local_shell.invalidation());
        shell.request_redraw_at(local_shell.redraw_request());
        shell.request_input_method(local_shell.input_method());
        shell.clipboard_mut().merge(local_shell.clipboard_mut());
        drop(local_shell);

        if !local_messages.is_empty() {
            let mut component = self.component.borrow_mut();
            let mut state = self.state.borrow_mut();

            for (event, receipt) in local_messages.drain() {
                if let Some(message) = component.update(&mut state, event) {
                    shell.forward(message, receipt);
                }
            }

            shell.request_redraw();
            // The view is only rebuilt in `Widget::layout`, so an overlay
            // alone is not enough to reflect the new state
            shell.invalidate_layout();
        }
    }

    fn mouse_interaction(&self, cursor: mouse::Cursor, renderer: &Renderer) -> mouse::Interaction {
        self.element
            .as_overlay()
            .mouse_interaction(cursor, renderer)
    }

    fn index(&self) -> f32 {
        self.element.as_overlay().index()
    }

    fn operate(&mut self, renderer: &Renderer, operation: &mut dyn widget::Operation) {
        self.element.as_overlay_mut().operate(renderer, operation);
    }

    fn overlay<'c>(
        &'c mut self,
        renderer: &Renderer,
    ) -> Vec<overlay::Element<'c, Message, Theme, Renderer>> {
        self.element
            .as_overlay_mut()
            .overlay(renderer)
            .into_iter()
            .map(|element| Overlay::overlay(element, self.component, self.state))
            .collect()
    }
}

pub fn atom<'a, T, Message, Theme, Renderer>(component: T) -> Atom<'a, T, Message, Theme, Renderer>
where
    T: Component<'a, Message, Theme, Renderer> + 'a,
    T::State: 'static,
    Message: 'a,
    Theme: 'a,
    Renderer: core::Renderer + 'a,
{
    Atom::new(component)
}
