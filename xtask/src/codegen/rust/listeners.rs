//! A handle's listener in the Rust handles (ADR 0081): Rust reads events by
//! meaning, so a handle's `on` is its `events` member, which takes the
//! marker type of one event and returns the crate's `EventStream` of its
//! payloads, and the `events` module holds the markers, one unit struct for
//! each event, with the trait that names what each carries.
//!
//! A marker is the event's name in upper camel case, such as `Console`; an
//! event whose name would end in `Error`, the suffix of ADR 0079's error
//! types, takes its payload's name instead, so `page_error` is `WebError`,
//! Playwright's own name for it. `goodfetch_protocol::naming` holds that
//! rule, which the engine names the event by in its messages.
//!
//! A listener whose events the engine publishes on its event stream, such
//! as a Session's, reads them from the Connection's own Subscription: its
//! `events` joins it with the handle's ID, which each payload names, and
//! its trait builds what the stream hands of a payload, the handle an
//! event tells of, such as a `Page` for `session.page`, or the payload
//! itself, and its blocking twin.
//!
//! A listener one of whose events opens a value with events of its own,
//! which the same call carries, such as a page's `web_socket`, hands that
//! value in place of its payload: a struct named after it, such as
//! `WebSocket`, with the opening payload's fields but the handle's ID and
//! an `events` that hands its own events, as an enum such as
//! `WebSocketEvent`, its close last. Each of the listener's markers then
//! sends its call itself, and builds the blocking twin of what its stream
//! hands.

use std::fmt::Write as _;

use anyhow::{Context as _, Result};

use super::super::handles::{Handle, Listened, Listener, Opens};
use super::super::naming;
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
    /// The handle's type, such as `Page`, when one of its events opens a
    /// value whose own events its call carries.
    opening: Option<String>,
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
    /// The value the event opens, which a stream hands in place of its
    /// payload.
    opens: Option<OpenedValue>,
    doc: String,
    /// The warning of the event's own markers, if it has any.
    warning: Option<String>,
}

/// A value an event opens, whose own events its listener's call carries.
struct OpenedValue {
    /// Its type, such as `WebSocket`.
    type_name: String,
    /// The namespace of its own events, such as `web_socket`.
    namespace: String,
    /// The property its events name it by, such as `web_socket_id`.
    id: String,
    /// What it holds of the opening payload.
    fields: Vec<ValueField>,
    /// Its own events, in the catalog's order.
    events: Vec<OwnEvent>,
}

/// A field of an [`OpenedValue`], as its opening payload has it.
struct ValueField {
    ident: String,
    ty: String,
    doc: String,
}

/// An event of an [`OpenedValue`]'s own: a variant of its events' enum.
struct OwnEvent {
    /// The variant, such as `FrameSent`.
    variant: String,
    /// The event's name on the wire, such as `web_socket.frame_sent`.
    event: String,
    /// Its payload's type.
    payload: String,
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
        let opened = if listener.opens() {
            " An event that opens a value hands that value, whose own `events` hand the events of its own the same call carries, in order, its close last; dropping that stream passes over them, and the call goes on."
        } else {
            ""
        };
        let description = format!("{}\n\n{reads}{opened}", operation.description.trim());
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
                Some(errors),
            ),
            warning: warning(
                &operation.name,
                operation.stability,
                operation.deprecated.as_ref(),
            ),
            markers,
            published,
            opening: if listener.opens() {
                Some(type_name(handle.namespace)?)
            } else {
                None
            },
        }))
    }

    /// The struct and events' enum of each value the listener's events
    /// open, in `twin`: the blocking twin's struct hands its events through
    /// a blocking iterator, of the same enum.
    pub(super) fn values(&self, twin: Twin) -> String {
        let mut text = String::new();
        for value in self
            .markers
            .iter()
            .filter_map(|marker| marker.opens.as_ref())
        {
            text.push_str(&value_text(value, twin));
        }
        text
    }

    /// The handle's `events` member in `twin`, which warns of the markers
    /// of the listener and of the event read, once per process (ADR 0030);
    /// the blocking twin calls the async one.
    pub(super) fn member(&self, twin: Twin) -> String {
        let trait_name = &self.trait_name;
        let sealed = format!("<E as crate::events::sealed::{trait_name}>");
        let (returns, body) = match twin {
            Twin::Async => {
                let warning = format!(
                    "{}{sealed}::warn();\n        ",
                    self.warning
                        .as_ref()
                        .map_or_else(String::new, |warning| format!("{warning}\n        "))
                );
                let params = format!(
                    "{} {{ {}: self.host.id().clone(), {}: {sealed}::EVENT }}",
                    self.params, self.id_field, self.event_field
                );
                let body = if self.published.is_some() {
                    format!(
                        "{warning}let handle = self.clone();\n        self.host.listen_published({:?}, {params}, {sealed}::NAME, {:?}, move |data| {sealed}::build(&handle, data))",
                        self.method, self.id_field
                    )
                } else if self.opening.is_some() {
                    format!("{warning}{sealed}::listen(self, {params})")
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
            Twin::Blocking if self.opening.is_some() => (
                "crate::blocking::EventIter<E::BlockingData>".to_owned(),
                format!("let connection = self.inner.host.connection().clone();\n        crate::blocking::EventIter::new(self.inner.events(event).map(move |data| {sealed}::blocking(data, &connection)), self.inner.host.connection())"),
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
            Ok(Marker {
                name: marker_name(listened)?,
                variant: type_name(&listened.wire)?,
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
                opens: listened
                    .opens
                    .as_ref()
                    .map(|opens| opened_value(index, listened, opens))
                    .transpose()?,
                doc: comment(&listened.description),
                warning: warning(
                    &format!("{namespace}.{}", listened.wire),
                    listened.stability,
                    listened.deprecated.as_ref(),
                ),
            })
        })
        .collect()
}

/// The value `listened` opens as `opens` says, resolved against the
/// protocol crate.
fn opened_value(index: &Index, listened: &Listened, opens: &Opens) -> Result<OpenedValue> {
    let owner = format!("the payload of `{}`", listened.wire);
    let held = index.fields(&listened.payload).context(owner.clone())?;
    let fields = opens
        .fields
        .iter()
        .map(|(wire, description)| {
            let found = field(&held, wire, &owner)?;
            Ok(ValueField {
                ident: found.ident.clone(),
                ty: found.ty.clone(),
                doc: comment(description),
            })
        })
        .collect::<Result<_>>()?;
    let events = opens
        .events
        .iter()
        .map(|own| {
            let event = format!("{}.{}", opens.namespace, own.wire);
            Ok(OwnEvent {
                variant: type_name(&own.wire)?,
                payload: index
                    .type_path(&own.payload)
                    .with_context(|| format!("the payload of `{event}`"))?,
                event,
                doc: comment(&own.description),
            })
        })
        .collect::<Result<_>>()?;
    Ok(OpenedValue {
        type_name: type_name(&opens.namespace)?,
        namespace: opens.namespace.clone(),
        id: opens.id.clone(),
        fields,
        events,
    })
}

/// `value`'s struct, its events' enum in the async twin, and their impls.
fn value_text(value: &OpenedValue, twin: Twin) -> String {
    let name = &value.type_name;
    let fields = value.fields.iter().fold(String::new(), |mut text, field| {
        let _ = writeln!(
            text,
            "{}    pub {}: {},",
            indent(&field.doc),
            field.ident,
            field.ty
        );
        text
    });
    // Each field's ident, each before a comma, as a struct pattern lists
    // them before `events`.
    let idents = value.fields.iter().fold(String::new(), |mut text, field| {
        let _ = write!(text, "{}, ", field.ident);
        text
    });
    match twin {
        Twin::Async => {
            let mut text = format!(
                "\n/// A `{namespace}` an event opened (ADR 0081): what that event told of it,\n/// and its own events, which the same call carries.\n#[derive(Debug)]\npub struct {name} {{\n{fields}    pub(crate) events: crate::EventStream<{name}Event>,\n}}\n\nimpl {name} {{\n    /// Its own events, in order, each once, its close last, after which\n    /// the stream ends; the stream also ends with the call that opened it.\n    /// Dropping the stream passes over the events it has not read, and\n    /// the call's other streams go on. The call stays lossless: events\n    /// held unread keep their credits, so once the engine waits on them,\n    /// the call's other streams wait too.\n    pub fn events(self) -> crate::EventStream<{name}Event> {{\n        self.events\n    }}\n}}\n\n/// An event of a [`{name}`]'s own.\n#[derive(Clone, Debug, ::serde::Deserialize)]\n#[serde(tag = \"event\", content = \"data\")]\n#[non_exhaustive]\npub enum {name}Event {{\n",
                namespace = value.namespace,
            );
            for event in &value.events {
                let _ = writeln!(
                    text,
                    "{}    #[serde(rename = {:?})]\n    {}({}),",
                    indent(&event.doc),
                    event.event,
                    event.variant,
                    event.payload
                );
            }
            text.push_str("}\n");
            text
        }
        Twin::Blocking => format!(
            "\n/// The blocking twin of a [`crate::{name}`], whose own events a blocking\n/// iterator hands.\n#[derive(Debug)]\npub struct {name} {{\n{fields}    events: crate::blocking::EventIter<crate::{name}Event>,\n}}\n\nimpl {name} {{\n    /// The blocking twin of `value`, whose events are read on\n    /// `connection`'s runtime.\n    pub(crate) fn new(value: crate::{name}, connection: &crate::ClientConnection) -> Self {{\n        let crate::{name} {{ {idents}events }} = value;\n        Self {{ {idents}events: crate::blocking::EventIter::new(events, connection) }}\n    }}\n\n    /// Its own events, in order, each once, its close last, after which\n    /// the iterator ends; it also ends with the call that opened it.\n    /// Dropping the iterator passes over the events it has not read, and\n    /// the call's other streams go on. The call stays lossless: events\n    /// held unread keep their credits, so once the engine waits on them,\n    /// the call's other streams wait too.\n    pub fn events(self) -> crate::blocking::EventIter<crate::{name}Event> {{\n        self.events\n    }}\n}}\n"
        ),
    }
}

/// The marker type of `listened`, by the rule the engine names it by in
/// its messages (ADR 0099).
fn marker_name(listened: &Listened) -> Result<String> {
    naming::event_marker(&listened.wire, &listened.payload)
        .with_context(|| format!("`{}` is no Rust type name", listened.wire))
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
            "    /// What a `{trait_name}` names its event by.\n    pub trait {trait_name} {{\n        /// The value the call names it by.\n        const EVENT: {};\n        /// Its name in the engine's batches.\n        const NAME: &'static str;\n\n        /// Warns of its markers, once per process (ADR 0030).\n        fn warn() {{}}\n",
            listener.event_type
        );
        if let Some(handle) = &listener.published {
            let _ = write!(
                text,
                "\n        /// What a stream of `handle`'s events hands of the event's\n        /// `data`.\n        ///\n        /// # Errors\n        ///\n        /// `internal` when `data` does not fit the event's payload.\n        fn build(\n            handle: &crate::{handle},\n            data: ::serde_json::Value,\n        ) -> Result<<Self as super::{trait_name}>::Data, crate::Error>\n        where\n            Self: super::{trait_name};\n\n        /// The blocking twin of what a stream hands.\n        fn blocking(\n            data: <Self as super::{trait_name}>::Data,\n        ) -> <Self as super::{trait_name}>::BlockingData\n        where\n            Self: super::{trait_name};\n"
            );
        }
        if let Some(handle) = &listener.opening {
            let _ = write!(
                text,
                "\n        /// The stream of what `handle`'s call with `params`, sent now,\n        /// hands of the event.\n        fn listen(\n            handle: &crate::{handle},\n            params: {},\n        ) -> crate::EventStream<<Self as super::{trait_name}>::Data>\n        where\n            Self: super::{trait_name};\n\n        /// The blocking twin of what a stream hands, whose own events are\n        /// read on `connection`'s runtime.\n        fn blocking(\n            data: <Self as super::{trait_name}>::Data,\n            connection: &crate::ClientConnection,\n        ) -> <Self as super::{trait_name}>::BlockingData\n        where\n            Self: super::{trait_name};\n",
                listener.params
            );
        }
        text.push_str("    }\n");
    }
    text.push_str("}\n");
    for listener in listeners {
        let trait_name = &listener.trait_name;
        let carries = if listener.published.is_some() {
            "    /// What a stream hands of the event: the handle it tells of, or\n    /// its payload.\n    type Data: Send + 'static;\n    /// What a blocking iterator hands of it.\n    type BlockingData: Send + 'static;\n"
        } else if listener.opening.is_some() {
            "    /// What a stream hands of the event: the value it opens, or its\n    /// payload.\n    type Data: Send + 'static;\n    /// What a blocking iterator hands of it.\n    type BlockingData: Send + 'static;\n"
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
    let warn = marker.warning.as_ref().map_or_else(String::new, |warning| {
        format!("\n    fn warn() {{\n        {warning}\n    }}\n")
    });
    let constants = format!(
        "    const EVENT: {} = {}::{};\n    const NAME: &'static str = {:?};\n{warn}",
        listener.event_type, listener.event_type, marker.variant, marker.event
    );
    if let Some(handle) = &listener.opening {
        text.push_str(&opening_impls(listener, marker, handle, &constants));
        return text;
    }
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

/// `marker`'s impls of `listener`'s traits when one of its events opens a
/// value, `handle` the listening handle's type: the marker sends the call,
/// and the opening event's builds the value with its own stream.
fn opening_impls(
    listener: &ListenerModel,
    marker: &Marker,
    handle: &str,
    constants: &str,
) -> String {
    let (trait_name, name, payload) = (&listener.trait_name, &marker.name, &marker.payload);
    let params = &listener.params;
    let method = &listener.method;
    let (data, blocking_data, listen, blocking) = match &marker.opens {
        Some(value) => {
            let value_name = &value.type_name;
            let idents = value.fields.iter().fold(String::new(), |mut text, field| {
                let _ = write!(text, "{0}: opened.{0}, ", field.ident);
                text
            });
            let own = value.events.iter().fold(String::new(), |mut text, event| {
                let _ = write!(text, "{:?}, ", event.event);
                text
            });
            (
                format!("crate::{value_name}"),
                format!("crate::blocking::{value_name}"),
                format!(
                    "let opens = crate::event_stream::Opens {{ opener: Self::NAME, namespace: {:?}, id: {:?}, events: &[{own}] }};\n        handle.host.listen_opening({method:?}, params, opens, |opened: {payload}, events| crate::{value_name} {{ {idents}events }})",
                    value.namespace, value.id
                ),
                format!("crate::blocking::{value_name}::new(data, connection)"),
            )
        }
        None => (
            payload.clone(),
            payload.clone(),
            format!("handle.host.listen({method:?}, params, Self::NAME)"),
            "data".to_owned(),
        ),
    };
    let connection = if marker.opens.is_some() {
        "connection"
    } else {
        "_connection"
    };
    format!(
        "\nimpl {trait_name} for {name} {{\n    type Data = {data};\n    type BlockingData = {blocking_data};\n}}\n\nimpl sealed::{trait_name} for {name} {{\n{constants}\n    fn listen(handle: &crate::{handle}, params: {params}) -> crate::EventStream<{data}> {{\n        {listen}\n    }}\n\n    fn blocking(data: {data}, {connection}: &crate::ClientConnection) -> {blocking_data} {{\n        {blocking}\n    }}\n}}\n"
    )
}

#[cfg(test)]
mod tests;
