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
//!
//! A listener whose events the engine publishes on its event stream, such
//! as a Session's, reads them from the Connection's own Subscription: its
//! `events` joins it with the handle's ID, which each payload names, and
//! its trait builds what the stream hands of a payload, the handle an
//! event tells of, such as a `Page` for `session.page`, or the payload
//! itself, and its blocking twin.

use std::fmt::Write as _;

use anyhow::{Context as _, Result};

use super::super::handles::{Handle, Listener};
use super::index::Index;
use super::{Twin, comment, doc, field, indent, type_name, warning};

/// The `# Errors` section of an `events` member.
const EVENTS_ERRORS: &str = "# Errors\n\nIts stream hands `closed` when its handle or the Connection is closed, the engine's error, such as `page_events_lost`, and `internal` for an event that does not fit its type.";

/// The `# Errors` section of an `events` member whose events the engine
/// publishes.
const PUBLISHED_ERRORS: &str = "# Errors\n\nIts stream hands `closed` when its handle or the Connection is closed, the engine's error, and `internal` for an event that does not fit its type.";

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
    /// The handle's type, such as `Session`, when the engine publishes its
    /// events on its event stream.
    published: Option<String>,
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
    /// The handle the event tells of, which a stream hands in place of its
    /// payload.
    builds: Option<Built>,
    doc: String,
}

/// A handle an event's payload names, as a stream hands it.
struct Built {
    /// Its type, such as `Page`.
    type_name: String,
    /// The payload's field its ID is in, such as `page_id`.
    id_field: String,
}

impl ListenerModel {
    /// `handle`'s listener, if it has one.
    pub(super) fn new(index: &Index, handle: &Handle<'_>) -> Result<Option<Self>> {
        let (listener, published) = match (&handle.listener, &handle.published_listener) {
            (Some(listener), _) => (listener, None),
            (None, Some(listener)) => (listener, Some(type_name(handle.namespace)?)),
            (None, None) => return Ok(None),
        };
        let operation = listener.operation;
        let owner = format!("operation `{}`", operation.name);
        let fields = index.fields(&operation.params.0)?;
        let markers = markers(index, handle.namespace, listener)?;
        let (reads, errors) = if published.is_some() {
            (
                "In Rust, the marker type of `event`, from [`crate::events`], names the event read, and the stream answered, its call already sent, hands its events in order. It reads them from the Connection's own Subscription, which the first such stream sends before its call and the last one dropped cancels; an event it has not read holds one of the Subscription's credits. Dropping the stream cancels its call without blocking and without a warning.",
                PUBLISHED_ERRORS,
            )
        } else {
            (
                "In Rust, the marker type of `event`, from [`crate::events`], names the event read, and the stream answered, already sent, hands its events in order; dropping the stream cancels the call without blocking and without a warning.",
                EVENTS_ERRORS,
            )
        };
        let description = format!("{}\n\n{reads}", operation.description.trim());
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
                errors,
            ),
            warning: warning(
                &operation.name,
                operation.stability,
                operation.deprecated.as_ref(),
            ),
            markers,
            published,
        }))
    }

    /// The handle's `events` member in `twin`.
    pub(super) fn member(&self, twin: Twin) -> String {
        let trait_name = &self.trait_name;
        let sealed = format!("<E as crate::events::sealed::{trait_name}>");
        let (returns, body) = match twin {
            Twin::Async => {
                let warning = self
                    .warning
                    .as_ref()
                    .map_or_else(String::new, |warning| format!("{warning}\n        "));
                let params = format!(
                    "{} {{ {}: self.host.id().clone(), {}: {sealed}::EVENT }}",
                    self.params, self.id_field, self.event_field
                );
                let body = if self.published.is_some() {
                    format!(
                        "{warning}let handle = self.clone();\n        self.host.listen_published({:?}, {params}, {sealed}::NAME, {:?}, move |data| {sealed}::build(&handle, data))",
                        self.method, self.id_field
                    )
                } else {
                    format!(
                        "{warning}self.host.listen({:?}, {params}, {sealed}::NAME)",
                        self.method
                    )
                };
                ("crate::EventStream<E::Data>".to_owned(), body)
            }
            Twin::Blocking if self.published.is_some() => (
                "crate::blocking::EventIter<E::BlockingData>".to_owned(),
                format!("crate::blocking::EventIter::new(self.inner.events(event).map({sealed}::blocking), self.inner.host.connection())"),
            ),
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
                builds: listened
                    .builds
                    .map(|built| {
                        Ok::<_, anyhow::Error>(Built {
                            type_name: type_name(built)?,
                            id_field: format!("{built}_id"),
                        })
                    })
                    .transpose()?,
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
        "\n/// What the markers' traits build on, which only the crate implements:\n/// the value that names each event to the engine, its name in the\n/// engine's batches, and what a stream builds of an event the engine\n/// publishes.\npub(crate) mod sealed {\n",
    );
    for (index, listener) in listeners.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        let trait_name = &listener.trait_name;
        let _ = write!(
            text,
            "    /// What a `{trait_name}` names its event by.\n    pub trait {trait_name} {{\n        /// The value the call names it by.\n        const EVENT: {};\n        /// Its name in the engine's batches.\n        const NAME: &'static str;\n",
            listener.event_type
        );
        if let Some(handle) = &listener.published {
            let _ = write!(
                text,
                "\n        /// What a stream of `handle`'s events hands of the event's\n        /// `data`.\n        ///\n        /// # Errors\n        ///\n        /// `internal` when `data` does not fit the event's payload.\n        fn build(\n            handle: &crate::{handle},\n            data: ::serde_json::Value,\n        ) -> Result<<Self as super::{trait_name}>::Data, crate::Error>\n        where\n            Self: super::{trait_name};\n\n        /// The blocking twin of what a stream hands.\n        fn blocking(\n            data: <Self as super::{trait_name}>::Data,\n        ) -> <Self as super::{trait_name}>::BlockingData\n        where\n            Self: super::{trait_name};\n"
            );
        }
        text.push_str("    }\n");
    }
    text.push_str("}\n");
    for listener in listeners {
        let trait_name = &listener.trait_name;
        let carries = if listener.published.is_some() {
            "    /// What a stream hands of the event: the handle it tells of, or\n    /// its payload.\n    type Data: Send + 'static;\n    /// What a blocking iterator hands of it.\n    type BlockingData: Send + 'static;\n"
        } else {
            "    /// What the event carries.\n    type Data: ::serde::de::DeserializeOwned + Send + 'static;\n"
        };
        let _ = write!(
            text,
            "\n/// An event a handle's `events` reads by its marker, such as\n/// [`{}`]; only this module's markers implement it.\npub trait {trait_name}: sealed::{trait_name} + Copy + Send + Sync + 'static {{\n{carries}}}\n",
            listener
                .markers
                .first()
                .map_or(trait_name.as_str(), |marker| marker.name.as_str())
        );
        for marker in &listener.markers {
            text.push_str(&marker_text(listener, marker));
        }
    }
    text
}

/// `marker`'s struct, and its impls of `listener`'s traits.
fn marker_text(listener: &ListenerModel, marker: &Marker) -> String {
    let (trait_name, name) = (&listener.trait_name, &marker.name);
    let mut text = format!(
        "\n{}#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]\npub struct {name};\n",
        marker.doc
    );
    let constants = format!(
        "    const EVENT: {} = {}::{};\n    const NAME: &'static str = {:?};\n",
        listener.event_type, listener.event_type, marker.variant, marker.event
    );
    let Some(handle) = &listener.published else {
        let _ = write!(
            text,
            "\nimpl {trait_name} for {name} {{\n    type Data = {};\n}}\n\nimpl sealed::{trait_name} for {name} {{\n{constants}}}\n",
            marker.payload
        );
        return text;
    };
    let payload = &marker.payload;
    let (data, blocking_data, build, blocking) = match &marker.builds {
        Some(built) => (
            format!("crate::{}", built.type_name),
            format!("crate::blocking::{}", built.type_name),
            format!(
                "let payload: {payload} = ::serde_json::from_value(data).map_err(crate::Error::shape)?;\n        Ok(crate::{} {{ host: handle.host.reached(payload.{}) }})",
                built.type_name, built.id_field
            ),
            format!("crate::blocking::{} {{ inner: data }}", built.type_name),
        ),
        None => (
            payload.clone(),
            payload.clone(),
            "::serde_json::from_value(data).map_err(crate::Error::shape)".to_owned(),
            "data".to_owned(),
        ),
    };
    let unused = if marker.builds.is_some() {
        "handle"
    } else {
        "_handle"
    };
    let _ = write!(
        text,
        "\nimpl {trait_name} for {name} {{\n    type Data = {data};\n    type BlockingData = {blocking_data};\n}}\n\nimpl sealed::{trait_name} for {name} {{\n{constants}\n    fn build({unused}: &crate::{handle}, data: ::serde_json::Value) -> Result<{data}, crate::Error> {{\n        {build}\n    }}\n\n    fn blocking(data: {data}) -> {blocking_data} {{\n        {blocking}\n    }}\n}}\n"
    );
    text
}
