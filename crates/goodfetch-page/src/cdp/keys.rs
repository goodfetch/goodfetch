//! Keys by Playwright's names (ADR 0085): a key or a combination such as
//! `Enter`, `a` or `Shift+Tab` becomes the CDP key events a keyboard sends,
//! modifiers pressed first and released last, so the page receives trusted
//! `keydown`, `keypress` and `keyup` events. A key goes down with the keys
//! a page's keyboard holds (`page.keyboard.down`): their modifiers' bits,
//! and a repeat when it is held already.

use goodfetch_cdp::generated::input::{DispatchKeyEventParams, DispatchKeyEventType};

use super::KeyPress;

/// One key: its `key` value, its physical `code`, its Windows virtual key
/// code and the text it types, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    name: String,
    code: String,
    virtual_code: i64,
    text: Option<String>,
}

/// The keys typed by name, with their codes and the text they type.
const NAMED: [(&str, &str, i64, Option<&str>); 19] = [
    ("Enter", "Enter", 13, Some("\r")),
    ("Tab", "Tab", 9, None),
    ("Backspace", "Backspace", 8, None),
    ("Delete", "Delete", 46, None),
    ("Escape", "Escape", 27, None),
    ("ArrowUp", "ArrowUp", 38, None),
    ("ArrowDown", "ArrowDown", 40, None),
    ("ArrowLeft", "ArrowLeft", 37, None),
    ("ArrowRight", "ArrowRight", 39, None),
    ("Home", "Home", 36, None),
    ("End", "End", 35, None),
    ("PageUp", "PageUp", 33, None),
    ("PageDown", "PageDown", 34, None),
    ("Insert", "Insert", 45, None),
    ("Space", "Space", 32, Some(" ")),
    ("Shift", "ShiftLeft", 16, None),
    ("Control", "ControlLeft", 17, None),
    ("Alt", "AltLeft", 18, None),
    ("Meta", "MetaLeft", 91, None),
];

/// The modifiers, each with its bit in CDP's `modifiers`.
const MODIFIERS: [(&str, i64); 4] = [("Alt", 1), ("Control", 2), ("Meta", 4), ("Shift", 8)];

/// A key or combination the keyboard has no key for.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{0}` names no key: give a key name such as `Enter`, `Tab` or `ArrowDown`, a single character, or a combination such as `Shift+Tab`"
)]
pub(crate) struct UnknownKey(pub(crate) String);

/// The key events that press `combination`, such as `Control+a`, with no
/// key held before: at most ten, a modifier named twice being held once.
pub(crate) fn events(combination: &str) -> Result<KeyPress, UnknownKey> {
    Ok(press(&keys(combination)?, &mut Vec::new()))
}

/// The keys of `combination`, such as `Control+a`, in the order they go
/// down: its modifiers, each once, then its key.
pub(crate) fn keys(combination: &str) -> Result<Vec<Key>, UnknownKey> {
    let unknown = || UnknownKey(combination.to_owned());
    // `+` alone, or ending a combination, is the plus key.
    let (modifiers, last) = match combination.strip_suffix('+') {
        Some(rest) if rest.is_empty() || rest.ends_with('+') => {
            (rest.strip_suffix('+').unwrap_or(rest), "+")
        }
        _ => combination
            .rsplit_once('+')
            .map_or(("", combination), |(modifiers, last)| (modifiers, last)),
    };
    let mut keys: Vec<Key> = Vec::new();
    for name in modifiers.split('+').filter(|name| !name.is_empty()) {
        let modifier = modifier(name).ok_or_else(unknown)?;
        if !keys.contains(&modifier) {
            keys.push(modifier);
        }
    }
    keys.push(named(last).ok_or_else(unknown)?);
    Ok(keys)
}

/// The one key `name` names, a modifier as any other.
pub(crate) fn key(name: &str) -> Result<Key, UnknownKey> {
    let name = match name {
        "ControlOrMeta" => control_or_meta(),
        other => other,
    };
    named(name).ok_or_else(|| UnknownKey(name.to_owned()))
}

/// The key a US keyboard types `character` with, a line break being
/// `Enter`; none for a character it has no key for.
pub(crate) fn typed(character: char) -> Option<Key> {
    match character {
        '\n' | '\r' => named("Enter"),
        ' '..='~' => named(character.encode_utf8(&mut [0; 4])),
        _ => None,
    }
}

/// The key events that press `keys` with the keys `held` down: each key
/// goes down in order, then up in the reverse order, as one keyboard press.
pub(crate) fn press(keys: &[Key], held: &mut Vec<String>) -> KeyPress {
    let mut sent: Vec<DispatchKeyEventParams> = keys.iter().map(|key| down(key, held)).collect();
    sent.extend(keys.iter().rev().map(|key| up(key, held)));
    KeyPress(sent)
}

/// The event that presses `key` down with the keys `held` down, which it
/// joins: a modifier's bit is set from its own press on, a shortcut types
/// nothing, as a keyboard's does, and a key held already repeats.
pub(crate) fn down(key: &Key, held: &mut Vec<String>) -> DispatchKeyEventParams {
    let repeat = held.contains(&key.name);
    if !repeat {
        held.push(key.name.clone());
    }
    let bits = modifiers(held);
    let shortcut = bits & (bit("Control") | bit("Meta") | bit("Alt")) != 0;
    let text = if shortcut {
        None
    } else if bits & bit("Shift") != 0 {
        key.text.as_deref().map(str::to_uppercase)
    } else {
        key.text.clone()
    };
    let kind = if text.is_some() {
        DispatchKeyEventType::KeyDown
    } else {
        DispatchKeyEventType::RawKeyDown
    };
    let commands = editing_command(bits, key).map(|command| vec![command.to_owned()]);
    let mut event = event(kind, key, bits, text, commands);
    event.auto_repeat = repeat.then_some(true);
    event
}

/// The event that releases `key`, which leaves the keys `held` down: a
/// modifier's bit is cleared from its own release on.
pub(crate) fn up(key: &Key, held: &mut Vec<String>) -> DispatchKeyEventParams {
    held.retain(|name| *name != key.name);
    event(
        DispatchKeyEventType::KeyUp,
        key,
        modifiers(held),
        None,
        None,
    )
}

/// `ControlOrMeta`'s key: Meta on macOS, Control elsewhere.
const fn control_or_meta() -> &'static str {
    if cfg!(target_os = "macos") {
        "Meta"
    } else {
        "Control"
    }
}

/// Blink editing command for a primary-modifier shortcut (`Control` on
/// Linux/Windows, `Meta` on macOS) so `ControlOrMeta+a/c/v/x` invoke the
/// browser's native selection and clipboard actions reliably in headless
/// Chromium (ticket #3290).
fn editing_command(modifiers: i64, pressed: &Key) -> Option<&'static str> {
    let primary = if cfg!(target_os = "macos") {
        bit("Meta")
    } else {
        bit("Control")
    };
    if modifiers != primary {
        return None;
    }
    match pressed.code.as_str() {
        "KeyA" => Some("selectAll"),
        "KeyC" => Some("copy"),
        "KeyV" => Some("paste"),
        "KeyX" => Some("cut"),
        _ => None,
    }
}

/// A modifier by name; `ControlOrMeta` is Meta on macOS, Control elsewhere.
fn modifier(name: &str) -> Option<Key> {
    let name = match name {
        "ControlOrMeta" => control_or_meta(),
        other => other,
    };
    MODIFIERS
        .iter()
        .any(|(modifier, _)| *modifier == name)
        .then(|| named(name))
        .flatten()
}

/// CDP's `modifiers` bits of the keys `held` down, which every input sent
/// while they are carries.
pub(crate) fn modifiers(held: &[String]) -> i64 {
    held.iter().fold(0, |bits, name| bits | bit(name))
}

fn bit(name: &str) -> i64 {
    MODIFIERS
        .iter()
        .find(|(modifier, _)| *modifier == name)
        .map_or(0, |(_, bit)| *bit)
}

/// A key by name, or a single character, which types itself.
fn named(name: &str) -> Option<Key> {
    if let Some((key, code, virtual_code, text)) = NAMED.iter().find(|(key, ..)| *key == name) {
        return Some(Key {
            name: (*key).to_owned(),
            code: (*code).to_owned(),
            virtual_code: *virtual_code,
            text: text.map(str::to_owned),
        });
    }
    let mut characters = name.chars();
    let character = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    let upper = character.to_ascii_uppercase();
    let (code, virtual_code) = if upper.is_ascii_alphabetic() {
        (format!("Key{upper}"), i64::from(u32::from(upper)))
    } else if character.is_ascii_digit() {
        (format!("Digit{character}"), i64::from(u32::from(character)))
    } else if character == ' ' {
        ("Space".to_owned(), 32)
    } else {
        (String::new(), 0)
    };
    Some(Key {
        name: character.to_string(),
        code,
        virtual_code,
        text: Some(character.to_string()),
    })
}

fn event(
    kind: DispatchKeyEventType,
    key: &Key,
    modifiers: i64,
    text: Option<String>,
    commands: Option<Vec<String>>,
) -> DispatchKeyEventParams {
    DispatchKeyEventParams {
        r#type: kind,
        modifiers: (modifiers != 0).then_some(modifiers),
        timestamp: None,
        unmodified_text: text.clone(),
        text,
        key_identifier: None,
        code: (!key.code.is_empty()).then(|| key.code.clone()),
        key: Some(key.name.clone()),
        windows_virtual_key_code: Some(key.virtual_code),
        native_virtual_key_code: Some(key.virtual_code),
        auto_repeat: None,
        is_keypad: None,
        is_system_key: None,
        location: None,
        commands,
    }
}

#[cfg(test)]
mod tests;
