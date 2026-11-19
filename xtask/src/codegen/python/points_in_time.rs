//! Every `date-time` the schema declares as the client package's hand-written
//! `PointInTime`, a time-zone-aware `datetime` in UTC that reads any offset
//! and sends RFC 3339 in UTC (spec 606, Casing and times; ADR 0092), in
//! place of the `RootModel` holding the text that `datamodel-code-generator`
//! would write.

use anyhow::{Result, bail};
use schemars::JsonSchema as _;
use serde_json::{Map, Value};

use goodfetch_protocol::run::PointInTime;

/// The client package's point in time, which `result_of` builds from the
/// engine's text.
const POINT_IN_TIME: &str = "goodfetch_collie_client.point_in_time.PointInTime";

/// The name of the protocol's point in time, the `$defs` entry every
/// `date-time` field refers to.
fn protocol_name() -> String {
    PointInTime::schema_name().into_owned()
}

/// The client package's point in time when `name`, a Collie type a layer
/// refers to, is the protocol's: Foxhound's layer imports it from where
/// Collie's does.
pub(super) fn client_type(name: &str) -> Option<&'static str> {
    (name == protocol_name()).then_some(POINT_IN_TIME)
}

/// `input`, a generator input, with the protocol's point in time out of its
/// `$defs` and every reference to it, and every inline `date-time`, naming
/// the client package's type instead. A `date-time` entry under another name
/// is refused: Foxhound's layer imports Collie's by the protocol's name.
pub(super) fn use_client_type(input: &mut Value) -> Result<()> {
    let name = protocol_name();
    if let Some(definitions) = input.get_mut("$defs").and_then(Value::as_object_mut) {
        for (other, schema) in definitions.iter() {
            if *other != name && schema.as_object().is_some_and(is_date_time) {
                bail!(
                    "`{other}` is a `date-time` of its own: refer to `{name}`, which the Python layer maps to its point in time"
                );
            }
        }
        definitions.remove(&name);
    }
    replace(input, &format!("#/$defs/{name}"))
}

fn is_date_time(schema: &Map<String, Value>) -> bool {
    schema.get("format").and_then(Value::as_str) == Some("date-time")
}

fn replace(value: &mut Value, reference: &str) -> Result<()> {
    match value {
        Value::Object(object) => {
            if object.get("$ref").and_then(Value::as_str) == Some(reference) {
                object.remove("$ref");
                name_client_type(object);
            } else if is_date_time(object) {
                if object.get("type").and_then(Value::as_str) != Some("string") {
                    bail!("a `date-time` whose type is not `string` alone: {object:?}");
                }
                object.remove("type");
                object.remove("format");
                name_client_type(object);
            }
            object
                .values_mut()
                .try_for_each(|child| replace(child, reference))
        }
        Value::Array(items) => items
            .iter_mut()
            .try_for_each(|item| replace(item, reference)),
        _ => Ok(()),
    }
}

fn name_client_type(object: &mut Map<String, Value>) {
    object.insert(
        "customTypePath".to_owned(),
        Value::String(POINT_IN_TIME.to_owned()),
    );
}

#[cfg(test)]
mod tests;
