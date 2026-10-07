use iced_core as core;

use core::layout::{self, Layout};
use core::mouse;
use core::renderer;
use core::widget::operation::{self, Operation};
use core::widget::tree::{self, Tree};
use core::{Element, Event, Length, Rectangle, Shell, Size, Vector, Widget, window};

use iced::Widget as _;

pub struct Hover<'a, Message, Theme, Renderer> {
    base: Element<'a, Message, Theme, Renderer>,
    top: Element<'a, Message, Theme, Renderer>,
    is_top_focused: bool,
    is_top_overlay_active: bool,
    is_hovered: bool,
    clip: bool,
}

impl<'a, Message, Theme, Renderer> Hover<'a, Message, Theme, Renderer> {
    pub fn new(
        base: impl Widget<Message, Theme, Renderer> + 'a,
        top: impl Widget<Message, Theme, Renderer> + 'a,
    ) -> Hover<'a, Message, Theme, Renderer> {
        Hover {
            base: base.boxed(),
            top: top.boxed(),
            is_top_focused: false,
            is_top_overlay_active: false,
            is_hovered: false,
            clip: false,
        }
    }

    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }
}

impl<Message, Theme, Renderer> core::widget::Meta for Hover<'_, Message, Theme, Renderer> {}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Hover<'_, Message, Theme, Renderer>
where
    Renderer: core::Renderer,
{
    fn tag(&self) -> tree::Tag {
        struct Tag;
        tree::Tag::of::<Tag>()
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(&mut [&mut self.base, &mut self.top]);
    }

    fn size(&self) -> Size<Length> {
        self.base.size()
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        self.base.layout(&mut tree.children[0], renderer, limits);

        let size = tree.children[0].size;

        self.top.layout(
            &mut tree.children[1],
            renderer,
            &layout::Limits::new(Size::ZERO, size),
        );

        tree.children[0].translation = Vector::ZERO;
        tree.children[1].translation = Vector::ZERO;
        tree.size = size;
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
        if let Some(clipped_viewport) = layout.bounds().intersection(viewport) {
            let bounds = if self.clip {
                clipped_viewport
            } else {
                *viewport
            };
            let mut children = layout.iter(&tree.children);

            let (base_layout, base_tree) = children.next().unwrap();

            self.base.draw(
                base_tree,
                renderer,
                theme,
                style,
                base_layout,
                cursor,
                viewport,
            );

            if cursor.is_over(layout.bounds()) || self.is_top_focused || self.is_top_overlay_active
            {
                let (top_layout, top_tree) = children.next().unwrap();

                renderer.with_layer(bounds, |renderer| {
                    self.top.draw(
                        top_tree, renderer, theme, style, top_layout, cursor, viewport,
                    );
                });
            }
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn operation::Operation,
    ) {
        let children = [&mut self.base, &mut self.top]
            .into_iter()
            .zip(layout.iter_mut(&mut tree.children));

        for (child, (layout, tree)) in children {
            child.operate(tree, layout, viewport, renderer, operation);
        }
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
        let mut children = layout.iter_mut(&mut tree.children);
        let (base_layout, base_tree) = children.next().unwrap();
        let (top_layout, top_tree) = children.next().unwrap();

        let is_hovered = cursor.is_over(layout.bounds());

        if matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
            let mut count_focused = operation::focusable::count();

            self.top.operate(
                top_tree,
                top_layout,
                viewport,
                renderer,
                &mut operation::black_box(&mut count_focused),
            );

            self.is_top_focused = match count_focused.finish() {
                operation::Outcome::Some(count) => count.focused.is_some(),
                _ => false,
            };

            self.is_hovered = is_hovered;
        } else if is_hovered != self.is_hovered {
            shell.request_redraw();
        }

        let is_visible = is_hovered || self.is_top_focused || self.is_top_overlay_active;

        if matches!(
            event,
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::ButtonReleased(_))
        ) || is_visible
        {
            let redraw_request = shell.redraw_request();

            self.top.update(
                top_tree, event, top_layout, cursor, renderer, shell, viewport,
            );

            // Ignore redraw requests of invisible content
            if !is_visible {
                Shell::replace_redraw_request(shell, redraw_request);
            }
        };

        if shell.is_event_captured() {
            return;
        }

        self.base.update(
            base_tree,
            event,
            base_layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        [&self.base, &self.top]
            .into_iter()
            .rev()
            .zip(layout.iter(&tree.children).rev())
            .map(|(child, (layout, tree))| {
                child.mouse_interaction(tree, layout, cursor, viewport, renderer)
            })
            .find(|&interaction| interaction != mouse::Interaction::None)
            .unwrap_or_default()
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut core::widget::Tree,
        layout: core::Layout,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: core::Vector,
        window: Size,
    ) -> Vec<core::overlay::Element<'b, Message, Theme, Renderer>> {
        let mut children = [&mut self.base, &mut self.top]
            .into_iter()
            .zip(layout.iter_mut(&mut tree.children));

        let (base, (base_layout, base_tree)) = children.next().unwrap();
        let base_overlays = base.overlay(
            base_tree,
            base_layout,
            renderer,
            viewport,
            translation,
            window,
        );

        if !base_overlays.is_empty() {
            return base_overlays;
        }

        let (top, (top_layout, top_tree)) = children.next().unwrap();
        let top_overlays = top.overlay(
            top_tree,
            top_layout,
            renderer,
            viewport,
            translation,
            window,
        );
        self.is_top_overlay_active = !top_overlays.is_empty();

        top_overlays
    }
}

pub fn hover<'a, Message, Theme, Renderer>(
    base: impl Widget<Message, Theme, Renderer> + 'a,
    top: impl Widget<Message, Theme, Renderer> + 'a,
) -> Hover<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: iced_core::Renderer + 'a,
{
    Hover::new(base, top)
}
