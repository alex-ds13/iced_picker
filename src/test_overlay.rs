use iced::{
    Element, Point, Renderer, Size, Theme,
    advanced::{
        self, Layout, Renderer as _, Widget,
        layout::{Limits, Node},
        mouse, overlay, renderer,
        widget::{
            Tree,
            tree::{self, Tag},
        },
    },
    widget::{button, column, row, text},
};

// use crate::Message;

pub struct Bar<'a, Message: Clone> {
    pub show_overlay: bool,
    pub content: Element<'a, Message, Theme, Renderer>,
    pub overlay_el: Element<'a, Message, Theme, Renderer>,
    pub on_cancel: Message,
}

impl<'a, Message: Clone + 'a> Bar<'a, Message> {
    pub fn new(
        show_overlay: bool,
        content: Element<'a, Message, Theme, Renderer>,
        on_cancel: Message,
    ) -> Self {
        let overlay_el = column![
            "Some Buttons:",
            row![
                button("x").on_press(on_cancel.clone()),
                button("v").on_press(on_cancel.clone()),
            ]
            .padding(10)
            .spacing(10)
        ]
        .padding(10)
        .spacing(10)
        .into();

        Self {
            show_overlay,
            content,
            overlay_el,
            on_cancel,
        }
    }
}

pub struct State {
    pub tree: Tree,
}

impl Default for State {
    fn default() -> Self {
        Self {
            tree: Tree::empty(),
        }
    }
}

impl<'a, Message: Clone + 'a> Widget<Message, Theme, Renderer> for Bar<'a, Message> {
    fn tag(&self) -> Tag {
        Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content), Tree::new(&self.overlay_el)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.content, &self.overlay_el]);
    }

    fn size(&self) -> Size<iced::Length> {
        Size {
            width: iced::Length::Shrink,
            height: iced::Length::Shrink,
        }
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &advanced::layout::Limits,
    ) -> advanced::layout::Node {
        self.content
            .as_widget()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        state: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn advanced::Clipboard,
        shell: &mut advanced::Shell<'_, Message>,
        viewport: &iced::Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut state.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        state: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &state.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        _state: &'b mut Tree,
        _layout: Layout<'_>,
        _renderer: &Renderer,
        _translation: iced::Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        if self.show_overlay {
            let state = &mut _state.children[1];

            let position = _layout.position() + iced::Vector::new(0.0, _layout.bounds().height);

            Some(
                Overlay::new(
                    state,
                    &mut self.overlay_el,
                    position,
                    self.on_cancel.clone(),
                )
                .overlay(),
            )
            // Some(overlay::Element::new(Box::new(Overlay {
            //     state,
            //     element: &mut self.overlay_el,
            //     position,
            // })))
        } else {
            None
        }
    }
}

impl<'a, Message> From<Bar<'a, Message>> for Element<'a, Message, Theme>
where
    Message: Clone + 'a,
{
    fn from(value: Bar<'a, Message>) -> Self {
        Element::new(value)
    }
}

pub struct Overlay<'a, 'b, Message, Theme> {
    pub element: &'b mut Element<'a, Message, Theme>,
    pub state: &'b mut Tree,
    pub position: Point,
}

impl<'a, 'b, Message: Clone + 'a> Overlay<'a, 'b, Message, Theme> {
    pub fn new(
        state: &'b mut Tree,
        element: &'b mut Element<'a, Message>,
        position: Point,
        _on_cancel: Message,
    ) -> Self {
        // let element = column![
        //     "Some Buttons:",
        //     row![
        //         button("x").on_press(on_cancel.clone()),
        //         button("v").on_press(on_cancel),
        //     ]
        //     .padding(10)
        //     .spacing(10)
        // ]
        // .padding(10)
        // .spacing(10);
        //
        // state.diff(&element as &dyn Widget<_, _, _>);

        Self {
            element,
            state,
            position,
        }
    }

    pub fn overlay(self) -> overlay::Element<'b, Message, Theme, Renderer> {
        overlay::Element::new(Box::new(self))
    }
}

impl<Message, Theme> advanced::Overlay<Message, Theme, Renderer> for Overlay<'_, '_, Message, Theme>
where
    Theme: button::Catalog + text::Catalog,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> Node {
        let limits = Limits::new(Size::ZERO, bounds);
        self.element
            .as_widget()
            .layout(self.state, renderer, &limits)
            .move_to(self.position)
    }

    fn update(
        &mut self,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn advanced::Clipboard,
        shell: &mut advanced::Shell<'_, Message>,
    ) {
        let viewport = &layout.bounds();
        self.element.as_widget_mut().update(
            self.state, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let bounds = layout.bounds();
        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            iced::Color::BLACK,
        );
        self.element.as_widget().draw(
            self.state,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &layout.bounds(),
        )
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.element
            .as_widget()
            .mouse_interaction(self.state, layout, cursor, viewport, renderer)
    }
}
