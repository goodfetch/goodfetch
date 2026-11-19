//! A handle's listener in the TypeScript handles (ADR 0081): the
//! `<Class>Events` interface that types each event's payload under
//! Playwright's names, the value its call names each by, and the `on`
//! member that registers a listener and returns its `Subscription`.

use std::fmt::Write as _;

use super::super::handles::{Handle, Listener};
use super::super::layer::Stability;
use super::super::naming::snake;
use super::doc;

/// What `handle`'s listener adds to its class, if it has one: the
/// declarations before the class, and its `on` member.
pub(super) fn emit(class: &str, handle: &Handle<'_>) -> (String, String) {
    handle
        .listener
        .as_ref()
        .map_or_else(Default::default, |listener| {
            let events = events_constant(handle);
            (
                listened_events(class, listener, &events),
                format!("\n{}", listener_member(class, listener, handle, &events)),
            )
        })
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
    let params = &listener.operation.params.0;
    let mut text = format!(
        "\n/** The events a `{class}` listens to with `on`, under Playwright's names (ADR 0081), and what its listener gets of each. */\nexport interface {class}Events {{\n"
    );
    for listened in &listener.events {
        text.push_str(&doc("  ", &listened.description, Stability::Stable, None));
        let _ = writeln!(
            text,
            "  readonly {}: types.{};",
            listened.joined(),
            listened.payload
        );
    }
    let _ = write!(
        text,
        "}}\n\n/** The value `{}` names each of `{class}Events` by. */\nconst {events} = {{\n",
        listener.operation.name,
    );
    for listened in &listener.events {
        let _ = writeln!(text, "  {}: {:?},", listened.joined(), listened.wire);
    }
    let _ = writeln!(
        text,
        "}} as const satisfies {{ readonly [Name in keyof {class}Events]: types.{params}[{:?}] }};",
        listener.param
    );
    text
}

/// A handle's `on` (ADR 0081): it registers `listener` for one of its
/// events and returns the `Subscription` that removes it.
fn listener_member(
    class: &str,
    listener: &Listener<'_>,
    handle: &Handle<'_>,
    events: &str,
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
        "  on<Name extends keyof {class}Events>(event: Name, listener: Listener<{class}Events[Name]>): Subscription {{\n    const listened = {events}[event];\n    return this.#host.listen({:?}, {{ {}: this.#host.id, {}: listened }}, `{namespace}.${{listened}}`, listener);\n  }}\n",
        operation.name, handle.id.param, listener.param
    );
    text
}
