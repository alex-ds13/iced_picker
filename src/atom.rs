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
    ///
    /// The [`Atom`] fits its content by default. Parents only see the [`Atom`]'s own
    /// width and height (the view is built during layout), so a content that should
    /// fill or share space needs the matching [`Length`] on the [`Atom`] too.
    pub fn new(component: T) -> Self {
        Self {
            component: RefCell::new(component),
            width: Length::Fit,
            height: Length::Fit,
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
        // `content` is only built in `layout`, so before the first layout it is a
        // placeholder whose sizing says nothing about the component.
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        let state = tree.state.downcast_ref::<RefCell<T::State>>().borrow();
        self.content = self.component.borrow().view(&state).boxed();
        drop(state);

        tree.diff_children(std::slice::from_mut(&mut self.content));

        let limits = limits.width(self.width).height(self.height);
        let content = &mut tree.children[0];
        self.content.layout(content, renderer, &limits);
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

#[cfg(test)]
mod tests {
    use super::*;

    use iced::widget::{Row, Space};

    struct Demo(Length);

    impl Component<'static, (), iced::Theme, ()> for Demo {
        type State = ();
        type Event = ();

        fn update(&mut self, _state: &mut (), _event: ()) -> Option<()> {
            None
        }

        fn view(&self, _state: &()) -> impl iced::Widget<(), iced::Theme, ()> + 'static {
            Space::new().width(self.0).height(10)
        }
    }

    fn layout_row(row: &mut Row<Element<'static, (), iced::Theme, ()>>) -> Tree {
        let mut tree = Tree::new(&*row);
        tree.diff(&mut *row);
        row.layout(
            &mut tree,
            &(),
            &layout::Limits::new(Size::ZERO, Size::new(500.0, 100.0)),
        );
        tree
    }

    #[test]
    fn fill_atoms_share_the_row_and_constrain_their_content() {
        let mut row = Row::new()
            .push(Atom::new(Demo(Length::Fill)).width(Length::Fill).boxed())
            .push(Atom::new(Demo(Length::Fill)).width(Length::Fill).boxed());
        let tree = layout_row(&mut row);

        for atom in &tree.children {
            assert_eq!(atom.size.width, 250.0);
            assert_eq!(atom.children[0].size.width, 250.0);
        }
    }

    #[test]
    fn default_atom_fits_its_content() {
        let mut row = Row::new()
            .push(Atom::new(Demo(Length::Fixed(10.0))).boxed())
            .push(Space::new().width(Length::Fill).boxed());
        let tree = layout_row(&mut row);

        assert_eq!(tree.children[0].size, Size::new(10.0, 10.0));
        assert_eq!(tree.children[1].size.width, 490.0);
    }
}
