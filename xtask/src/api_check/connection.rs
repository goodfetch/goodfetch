//! The bundle's `connection` section, the messages that open and govern
//! every Connection, which the Collie client packages' generated layer
//! carries beside Collie's projection (ADR 0095): compared with the
//! previous release's, each message by the side its data flows, under the
//! subject `collie-client connection <message>`. The packages' hand-written
//! code is `xtask sdk-api`'s and `xtask semver-checks`' (issue 148).

use std::collections::BTreeMap;

use serde_json::Value;

use super::{Definitions, Schemas, Side};
use crate::api_baseline::Changes;

/// How the subject of a `connection` message's change starts.
const SUBJECT: &str = "collie-client connection";

/// The side each part of a message describes. A part both peers send, or
/// one this table does not name, may neither narrow nor widen.
fn side(message: &str, part: &str) -> Side {
    match (message, part) {
        // What the Client sends: its `hello`, and the seeds it feeds a Run.
        ("hello" | "seeds", "params") => Side::Input,
        // What the engine sends: its `hello` answer and the streams' notifications.
        ("hello", "result")
        | ("events" | "chunk" | "log" | "key" | "enqueued" | "record", "params") => Side::Output,
        // `$/cancel` and `$/credit` go both ways, as does `meta`, the
        // `_meta` of the engine's handler requests too.
        _ => Side::Both,
    }
}

/// Compares the `connection` sections of two bundles into `entries`.
pub(super) fn compare(previous: &Value, current: &Value, entries: &mut BTreeMap<String, Changes>) {
    let definitions = Definitions::of(previous, current);
    let messages = |bundle: &Value| -> BTreeMap<String, Value> {
        bundle
            .get("connection")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(message, parts)| (message.clone(), parts.clone()))
            .collect()
    };
    let (before, after) = (messages(previous), messages(current));
    for (message, previous_parts) in &before {
        let mut changes = Changes::default();
        match after.get(message) {
            None => changes.breaks(message, "removed"),
            Some(current_parts) => {
                for (part, previous_schema, current_schema) in parts(previous_parts, current_parts)
                {
                    let at = match part {
                        Some(part) => format!("{message}.{part}"),
                        None => message.clone(),
                    };
                    Schemas::new(&definitions).compare(
                        previous_schema,
                        current_schema,
                        side(message, part.unwrap_or_default()),
                        &at,
                        &mut changes,
                    );
                }
            }
        }
        if !changes.is_empty() {
            entries.insert(format!("{SUBJECT} {message}"), changes);
        }
    }
    for message in after
        .keys()
        .filter(|message| !before.contains_key(*message))
    {
        entries
            .entry(format!("{SUBJECT} {message}"))
            .or_default()
            .adds(message, "added");
    }
}

/// A message's parts, `params` and `result`, each side's or `null`; a
/// message that is a schema itself, as `meta`, is its one unnamed part.
fn parts<'a>(
    previous: &'a Value,
    current: &'a Value,
) -> Vec<(Option<&'static str>, &'a Value, &'a Value)> {
    let is_parts = |message: &Value| {
        message.as_object().is_some_and(|parts| {
            !parts.is_empty()
                && parts
                    .keys()
                    .all(|part| part == "params" || part == "result")
        })
    };
    if !is_parts(previous) || !is_parts(current) {
        return vec![(None, previous, current)];
    }
    ["params", "result"]
        .into_iter()
        .map(|part| {
            (
                Some(part),
                previous.get(part).unwrap_or(&Value::Null),
                current.get(part).unwrap_or(&Value::Null),
            )
        })
        .collect()
}
