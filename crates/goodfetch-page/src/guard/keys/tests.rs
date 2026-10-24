use goodfetch_cdp::generated::input::{DispatchKeyEventParams, DispatchKeyEventType};
use proptest::prelude::*;
use rstest::rstest;

use super::{UnknownKey, events};

/// One key event as a keyboard sends it: its kind, its `key`, its `code`,
/// its modifier bits and the text it types.
type Sent = (
    &'static str,
    String,
    Option<String>,
    Option<i64>,
    Option<String>,
);

fn sent(events: &[DispatchKeyEventParams]) -> Vec<Sent> {
    events
        .iter()
        .map(|event| {
            let kind = match event.r#type {
                DispatchKeyEventType::KeyDown => "keyDown",
                DispatchKeyEventType::RawKeyDown => "rawKeyDown",
                DispatchKeyEventType::KeyUp => "keyUp",
                _ => "other",
            };
            (
                kind,
                event.key.clone().unwrap_or_default(),
                event.code.clone(),
                event.modifiers,
                event.text.clone(),
            )
        })
        .collect()
}

fn down(key: &str, code: &str, modifiers: Option<i64>, text: Option<&str>) -> Sent {
    let kind = if text.is_some() {
        "keyDown"
    } else {
        "rawKeyDown"
    };
    (
        kind,
        key.to_owned(),
        Some(code.to_owned()),
        modifiers,
        text.map(str::to_owned),
    )
}

fn up(key: &str, code: &str, modifiers: Option<i64>) -> Sent {
    (
        "keyUp",
        key.to_owned(),
        Some(code.to_owned()),
        modifiers,
        None,
    )
}

/// CDP's modifier bits: Alt 1, Control 2, Meta 4, Shift 8.
#[rstest]
#[case::enter_types_a_return("Enter", vec![
    down("Enter", "Enter", None, Some("\r")),
    up("Enter", "Enter", None),
])]
#[case::a_letter_types_itself("a", vec![
    down("a", "KeyA", None, Some("a")),
    up("a", "KeyA", None),
])]
#[case::a_digit_types_itself("7", vec![
    down("7", "Digit7", None, Some("7")),
    up("7", "Digit7", None),
])]
#[case::space_by_name_types_a_space("Space", vec![
    down("Space", "Space", None, Some(" ")),
    up("Space", "Space", None),
])]
#[case::shift_tab_presses_shift_first_and_releases_it_last("Shift+Tab", vec![
    down("Shift", "ShiftLeft", Some(8), None),
    down("Tab", "Tab", Some(8), None),
    up("Tab", "Tab", Some(8)),
    up("Shift", "ShiftLeft", None),
])]
#[case::shift_types_a_capital("Shift+a", vec![
    down("Shift", "ShiftLeft", Some(8), None),
    down("a", "KeyA", Some(8), Some("A")),
    up("a", "KeyA", Some(8)),
    up("Shift", "ShiftLeft", None),
])]
#[case::a_shortcut_types_nothing("Control+a", vec![
    down("Control", "ControlLeft", Some(2), None),
    down("a", "KeyA", Some(2), None),
    up("a", "KeyA", Some(2)),
    up("Control", "ControlLeft", None),
])]
#[case::modifiers_add_their_bits_in_order("Control+Alt+Delete", vec![
    down("Control", "ControlLeft", Some(2), None),
    down("Alt", "AltLeft", Some(3), None),
    down("Delete", "Delete", Some(3), None),
    up("Delete", "Delete", Some(3)),
    up("Alt", "AltLeft", Some(2)),
    up("Control", "ControlLeft", None),
])]
fn each_key_name_presses_as_a_keyboard_does(
    #[case] combination: &str,
    #[case] expected: Vec<Sent>,
) {
    assert_eq!(
        events(combination).map(|events| sent(&events)),
        Ok(expected)
    );
}

/// `+` alone, or ending a combination, is the plus key, which has no
/// physical code of its own here.
#[rstest]
#[case::alone("+", None)]
#[case::after_a_modifier("Shift++", Some(8))]
fn a_trailing_plus_is_the_plus_key(#[case] combination: &str, #[case] modifiers: Option<i64>) {
    let pressed = events(combination).unwrap();
    let plus = pressed
        .iter()
        .find(|event| event.key.as_deref() == Some("+"))
        .unwrap();

    assert_eq!((plus.modifiers, plus.code.clone()), (modifiers, None));
}

#[rstest]
#[case::no_key("")]
#[case::unknown_name("Return")]
#[case::unknown_modifier("Hyper+a")]
#[case::a_key_as_a_modifier("a+b")]
#[case::two_characters("ab")]
fn a_name_the_keyboard_has_no_key_for_is_refused(#[case] combination: &str) {
    assert_eq!(events(combination), Err(UnknownKey(combination.to_owned())));
}

proptest! {
    /// Whatever modifiers a combination holds, every key pressed is
    /// released, the modifiers after the key in the reverse order, and no
    /// modifier bit is left set once the last key is up.
    #[test]
    fn every_key_a_combination_presses_is_released(
        modifiers in proptest::sample::subsequence(vec!["Alt", "Control", "Meta", "Shift"], 0..=4),
        key in prop_oneof![
            Just("Enter".to_owned()),
            Just("Tab".to_owned()),
            Just("ArrowDown".to_owned()),
            "[a-z0-9]",
        ],
    ) {
        let combination = modifiers
            .iter()
            .copied()
            .chain(std::iter::once(key.as_str()))
            .collect::<Vec<_>>()
            .join("+");

        let pressed = events(&combination).unwrap();

        let downs: Vec<String> = pressed
            .iter()
            .filter(|event| event.r#type != DispatchKeyEventType::KeyUp)
            .map(|event| event.key.clone().unwrap_or_default())
            .collect();
        let mut ups: Vec<String> = pressed
            .iter()
            .filter(|event| event.r#type == DispatchKeyEventType::KeyUp)
            .map(|event| event.key.clone().unwrap_or_default())
            .collect();
        ups.reverse();
        prop_assert_eq!(downs, ups);
        prop_assert_eq!(pressed.last().and_then(|event| event.modifiers), None);
    }
}
