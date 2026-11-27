//! Goodfetch's own human-like typing model (ADR 0035): types text
//! character by character with human inter-key pauses and occasional
//! neighbouring-key slips on a QWERTY layout, each slip followed by a
//! short recognition pause, a `Backspace` and the intended character, so
//! playing the plan back always produces the requested text verbatim while
//! staying inside the guard's deadline.

use std::time::Duration;

use goodfetch_engine::entropy::Entropy;

use super::{below, between, millis, scale_delays, unit};

/// The pause between two intended keystrokes, in milliseconds.
const KEY_PAUSE: (f64, f64) = (35.0, 125.0);

/// The probability that an ASCII alphanumeric character slips to an
/// adjacent key before being corrected.
const SLIP_PROBABILITY: f64 = 0.05;

/// The pause after a slip before pressing `Backspace`, in milliseconds.
const SLIP_PAUSE: (f64, f64) = (70.0, 160.0);

/// The pause after `Backspace` before typing the intended character, in
/// milliseconds.
const CORRECTION_PAUSE: (f64, f64) = (45.0, 110.0);

/// One step of a typing plan: what key or character to send once `after`
/// has passed since the step before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Keystroke {
    pub(crate) kind: KeyKind,
    pub(crate) after: Duration,
}

/// What one typing step sends to the focused field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyKind {
    /// Types this character (through a key press when it is a keyboard key,
    /// or text insertion otherwise).
    Character(char),
    /// Presses `Backspace` to remove the character just typed in error.
    Backspace,
}

/// Plans human-like keystrokes that produce `text` verbatim, their delays
/// adding up to at most `budget`.
pub(super) fn plan(entropy: &mut Entropy, text: &str, budget: Duration) -> Vec<Keystroke> {
    let mut strokes = Vec::with_capacity(text.chars().count());
    let mut first = true;
    for ch in text.chars() {
        let pause = if first {
            first = false;
            Duration::ZERO
        } else {
            millis(between(entropy, KEY_PAUSE))
        };
        if let Some(wrong) = maybe_slip(entropy, ch) {
            strokes.push(Keystroke {
                kind: KeyKind::Character(wrong),
                after: pause,
            });
            strokes.push(Keystroke {
                kind: KeyKind::Backspace,
                after: millis(between(entropy, SLIP_PAUSE)),
            });
            strokes.push(Keystroke {
                kind: KeyKind::Character(ch),
                after: millis(between(entropy, CORRECTION_PAUSE)),
            });
        } else {
            strokes.push(Keystroke {
                kind: KeyKind::Character(ch),
                after: pause,
            });
        }
    }
    scale_delays(&mut strokes, |step| &mut step.after, budget);
    strokes
}

/// Draws a QWERTY neighbouring key when `ch` slips, or `None` when typed
/// cleanly.
fn maybe_slip(entropy: &mut Entropy, ch: char) -> Option<char> {
    let neighbours = qwerty_neighbours(ch.to_ascii_lowercase())?;
    if unit(entropy) >= SLIP_PROBABILITY {
        return None;
    }
    let chosen = neighbours.as_bytes()[below(entropy, neighbours.len())] as char;
    Some(if ch.is_ascii_uppercase() {
        chosen.to_ascii_uppercase()
    } else {
        chosen
    })
}

/// Adjacent keys on a standard QWERTY layout for lowercase ASCII letters
/// and digits.
const fn qwerty_neighbours(ch: char) -> Option<&'static str> {
    match ch {
        'a' => Some("qwsz"),
        'b' => Some("vghn"),
        'c' => Some("xdfv"),
        'd' => Some("serfcx"),
        'e' => Some("wsdr"),
        'f' => Some("drtgvc"),
        'g' => Some("ftyhbv"),
        'h' => Some("gyujnb"),
        'i' => Some("ujko"),
        'j' => Some("huikmn"),
        'k' => Some("jiolm"),
        'l' => Some("kop"),
        'm' => Some("njk"),
        'n' => Some("bhjm"),
        'o' => Some("iklp"),
        'p' => Some("ol"),
        'q' => Some("wa"),
        'r' => Some("edft"),
        's' => Some("awedxz"),
        't' => Some("rfgy"),
        'u' => Some("yhji"),
        'v' => Some("cfgb"),
        'w' => Some("qase"),
        'x' => Some("zsdc"),
        'y' => Some("tghu"),
        'z' => Some("asx"),
        '0' => Some("9"),
        '1' => Some("2q"),
        '2' => Some("13qw"),
        '3' => Some("24we"),
        '4' => Some("35er"),
        '5' => Some("46rt"),
        '6' => Some("57ty"),
        '7' => Some("68yu"),
        '8' => Some("79ui"),
        '9' => Some("80io"),
        _ => None,
    }
}
