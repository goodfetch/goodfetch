//! What both settings files' corpus writers share (ADR 0066): a file that
//! sets every key of either Product in each of its tables, each to a sample
//! value of its kind, written through the engine's own rewrite and read
//! back through its own reader, so the set covers every key the schema
//! holds and is a file the engine reads.

use goodfetch_engine::settings::{LOWEST_MINOR_KEY, rewrite};
use goodfetch_protocol::catalog::{Setting, ValueKind};
use goodfetch_protocol::entries::MODULES;
use serde_json::{Value, json};

/// A value of `kind`, with an entry in each list and table.
pub(crate) fn sample(kind: &ValueKind) -> Value {
    match kind {
        ValueKind::Boolean => json!(true),
        ValueKind::Count | ValueKind::CountUpTo(_) | ValueKind::CountFromOne => json!(4),
        ValueKind::Duration => json!(30_000),
        ValueKind::Text => json!("example"),
        ValueKind::Choice(words) => json!(words.first().copied().unwrap_or_default()),
        ValueKind::Price => json!("0.05 USD"),
        ValueKind::Address => json!("127.0.0.1"),
        ValueKind::List(kind) => json!([sample(kind)]),
        ValueKind::Map(kind) => json!({ "example.test": sample(kind) }),
        ValueKind::Optional(kind) => sample(kind),
        ValueKind::SecretReference => json!("env:EXAMPLE"),
        ValueKind::Shape(shape) => (shape.examples)().into_iter().next().unwrap_or(Value::Null),
    }
}

/// Further files of version `0.1`, one for each example of a shaped key
/// `holds` accepts past its first, each setting that key alone in `table`,
/// so that every field of the shape is given in some file: each named by
/// the table, the key and the example's place, from 2.
pub(crate) fn shaped_examples(table: &str, holds: Holds) -> Vec<(String, String)> {
    let header = format!("version = \"0.1\"\n{LOWEST_MINOR_KEY} = 0\n");
    MODULES
        .iter()
        .flat_map(|entries| entries.settings)
        .filter(|setting| holds(setting))
        .filter_map(|setting| match setting.kind {
            ValueKind::Shape(shape) => Some((setting, (shape.examples)())),
            _ => None,
        })
        .flat_map(|(setting, examples)| {
            let header = header.clone();
            examples
                .into_iter()
                .zip(1_usize..)
                .skip(1)
                .map(move |(example, place)| {
                    let text = rewrite(&header, table, setting.name, &example)
                        .expect("invariant: an example value is one TOML holds");
                    (format!("{table}-{}-{place}", setting.name), text)
                })
        })
        .collect()
}

/// Which keys a table of the file holds.
pub(crate) type Holds = fn(&Setting) -> bool;

/// The text of a file of version `0.1`, which every minor of its major can
/// use (ADR 0098), with each key `holds` accepts set in each of `tables`.
pub(crate) fn every_key(tables: &[(&str, Holds)]) -> String {
    let mut text = format!("version = \"0.1\"\n{LOWEST_MINOR_KEY} = 0\n");
    for (table, holds) in tables {
        for setting in MODULES
            .iter()
            .flat_map(|entries| entries.settings)
            .filter(|setting| holds(setting))
        {
            text = rewrite(&text, table, setting.name, &sample(&setting.kind))
                .expect("invariant: a sample value is one TOML holds");
        }
    }
    text
}
