//! Every duration the schema declares as the client package's hand-written
//! `Duration`, a `timedelta` that crosses the wire as whole milliseconds
//! (ADR 0080), in place of the `RootModel` holding an `int` that
//! `datamodel-code-generator` would write.

use schemars::JsonSchema as _;
use serde_json::Value;

use goodfetch_protocol::milliseconds::Milliseconds;

/// The client package's duration, which `result_of` builds from the
/// engine's milliseconds.
const DURATION: &str = "goodfetch_collie_client.duration.Duration";

/// The name of the protocol's duration, the `$defs` entry every duration
/// field refers to.
fn protocol_name() -> String {
    Milliseconds::schema_name().into_owned()
}

/// The client package's duration when `name`, a Collie type a layer refers
/// to, is the protocol's: Foxhound's layer imports it from where Collie's
/// does.
pub(super) fn client_type(name: &str) -> Option<&'static str> {
    (name == protocol_name()).then_some(DURATION)
}

/// `input`, a generator input, with the protocol's duration out of its
/// `$defs` and every reference to it naming the client package's type
/// instead.
pub(super) fn use_client_type(input: &mut Value) {
    let name = protocol_name();
    if let Some(definitions) = input.get_mut("$defs").and_then(Value::as_object_mut) {
        definitions.remove(&name);
    }
    replace(input, &format!("#/$defs/{name}"));
}

fn replace(value: &mut Value, reference: &str) {
    match value {
        Value::Object(object) => {
            if object.get("$ref").and_then(Value::as_str) == Some(reference) {
                object.remove("$ref");
                object.insert(
                    "customTypePath".to_owned(),
                    Value::String(DURATION.to_owned()),
                );
            }
            object
                .values_mut()
                .for_each(|child| replace(child, reference));
        }
        Value::Array(items) => items.iter_mut().for_each(|item| replace(item, reference)),
        _ => {}
    }
}

#[cfg(test)]
mod tests;
