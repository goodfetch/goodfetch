//! What both settings files' corpus writers share (ADR 0066): a file that
//! sets every key of either Product in each of its tables, each to a sample
//! value of its kind, written through the engine's own rewrite and read
//! back through its own reader, so the set covers every key the schema
//! holds and is a file the engine reads.

use goodfetch_engine::settings::rewrite;
use goodfetch_protocol::catalog::{Setting, ValueKind};
use goodfetch_protocol::entries::MODULES;
use serde_json::{Value, json};

/// A value of `kind`, with an entry in each list and table.
pub(crate) fn sample(kind: &ValueKind) -> Value {
    match kind {
        ValueKind::Boolean => json!(true),
        ValueKind::Count => json!(4),
        ValueKind::Duration => json!(30_000),
        ValueKind::Text => json!("example"),
        ValueKind::Choice(words) => json!(words.first().copied().unwrap_or_default()),
        ValueKind::Price => json!("0.05 USD"),
        ValueKind::Address => json!("127.0.0.1"),
        ValueKind::List(kind) => json!([sample(kind)]),
        ValueKind::Map(kind) => json!({ "example.test": sample(kind) }),
        ValueKind::Optional(kind) => sample(kind),
    }
}

/// Which keys a table of the file holds.
pub(crate) type Holds = fn(&Setting) -> bool;

/// The text of a file of version `0.1` with each key `holds` accepts set in
/// each of `tables`.
pub(crate) fn every_key(tables: &[(&str, Holds)]) -> String {
    let mut text = "version = \"0.1\"\n".to_owned();
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
