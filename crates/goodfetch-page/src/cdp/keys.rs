//! Keys by Playwright's names (ADR 0085): a key or a combination such as
//! `Enter`, `a` or `Shift+Tab` becomes the CDP key events a keyboard sends,
//! modifiers pressed first and released last, so the page receives trusted
//! `keydown`, `keypress` and `keyup` events.

use goodfetch_cdp::generated::input::{DispatchKeyEventParams, DispatchKeyEventType};

/// One key: its `key` value, its physical `code`, its Windows virtual key
/// code and the text it types, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Definition {
    key: String,
    code: String,
    key_code: i64,
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

/// The key events that press `combination`, such as `Control+a`: at most
/// ten, a modifier named twice being held once.
pub(crate) fn events(combination: &str) -> Result<Vec<DispatchKeyEventParams>, UnknownKey> {
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
    let mut held: Vec<Definition> = Vec::new();
    for name in modifiers.split('+').filter(|name| !name.is_empty()) {
        let modifier = modifier(name).ok_or_else(unknown)?;
        if !held.contains(&modifier) {
            held.push(modifier);
        }
    }
    let pressed = key(last).ok_or_else(unknown)?;
    let mut bits = 0;
    let mut sent = Vec::new();
    for modifier in &held {
        bits |= bit(&modifier.key);
        sent.push(event(
            DispatchKeyEventType::RawKeyDown,
            modifier,
            bits,
            None,
        ));
    }
    // A shortcut types nothing, as a keyboard's does.
    let shortcut = bits & (bit("Control") | bit("Meta") | bit("Alt")) != 0;
    let text = if shortcut {
        None
    } else if bits & bit("Shift") != 0 {
        pressed.text.as_deref().map(str::to_uppercase)
    } else {
        pressed.text.clone()
    };
    let down = if text.is_some() {
        DispatchKeyEventType::KeyDown
    } else {
        DispatchKeyEventType::RawKeyDown
    };
    sent.push(event(down, &pressed, bits, text));
    sent.push(event(DispatchKeyEventType::KeyUp, &pressed, bits, None));
    for modifier in held.iter().rev() {
        bits &= !bit(&modifier.key);
        sent.push(event(DispatchKeyEventType::KeyUp, modifier, bits, None));
    }
    Ok(sent)
}

/// A modifier by name; `ControlOrMeta` is Meta on macOS, Control elsewhere.
fn modifier(name: &str) -> Option<Definition> {
    let name = match name {
        "ControlOrMeta" if cfg!(target_os = "macos") => "Meta",
        "ControlOrMeta" => "Control",
        other => other,
    };
    MODIFIERS
        .iter()
        .any(|(modifier, _)| *modifier == name)
        .then(|| key(name))
        .flatten()
}

fn bit(name: &str) -> i64 {
    MODIFIERS
        .iter()
        .find(|(modifier, _)| *modifier == name)
        .map_or(0, |(_, bit)| *bit)
}

/// A key by name, or a single character, which types itself.
fn key(name: &str) -> Option<Definition> {
    if let Some((key, code, key_code, text)) = NAMED.iter().find(|(key, ..)| *key == name) {
        return Some(Definition {
            key: (*key).to_owned(),
            code: (*code).to_owned(),
            key_code: *key_code,
            text: text.map(str::to_owned),
        });
    }
    let mut characters = name.chars();
    let character = characters.next()?;
    if characters.next().is_some() {
        return None;
    }
    let upper = character.to_ascii_uppercase();
    let (code, key_code) = if upper.is_ascii_alphabetic() {
        (format!("Key{upper}"), i64::from(u32::from(upper)))
    } else if character.is_ascii_digit() {
        (format!("Digit{character}"), i64::from(u32::from(character)))
    } else if character == ' ' {
        ("Space".to_owned(), 32)
    } else {
        (String::new(), 0)
    };
    Some(Definition {
        key: character.to_string(),
        code,
        key_code,
        text: Some(character.to_string()),
    })
}

fn event(
    kind: DispatchKeyEventType,
    key: &Definition,
    modifiers: i64,
    text: Option<String>,
) -> DispatchKeyEventParams {
    DispatchKeyEventParams {
        r#type: kind,
        modifiers: (modifiers != 0).then_some(modifiers),
        timestamp: None,
        unmodified_text: text.clone(),
        text,
        key_identifier: None,
        code: (!key.code.is_empty()).then(|| key.code.clone()),
        key: Some(key.key.clone()),
        windows_virtual_key_code: Some(key.key_code),
        native_virtual_key_code: Some(key.key_code),
        auto_repeat: None,
        is_keypad: None,
        is_system_key: None,
        location: None,
        commands: None,
    }
}

#[cfg(test)]
mod tests;
