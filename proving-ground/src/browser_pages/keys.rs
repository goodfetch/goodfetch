//! The `keys` family: a text field (`/type`) whose script records every
//! key, input and composition event it receives, in order, in the page's
//! event log (with each event's key, code, data and composition state, and
//! the keyboard layout when the browser tells it), and reports to the site
//! each key press of a character (its key and code), each input outside a
//! composition (the field's value after it) and each composition's end
//! (the text it committed). Each task's page shows the text to type: on a
//! US QWERTY layout, on a French AZERTY layout, or through an input
//! method's composition. Since each input reports the whole value so far,
//! a sequence typed out of order, with a character missed or doubled, or
//! pasted in one go, differs from the one asked for.

use std::collections::BTreeMap;

use super::document;
use crate::exchange::Reply;
use crate::generator;
use crate::task::{self, Effect, Entry, Expected, Kind, Task};

pub(super) const HOST: &str = "keys.test";
pub(super) const FAMILY: &str = "keys";
pub(super) const TASKS: u32 = 3;

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 25;

/// The texts typed on a US QWERTY layout.
const QWERTY_TEXTS: [&str; 6] = [
    "Hello, World 42!",
    "Pack my box with 5 jugs.",
    "Quiet fox, 7 lazy dogs?",
    "Jump over 19 bricks!",
    "Seven owls, 3 cats.",
    "Bright Lamp 86, Dim Room.",
];

/// The texts typed on a French AZERTY layout: lowercase words whose keys
/// sit elsewhere than on QWERTY.
const AZERTY_TEXTS: [&str; 4] = [
    "quiz maze swamp",
    "aqua mozart wiz",
    "zoom map quest",
    "wax quartz amaze",
];

/// The texts an input method composes.
const COMPOSED_TEXTS: [&str; 6] = [
    "日本語",
    "東京",
    "漢字",
    "ひらがな",
    "カタカナ",
    "こんにちは",
];

const SCRIPT: &str = include_str!("../../pages/browser/keys.js");

/// How a task's text is typed.
#[derive(Clone, Copy)]
enum Typing {
    Qwerty,
    Azerty,
    Composed,
}

/// Each task's typing, by ordinal.
const TYPINGS: [Typing; 3] = [Typing::Qwerty, Typing::Azerty, Typing::Composed];

/// The text task `ordinal` asks for, for `seed`; `None` past the tasks.
fn text(seed: u64, ordinal: u32) -> Option<&'static str> {
    let typing = TYPINGS.get(usize::try_from(ordinal).ok()?.checked_sub(1)?)?;
    let texts: &[&str] = match typing {
        Typing::Qwerty => &QWERTY_TEXTS,
        Typing::Azerty => &AZERTY_TEXTS,
        Typing::Composed => &COMPOSED_TEXTS,
    };
    let index = generator::below(
        seed,
        &[FAMILY_PLACE, u64::from(ordinal)],
        texts.len() as u64,
    );
    texts.get(usize::try_from(index).ok()?).copied()
}

/// The `code` of the physical key a US QWERTY layout types `character`
/// with, for the characters of [`QWERTY_TEXTS`].
fn qwerty_code(character: char) -> String {
    match character {
        ' ' => "Space".to_owned(),
        ',' => "Comma".to_owned(),
        '.' => "Period".to_owned(),
        '!' => "Digit1".to_owned(),
        '?' => "Slash".to_owned(),
        digit if digit.is_ascii_digit() => format!("Digit{digit}"),
        letter => format!("Key{}", letter.to_ascii_uppercase()),
    }
}

/// The `code` of the physical key a French AZERTY layout types
/// `character` with, for the characters of [`AZERTY_TEXTS`].
fn azerty_code(character: char) -> String {
    match character {
        'a' => "KeyQ".to_owned(),
        'q' => "KeyA".to_owned(),
        'z' => "KeyW".to_owned(),
        'w' => "KeyZ".to_owned(),
        'm' => "Semicolon".to_owned(),
        other => qwerty_code(other),
    }
}

/// An event the page reports on its field.
fn event(event: &str, details: &[(&str, String)]) -> Effect {
    Effect::Event {
        event: event.to_owned(),
        target: "field".to_owned(),
        trusted: true,
        details: details
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
    }
}

/// What typing `text` key by key reports: each key press with its key and
/// code, and each input with the field's value after it.
fn typed(text: &str, code: fn(char) -> String) -> Vec<Effect> {
    let mut so_far = String::new();
    text.chars()
        .flat_map(|character| {
            so_far.push(character);
            [
                event(
                    "keydown",
                    &[("key", character.to_string()), ("code", code(character))],
                ),
                event("input", &[("value", so_far.clone())]),
            ]
        })
        .collect()
}

/// A `GET` of `path` on task `ordinal`'s host.
pub(super) fn page(seed: u64, ordinal: u32, path: &str) -> Option<Reply> {
    if path != "/type" {
        return None;
    }
    let text = text(seed, ordinal)?;
    let how = match TYPINGS[usize::try_from(ordinal - 1).ok()?] {
        Typing::Qwerty => "on a US QWERTY keyboard",
        Typing::Azerty => "on a French AZERTY keyboard",
        Typing::Composed => "through an input method",
    };
    Some(Reply::html(document(
        "Key recorder",
        "",
        &format!(
            "<h1>Key recorder</h1>\n<p id=\"ask\">Type <q class=\"text\">{text}</q> {how}.</p>\n<label for=\"field\">Text</label>\n<textarea id=\"field\" rows=\"3\" cols=\"40\" autocomplete=\"off\" spellcheck=\"false\"></textarea>\n<p>Keyboard layout: <output id=\"layout\">unknown</output></p>\n<h2>Events</h2>\n<ol id=\"log\"></ol>\n"
        ),
        &format!("<script>\n{SCRIPT}</script>\n"),
    )))
}

/// The family's tasks for `seed`. Ordinals are append-only (ADR 0063).
pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    const DESCRIPTIONS: [&str; 3] = [
        "Type into this page's text field the text the page asks for, one key press per \
         character, as a US QWERTY keyboard sends it.",
        "Type into this page's text field the text the page asks for, one key press per \
         character, as a French AZERTY keyboard sends it: each key press's `code` names the \
         physical key that types the character on that layout.",
        "Type into this page's text field the text the page asks for through an input method: \
         one composition that commits the whole text.",
    ];
    (1..)
        .zip(TYPINGS.into_iter().zip(DESCRIPTIONS))
        .map(|(ordinal, (typing, description))| {
            let text = text(seed, ordinal).expect("invariant: every task has its text");
            let effects = match typing {
                Typing::Qwerty => typed(text, qwerty_code),
                Typing::Azerty => typed(text, azerty_code),
                Typing::Composed => vec![event("compositionend", &[("data", text.to_owned())])],
            };
            let host = task::task_host(ordinal, HOST);
            Task {
                entry: Entry {
                    id: format!("{FAMILY}/{ordinal}"),
                    family: FAMILY,
                    kind: Kind::Act,
                    start_url: format!("{}/type", task::origin(&host, port)),
                    description,
                    account: None,
                    time_steps: None,
                },
                host,
                expected: Expected::effects(effects),
            }
        })
        .collect()
}
