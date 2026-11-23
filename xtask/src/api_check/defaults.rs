//! A setting's changed default (ADR 0030): a tuning default may change,
//! and one with a value that weakens a guarantee breaks only when it moves
//! toward the values that weaken, as the previous release's `weakening`
//! says (issue 148): the direction that release published is the promise,
//! so a change that moves the direction and the default together is judged
//! by the old one. Where it says no direction, any changed default breaks.

use serde_json::{Map, Value};

use crate::api_baseline::Changes;

/// Judges a setting's default changed from `before` to `after`, `previous`
/// and `current` being its entries in the previous release and the tree.
pub(super) fn compare(
    previous: &Map<String, Value>,
    current: &Map<String, Value>,
    before: &Value,
    after: &Value,
    changes: &mut Changes,
) {
    let moved = format!("changed from {before} to {after}");
    let weakening_setting = [previous, current]
        .iter()
        .any(|entry| entry.get("weakens") == Some(&Value::Bool(true)));
    if !weakening_setting {
        return changes.adds("default", format!("{moved}, a tuning default"));
    }
    match weakens(previous.get("weakening"), before, after) {
        Some(true) => changes.breaks(
            "default",
            format!("{moved}, toward the values that weaken a guarantee"),
        ),
        Some(false) => changes.adds(
            "default",
            format!("{moved}, away from the values that weaken a guarantee"),
        ),
        None => changes.breaks(
            "default",
            format!("{moved} on a setting with a value that weakens a guarantee, in no direction the previous release says"),
        ),
    }
}

/// Whether the move from `before` to `after` goes toward the values that
/// weaken, by `weakening`; none when it cannot say.
fn weakens(weakening: Option<&Value>, before: &Value, after: &Value) -> Option<bool> {
    let weakening = weakening?;
    if let Some(values) = weakening.get("values").and_then(Value::as_array) {
        return Some(values.contains(after));
    }
    let (before, after) = (before.as_u64()?, after.as_u64()?);
    match weakening.get("toward")?.as_str()? {
        "lower" => Some(after < before),
        "higher" => Some(after > before),
        _ => None,
    }
}
