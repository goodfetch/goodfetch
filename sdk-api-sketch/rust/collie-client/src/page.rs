//! Collie's page API (ADR 0085, decided in #472).
//!
//! Playwright's page API (ADR 0019) under its names, in snake case, where the meaning is the
//! same, with the guard's guarantees:
//!
//! - Pages, Sessions and frames are values. A [`Session`] is Playwright's context: pages open with
//!   `session.new_page()` or arrive as popups, and frames are reached through [`FrameLocator`]s.
//!   There is no active page, no frame pointer and no element handle: a [`Locator`] is resolved
//!   again at each use.
//! - Every action, and every read of one element, waits through the guard until exactly one
//!   element matches and is actionable, or its deadline passes; a match still ambiguous then fails
//!   with `target_not_unique`, listing every match.
//! - The engine runs a page's gestures (actions, keyboard, mouse and touch input, navigations) one
//!   at a time in arrival order; reads, waits, `evaluate` and answers to routes and dialogs run
//!   concurrently.
//! - A page function is JavaScript source run in the page, with one explicit argument; values
//!   cross as JSON.
//! - Routes and dialogs arrive as [`EventStream`]s the program drives (ADR 0081). Later routes get
//!   a request first; the first answer wins; an item dropped unanswered falls through, and past the
//!   last one a request goes to the network and a dialog gets Playwright's default, so a handler
//!   that answers nothing never stalls the page. A gesture that waits on the program (a locator
//!   handler's turn before an attempt, or a route's answer to a request it waits on, after its
//!   input) gives up its page's turn, only between its own inputs, so a handler's own gestures run
//!   and no two gestures' inputs interleave. An open dialog blocks input, so a dialog's answer
//!   comes before any gesture on the page.
//!
//! Every call is a builder implementing `IntoFuture`, with `.timeout(d)` and `.no_timeout()`
//! (ADR 0080); a question that never waits ([`Query`]) has neither.

use std::collections::BTreeMap;
use std::fmt;
use std::future::IntoFuture;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::concurrency::EventStream;
use crate::lifecycle::{Page, Session};
use crate::results::Error;
use crate::sessions::TextInput;
use crate::time::{Call, CallFuture, Count, ElementState, LoadState, UrlMatcher};

// Locators.

/// ARIA roles, as `get_by_role` takes them. A sample: the SDK lists every ARIA role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AriaRole {
    /// `alert`
    Alert,
    /// `button`
    Button,
    /// `checkbox`
    Checkbox,
    /// `combobox`
    Combobox,
    /// `dialog`
    Dialog,
    /// `heading`
    Heading,
    /// `link`
    Link,
    /// `listbox`
    Listbox,
    /// `listitem`
    ListItem,
    /// `menuitem`
    MenuItem,
    /// `option`
    Option,
    /// `radio`
    Radio,
    /// `row`
    Row,
    /// `searchbox`
    SearchBox,
    /// `tab`
    Tab,
    /// `table`
    Table,
    /// `textbox`
    TextBox,
}

impl AriaRole {
    /// This role with an accessible name: `AriaRole::Button.name("Search")`.
    pub fn name(self, name: impl Into<TextMatch>) -> ByRole {
        let _ = name.into();
        unimplemented!()
    }
}

/// A role and the filters `get_by_role` takes; built from an [`AriaRole`].
#[derive(Debug, Clone)]
pub struct ByRole {
    _private: (),
}

impl ByRole {
    /// The accessible name matches exactly, case included.
    pub fn exact(self) -> Self {
        unimplemented!()
    }

    /// Only checked, or only unchecked, elements.
    pub fn checked(self, checked: bool) -> Self {
        let _ = checked;
        unimplemented!()
    }

    /// Only disabled, or only enabled, elements.
    pub fn disabled(self, disabled: bool) -> Self {
        let _ = disabled;
        unimplemented!()
    }

    /// Only expanded, or only collapsed, elements.
    pub fn expanded(self, expanded: bool) -> Self {
        let _ = expanded;
        unimplemented!()
    }

    /// Also hidden elements.
    pub fn include_hidden(self) -> Self {
        unimplemented!()
    }

    /// A heading's level.
    pub fn level(self, level: u8) -> Self {
        let _ = level;
        unimplemented!()
    }

    /// Only pressed, or only unpressed, elements.
    pub fn pressed(self, pressed: bool) -> Self {
        let _ = pressed;
        unimplemented!()
    }

    /// Only selected, or only unselected, elements.
    pub fn selected(self, selected: bool) -> Self {
        let _ = selected;
        unimplemented!()
    }
}

impl From<AriaRole> for ByRole {
    fn from(role: AriaRole) -> Self {
        let _ = role;
        unimplemented!()
    }
}

/// Text to match. A `&str` matches as a case-insensitive substring after whitespace is
/// normalized, as in Playwright; [`TextMatch::exact`] matches it whole, case included;
/// [`TextMatch::regex`] runs in the engine in the Rust `regex` syntax (ADR 0081). No regex crate
/// is required (G7).
#[derive(Debug, Clone)]
pub struct TextMatch {
    _private: (),
}

impl TextMatch {
    /// The whole text, case included.
    pub fn exact(text: &str) -> Self {
        let _ = text;
        unimplemented!()
    }

    /// A regular expression, run in the engine.
    pub fn regex(pattern: &str) -> Self {
        let _ = pattern;
        unimplemented!()
    }
}

impl From<&str> for TextMatch {
    fn from(text: &str) -> Self {
        let _ = text;
        unimplemented!()
    }
}

/// Declarative filters, resolved by the engine: no host callback ever runs in the page.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    _private: (),
}

impl Filter {
    /// No filter yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Elements containing this text.
    pub fn has_text(self, text: impl Into<TextMatch>) -> Self {
        let _ = text.into();
        unimplemented!()
    }

    /// Elements not containing this text.
    pub fn has_not_text(self, text: impl Into<TextMatch>) -> Self {
        let _ = text.into();
        unimplemented!()
    }

    /// Elements containing a match of this locator.
    pub fn has(self, locator: &Locator) -> Self {
        let _ = locator;
        unimplemented!()
    }

    /// Elements containing no match of this locator.
    pub fn has_not(self, locator: &Locator) -> Self {
        let _ = locator;
        unimplemented!()
    }

    /// Only visible, or only hidden, elements.
    pub fn visible(self, visible: bool) -> Self {
        let _ = visible;
        unimplemented!()
    }
}

/// The locator constructors pages, frame locators and locators share. None does I/O.
macro_rules! locates {
    ($ty:ty) => {
        impl $ty {
            /// CSS, or XPath when the string starts with `//`, `..` or `xpath=`. CSS pierces open
            /// shadow roots; XPath does not. Playwright's other selector engines, `>>` chains and its
            /// CSS extensions (`:has-text()`, `:text()`, `:visible`, `:nth-match()`) fail with
            /// `selector_invalid`, whose fix names the method that replaces them.
            pub fn locator(&self, selector: &str) -> Locator {
                let _ = selector;
                unimplemented!()
            }

            /// By ARIA role: `get_by_role(AriaRole::Button.name("Search"))`.
            pub fn get_by_role(&self, role: impl Into<ByRole>) -> Locator {
                let _ = role.into();
                unimplemented!()
            }

            /// By text content.
            pub fn get_by_text(&self, text: impl Into<TextMatch>) -> Locator {
                let _ = text.into();
                unimplemented!()
            }

            /// By the text of its label.
            pub fn get_by_label(&self, text: impl Into<TextMatch>) -> Locator {
                let _ = text.into();
                unimplemented!()
            }

            /// By placeholder.
            pub fn get_by_placeholder(&self, text: impl Into<TextMatch>) -> Locator {
                let _ = text.into();
                unimplemented!()
            }

            /// By alternative text.
            pub fn get_by_alt_text(&self, text: impl Into<TextMatch>) -> Locator {
                let _ = text.into();
                unimplemented!()
            }

            /// By title.
            pub fn get_by_title(&self, text: impl Into<TextMatch>) -> Locator {
                let _ = text.into();
                unimplemented!()
            }

            /// By the attribute the setting `locator.test_id_attribute` names, `data-testid` by
            /// default.
            pub fn get_by_test_id(&self, test_id: impl Into<TextMatch>) -> Locator {
                let _ = test_id.into();
                unimplemented!()
            }

            /// Enters an iframe this selector matches, same-origin or cross-origin alike; `None`,
            /// Playwright's selector-less form, starts the search in every frame of the page, and
            /// the rest resolves inside one frame. One name for both shapes (ADR 0099).
            pub fn frame_locator<'s>(&self, selector: impl Into<Option<&'s str>>) -> FrameLocator {
                let _ = selector.into();
                unimplemented!()
            }
        }
    };
}

/// A lazy description of elements, built without I/O and resolved again at each use. A cheap
/// clone that holds its page, so its calls own what they need and are `Send + 'static` (ADR 0081).
#[derive(Debug, Clone)]
pub struct Locator {
    _page: Page,
}

/// An iframe's content, resolved again at each use. Strict: its `iframe` must be unique.
#[derive(Debug, Clone)]
pub struct FrameLocator {
    _page: Page,
}

locates!(Page);
locates!(Locator);
locates!(FrameLocator);

impl FrameLocator {
    /// The `iframe` element itself.
    pub fn owner(&self) -> Locator {
        unimplemented!()
    }
}

/// A modifier key held during a pointer action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Modifier {
    /// Alt or Option.
    Alt,
    /// Control.
    Control,
    /// Control, or Meta on macOS.
    ControlOrMeta,
    /// Meta, Command or Windows.
    Meta,
    /// Shift.
    Shift,
}

/// A mouse button.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseButton {
    /// The primary button.
    #[default]
    Left,
    /// The secondary button.
    Right,
    /// The middle button.
    Middle,
}

/// A point, relative to an element's top-left corner or to the viewport.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    /// Pixels from the left.
    pub x: f64,
    /// Pixels from the top.
    pub y: f64,
}

/// A rectangle in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    /// Its left edge.
    pub x: f64,
    /// Its top edge.
    pub y: f64,
    /// Its width.
    pub width: f64,
    /// Its height.
    pub height: f64,
}

/// A file held in memory.
#[derive(Debug, Clone)]
pub struct FilePayload {
    /// Its file name.
    pub name: String,
    /// Its media type.
    pub mime_type: String,
    /// Its bytes.
    pub buffer: Vec<u8>,
}

/// Files to upload: paths on the program's machine, which the SDK reads and streams over the
/// connection, or files in memory. An empty list clears the input.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum InputFiles {
    /// Files on the program's machine.
    Paths(Vec<PathBuf>),
    /// Files in memory.
    Payloads(Vec<FilePayload>),
}

impl From<&str> for InputFiles {
    fn from(path: &str) -> Self {
        Self::Paths(vec![PathBuf::from(path)])
    }
}

impl From<FilePayload> for InputFiles {
    fn from(file: FilePayload) -> Self {
        Self::Payloads(vec![file])
    }
}

/// An option of a `<select>`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectOption {
    /// By its `value`.
    Value(String),
    /// By its label.
    Label(String),
    /// By its index.
    Index(usize),
}

impl From<&str> for SelectOption {
    fn from(value: &str) -> Self {
        Self::Value(value.to_owned())
    }
}

/// The world a page function runs in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum World {
    /// The page's own: its globals are visible. Marked in Provenance (ADR 0019).
    #[default]
    Main,
    /// One the page can neither see nor hook.
    Isolated,
}

/// What a guarded action is, and what it returns.
pub mod action {
    /// A guarded action's kind.
    pub trait ActionKind: Send + 'static {
        /// What the action returns.
        type Output: Send + 'static;
    }

    /// `click`.
    pub struct Click;
    /// `dblclick`, `tap`, `hover`, `check`, `uncheck` and `set_checked`.
    pub struct Pointer;
    /// `drag_to`.
    pub struct Drag;
    /// `fill`, `clear` and `select_text`.
    pub struct Text;
    /// `press` and `press_sequentially`.
    pub struct Keys;
    /// `select_option`: the values selected.
    pub struct Select;
    /// `set_input_files`.
    pub struct Upload;

    impl ActionKind for Click {
        type Output = ();
    }
    impl ActionKind for Pointer {
        type Output = ();
    }
    impl ActionKind for Drag {
        type Output = ();
    }
    impl ActionKind for Text {
        type Output = ();
    }
    impl ActionKind for Keys {
        type Output = ();
    }
    impl ActionKind for Select {
        type Output = Vec<String>;
    }
    impl ActionKind for Upload {
        type Output = ();
    }
}

/// A guarded action, not sent until awaited. Its timeout is the action phase's (ADR 0080).
#[must_use = "an action does nothing until it is awaited"]
pub struct Action<K: action::ActionKind> {
    _kind: PhantomData<fn() -> K>,
}

impl<K: action::ActionKind> fmt::Debug for Action<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Action(..)")
    }
}

impl<K: action::ActionKind> Action<K> {
    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }

    /// Skips the guard's actionability checks and its replay, keeps uniqueness, and marks the
    /// action unguarded in Provenance: the setting `action.force` for this call (ADR 0023).
    pub fn force(self) -> Self {
        unimplemented!()
    }
}

impl Action<action::Click> {
    /// The button to click with.
    pub fn button(self, button: MouseButton) -> Self {
        let _ = button;
        unimplemented!()
    }

    /// Clicks this many times.
    pub fn click_count(self, count: u32) -> Self {
        let _ = count;
        unimplemented!()
    }

    /// Between pressing and releasing.
    pub fn delay(self, delay: Duration) -> Self {
        let _ = delay;
        unimplemented!()
    }

    /// Keys held during the click.
    pub fn modifiers(self, modifiers: &[Modifier]) -> Self {
        let _ = modifiers;
        unimplemented!()
    }

    /// Where, relative to the element's top-left corner; its visible centre by default.
    pub fn position(self, position: Position) -> Self {
        let _ = position;
        unimplemented!()
    }
}

impl Action<action::Pointer> {
    /// Keys held during the action.
    pub fn modifiers(self, modifiers: &[Modifier]) -> Self {
        let _ = modifiers;
        unimplemented!()
    }

    /// Where, relative to the element's top-left corner.
    pub fn position(self, position: Position) -> Self {
        let _ = position;
        unimplemented!()
    }
}

impl Action<action::Drag> {
    /// Where to grab the source.
    pub fn source_position(self, position: Position) -> Self {
        let _ = position;
        unimplemented!()
    }

    /// Where to drop on the target.
    pub fn target_position(self, position: Position) -> Self {
        let _ = position;
        unimplemented!()
    }
}

impl Action<action::Keys> {
    /// Between key presses.
    pub fn delay(self, delay: Duration) -> Self {
        let _ = delay;
        unimplemented!()
    }
}

impl<K: action::ActionKind> IntoFuture for Action<K> {
    type Output = Result<K::Output, Error>;
    type IntoFuture = CallFuture<K::Output>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// A question answered at once: it never waits, so it takes no timeout.
#[must_use = "a query does nothing until it is awaited"]
pub struct Query<T> {
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for Query<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Query(..)")
    }
}

impl<T: Send + 'static> IntoFuture for Query<T> {
    type Output = Result<T, Error>;
    type IntoFuture = CallFuture<T>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// A page function: JavaScript source run in the page with one explicit argument, its result
/// deserialized from JSON. A value that is not JSON fails with `page_value_invalid`, naming its
/// path, in either direction; `serde_json` writes a non-finite float as `null`, so the SDK checks
/// the argument before sending it rather than relying on that.
#[must_use = "a page function does nothing until it is awaited"]
pub struct Evaluate<T> {
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for Evaluate<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Evaluate(..)")
    }
}

impl<T: DeserializeOwned + Send + 'static> Evaluate<T> {
    /// The one argument the page function receives.
    pub fn arg(self, arg: impl Serialize) -> Self {
        let _ = serde_json::to_value(arg);
        unimplemented!()
    }

    /// The world it runs in, [`World::Main`] by default.
    pub fn world(self, world: World) -> Self {
        let _ = world;
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }
}

impl<T: DeserializeOwned + Send + 'static> IntoFuture for Evaluate<T> {
    type Output = Result<T, Error>;
    type IntoFuture = CallFuture<T>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// How often `wait_for_function` checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Polling {
    /// On every animation frame.
    #[default]
    AnimationFrame,
    /// Every this long.
    Interval(Duration),
}

/// Waits until a page function returns a truthy value, and returns it (ADR 0080).
#[must_use = "a wait does nothing until it is awaited"]
pub struct WaitForFunction<T> {
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for WaitForFunction<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WaitForFunction(..)")
    }
}

impl<T: DeserializeOwned + Send + 'static> WaitForFunction<T> {
    /// The one argument the page function receives.
    pub fn arg(self, arg: impl Serialize) -> Self {
        let _ = serde_json::to_value(arg);
        unimplemented!()
    }

    /// How often to check.
    pub fn polling(self, polling: Polling) -> Self {
        let _ = polling;
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }
}

impl<T: DeserializeOwned + Send + 'static> IntoFuture for WaitForFunction<T> {
    type Output = Result<T, Error>;
    type IntoFuture = CallFuture<T>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// Waits until a page function, given a locator's element first, returns a truthy value, and
/// returns it; the locator is resolved again at each check (ADR 0080).
#[must_use = "a wait does nothing until it is awaited"]
pub struct ElementWait<T> {
    _output: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for ElementWait<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ElementWait(..)")
    }
}

impl<T: DeserializeOwned + Send + 'static> ElementWait<T> {
    /// The one argument the page function receives after the element.
    pub fn arg(self, arg: impl Serialize) -> Self {
        let _ = serde_json::to_value(arg);
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }
}

impl<T: DeserializeOwned + Send + 'static> IntoFuture for ElementWait<T> {
    type Output = Result<T, Error>;
    type IntoFuture = CallFuture<T>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// A screenshot, as image bytes. Secrets are masked by the engine (ADR 0014).
#[must_use = "a screenshot does nothing until it is awaited"]
pub struct Screenshot {
    _private: (),
}

impl fmt::Debug for Screenshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Screenshot(..)")
    }
}

impl Screenshot {
    /// The whole scrollable page, not only the viewport (pages only).
    pub fn full_page(self) -> Self {
        unimplemented!()
    }

    /// Only this rectangle (pages only).
    pub fn clip(self, clip: BoundingBox) -> Self {
        let _ = clip;
        unimplemented!()
    }

    /// Covers what this locator matches with a box.
    pub fn mask(self, locator: &Locator) -> Self {
        let _ = locator;
        unimplemented!()
    }

    /// JPEG at this quality, instead of PNG.
    pub fn jpeg(self, quality: u8) -> Self {
        let _ = quality;
        unimplemented!()
    }

    /// Also writes it to this path on the program's machine, whole or not at all; a failed write
    /// fails the call with `path_not_writable` or `file_write_failed` (ADR 0116).
    pub fn path(self, path: impl Into<PathBuf>) -> Self {
        let _ = path.into();
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }
}

impl IntoFuture for Screenshot {
    type Output = Result<Vec<u8>, Error>;
    type IntoFuture = CallFuture<Vec<u8>>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

impl Locator {
    /// Narrows the matches with declarative filters.
    pub fn filter(&self, filter: Filter) -> Locator {
        let _ = filter;
        unimplemented!()
    }

    /// Elements both locators match.
    pub fn and(&self, locator: &Locator) -> Locator {
        let _ = locator;
        unimplemented!()
    }

    /// Elements either locator matches.
    pub fn or(&self, locator: &Locator) -> Locator {
        let _ = locator;
        unimplemented!()
    }

    /// Opts out of strictness: the first match.
    pub fn first(&self) -> Locator {
        unimplemented!()
    }

    /// The last match.
    pub fn last(&self) -> Locator {
        unimplemented!()
    }

    /// The match at this index.
    pub fn nth(&self, index: usize) -> Locator {
        let _ = index;
        unimplemented!()
    }

    /// The content of the `iframe` this locator matches.
    pub fn content_frame(&self) -> FrameLocator {
        unimplemented!()
    }

    /// One locator per element matching now. Does not wait.
    pub fn all(&self) -> Query<Vec<Locator>> {
        unimplemented!()
    }

    // Gestures: guarded, and run one at a time on the page.

    /// Waits until the target is unique, visible, stable, enabled and receiving the click, then
    /// clicks; a click that lands elsewhere is cancelled and replayed.
    pub fn click(&self) -> Action<action::Click> {
        unimplemented!()
    }

    /// Double-clicks.
    pub fn dblclick(&self) -> Action<action::Pointer> {
        unimplemented!()
    }

    /// Taps.
    pub fn tap(&self) -> Action<action::Pointer> {
        unimplemented!()
    }

    /// Hovers.
    pub fn hover(&self) -> Action<action::Pointer> {
        unimplemented!()
    }

    /// Checks a checkbox or a radio button.
    pub fn check(&self) -> Action<action::Pointer> {
        unimplemented!()
    }

    /// Unchecks a checkbox.
    pub fn uncheck(&self) -> Action<action::Pointer> {
        unimplemented!()
    }

    /// Checks or unchecks.
    pub fn set_checked(&self, checked: bool) -> Action<action::Pointer> {
        let _ = checked;
        unimplemented!()
    }

    /// Drags to the target, which the guard checks too.
    pub fn drag_to(&self, target: &Locator) -> Action<action::Drag> {
        let _ = target;
        unimplemented!()
    }

    /// Clears the field and inserts the text as typed text arrives, so controlled inputs see it; on
    /// a `<label>`, into its control.
    /// A target that cannot hold text fails at once with `target_unsupported`; a read-only one is
    /// waited for.
    /// A Secret placeholder is typed by the engine, checked against the domain of the field's frame
    /// (ADR 0092); reads of the field then show `%name%`.
    pub fn fill(&self, value: impl Into<TextInput>) -> Action<action::Text> {
        let _ = value.into();
        unimplemented!()
    }

    /// Clears the field.
    pub fn clear(&self) -> Action<action::Text> {
        unimplemented!()
    }

    /// Selects the element's text.
    pub fn select_text(&self) -> Action<action::Text> {
        unimplemented!()
    }

    /// A key or a combination, such as `Enter` or `ControlOrMeta+A`.
    pub fn press(&self, key: &str) -> Action<action::Keys> {
        let _ = key;
        unimplemented!()
    }

    /// Types key by key, for fields that react to each keystroke; a Secret placeholder as `fill`
    /// types it.
    pub fn press_sequentially(&self, text: impl Into<TextInput>) -> Action<action::Keys> {
        let _ = text.into();
        unimplemented!()
    }

    /// On a `<select>`, or the control a `<label>` points to; returns the values selected.
    pub fn select_option(
        &self,
        options: impl IntoIterator<Item = SelectOption>,
    ) -> Action<action::Select> {
        let _ = options.into_iter();
        unimplemented!()
    }

    /// On a file input, or the one a `<label>` points to.
    pub fn set_input_files(&self, files: impl Into<InputFiles>) -> Action<action::Upload> {
        let _ = files.into();
        unimplemented!()
    }

    /// Focuses the element.
    pub fn focus(&self) -> Call<()> {
        unimplemented!()
    }

    /// Removes focus from the element.
    pub fn blur(&self) -> Call<()> {
        unimplemented!()
    }

    /// Scrolls the element into view.
    pub fn scroll_into_view_if_needed(&self) -> Call<()> {
        unimplemented!()
    }

    // Reads: one round trip each, whatever the number of matches.

    /// Waits for one element, as an action does, under the action timeout.
    pub fn text_content(&self) -> Call<String> {
        unimplemented!()
    }

    /// Its rendered text.
    pub fn inner_text(&self) -> Call<String> {
        unimplemented!()
    }

    /// Its inner HTML.
    pub fn inner_html(&self) -> Call<String> {
        unimplemented!()
    }

    /// An input's value.
    pub fn input_value(&self) -> Call<String> {
        unimplemented!()
    }

    /// `None` when the attribute is absent: an answer, not a failure.
    pub fn get_attribute(&self, name: &str) -> Call<Option<String>> {
        let _ = name;
        unimplemented!()
    }

    /// `None` when the element is not visible.
    pub fn bounding_box(&self) -> Call<Option<BoundingBox>> {
        unimplemented!()
    }

    /// Waits for one element, then answers.
    pub fn is_enabled(&self) -> Call<bool> {
        unimplemented!()
    }

    /// Waits for one element, then answers.
    pub fn is_disabled(&self) -> Call<bool> {
        unimplemented!()
    }

    /// Waits for one element, then answers.
    pub fn is_checked(&self) -> Call<bool> {
        unimplemented!()
    }

    /// Waits for one element, then answers.
    pub fn is_editable(&self) -> Call<bool> {
        unimplemented!()
    }

    /// Answers at once: no match is not visible; two matches fail with `target_not_unique`.
    pub fn is_visible(&self) -> Query<bool> {
        unimplemented!()
    }

    /// Answers at once.
    pub fn is_hidden(&self) -> Query<bool> {
        unimplemented!()
    }

    /// Answers at once. To wait for a count, [`Locator::wait_for_count`].
    pub fn count(&self) -> Query<usize> {
        unimplemented!()
    }

    /// Every match's text content, at once.
    pub fn all_text_contents(&self) -> Query<Vec<String>> {
        unimplemented!()
    }

    /// Every match's rendered text, at once.
    pub fn all_inner_texts(&self) -> Query<Vec<String>> {
        unimplemented!()
    }

    /// A screenshot of the element.
    pub fn screenshot(&self) -> Screenshot {
        unimplemented!()
    }

    /// A page function given the element first, then its argument.
    pub fn evaluate<T: DeserializeOwned + Send + 'static>(&self, source: &str) -> Evaluate<T> {
        let _ = source;
        unimplemented!()
    }

    /// A page function given every matching element first.
    pub fn evaluate_all<T: DeserializeOwned + Send + 'static>(&self, source: &str) -> Evaluate<T> {
        let _ = source;
        unimplemented!()
    }

    // Waits (ADR 0080).

    /// Waits until a page function, given the element first, returns a truthy value; the locator is
    /// resolved again at each check.
    pub fn wait_for_function<T: DeserializeOwned + Send + 'static>(
        &self,
        source: &str,
    ) -> ElementWait<T> {
        let _ = source;
        unimplemented!()
    }

    /// Playwright's `wait_for`: one element in the given state.
    pub fn wait_for(&self, state: ElementState) -> Call<()> {
        let _ = state;
        unimplemented!()
    }

    /// Waits until the number of matches is in range, and returns it: a different behaviour, so a
    /// different name (S3).
    pub fn wait_for_count(&self, count: Count) -> Call<usize> {
        let _ = count;
        unimplemented!()
    }
}

// Requests and responses.

/// Header names lower-cased, as the browser sent or received them.
pub type Headers = BTreeMap<String, String>;

/// A request's resource type. A newer engine's is kept as received (G3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceType {
    /// `document`
    Document,
    /// `stylesheet`
    Stylesheet,
    /// `image`
    Image,
    /// `media`
    Media,
    /// `font`
    Font,
    /// `script`
    Script,
    /// `xhr`
    Xhr,
    /// `fetch`
    Fetch,
    /// `websocket`
    WebSocket,
    /// `other`, `manifest`, `texttrack`, `eventsource`.
    Other,
    /// A type this SDK does not know, as received.
    Unknown(String),
}

/// A request the page sent, under Playwright's accessors.
#[derive(Debug, Clone)]
pub struct Request {
    _private: (),
}

impl Request {
    /// Its URL.
    pub fn url(&self) -> &str {
        unimplemented!()
    }

    /// Its HTTP method.
    pub fn method(&self) -> &str {
        unimplemented!()
    }

    /// Its headers.
    pub fn headers(&self) -> &Headers {
        unimplemented!()
    }

    /// Its body, as text, if any.
    pub fn post_data(&self) -> Option<&str> {
        unimplemented!()
    }

    /// Its resource type.
    pub fn resource_type(&self) -> &ResourceType {
        unimplemented!()
    }

    /// Whether it drives a frame's navigation.
    pub fn is_navigation_request(&self) -> bool {
        unimplemented!()
    }

    /// The request a redirect turned into this one.
    pub fn redirected_from(&self) -> Option<&Request> {
        unimplemented!()
    }

    /// The browser's network error for a failed request, such as `net::ERR_CONNECTION_RESET`.
    pub fn failure(&self) -> Option<&str> {
        unimplemented!()
    }

    /// Its response, once received; `None` when it failed.
    pub fn response(&self) -> Call<Option<Response>> {
        unimplemented!()
    }
}

/// A response, from the page, from [`Route::fetch`] or from [`PageRequest`].
#[derive(Debug, Clone)]
pub struct Response {
    _private: (),
}

impl Response {
    /// Its final URL.
    pub fn url(&self) -> &str {
        unimplemented!()
    }

    /// Its HTTP status.
    pub fn status(&self) -> u16 {
        unimplemented!()
    }

    /// Its status text.
    pub fn status_text(&self) -> &str {
        unimplemented!()
    }

    /// `true` for a status from 200 to 299. It says nothing about the page itself: a login page
    /// that answers 200 is `ok`, and the Page check judges pages (ADR 0006).
    pub fn ok(&self) -> bool {
        unimplemented!()
    }

    /// Its headers.
    pub fn headers(&self) -> &Headers {
        unimplemented!()
    }

    /// The request it answers.
    pub fn request(&self) -> &Request {
        unimplemented!()
    }

    /// `true` when a site's service worker's fetch handler supplied the response, from its own
    /// cache or through its own fetch, Playwright's accessor (ADR 0121). No page route saw the
    /// page's request; what reached the network was the service worker's own request, which
    /// Session routes see.
    pub fn from_service_worker(&self) -> bool {
        unimplemented!()
    }

    /// Read on demand. Never fails for a response from `wait_for_request_finished`,
    /// [`Route::fetch`] or [`PageRequest`], whose bodies the engine keeps; elsewhere a body the
    /// browser has dropped fails with `body_unavailable`.
    pub fn body(&self) -> Call<Vec<u8>> {
        unimplemented!()
    }

    /// The body decoded with the response's own encoding, where Playwright always decodes UTF-8
    /// (ADR 0119): a navigation's document response with the encoding its document was decoded
    /// with, so it agrees with `page.content()`; any other with its BOM, its `Content-Type`
    /// charset, the `<meta>` prescan for HTML or the XML declaration for XML, then UTF-8. Bad bytes
    /// become U+FFFD; it never fails on them.
    pub fn text(&self) -> Call<String> {
        unimplemented!()
    }

    /// The body as JSON, read as UTF-8 whatever the charset (RFC 8259), a leading BOM ignored; one
    /// that is not JSON fails with `response_not_json`. Deserialize it into the program's own type.
    pub fn json(&self) -> Call<serde_json::Value> {
        unimplemented!()
    }
}

/// The network error an aborted request reports to the page, under Playwright's names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum AbortReason {
    /// `aborted`
    Aborted,
    /// `accessdenied`
    AccessDenied,
    /// `addressunreachable`
    AddressUnreachable,
    /// `blockedbyclient`
    BlockedByClient,
    /// `blockedbyresponse`
    BlockedByResponse,
    /// `connectionaborted`
    ConnectionAborted,
    /// `connectionclosed`
    ConnectionClosed,
    /// `connectionfailed`
    ConnectionFailed,
    /// `connectionrefused`
    ConnectionRefused,
    /// `connectionreset`
    ConnectionReset,
    /// `internetdisconnected`
    InternetDisconnected,
    /// `namenotresolved`
    NameNotResolved,
    /// `timedout`
    TimedOut,
    /// `failed`
    #[default]
    Failed,
}

/// An intercepted request waiting for the program's answer. The first route to answer wins:
/// [`Route::r#continue`], [`Route::fulfill`], [`Route::abort`], or [`Route::fetch`] then
/// `fulfill`; each takes the route, so it cannot be answered twice. [`Route::fallback`] passes it
/// to the route registered before this one, and from the page's first route to the Session's last;
/// dropping it unanswered falls back too, and past the last route the request goes to the network. What a route sends on still passes the run's
/// domain policy (ADR 0014) and Filter lists (ADR 0047).
#[derive(Debug)]
pub struct Route {
    _private: (),
}

impl Route {
    /// The request as the page sent it, before any override.
    pub fn request(&self) -> &Request {
        unimplemented!()
    }

    /// Sends the request on, with any overrides.
    pub fn r#continue(self) -> Forward {
        unimplemented!()
    }

    /// Passes the request, with any overrides, to the route registered before this one.
    pub fn fallback(self) -> Forward {
        unimplemented!()
    }

    /// Answers with a response of the program's own; nothing reaches the network.
    pub fn fulfill(self) -> Fulfill {
        unimplemented!()
    }

    /// Fails the request.
    pub fn abort(self) -> Abort {
        unimplemented!()
    }

    /// Sends the request through the engine and returns its response, body kept, for `fulfill`.
    pub fn fetch(&self) -> Call<Response> {
        unimplemented!()
    }
}

impl Drop for Route {
    /// An unanswered route falls back, without blocking.
    fn drop(&mut self) {}
}

/// Overrides `r#continue` and `fallback` pass on with the request.
#[must_use = "an answer does nothing until it is awaited"]
pub struct Forward {
    _private: (),
}

impl fmt::Debug for Forward {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Forward(..)")
    }
}

impl Forward {
    /// Another URL, re-checked by the domain policy.
    pub fn url(self, url: &str) -> Self {
        let _ = url;
        unimplemented!()
    }

    /// Another method.
    pub fn method(self, method: &str) -> Self {
        let _ = method;
        unimplemented!()
    }

    /// Sets a header; a Secret placeholder is checked against the request's URL (ADR 0092).
    pub fn header(self, name: &str, value: impl Into<TextInput>) -> Self {
        let _ = (name, value.into());
        unimplemented!()
    }

    /// Another body.
    pub fn post_data(self, body: impl Into<Vec<u8>>) -> Self {
        let _ = body.into();
        unimplemented!()
    }
}

impl IntoFuture for Forward {
    type Output = Result<(), Error>;
    type IntoFuture = CallFuture<()>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// A response the program answers a route with.
#[must_use = "an answer does nothing until it is awaited"]
pub struct Fulfill {
    _private: (),
}

impl fmt::Debug for Fulfill {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Fulfill(..)")
    }
}

impl Fulfill {
    /// Starts from a response [`Route::fetch`] returned.
    pub fn response(self, response: &Response) -> Self {
        let _ = response;
        unimplemented!()
    }

    /// Its status, 200 by default.
    pub fn status(self, status: u16) -> Self {
        let _ = status;
        unimplemented!()
    }

    /// Sets a header.
    pub fn header(self, name: &str, value: &str) -> Self {
        let _ = (name, value);
        unimplemented!()
    }

    /// Its content type.
    pub fn content_type(self, content_type: &str) -> Self {
        let _ = content_type;
        unimplemented!()
    }

    /// Its body.
    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        let _ = body.into();
        unimplemented!()
    }

    /// A JSON body.
    pub fn json(self, value: impl Serialize) -> Self {
        let _ = serde_json::to_value(value);
        unimplemented!()
    }
}

impl IntoFuture for Fulfill {
    type Output = Result<(), Error>;
    type IntoFuture = CallFuture<()>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// Fails a route's request.
#[must_use = "an answer does nothing until it is awaited"]
pub struct Abort {
    _private: (),
}

impl fmt::Debug for Abort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Abort(..)")
    }
}

impl Abort {
    /// The network error the page sees, [`AbortReason::Failed`] by default.
    pub fn reason(self, reason: AbortReason) -> Self {
        let _ = reason;
        unimplemented!()
    }
}

impl IntoFuture for Abort {
    type Output = Result<(), Error>;
    type IntoFuture = CallFuture<()>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// An HTTP request on the page's Session, through the browser-fetch tier so the Session's
/// identity does not change (ADR 0020). Its response keeps its body.
#[must_use = "a request does nothing until it is awaited"]
pub struct ApiRequest {
    _private: (),
}

impl fmt::Debug for ApiRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiRequest(..)")
    }
}

impl ApiRequest {
    /// Sets a header; a Secret placeholder is checked against the request's URL (ADR 0092).
    pub fn header(self, name: &str, value: impl Into<TextInput>) -> Self {
        let _ = (name, value.into());
        unimplemented!()
    }

    /// Adds a query parameter.
    pub fn query(self, name: &str, value: &str) -> Self {
        let _ = (name, value);
        unimplemented!()
    }

    /// A body as it is.
    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        let _ = body.into();
        unimplemented!()
    }

    /// A JSON body.
    pub fn json(self, value: impl Serialize) -> Self {
        let _ = serde_json::to_value(value);
        unimplemented!()
    }

    /// Sets the string at this JSON Pointer of the JSON body to a Secret placeholder, which the
    /// engine resolves at its own interception, checked against the request's URL at every hop
    /// (ADR 0092).
    pub fn json_secret(self, pointer: &str, placeholder: crate::sessions::SecretPlaceholder) -> Self {
        let _ = (pointer, placeholder);
        unimplemented!()
    }

    /// At most this many redirects: Playwright's, 20 by default, the browser-fetch tier's most;
    /// `0` returns the 3xx itself. Past the count, `too_many_redirects`; above 20,
    /// `setting_invalid` (ADR 0118).
    pub fn max_redirects(self, redirects: u32) -> Self {
        let _ = redirects;
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }
}

impl IntoFuture for ApiRequest {
    type Output = Result<Response, Error>;
    type IntoFuture = CallFuture<Response>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// Playwright's request API on the page's Session.
#[derive(Debug, Clone)]
pub struct PageRequest {
    _page: Page,
}

impl PageRequest {
    /// A request with this method.
    pub fn fetch(&self, method: &str, url: &str) -> ApiRequest {
        let _ = (method, url);
        unimplemented!()
    }

    /// A `GET`.
    pub fn get(&self, url: &str) -> ApiRequest {
        let _ = url;
        unimplemented!()
    }

    /// A `POST`.
    pub fn post(&self, url: &str) -> ApiRequest {
        let _ = url;
        unimplemented!()
    }
}

// Events and requests that need an answer. Their markers are in `events`.

/// Where a console message came from.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ConsoleLocation {
    /// The script's URL.
    pub url: String,
    /// Its line, from zero.
    pub line_number: u32,
    /// Its column, from zero.
    pub column_number: u32,
}

/// A console message.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ConsoleMessage {
    /// `log`, `debug`, `info`, `error`, `warning`, …, kept as received.
    pub kind: String,
    /// Its text.
    pub text: String,
    /// Where it came from.
    pub location: ConsoleLocation,
}

/// An exception the page's own scripts did not catch (`pageerror`), under Playwright's name for it.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WebError {
    /// Its `name`, such as `TypeError`.
    pub name: String,
    /// Its message.
    pub message: String,
    /// Its stack, when the page gave one.
    pub stack: Option<String>,
}

/// A JavaScript dialog's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DialogKind {
    /// `alert()`
    Alert,
    /// `confirm()`
    Confirm,
    /// `prompt()`
    Prompt,
    /// `beforeunload`: accepted by default; raised by a navigation, or by closing with
    /// `run_before_unload`.
    BeforeUnload,
}

/// A JavaScript dialog waiting for the program's answer. Later subscribers get it first; dropping
/// it unanswered passes it on, and past the last one, or with none, it gets Playwright's default:
/// an `alert`, `confirm` or `prompt` is dismissed, since accepting a `confirm()` may confirm an
/// Effect (ADR 0014), and a `beforeunload` dialog is accepted.
#[derive(Debug)]
pub struct Dialog {
    _private: (),
}

impl Dialog {
    /// Its kind.
    pub fn kind(&self) -> DialogKind {
        unimplemented!()
    }

    /// Its message.
    pub fn message(&self) -> &str {
        unimplemented!()
    }

    /// A prompt's default value.
    pub fn default_value(&self) -> &str {
        unimplemented!()
    }

    /// Accepts it.
    pub fn accept(self) -> Call<()> {
        unimplemented!()
    }

    /// Accepts a prompt with this text.
    pub fn accept_with(self, prompt_text: &str) -> Call<()> {
        let _ = prompt_text;
        unimplemented!()
    }

    /// Dismisses it.
    pub fn dismiss(self) -> Call<()> {
        unimplemented!()
    }
}

impl Drop for Dialog {
    /// An unanswered dialog passes on, without blocking.
    fn drop(&mut self) {}
}

/// A download, kept by the engine with its Run.
#[derive(Debug, Clone)]
pub struct Download {
    _private: (),
}

impl Download {
    /// Where it came from.
    pub fn url(&self) -> &str {
        unimplemented!()
    }

    /// The file name the site suggests.
    pub fn suggested_filename(&self) -> &str {
        unimplemented!()
    }

    /// Waits for it to end: the reason it failed, or `None`. An answer, not a failure.
    pub fn failure(&self) -> Call<Option<String>> {
        unimplemented!()
    }

    /// Streams the file to this path on the program's machine; a failed download fails with
    /// `download_failed`, and a failed write with `path_not_writable` or `file_write_failed`,
    /// leaving the path as it was (ADR 0116).
    pub fn save_as(&self, path: impl Into<PathBuf>) -> Call<()> {
        let _ = path.into();
        unimplemented!()
    }

    /// Cancels it.
    pub fn cancel(&self) -> Call<()> {
        unimplemented!()
    }
}

/// A file chooser the page opened, answered with [`FileChooser::set_files`], a gesture.
#[derive(Debug)]
pub struct FileChooser {
    _private: (),
}

impl FileChooser {
    /// Whether it takes several files.
    pub fn is_multiple(&self) -> bool {
        unimplemented!()
    }

    /// Answers it.
    pub fn set_files(self, files: impl Into<InputFiles>) -> Call<()> {
        let _ = files.into();
        unimplemented!()
    }
}

/// A WebSocket frame's payload.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum WebSocketPayload {
    /// A text frame.
    Text(String),
    /// A binary frame.
    Binary(Vec<u8>),
}

/// A WebSocket the page opened.
#[derive(Debug, Clone)]
pub struct WebSocket {
    _private: (),
}

impl WebSocket {
    /// Its URL.
    pub fn url(&self) -> &str {
        unimplemented!()
    }

    /// Whether it has closed.
    pub fn is_closed(&self) -> bool {
        unimplemented!()
    }

    /// Frames the page sent, subscribed when called.
    pub fn frames_sent(&self) -> EventStream<WebSocketPayload> {
        unimplemented!()
    }

    /// Frames the page received, subscribed when called.
    pub fn frames_received(&self) -> EventStream<WebSocketPayload> {
        unimplemented!()
    }

    /// Waits until it closes: its `close` event.
    pub fn closed(&self) -> Call<()> {
        unimplemented!()
    }
}

/// A locator handler's turn: the guard found its locator visible before an attempt of a guarded
/// action and waits until this is dropped, then for the locator to be hidden. Meanwhile the action
/// gives up the page's turn, so the program's own gestures run.
#[derive(Debug)]
pub struct LocatorHandlerCall {
    _private: (),
}

impl LocatorHandlerCall {
    /// The locator that appeared.
    pub fn locator(&self) -> &Locator {
        unimplemented!()
    }
}

impl Drop for LocatorHandlerCall {
    /// Lets the action go on, without blocking.
    fn drop(&mut self) {}
}

/// Options of [`Page::add_locator_handler`].
#[derive(Debug, Clone, Default)]
pub struct LocatorHandlerOptions {
    _private: (),
}

impl LocatorHandlerOptions {
    /// Default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// After the handler, do not wait for the locator to be hidden.
    pub fn no_wait_after(self) -> Self {
        unimplemented!()
    }

    /// Removed after this many turns.
    pub fn times(self, times: u32) -> Self {
        let _ = times;
        unimplemented!()
    }
}

/// An init script, registered when added and removed, without blocking, when dropped.
#[must_use = "dropping a registration removes its init script"]
#[derive(Debug)]
pub struct Registration {
    _private: (),
}

impl Registration {
    /// Keeps the init script until its page or Session closes, as TypeScript's stays until
    /// disposed: a crawl's Hook adds one this way for the rest of its attempt (ADR 0111).
    pub fn keep(self) {
        unimplemented!()
    }
}

impl Drop for Registration {
    fn drop(&mut self) {}
}

// Input with no target: nothing to guard, so marked unguarded in Provenance.

/// The page's keyboard.
#[derive(Debug, Clone)]
pub struct Keyboard {
    _page: Page,
}

impl Keyboard {
    /// Presses a key or a combination.
    pub fn press(&self, key: &str) -> Call<()> {
        let _ = key;
        unimplemented!()
    }

    /// Holds a key down.
    pub fn down(&self, key: &str) -> Call<()> {
        let _ = key;
        unimplemented!()
    }

    /// Releases a key.
    pub fn up(&self, key: &str) -> Call<()> {
        let _ = key;
        unimplemented!()
    }

    /// Types text key by key.
    pub fn r#type(&self, text: &str) -> Call<()> {
        let _ = text;
        unimplemented!()
    }

    /// Inserts text with no key events.
    pub fn insert_text(&self, text: &str) -> Call<()> {
        let _ = text;
        unimplemented!()
    }
}

/// The page's mouse.
#[derive(Debug, Clone)]
pub struct Mouse {
    _page: Page,
}

impl Mouse {
    /// Moves to a point of the viewport.
    pub fn r#move(&self, x: f64, y: f64) -> Call<()> {
        let _ = (x, y);
        unimplemented!()
    }

    /// Presses a button.
    pub fn down(&self, button: MouseButton) -> Call<()> {
        let _ = button;
        unimplemented!()
    }

    /// Releases a button.
    pub fn up(&self, button: MouseButton) -> Call<()> {
        let _ = button;
        unimplemented!()
    }

    /// Clicks at a point of the viewport.
    pub fn click(&self, x: f64, y: f64) -> Call<()> {
        let _ = (x, y);
        unimplemented!()
    }

    /// Scrolls by this much.
    pub fn wheel(&self, delta_x: f64, delta_y: f64) -> Call<()> {
        let _ = (delta_x, delta_y);
        unimplemented!()
    }
}

/// The page's touchscreen.
#[derive(Debug, Clone)]
pub struct Touchscreen {
    _page: Page,
}

impl Touchscreen {
    /// Taps a point of the viewport.
    pub fn tap(&self, x: f64, y: f64) -> Call<()> {
        let _ = (x, y);
        unimplemented!()
    }
}

// The way down.

/// Raw Chrome DevTools Protocol for one page, over the program's own connection. It is a raw CDP
/// attach under ADR 0017: the Session's traffic goes through the egress proxy (on a Borrowed
/// browser, ADR 0056's interception instead), no Secret is typed,
/// its results are marked unguarded and the attach is in the Audit log. It reaches only its page's
/// target: commands that close or attach other targets, and anything that would close a Borrowed
/// browser, fail with `cdp_command_refused`. Semver covers this shape, never the protocol, which
/// is Chromium's. A cheap clone (ADR 0081).
#[derive(Debug, Clone)]
pub struct CdpSession {
    _private: (),
}

impl CdpSession {
    /// Sends a command and returns its result.
    pub fn send(&self, method: &str, params: serde_json::Value) -> Call<serde_json::Value> {
        let _ = (method, params);
        unimplemented!()
    }

    /// An event's stream, subscribed when called.
    pub fn events(&self, event: &str) -> EventStream<serde_json::Value> {
        let _ = event;
        unimplemented!()
    }

    /// Detaches. Idempotent.
    pub fn close(&self) -> Call<()> {
        unimplemented!()
    }
}

/// A navigation: a gesture, whose timeout is the navigation phase's.
#[must_use = "a navigation does nothing until it is awaited"]
pub struct Navigation {
    _private: (),
}

impl fmt::Debug for Navigation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Navigation(..)")
    }
}

impl Navigation {
    /// The load state to wait for, `load` by default.
    pub fn wait_until(self, state: LoadState) -> Self {
        let _ = state;
        unimplemented!()
    }

    /// The `Referer` header.
    pub fn referer(self, referer: &str) -> Self {
        let _ = referer;
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }

    /// No timeout of its own; an enclosing timeout still applies.
    pub fn no_timeout(self) -> Self {
        unimplemented!()
    }
}

impl IntoFuture for Navigation {
    /// A 404 or a 500 is a response, a value; `None` for a navigation within the same document.
    type Output = Result<Option<Response>, Error>;
    type IntoFuture = CallFuture<Option<Response>>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// Closing a page.
#[must_use = "a close does nothing until it is awaited"]
pub struct PageClose {
    _private: (),
}

impl fmt::Debug for PageClose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PageClose(..)")
    }
}

impl PageClose {
    /// Runs the page's `beforeunload` handlers, which closing otherwise skips.
    pub fn run_before_unload(self) -> Self {
        unimplemented!()
    }

    /// At most this long, within what remains of any enclosing timeout.
    pub fn timeout(self, timeout: Duration) -> Self {
        let _ = timeout;
        unimplemented!()
    }
}

impl IntoFuture for PageClose {
    type Output = Result<(), Error>;
    type IntoFuture = CallFuture<()>;

    fn into_future(self) -> Self::IntoFuture {
        unimplemented!()
    }
}

/// The page API beside the waits (ADR 0080) and events (ADR 0081).
impl Page {
    /// Navigates. A 404 or a 500 is a response, a value.
    pub fn goto(&self, url: &str) -> Navigation {
        let _ = url;
        unimplemented!()
    }

    /// Reloads.
    pub fn reload(&self) -> Navigation {
        unimplemented!()
    }

    /// Goes back in history.
    pub fn go_back(&self) -> Navigation {
        unimplemented!()
    }

    /// Goes forward in history.
    pub fn go_forward(&self) -> Navigation {
        unimplemented!()
    }

    /// Replaces the document.
    pub fn set_content(&self, html: &str) -> Call<()> {
        let _ = html;
        unimplemented!()
    }

    /// Read without a round trip, kept current by the engine's navigation events.
    pub fn url(&self) -> String {
        unimplemented!()
    }

    /// Its title.
    pub fn title(&self) -> Call<String> {
        unimplemented!()
    }

    /// Its HTML.
    pub fn content(&self) -> Call<String> {
        unimplemented!()
    }

    /// Whether it has closed.
    pub fn is_closed(&self) -> bool {
        unimplemented!()
    }

    /// The Session this page belongs to: Playwright's `page.context()`.
    pub fn session(&self) -> Session {
        unimplemented!()
    }

    /// Its keyboard.
    pub fn keyboard(&self) -> Keyboard {
        unimplemented!()
    }

    /// Its mouse.
    pub fn mouse(&self) -> Mouse {
        unimplemented!()
    }

    /// Its touchscreen.
    pub fn touchscreen(&self) -> Touchscreen {
        unimplemented!()
    }

    /// HTTP requests on its Session.
    pub fn request(&self) -> PageRequest {
        unimplemented!()
    }

    /// JavaScript source run in the page with one explicit argument: a function if it parses as
    /// one, otherwise an expression.
    pub fn evaluate<T: DeserializeOwned + Send + 'static>(&self, source: &str) -> Evaluate<T> {
        let _ = source;
        unimplemented!()
    }

    /// Waits until a page function returns a truthy value, and returns it.
    pub fn wait_for_function<T: DeserializeOwned + Send + 'static>(
        &self,
        source: &str,
    ) -> WaitForFunction<T> {
        let _ = source;
        unimplemented!()
    }

    /// Runs before the page's own scripts in every new document, marked in Provenance; registered
    /// when called. An argument that is not JSON fails here with `page_value_invalid`.
    pub fn add_init_script(
        &self,
        source: &str,
        arg: impl Serialize,
    ) -> Result<Registration, Error> {
        let _ = (source, serde_json::to_value(arg));
        unimplemented!()
    }

    /// A screenshot of the page.
    pub fn screenshot(&self) -> Screenshot {
        unimplemented!()
    }

    /// The page as a PDF. Chromium only.
    pub fn pdf(&self) -> Call<Vec<u8>> {
        unimplemented!()
    }

    /// Requests matching `url`, each waiting for the program's answer, subscribed when called. A
    /// glob or a regular expression matches in the engine; a predicate runs in the program and, like
    /// a route on every URL, has every request wait for it (ADR 0081). Page routes get a request
    /// before the Session's. A stream kept but never read holds its requests until whatever waits
    /// on them reaches its deadline.
    pub fn route(&self, url: impl Into<UrlMatcher>) -> EventStream<Route> {
        let _ = url.into();
        unimplemented!()
    }

    /// The locator handler's turns, subscribed when called: before each attempt of a guarded
    /// action, when `locator` is visible, the guard yields a turn and waits for it to be dropped.
    /// Turns come one at a time, their own actions trigger none, and their time counts toward the
    /// action's timeout.
    pub fn add_locator_handler(
        &self,
        locator: &Locator,
        options: LocatorHandlerOptions,
    ) -> EventStream<LocatorHandlerCall> {
        let _ = (locator, options);
        unimplemented!()
    }

    /// Raw CDP for this page. Chromium only: elsewhere it fails with `capability_unavailable`.
    pub fn new_cdp_session(&self) -> Call<CdpSession> {
        unimplemented!()
    }
}

/// A Session's page members (ADR 0085); its other API is #476's.
impl Session {
    /// The Session's open pages, read without a round trip.
    pub fn pages(&self) -> Vec<Page> {
        unimplemented!()
    }

    /// Requests of every page of the Session, a popup's first request included; page routes get
    /// them first.
    pub fn route(&self, url: impl Into<UrlMatcher>) -> EventStream<Route> {
        let _ = url.into();
        unimplemented!()
    }

    /// An init script for every new document of the Session.
    pub fn add_init_script(
        &self,
        source: &str,
        arg: impl Serialize,
    ) -> Result<Registration, Error> {
        let _ = (source, serde_json::to_value(arg));
        unimplemented!()
    }
}
