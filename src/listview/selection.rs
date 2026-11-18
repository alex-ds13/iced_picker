//! App-owned selection (section 5): the [`Selection`] the application keeps, the
//! [`Action`]s the list emits, and how presses, taps and keys become actions.
//!
//! The list resolves modifiers, ranges and anchors against the sequence it was
//! given, so [`Selection::perform`] needs no knowledge of the list's order.

use super::window::Core;
use super::{Catalog, ListView};

use std::borrow::Cow;
use std::collections::HashSet;
use std::hash::Hash;
use std::mem;
use std::ops::Deref;

use iced::advanced::layout::Layout;
use iced::advanced::shell::Tracking;
use iced::advanced::widget::Tree;
use iced::advanced::widget::operation::Focusable;
use iced::advanced::{Shell, Widget};
use iced::keyboard::key::Named;
use iced::keyboard::{self, Key, Modifiers};
use iced::{Event, Point, mouse, touch};
use iced_core::mouse::Click;
use iced_core::mouse::click::Kind;

/// How far a finger may move between press and lift for a tap.
const TAP: f32 = 5.0;

/// The selected keys of a list, the anchor of shift ranges and the keyboard
/// cursor.
///
/// The application owns it, passes it to
/// [`ListView::selection`](super::ListView::selection) and applies the list's
/// [`Action`]s with [`Selection::perform`]. It can also change it directly, with
/// the same [`Change`]s through `Action::from`, or with the methods below. Keys
/// that leave the data stay selected until [`Selection::retain`] drops them.
#[derive(Debug, Clone)]
pub struct Selection<K> {
    selected: HashSet<K>,
    anchor: Option<K>,
    cursor: Option<K>,
}

impl<K: Eq + Hash> PartialEq for Selection<K> {
    fn eq(&self, other: &Self) -> bool {
        self.selected == other.selected
            && self.anchor == other.anchor
            && self.cursor == other.cursor
    }
}

impl<K: Eq + Hash> Eq for Selection<K> {}

impl<K> Default for Selection<K> {
    fn default() -> Self {
        Self {
            selected: HashSet::new(),
            anchor: None,
            cursor: None,
        }
    }
}

impl<K: Clone + Eq + Hash> FromIterator<K> for Selection<K> {
    /// Selects the keys, with no anchor or cursor.
    fn from_iter<I: IntoIterator<Item = K>>(keys: I) -> Self {
        let mut selection = Self::new();
        selection.extend(keys);
        selection
    }
}

impl<K: Clone + Eq + Hash> Selection<K> {
    /// An empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `key` is selected.
    pub fn contains(&self, key: &K) -> bool {
        self.selected.contains(key)
    }

    /// The selected keys, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = &K> {
        self.selected.iter()
    }

    /// The number of selected keys.
    pub fn len(&self) -> usize {
        self.selected.len()
    }

    /// Whether no key is selected.
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// The key keyboard moves start from.
    pub fn cursor(&self) -> Option<&K> {
        self.cursor.as_ref()
    }

    /// The key shift ranges start from.
    pub fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref()
    }

    /// Applies an action of the list, or a [`Change`] of the application's own
    /// (`selection.perform(Change::Toggle(key).into())`).
    ///
    /// * `Select(k)` selects only `k`. `Toggle(k)` flips `k`. Both move the anchor
    ///   and the cursor to `k`.
    /// * `Range` replaces the selection with its keys, or adds them when
    ///   `additive`, and sets the anchor and the cursor it carries.
    /// * `Cursor(k)` moves the cursor only.
    /// * `All(keys)` selects exactly `keys`. `Clear` selects nothing. Both keep the
    ///   anchor and the cursor.
    pub fn perform(&mut self, action: Action<K>) {
        match action.change {
            Change::Select(key) => {
                self.selected.clear();
                self.selected.insert(key.clone());
                self.anchor = Some(key.clone());
                self.cursor = Some(key);
            }
            Change::Toggle(key) => {
                if !self.selected.remove(&key) {
                    self.selected.insert(key.clone());
                }

                self.anchor = Some(key.clone());
                self.cursor = Some(key);
            }
            Change::Range {
                anchor,
                cursor,
                keys,
                additive,
            } => {
                if !additive {
                    self.selected.clear();
                }

                self.selected.extend(keys);
                self.anchor = Some(anchor);
                self.cursor = Some(cursor);
            }
            Change::Cursor(key) => self.cursor = Some(key),
            Change::All(keys) => self.selected = keys.into_iter().collect(),
            Change::Clear => self.selected.clear(),
        }
    }

    /// Selects only `key`, like a plain click on it.
    pub fn select(&mut self, key: K) {
        self.perform(Change::Select(key).into());
    }

    /// Adds `key` to the selection. Returns whether it was not selected.
    pub fn insert(&mut self, key: K) -> bool {
        self.selected.insert(key)
    }

    /// Removes `key` from the selection. Returns whether it was selected.
    pub fn remove(&mut self, key: &K) -> bool {
        self.selected.remove(key)
    }

    /// Adds `keys` to the selection.
    pub fn extend(&mut self, keys: impl IntoIterator<Item = K>) {
        self.selected.extend(keys);
    }

    /// Sets the key keyboard moves start from.
    pub fn set_cursor(&mut self, cursor: Option<K>) {
        self.cursor = cursor;
    }

    /// Sets the key shift ranges start from.
    pub fn set_anchor(&mut self, anchor: Option<K>) {
        self.anchor = anchor;
    }

    /// Selects nothing, keeping the anchor and the cursor.
    pub fn clear(&mut self) {
        self.perform(Change::Clear.into());
    }

    /// Keeps only the keys for which `keep` returns `true`. An anchor or cursor it
    /// rejects is dropped too.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.selected.retain(&mut keep);
        self.anchor = self.anchor.take().filter(&mut keep);
        self.cursor = self.cursor.take().filter(&mut keep);
    }
}

/// The selection a list was given: borrowed from the application, or a snapshot
/// the list owns.
pub(super) enum Supplied<'a, K> {
    Borrowed(&'a Selection<K>),
    Owned(Selection<K>),
}

impl<K> Deref for Supplied<'_, K> {
    type Target = Selection<K>;

    fn deref(&self) -> &Selection<K> {
        match self {
            Self::Borrowed(selection) => selection,
            Self::Owned(selection) => selection,
        }
    }
}

impl<'a, K: Clone> From<Cow<'a, Selection<K>>> for Supplied<'a, K> {
    fn from(selection: Cow<'a, Selection<K>>) -> Self {
        match selection {
            Cow::Borrowed(selection) => Self::Borrowed(selection),
            Cow::Owned(selection) => Self::Owned(selection),
        }
    }
}

impl<'a, K: Clone> From<&'a Selection<K>> for Cow<'a, Selection<K>> {
    fn from(selection: &'a Selection<K>) -> Self {
        Cow::Borrowed(selection)
    }
}

impl<K: Clone> From<Selection<K>> for Cow<'_, Selection<K>> {
    fn from(selection: Selection<K>) -> Self {
        Cow::Owned(selection)
    }
}

/// A selection change emitted by a list, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action<K> {
    /// What changes.
    pub change: Change<K>,
    keyboard: bool,
}

/// An action of the application's own, which reveals nothing.
impl<K> From<Change<K>> for Action<K> {
    fn from(change: Change<K>) -> Self {
        Self::mouse(change)
    }
}

impl<K> Action<K> {
    pub(super) fn mouse(change: Change<K>) -> Self {
        Self {
            change,
            keyboard: false,
        }
    }

    pub(super) fn keyboard(change: Change<K>) -> Self {
        Self {
            change,
            keyboard: true,
        }
    }

    /// The new cursor key when the action came from the keyboard, which the
    /// application can bring into view with [`scroll_to`](super::scroll_to). The
    /// list cannot move its scroller itself. Mouse and touch actions happen where
    /// the user points, and give `None`, as do `All` and `Clear`.
    pub fn reveal(&self) -> Option<&K> {
        if !self.keyboard {
            return None;
        }

        match &self.change {
            Change::Select(key) | Change::Toggle(key) | Change::Cursor(key) => Some(key),
            Change::Range { cursor, .. } => Some(cursor),
            Change::All(_) | Change::Clear => None,
        }
    }
}

/// A selection change, resolved by the list against its sequence.
///
/// More changes may come in later versions: a `match` on it needs a wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change<K> {
    /// Select only this key: a plain click or arrow.
    Select(K),
    /// Flip this key: Ctrl click or Ctrl+Space, in multiple selection.
    Toggle(K),
    /// Select `keys`, the items from `anchor` to `cursor` in the list's order,
    /// replacing the selection unless `additive` (Ctrl+Shift).
    Range {
        /// Where the range starts: the selection's anchor, else its cursor, else
        /// `cursor` itself.
        anchor: K,
        /// The item the range was extended to, the new cursor.
        cursor: K,
        /// The items from `anchor` to `cursor`, in the list's order.
        keys: Vec<K>,
        /// Add the keys to the selection instead of replacing it.
        additive: bool,
    },
    /// Move the cursor only: Ctrl with a move key, in multiple selection.
    Cursor(K),
    /// Select every item of the sequence: Ctrl+A, in multiple selection.
    All(Vec<K>),
    /// Select nothing: Escape, a click below the last item, or a Ctrl click on the
    /// selected item in single selection.
    Clear,
}

/// The list's own input state, kept in its tree.
#[derive(Debug)]
pub(super) struct Input<K> {
    pub(super) modifiers: Modifiers,
    /// The last left press on an item, for double clicks.
    click: Option<(Click, K)>,
    /// A finger pressed inside the list, with where it went down in the window.
    touch: Option<(touch::Finger, Point)>,
    /// The effects of the actions the list emitted whose messages the application
    /// has not handled yet, in order, with their tracking.
    pub(super) pending: Vec<(Tracking, Effect<K>)>,
    /// The supplied selection with the pending effects applied, while there are
    /// any. Events resolve against it.
    resolved: Option<Selection<K>>,
    /// The list may have been given another selection since the last event.
    pub(super) rebase: bool,
    /// Effects applied to working copies, for the tests.
    #[cfg(test)]
    pub(super) applied: usize,
    /// Working copies rebuilt from the supplied selection, for the tests.
    #[cfg(test)]
    pub(super) rebases: usize,
}

impl<K> Default for Input<K> {
    fn default() -> Self {
        Self {
            modifiers: Modifiers::default(),
            click: None,
            touch: None,
            pending: Vec::new(),
            resolved: None,
            rebase: false,
            #[cfg(test)]
            applied: 0,
            #[cfg(test)]
            rebases: 0,
        }
    }
}

/// What an emitted action does, stated so that applying it again changes nothing.
///
/// The owner may already have applied the action to the selection it supplies,
/// and still hold its message, for example a `Component` that applies it and
/// notifies its parent. Every change but `Toggle` assigns or inserts. A toggle is
/// stated as the membership it gives.
#[derive(Debug)]
pub(super) enum Effect<K> {
    Change(Change<K>),
    /// `key` ends up selected or not, and the anchor and the cursor move to it.
    Set {
        key: K,
        selected: bool,
    },
}

impl<K: Clone + Eq + Hash> Selection<K> {
    fn apply(&mut self, effect: &Effect<K>) {
        match effect {
            Effect::Change(change) => self.perform(change.clone().into()),
            Effect::Set { key, selected } => {
                if *selected {
                    self.selected.insert(key.clone());
                } else {
                    self.selected.remove(key);
                }

                self.anchor = Some(key.clone());
                self.cursor = Some(key.clone());
            }
        }
    }
}

/// What a point in the list hits.
enum Hit {
    /// The item at this position, or the spacing below it.
    Item(usize),
    /// The empty area below the last item, or an empty list.
    Below,
    /// The padding above the first item.
    Above,
}

/// A key the list handles.
enum Command<K> {
    Change(Change<K>),
    Activate(K),
}

impl<T, K, W, Message, Theme, Renderer> ListView<'_, T, K, W, Message, Theme, Renderer>
where
    K: Clone + Eq + Hash + Send + 'static,
    W: Widget<Message, Theme, Renderer>,
    Theme: Catalog,
    Renderer: iced::advanced::Renderer,
{
    /// The list's part of an event, after its rows had it (section 5): focus,
    /// clicks, taps and keys.
    pub(super) fn input(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) {
        let focused = tree.state.downcast_ref::<Core<K>>().focus.focused;

        self.react(tree, event, layout, cursor, shell);

        // The cursor's status depends on the focus, and nothing else redraws for it.
        if tree.state.downcast_ref::<Core<K>>().focus.focused != focused {
            shell.request_redraw();
        }
    }

    fn react(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, Message>,
    ) {
        let captured = shell.is_event_captured();
        let inside = cursor.position_over(layout.bounds()).filter(|_| !captured);
        let hit = match event {
            Event::Mouse(mouse::Event::ButtonPressed(_))
            | Event::Touch(touch::Event::FingerLifted { .. }) => inside.map(|point| {
                let core = tree.state.downcast_ref::<Core<K>>();
                hit(core, layout, &tree.children, point)
            }),
            _ => None,
        };
        let core = tree.state.downcast_mut::<Core<K>>();

        self.refresh(&mut core.input);

        match event {
            Event::Keyboard(
                keyboard::Event::ModifiersChanged(modifiers)
                | keyboard::Event::KeyReleased { modifiers, .. },
            ) => {
                core.input.modifiers = *modifiers;
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                core.input.modifiers = *modifiers;

                if captured || !core.focus.focused {
                    return;
                }

                let Some(command) = self.command(core, key, *modifiers) else {
                    return;
                };

                // The user acted before a waiting focus landed (section 4.7).
                core.focus.deferred = None;
                shell.capture_event();

                match command {
                    Command::Change(change) => {
                        self.emit(shell, &mut core.input, Action::keyboard(change));
                    }
                    Command::Activate(key) => {
                        if let Some(on_activate) = &self.on_activate {
                            shell.publish(on_activate(key));
                        }
                    }
                }
            }
            // A press outside the list, or one any widget took, also before the list,
            // unfocuses the list and cancels a deferred focus (section 4.7). Any other
            // press inside cancels the deferred focus: the user acted before it
            // landed. A levitating cursor, over an overlay such as a stuck header, is
            // outside, as for iced's text inputs.
            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let (Some(point), Some(hit)) = (inside, hit) else {
                    core.focus.unfocus();
                    core.input.click = None;
                    return;
                };

                core.focus.deferred = None;

                if *button != mouse::Button::Left || self.selection.is_none() {
                    return;
                }

                core.focus.focused = true;

                let change = match hit {
                    Hit::Item(position) => self.click(core, position, core.input.modifiers),
                    Hit::Below => Change::Clear,
                    Hit::Above => return,
                };

                shell.capture_event();
                self.emit(shell, &mut core.input, Action::mouse(change));

                let Hit::Item(position) = hit else {
                    core.input.click = None;
                    return;
                };

                let key = core.model.key(position).clone();
                let previous = core
                    .input
                    .click
                    .take()
                    .filter(|(_, previous)| *previous == key)
                    .map(|(click, _)| click);
                let click = Click::new(point, mouse::Button::Left, previous);

                if click.kind() == Kind::Double
                    && let Some(on_activate) = &self.on_activate
                {
                    shell.publish(on_activate(key.clone()));
                }

                core.input.click = Some((click, key));
            }
            // A touch press is left to the scrollable, which scrolls with the finger.
            // A tap selects when the finger lifts.
            Event::Touch(touch::Event::FingerPressed { id, position }) => {
                if inside.is_none() {
                    core.focus.unfocus();
                    core.input.touch = None;
                    return;
                }

                core.focus.deferred = None;
                core.input.touch = self.selection.is_some().then_some((*id, *position));
            }
            Event::Touch(touch::Event::FingerMoved { id, position }) => {
                if core
                    .input
                    .touch
                    .is_some_and(|(finger, start)| finger == *id && start.distance(*position) > TAP)
                {
                    core.input.touch = None;
                }
            }
            Event::Touch(touch::Event::FingerLifted { id, position }) => {
                let tap = core.input.touch.take().is_some_and(|(finger, start)| {
                    finger == *id && start.distance(*position) <= TAP
                });

                if !tap {
                    return;
                }

                let change = match hit {
                    Some(Hit::Item(position)) => Change::Select(core.model.key(position).clone()),
                    Some(Hit::Below) => Change::Clear,
                    Some(Hit::Above) | None => return,
                };

                core.focus.focused = true;
                self.emit(shell, &mut core.input, Action::mouse(change));
            }
            Event::Touch(touch::Event::FingerLost { .. }) => {
                core.input.touch = None;
            }
            _ => {}
        }
    }

    /// Resolves the next event against the supplied selection with the effects of
    /// the actions the list emitted that the application has not handled yet.
    ///
    /// The runtime can deliver several events in one batch before the application
    /// handles any of their messages and rebuilds the view, so the supplied
    /// selection is stale for every event after the first. Once a message is
    /// handled, its tracking says so: the application applied it, altered it or
    /// ignored it, and the selection it supplies now reflects that. A handled
    /// effect is dropped, and the working copy is rebuilt from the supplied
    /// selection and the remaining effects. It is rebuilt too when the list may
    /// have been given another selection. Otherwise it is kept, and each emitted
    /// effect is applied to it once.
    fn refresh(&self, input: &mut Input<K>) {
        let before = input.pending.len();
        input
            .pending
            .retain(|(tracking, _)| !tracking.is_processed());

        let handled = input.pending.len() != before;
        let rebase = mem::take(&mut input.rebase);

        let Some(supplied) = self
            .selection
            .as_deref()
            .filter(|_| !input.pending.is_empty())
        else {
            input.resolved = None;
            return;
        };

        if handled || rebase || input.resolved.is_none() {
            let mut selection = supplied.clone();

            for (_, effect) in &input.pending {
                selection.apply(effect);
            }

            input.resolved = Some(selection);

            #[cfg(test)]
            {
                input.applied += input.pending.len();
                input.rebases += 1;
            }
        }
    }

    /// The selection events resolve against now.
    fn resolving<'s>(&'s self, core: &'s Core<K>) -> Option<&'s Selection<K>> {
        core.input.resolved.as_ref().or(self.selection.as_deref())
    }

    /// Publishes an action and keeps its change pending until the application
    /// handles it.
    fn emit(&self, shell: &mut Shell<'_, Message>, input: &mut Input<K>, action: Action<K>) {
        let (Some(on_select), Some(supplied)) = (&self.on_select, self.selection.as_deref()) else {
            return;
        };

        let mut resolved = input.resolved.take().unwrap_or_else(|| supplied.clone());

        let effect = match &action.change {
            Change::Toggle(key) => Effect::Set {
                key: key.clone(),
                selected: !resolved.contains(key),
            },
            change => Effect::Change(change.clone()),
        };

        let tracking = shell.publish_and_track(on_select(action));

        resolved.apply(&effect);
        input.resolved = Some(resolved);
        input.pending.push((tracking, effect));

        #[cfg(test)]
        {
            input.applied += 1;
        }
    }

    /// The click table (section 5.1 of the first design, kept by section 5).
    fn click(&self, core: &Core<K>, position: usize, modifiers: Modifiers) -> Change<K> {
        let key = core.model.key(position).clone();

        if !self.multiple {
            let selected = self
                .resolving(core)
                .is_some_and(|selection| selection.contains(&key));

            return if modifiers.command() && !modifiers.shift() && selected {
                Change::Clear
            } else {
                Change::Select(key)
            };
        }

        if modifiers.shift() {
            self.range(core, position, modifiers.command())
        } else if modifiers.command() {
            Change::Toggle(key)
        } else {
            Change::Select(key)
        }
    }

    /// The range from the anchor to `position`. The anchor falls back to the
    /// cursor, then to `position` itself, when it is not in the sequence.
    fn range(&self, core: &Core<K>, position: usize, additive: bool) -> Change<K> {
        let present = |key: Option<&K>| {
            let key = key?;
            Some((key.clone(), core.model.position(key)?))
        };

        let selection = self.resolving(core);
        let (anchor, from) = present(selection.and_then(Selection::anchor))
            .or_else(|| present(selection.and_then(Selection::cursor)))
            .unwrap_or_else(|| (core.model.key(position).clone(), position));

        Change::Range {
            anchor,
            cursor: core.model.key(position).clone(),
            keys: (from.min(position)..=from.max(position))
                .map(|position| core.model.key(position).clone())
                .collect(),
            additive,
        }
    }

    /// The keyboard table (section 5.3 of the first design, kept by section 5).
    /// `None` leaves the key to other widgets.
    fn command(&self, core: &Core<K>, key: &Key, modifiers: Modifiers) -> Option<Command<K>> {
        let selection = self.resolving(core)?;
        let len = core.model.len();

        if len == 0 {
            return None;
        }

        let last = len - 1;
        let cursor = selection.cursor().and_then(|key| core.model.position(key));

        // A key's later duplicates resolve to its first position, so moves step over
        // them, else the cursor would stay or jump back.
        let owner =
            |position: usize| core.model.position(core.model.key(position)) == Some(position);
        let forward = |from: usize| (from..len).find(|&position| owner(position));
        let backward = |from: usize| {
            (0..=from.min(last))
                .rev()
                .find(|&position| owner(position))
                .unwrap_or(0)
        };

        // Without a cursor in the sequence, moves start at the first visible item.
        let first_visible = || {
            let first = core
                .geometry
                .visible
                .and_then(|visible| core.geometry.first_visible(visible.top))
                .unwrap_or(0)
                .min(last);

            forward(first).unwrap_or_else(|| backward(first))
        };

        // The item one visible height away, by the indexed heights: measured, or
        // estimated for items not measured yet. A page point in the spacing between
        // two items goes to the item below.
        let page = |cursor: usize, down: bool| {
            let height = core.geometry.visible.map_or(0.0, |visible| visible.height);
            let y = core.geometry.content_y(cursor);

            if down {
                let y = y + height;
                let target = core.geometry.first_visible(y).unwrap_or(last);
                target.max(cursor + 1).min(last)
            } else {
                let target = core.geometry.first_visible(y - height).unwrap_or(0);
                target.min(cursor.saturating_sub(1))
            }
        };

        let target = match key.as_ref() {
            Key::Named(Named::ArrowUp) => {
                Some(cursor.map_or_else(first_visible, |cursor| backward(cursor.saturating_sub(1))))
            }
            Key::Named(Named::ArrowDown) => Some(cursor.map_or_else(first_visible, |cursor| {
                forward(cursor + 1).unwrap_or(cursor)
            })),
            Key::Named(Named::PageUp) => {
                Some(cursor.map_or_else(first_visible, |cursor| backward(page(cursor, false))))
            }
            Key::Named(Named::PageDown) => Some(cursor.map_or_else(first_visible, |cursor| {
                forward(page(cursor, true)).unwrap_or_else(|| backward(last))
            })),
            Key::Named(Named::Home) => Some(0),
            Key::Named(Named::End) => Some(backward(last)),
            _ => None,
        };

        if let Some(target) = target {
            let key = core.model.key(target).clone();

            let change = if !self.multiple {
                Change::Select(key)
            } else if modifiers.shift() {
                self.range(core, target, modifiers.command())
            } else if modifiers.command() {
                Change::Cursor(key)
            } else {
                Change::Select(key)
            };

            return Some(Command::Change(change));
        }

        match key.as_ref() {
            Key::Named(Named::Space) => {
                let key = core.model.key(cursor?).clone();

                Some(Command::Change(if self.multiple && modifiers.command() {
                    Change::Toggle(key)
                } else {
                    Change::Select(key)
                }))
            }
            Key::Character(character)
                if self.multiple && modifiers.command() && character.eq_ignore_ascii_case("a") =>
            {
                Some(Command::Change(Change::All(
                    (0..len)
                        .map(|position| core.model.key(position).clone())
                        .collect(),
                )))
            }
            Key::Named(Named::Escape) if !selection.is_empty() => {
                Some(Command::Change(Change::Clear))
            }
            Key::Named(Named::Enter) if self.on_activate.is_some() => {
                Some(Command::Activate(core.model.key(cursor?).clone()))
            }
            _ => None,
        }
    }
}

/// What `point`, in the list's layout space, hits. An item owns the spacing below
/// it, so a press between two items selects the upper one.
fn hit<K: Clone + Eq + Hash>(
    core: &Core<K>,
    layout: Layout,
    children: &[Tree],
    point: Point,
) -> Hit {
    let len = core.model.len();
    let mut rows = layout
        .iter(children)
        .zip(&core.window)
        .map(|((layout, _), slot)| (slot.position, layout.bounds()))
        .peekable();

    while let Some((position, bounds)) = rows.next() {
        if point.y < bounds.y {
            return Hit::Above;
        }

        match rows.peek() {
            Some((_, next)) if point.y < next.y => return Hit::Item(position),
            Some(_) => {}
            None if position + 1 < len => return Hit::Item(position),
            None if point.y < bounds.y + bounds.height => return Hit::Item(position),
            None => return Hit::Below,
        }
    }

    Hit::Below
}
