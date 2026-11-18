//! The appearance of a list's items.

use iced::{Background, Border, Color, Shadow, Theme, Vector};

/// The state of one drawn item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    /// The cursor is over the item.
    pub hovered: bool,
    /// The item's key was passed to [`ListView::highlight`](super::ListView::highlight).
    pub highlighted: bool,
}

/// The appearance of one item, drawn behind its widget.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Style {
    /// The background of the item.
    pub background: Option<Background>,
    /// The color inherited by the item's text that has no color of its own.
    pub text_color: Option<Color>,
    /// The border of the item, drawn inside its bounds.
    pub border: Border,
    /// The shadow of the item.
    pub shadow: Shadow,
}

/// The theme catalog of a [`ListView`](super::ListView).
pub trait Catalog {
    /// The item class of the [`Catalog`].
    type Class<'a>;

    /// The default class produced by the [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style;
}

/// A styling function for the items of a [`ListView`](super::ListView).
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

/// The default item style, with the colors of the current list widget: a light
/// tint under the cursor and an accented border around highlighted items.
pub fn default(theme: &Theme, status: Status) -> Style {
    let palette = theme.palette();
    let tint = |color: Color, alpha: f32| Color { a: alpha, ..color };

    let background = status
        .hovered
        .then(|| tint(palette.background.base.text, 0.06).into());

    let (border, shadow) = if status.highlighted {
        let accent = palette.warning.base.color;

        (
            Border {
                color: accent,
                width: 1.5,
                radius: 0.0.into(),
            },
            Shadow {
                color: tint(accent, 0.45),
                offset: Vector::ZERO,
                blur_radius: 6.0,
            },
        )
    } else {
        (Border::default(), Shadow::default())
    };

    Style {
        background,
        text_color: Some(palette.background.base.text),
        border,
        shadow,
    }
}
