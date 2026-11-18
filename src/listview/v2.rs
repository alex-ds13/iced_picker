//! Candidate for the virtualized list view that will replace this module's widget.
//!
//! It only builds for debug tests until it is complete, because the headless tests
//! use iced's null renderer, which only exists in debug builds.
//!
//! A [`ListView`] is content, like a column: the application puts it in its own
//! scrollable. It builds, lays out and draws only a window of items around the
//! visible region it learns on every redraw, and keeps the visible items still across
//! measurements and structural changes by moving a phantom spacer above them.

mod geometry;
mod heights;
mod model;
mod style;
mod window;

#[cfg(test)]
mod tests;

pub use style::{Catalog, Status, Style, StyleFn, default};

use geometry::{Settings, Visible};
use window::{Core, Slot};

use std::collections::HashSet;
use std::hash::Hash;
use std::marker::PhantomData;

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Id, Meta, Operation, Tree, tree};
use iced::advanced::{Shell, Widget};
use iced::{Background, Color, Event, Length, Padding, Pixels, Rectangle, Size, Vector, mouse};

/// The items materialized beyond each edge of the viewport by default.
const ANTICIPATE: usize = 4;

/// A virtualized vertical list of items with variable heights.
///
/// Only the items around the visible region exist as widgets. Every other item is a
/// height in an index: measured once it has been shown, estimated before that.
///
/// # Limitations
///
/// The list has no intrinsic width, because items outside the window cannot report
/// one. A `Shrink` or `Fit` width takes the available width, like `Fill`.
///
/// The estimated height is only a starting guess for unmeasured items. When it is
/// much larger than their real height, a single frame that jumps into unmeasured
/// content can build and then discard extra items, about the estimate divided by the
/// real height per jump.
#[allow(missing_debug_implementations)]
pub struct ListView<'a, T, K, W, Message, Theme = iced::Theme, Renderer = iced::Renderer>
where
    Theme: Catalog,
{
    items: &'a [T],
    view: Box<dyn Fn(usize, &'a T) -> W + 'a>,
    key: Box<dyn Fn(usize, &'a T) -> K + 'a>,
    id: Option<Id>,
    width: Length,
    padding: Padding,
    spacing: f32,
    estimated_height: Option<f32>,
    anticipate: usize,
    highlight: HashSet<K>,
    class: Theme::Class<'a>,
    /// The widgets of the window, aligned with the tree's children. Built in `diff`
    /// and extended by the walk in `layout`.
    rows: Vec<W>,
    _widget: PhantomData<fn(Message) -> Renderer>,
}

/// Creates a [`ListView`] of `items`, building each visible item with `view`.
///
/// Items are identified by their index until [`ListView::key`] gives them a key.
pub fn list_view<'a, T, W, Message, Theme, Renderer>(
    items: &'a [T],
    view: impl Fn(usize, &'a T) -> W + 'a,
) -> ListView<'a, T, usize, W, Message, Theme, Renderer>
where
    W: Widget<Message, Theme, Renderer> + 'a,
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    ListView {
        items,
        view: Box::new(view),
        key: Box::new(|index, _| index),
        id: None,
        width: Length::Fill,
        padding: Padding::ZERO,
        spacing: 0.0,
        estimated_height: None,
        anticipate: ANTICIPATE,
        highlight: HashSet::new(),
        class: Theme::default(),
        rows: Vec::new(),
        _widget: PhantomData,
    }
}

impl<'a, T, K, W, Message, Theme, Renderer> ListView<'a, T, K, W, Message, Theme, Renderer>
where
    Theme: Catalog,
{
    /// Identifies items by `key` instead of their index.
    ///
    /// Measured heights and widget state follow the key, so inserting, removing or
    /// reordering items moves neither. Call it before [`Self::highlight`], whose keys
    /// it drops.
    pub fn key<Key>(
        self,
        key: impl Fn(&'a T) -> Key + 'a,
    ) -> ListView<'a, T, Key, W, Message, Theme, Renderer>
    where
        Key: Clone + Eq + Hash + Send + 'static,
    {
        if !self.highlight.is_empty() {
            log::warn!("list_view: `key` drops the keys passed to `highlight` before it");
        }

        ListView {
            items: self.items,
            view: self.view,
            key: Box::new(move |_, item| key(item)),
            id: self.id,
            width: self.width,
            padding: self.padding,
            spacing: self.spacing,
            estimated_height: self.estimated_height,
            anticipate: self.anticipate,
            highlight: HashSet::new(),
            class: self.class,
            rows: Vec::new(),
            _widget: PhantomData,
        }
    }

    /// Sets the [`Id`] of the list.
    pub fn id(mut self, id: impl Into<Id>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Sets the width of the list. Items always span it.
    ///
    /// `Shrink` and `Fit` take the available width like `Fill`, see the limitations of
    /// [`ListView`]. The height is always the height of the content.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the padding around the items.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    /// Sets the vertical spacing between items.
    pub fn spacing(mut self, spacing: impl Into<Pixels>) -> Self {
        self.spacing = spacing.into().0;
        self
    }

    /// Sets the height assumed for items that have not been measured yet.
    ///
    /// Without it, the first positive measurement is used, and 32 px before that.
    pub fn estimated_height(mut self, height: impl Into<Pixels>) -> Self {
        self.estimated_height = Some(height.into().0);
        self
    }

    /// Sets how many items are materialized beyond each edge of the viewport.
    pub fn anticipate(mut self, items: usize) -> Self {
        self.anticipate = items;
        self
    }

    /// Highlights the items with the given keys, see [`Status::highlighted`].
    pub fn highlight(mut self, keys: impl IntoIterator<Item = K>) -> Self
    where
        K: Eq + Hash,
    {
        self.highlight.extend(keys);
        self
    }

    /// Sets the style of the items.
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme, Status) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the style class of the items.
    #[must_use]
    pub fn class(mut self, class: impl Into<Theme::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    fn settings(&self) -> Settings {
        Settings {
            padding: self.padding,
            spacing: self.spacing,
            estimate: self.estimated_height,
            anticipate: self.anticipate,
        }
    }

    /// The width of the list. Items outside the window cannot report an intrinsic
    /// width, so the available width stands in for it.
    ///
    /// Limitation, to document with the public API: `Shrink` and `Fit` therefore
    /// behave like `Fill`. A width measured from the window alone would change while
    /// scrolling.
    fn resolve_width(&self, limits: &layout::Limits) -> f32 {
        let limits = limits.width(self.width);

        let available = if limits.max.width.is_finite() {
            limits.max.width
        } else {
            limits.min.width
        };

        limits.resolve_width(self.width, available)
    }
}

/// The limits of every item (section 4.3): the content width, and a height that is
/// compressed to the intrinsic one, so `Fill` items take their natural height.
fn item_limits(width: f32, limits: &layout::Limits) -> layout::Limits {
    layout::Limits::with_flags(
        Size::new(width, 0.0),
        Size::new(width, limits.max.height),
        Size::new(false, true),
        Size::new(false, true),
    )
}

/// The key and position of the drawn row under the cursor, decided like `draw`
/// decides the hovered status.
fn hovered<K: Clone>(
    window: &[Slot<K>],
    layout: Layout,
    children: &[Tree],
    cursor: mouse::Cursor,
    viewport: &Rectangle,
) -> Option<(K, usize)> {
    layout
        .iter(children)
        .zip(window)
        .find(|((layout, _), _)| {
            let bounds = layout.bounds();
            bounds.intersects(viewport) && cursor.is_over(bounds)
        })
        .map(|(_, slot)| (slot.key.clone(), slot.position))
}

impl<T, K, W, Message, Theme, Renderer> Meta for ListView<'_, T, K, W, Message, Theme, Renderer> where
    Theme: Catalog
{
}

impl<T, K, W, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for ListView<'_, T, K, W, Message, Theme, Renderer>
where
    K: Clone + Eq + Hash + Send + 'static,
    W: Widget<Message, Theme, Renderer>,
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: Length::Shrink,
        }
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Core<K>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Core::<K>::new(self.settings()))
    }

    fn diff(&mut self, tree: &mut Tree) {
        let Tree {
            state, children, ..
        } = tree;
        let core = state.downcast_mut::<Core<K>>();

        core.configure(self.settings());

        let keys = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| (self.key)(index, item));

        if let Some(rebuilt) = core.model.reconcile(keys, &mut core.geometry.heights) {
            core.restructure(&rebuilt, children);
        }

        self.rows = core
            .window
            .iter()
            .map(|slot| (self.view)(slot.position, &self.items[slot.position]))
            .collect();

        for (child, row) in children.iter_mut().zip(&mut self.rows) {
            child.diff(row);
        }
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) {
        let width = self.resolve_width(limits);
        let limits = item_limits((width - self.padding.x()).max(0.0), limits);

        let Tree {
            state,
            children,
            size,
            ..
        } = tree;
        let core = state.downcast_mut::<Core<K>>();

        let _ = self.sync(core, children, renderer, &limits, self.padding.left);

        *size = Size::new(width, core.geometry.total().max(0.0) as f32);
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
        if let Event::Window(iced::window::Event::RedrawRequested(now)) = event {
            let core = tree.state.downcast_mut::<Core<K>>();

            if core.redraw(Visible::new(*viewport, layout.bounds()), *now) {
                shell.invalidate_layout();
            }
        }

        for (row, (layout, tree)) in self
            .rows
            .iter_mut()
            .zip(layout.iter_mut(&mut tree.children))
        {
            row.update(tree, event, layout, cursor, renderer, shell, viewport);
        }

        // Compared with what the last draw showed, not with the last cursor event:
        // scrolling moves rows under a still cursor.
        if let Event::Mouse(
            mouse::Event::CursorMoved { .. }
            | mouse::Event::CursorEntered
            | mouse::Event::CursorLeft,
        ) = event
        {
            let core = tree.state.downcast_ref::<Core<K>>();
            let hovered = hovered(&core.window, layout, &tree.children, cursor, viewport);

            if hovered != *core.drawn_hover.borrow() {
                shell.request_redraw();
            }
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
        let core = tree.state.downcast_ref::<Core<K>>();

        *core.drawn_hover.borrow_mut() =
            hovered(&core.window, layout, &tree.children, cursor, viewport);

        for ((row, slot), (layout, tree)) in self
            .rows
            .iter()
            .zip(&core.window)
            .zip(layout.iter(&tree.children))
        {
            let bounds = layout.bounds();

            // Anticipated rows exist for continuity, not to be drawn.
            if !bounds.intersects(viewport) {
                continue;
            }

            let status = Status {
                hovered: cursor.is_over(bounds),
                highlighted: self.highlight.contains(&slot.key),
            };
            let appearance = <Theme as Catalog>::style(theme, &self.class, status);

            if appearance.background.is_some()
                || appearance.border.width > 0.0
                || appearance.shadow.color.a > 0.0
            {
                renderer.fill_quad(
                    Quad {
                        bounds,
                        border: appearance.border,
                        shadow: appearance.shadow,
                        ..Quad::default()
                    },
                    appearance
                        .background
                        .unwrap_or(Background::Color(Color::TRANSPARENT)),
                );
            }

            let style = renderer::Style {
                text_color: appearance.text_color.unwrap_or(style.text_color),
            };

            row.draw(tree, renderer, theme, &style, layout, cursor, viewport);
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(self.id.as_ref(), layout.bounds(), viewport);
        operation.traverse(&mut |operation| {
            for (row, (layout, tree)) in self
                .rows
                .iter_mut()
                .zip(layout.iter_mut(&mut tree.children))
            {
                row.operate(tree, layout, viewport, renderer, operation);
            }
        });
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.rows
            .iter()
            .zip(layout.iter(&tree.children))
            .map(|(row, (layout, tree))| {
                row.mouse_interaction(tree, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default()
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
        overlay::from_children(
            &mut self.rows,
            tree,
            layout,
            renderer,
            viewport,
            translation,
            window,
        )
    }
}
