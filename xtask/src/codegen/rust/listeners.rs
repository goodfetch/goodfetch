//! A handle's listener in the Rust handles (ADR 0081): Rust reads events by
//! meaning, so a handle's `on` is its `events` member, which takes the
//! marker type of one event and returns the crate's `EventStream` of its
//! payloads, and the `events` module holds the markers, one unit struct for
//! each event, with the trait that names what each carries.
//!
//! A marker is the event's name in upper camel case, such as `Console`; an
//! event whose name would end in `Error`, the suffix of ADR 0079's error
//! types, takes its payload's name instead, so `page_error` is `WebError`,
//! Playwright's own name for it.

use std::fmt::Write as _;

use anyhow::{Context as _, Result};

use super::super::handles::{Handle, Listener};
use super::index::Index;
use super::{Twin, comment, doc, field, indent, type_name, warning};

/// The `# Errors` section of an `events` member.
const EVENTS_ERRORS: &str = "# Errors\n\nIts stream hands `closed` when its handle or the Connection is closed, the engine's error, such as `page_events_lost`, and `internal` for an event that does not fit its type.";

/// A handle's listener, resolved against the protocol crate.
pub(super) struct ListenerModel {
    /// The trait of its events' markers, such as `PageEvent`.
    trait_name: String,
    /// The operation, such as `page.on`.
    method: String,
    /// Its params type.
    params: String,
    /// Its params' field the handle's ID goes in.
    id_field: String,
    /// Its params' field the event goes in.
    event_field: String,
    /// The type of that field.
    event_type: String,
    doc: String,
    warning: Option<String>,
    markers: Vec<Marker>,
}

/// The marker of one event.
struct Marker {
    name: String,
    /// The event's value of the listener's event type.
    variant: String,
    /// The event's name on the wire, such as `page.console`.
    event: String,
    /// Its payload's type.
    payload: String,
    doc: String,
}

impl ListenerModel {
    /// `handle`'s listener, if it has one.
    pub(super) fn new(index: &Index, handle: &Handle<'_>) -> Result<Option<Self>> {
        let Some(listener) = &handle.listener else {
            return Ok(None);
        };
        let operation = listener.operation;
        let owner = format!("operation `{}`", operation.name);
        let fields = index.fields(&operation.params.0)?;
        let markers = markers(index, handle.namespace, listener)?;
        let description = format!(
            "{}\n\nIn Rust, the marker type of `event`, from [`crate::events`], names the event read, and the stream answered, already sent, hands its events in order; dropping the stream cancels the call without blocking and without a warning.",
            operation.description.trim()
        );
        Ok(Some(Self {
            trait_name: format!("{}Event", type_name(handle.namespace)?),
            method: operation.name.clone(),
            params: index.type_path(&operation.params.0)?,
            id_field: field(&fields, &handle.id.param, &owner)?.ident.clone(),
            event_field: field(&fields, &listener.param, &owner)?.ident.clone(),
            event_type: index.type_path(&listener.event_type)?,
            doc: doc(
                &description,
                operation.stability,
                operation.deprecated.as_ref(),
                EVENTS_ERRORS,
            ),
            warning: warning(
                &operation.name,
                operation.stability,
                operation.deprecated.as_ref(),
            ),
            markers,
        }))
    }

    /// The handle's `events` member in `twin`.
    pub(super) fn member(&self, twin: Twin) -> String {
        let trait_name = &self.trait_name;
        let (returns, body) = match twin {
            Twin::Async => {
                let warning = self
                    .warning
                    .as_ref()
                    .map_or_else(String::new, |warning| format!("{warning}\n        "));
                (
                    "crate::EventStream<E::Data>".to_owned(),
                    format!(
                        "{warning}self.host.listen({:?}, {} {{ {}: self.host.id().clone(), {}: <E as crate::events::sealed::{trait_name}>::EVENT }}, <E as crate::events::sealed::{trait_name}>::NAME)",
                        self.method, self.params, self.id_field, self.event_field
                    ),
                )
            }
            Twin::Blocking => (
                "crate::blocking::EventIter<E::Data>".to_owned(),
                "crate::blocking::EventIter::new(self.inner.events(event), self.inner.host.connection())".to_owned(),
            ),
        };
        let event = match twin {
            Twin::Async => "_event",
            Twin::Blocking => "event",
        };
        format!(
            "\n{}    pub fn events<E: crate::events::{trait_name}>(&self, {event}: E) -> {returns} {{\n        {body}\n    }}\n",
            indent(&self.doc)
        )
    }
}

/// The marker of each event `listener` listens to.
fn markers(index: &Index, namespace: &str, listener: &Listener<'_>) -> Result<Vec<Marker>> {
    listener
        .events
        .iter()
        .map(|listened| {
            let variant = type_name(&listened.wire)?;
            let name = if variant.ends_with("Error") {
                listened.payload.clone()
            } else {
                variant.clone()
            };
            Ok(Marker {
                name,
                variant,
                event: format!("{namespace}.{}", listened.wire),
                payload: index
                    .type_path(&listened.payload)
                    .with_context(|| format!("the payload of `{namespace}.{}`", listened.wire))?,
                doc: comment(&listened.description),
            })
        })
        .collect()
}

/// The `events` module: the trait of each listener's markers, and the
/// markers.
pub(super) fn module(header: &str, listeners: &[ListenerModel]) -> String {
    let mut text = format!(
        "{header}\n//! The events a program reads by meaning (ADR 0081): a marker type for each, which a handle's `events` takes, and a trait for each handle's that names what each carries.\n"
    );
    if listeners.is_empty() {
        return text;
    }
    text.push_str(
        "\n/// What the markers' traits build on, which only the crate implements:\n/// the value that names each event to the engine, and its name in the\n/// engine's batches.\npub(crate) mod sealed {\n",
    );
    for listener in listeners {
        let _ = write!(
            text,
            "    /// What a `{}` names its event by.\n    pub trait {} {{\n        /// The value the call names it by.\n        const EVENT: {};\n        /// Its name in the call's batches.\n        const NAME: &'static str;\n    }}\n",
            listener.trait_name, listener.trait_name, listener.event_type
        );
    }
    text.push_str("}\n");
    for listener in listeners {
        let trait_name = &listener.trait_name;
        let _ = write!(
            text,
            "\n/// An event a handle's `events` reads by its marker, such as\n/// [`{}`]; only this module's markers implement it.\npub trait {trait_name}: sealed::{trait_name} + Copy + Send + Sync + 'static {{\n    /// What the event carries.\n    type Data: ::serde::de::DeserializeOwned + Send + 'static;\n}}\n",
            listener
                .markers
                .first()
                .map_or(trait_name.as_str(), |marker| marker.name.as_str())
        );
        for marker in &listener.markers {
            let name = &marker.name;
            let _ = write!(
                text,
                "\n{}#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]\npub struct {name};\n\nimpl {trait_name} for {name} {{\n    type Data = {};\n}}\n\nimpl sealed::{trait_name} for {name} {{\n    const EVENT: {} = {}::{};\n    const NAME: &'static str = {:?};\n}}\n",
                marker.doc,
                marker.payload,
                listener.event_type,
                listener.event_type,
                marker.variant,
                marker.event
            );
        }
    }
    text
}
