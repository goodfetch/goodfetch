//! A handle's listener in the TypeScript handles (ADR 0081): the
//! `<Class>Events` interface that types what its listener gets of each event
//! under Playwright's names, the value its call names each by, and the `on`
//! member that registers a listener and returns its `Subscription`. A page's
//! events come on its `on` call; a Session's, which the engine publishes on
//! its event stream, on the Connection's own Subscription, and a payload
//! that opened a handle reaches the listener as that handle.

use std::fmt::Write as _;

use goodfetch_protocol::connection::Form;

use super::super::handles::{Handle, Listened, Listener};
use super::super::layer::Stability;
use super::super::naming::{camel, pascal, snake};
use super::doc;
use super::markers::warnings;

/// What `handle`'s listener adds to its class, if it has one: the
/// declarations before the class, and its `on` member, which warns of the
/// markers the `registry` of events gives the event listened to.
pub(super) fn emit(
    class: &str,
    handle: &Handle<'_>,
    host: &str,
    registry: &str,
) -> (String, String) {
    let events = events_constant(handle);
    if let Some(listener) = &handle.listener {
        return (
            listened_events(class, listener, &events),
            format!(
                "\n{}",
                listener_member(class, listener, handle, &events, registry)
            ),
        );
    }
    if let Some(listener) = &handle.published_listener {
        return (
            published_events(class, listener, handle, host, &events),
            format!(
                "\n{}",
                published_member(class, listener, handle, &events, registry)
            ),
        );
    }
    Default::default()
}

/// The name of the value that names each of a handle's events by its wire
/// name: `PAGE_EVENTS` for `page`.
fn events_constant(handle: &Handle<'_>) -> String {
    format!("{}_EVENTS", snake(handle.namespace).to_uppercase())
}

/// The events a handle's `listener` listens to, under Playwright's names
/// (ADR 0081): the interface that types each one's payload, and the value
/// its call names each by.
fn listened_events(class: &str, listener: &Listener<'_>, events: &str) -> String {
    let mut text = events_interface(class, listener);
    text.push_str(&events_value(class, listener, events));
    text
}

/// The `<Class>Events` interface: what `listener`'s listener gets of each
/// event, the handle a payload opened or the payload itself.
fn events_interface(class: &str, listener: &Listener<'_>) -> String {
    let mut text = format!(
        "\n/** The events a `{class}` listens to with `on`, under Playwright's names (ADR 0081), and what its listener gets of each. */\nexport interface {class}Events {{\n"
    );
    for listened in &listener.events {
        text.push_str(&doc("  ", &listened.description, Stability::Stable, None));
        let _ = writeln!(
            text,
            "  readonly {}: {};",
            listened.spelled(Form::Typescript),
            gets(listened)
        );
    }
    text.push_str("}\n");
    text
}

/// What a listener gets of `listened`: the handle its payload opened, or
/// its payload.
fn gets(listened: &Listened) -> String {
    listened
        .builds
        .map_or_else(|| format!("types.{}", listened.payload), pascal)
}

/// The value `listener`'s call names each of `<Class>Events` by.
fn events_value(class: &str, listener: &Listener<'_>, events: &str) -> String {
    let params = &listener.operation.params.0;
    let mut text = String::new();
    let _ = write!(
        text,
        "\n/** The value `{}` names each of `{class}Events` by. */\nconst {events} = {{\n",
        listener.operation.name,
    );
    for listened in &listener.events {
        let _ = writeln!(
            text,
            "  {}: {:?},",
            listened.spelled(Form::Typescript),
            listened.wire
        );
    }
    let _ = writeln!(
        text,
        "}} as const satisfies {{ readonly [Name in keyof {class}Events]: types.{params}[{:?}] }};",
        camel(&listener.param)
    );
    text
}

/// A handle's `on` (ADR 0081): it warns of the markers the `registry`
/// gives the event listened to, once per process (ADR 0030), registers
/// `listener` for it and returns the `Subscription` that removes it.
fn listener_member(
    class: &str,
    listener: &Listener<'_>,
    handle: &Handle<'_>,
    events: &str,
    registry: &str,
) -> String {
    let operation = listener.operation;
    let namespace = handle.namespace;
    let mut text = doc(
        "  ",
        &operation.description,
        operation.stability,
        operation.deprecated.as_ref(),
    );
    let _ = write!(
        text,
        "  on<Name extends keyof {class}Events>(event: Name, listener: Listener<{class}Events[Name]>): Subscription {{\n{}    const listened = {events}[event];\n    warnEntry({registry}, `{namespace}.${{listened}}`);\n    return this.#host.listen({:?}, {{ {}: this.#host.id, {}: listened }}, `{namespace}.${{listened}}`, listener);\n  }}\n",
        warnings(operation),
        operation.name,
        camel(&handle.id.param),
        camel(&listener.param)
    );
    text
}

/// The events a handle's `listener` listens to when the engine publishes
/// them on its event stream: those of [`listened_events`], the payload of
/// each, and what builds the value its listener gets from that payload
/// with the handle's `host`.
fn published_events(
    class: &str,
    listener: &Listener<'_>,
    handle: &Handle<'_>,
    host: &str,
    events: &str,
) -> String {
    let mut text = events_interface(class, listener);
    let _ = write!(
        text,
        "\n/** The payload the engine publishes for each of `{class}Events`. */\ninterface {class}Payloads {{\n"
    );
    for listened in &listener.events {
        let _ = writeln!(
            text,
            "  readonly {}: types.{};",
            listened.spelled(Form::Typescript),
            listened.payload
        );
    }
    text.push_str("}\n");
    text.push_str(&events_value(class, listener, events));
    let _ = write!(
        text,
        "\n/** What builds each of `{class}Events` from its payload, with the host of the `{}` that listens. */\nconst {events}_VALUES: {{\n  readonly [Name in keyof {class}Events]: (host: {host}, payload: {class}Payloads[Name]) => {class}Events[Name];\n}} = {{\n",
        handle.namespace
    );
    for listened in &listener.events {
        let built = listened.builds.map_or_else(
            || "(_host, payload) => payload".to_owned(),
            |namespace| {
                format!(
                    "(host, payload) => new {}(host.child(payload.{}))",
                    pascal(namespace),
                    camel(&format!("{namespace}_id"))
                )
            },
        );
        let _ = writeln!(text, "  {}: {built},", listened.spelled(Form::Typescript));
    }
    text.push_str("};\n");
    text
}

/// A handle's `on` when the engine publishes its events on its event
/// stream (ADR 0081): it registers `listener` on the Connection's own
/// Subscription for the events whose payload names the handle, sends the
/// call that arms it, and returns the `Subscription` that removes it. It
/// warns of the markers the `registry` gives the event, as `on` does.
fn published_member(
    class: &str,
    listener: &Listener<'_>,
    handle: &Handle<'_>,
    events: &str,
    registry: &str,
) -> String {
    let operation = listener.operation;
    let namespace = handle.namespace;
    let mut text = doc(
        "  ",
        &operation.description,
        operation.stability,
        operation.deprecated.as_ref(),
    );
    let _ = write!(
        text,
        "  on<Name extends keyof {class}Events>(event: Name, listener: Listener<{class}Events[Name]>): Subscription {{\n{}    const listened = {events}[event];\n    warnEntry({registry}, `{namespace}.${{listened}}`);\n    const build = {events}_VALUES[event];\n    return this.#host.listenPublished({:?}, {{ {}: this.#host.id, {}: listened }}, `{namespace}.${{listened}}`, {id:?}, (payload: {class}Payloads[Name], dropped: number) => listener(build(this.#host, payload), dropped));\n  }}\n",
        warnings(operation),
        operation.name,
        camel(&handle.id.param),
        camel(&listener.param),
        id = handle.id.param,
    );
    text
}
