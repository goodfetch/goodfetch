//! A Run trace's raw call log as a test reads it from outside the engine
//! (ADR 0099, G8).

use std::path::Path;

use serde_json::Value;

/// The lines of the raw call log `file`, none when there is no such file,
/// each without when its call arrived, which every line says.
///
/// # Panics
///
/// When a line is not a JSON object that says when its call arrived.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the Run trace the engine wrote, from outside it and on no async worker"
)]
pub fn raw_call_lines(file: &Path) -> Vec<Value> {
    let log = std::fs::read_to_string(file).unwrap_or_default();
    log.lines()
        .map(|line| {
            let mut line: Value = serde_json::from_str(line).expect("a raw call's line is JSON");
            let arrived_at = line
                .as_object_mut()
                .and_then(|line| line.remove("arrived_at"));
            assert!(
                arrived_at.as_ref().is_some_and(Value::is_string),
                "a raw call's line says when it arrived: {line}"
            );
            line
        })
        .collect()
}
