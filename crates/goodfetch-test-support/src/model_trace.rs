//! A TLA+ model's trace (ADR 0060): the `tracing` layer that writes each
//! event under one of a model's targets, such as `goodfetch::lease` or
//! `goodfetch::ledger`, as one NDJSON line of its fields, the message left
//! out, which `xtask tlc validate` checks against the model's trace spec
//! (`test-support/models/`). Integers stay numbers, `%` and `?` values
//! become strings, and the fields of a line come in name order.

use std::fmt;
use std::io::Write as _;

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::Context;

/// The layer that writes each event under one of `targets` to `writer`.
pub fn model_trace<S, W>(targets: &[&'static str], writer: W) -> impl Layer<S> + Send + Sync
where
    S: tracing::Subscriber,
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    ModelTrace {
        targets: targets.to_vec(),
        writer,
    }
}

struct ModelTrace<W> {
    targets: Vec<&'static str>,
    writer: W,
}

impl<S, W> Layer<S> for ModelTrace<W>
where
    S: tracing::Subscriber,
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if !self.targets.contains(&event.metadata().target()) {
            return;
        }
        let mut fields = Fields(Map::new());
        event.record(&mut fields);
        let mut line = Value::Object(fields.0).to_string();
        line.push('\n');
        // A write that fails loses the line, and the trace then fails its
        // validation.
        let _ = self.writer.make_writer().write_all(line.as_bytes());
    }
}

/// An event's fields but its message, as JSON values.
struct Fields(Map<String, Value>);

impl Fields {
    fn insert(&mut self, field: &Field, value: Value) {
        if field.name() != "message" {
            self.0.insert(field.name().to_owned(), value);
        }
    }
}

impl Visit for Fields {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, value.into());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, value.into());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.insert(field, format!("{value:?}").into());
    }
}

#[cfg(test)]
mod tests;
