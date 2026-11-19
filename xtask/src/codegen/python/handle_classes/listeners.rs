//! A handle's listener in the Python handles (ADR 0081): the map that
//! names each event by its wire name under Playwright's name, with the
//! model of what its listener gets, and the `on` member, overloaded once a
//! payload so each event's listener is typed, that registers a listener
//! and returns its `Subscription`. TypeScript's `listeners.rs` is its twin.

use std::fmt::Write as _;

use anyhow::{Result, bail};

use super::super::super::handles::{Handle, Listener};
use super::super::super::layer::Layer;
use super::super::super::naming::snake;
use super::super::{deprecated_decorator, docstring, warnings};
use super::{ASYNC, Imports, Mode, Wrapped, wrapped};

/// The name of the map that names each of a handle's events: `_PAGE_EVENTS`
/// for `page`.
fn events_constant(handle: &Handle<'_>) -> String {
    format!("_{}_EVENTS", snake(handle.namespace).to_uppercase())
}

/// What `handle`'s listener writes before its classes, if it has one: the
/// map its `on` reads each event's wire name and payload from.
pub(super) fn declarations(handle: &Handle<'_>, imports: &mut Imports) -> String {
    let Some(listener) = &handle.listener else {
        return String::new();
    };
    imports.base_model = true;
    let mut text = format!(
        "\n\n{}: dict[str, tuple[str, type[BaseModel]]] = {{\n",
        events_constant(handle)
    );
    for listened in &listener.events {
        let _ = writeln!(
            text,
            "    {:?}: ({:?}, {}),",
            listened.joined(),
            listened.wire,
            imports.model(&listened.payload)
        );
    }
    let _ = writeln!(
        text,
        "}}\n\"\"\"Each event `{}` listens to, under Playwright's name (ADR 0081): its name on the wire, and the model of what its listener gets.\"\"\"",
        listener.operation.name
    );
    text
}

/// A handle's `on` in `mode` (ADR 0081): one overload a payload, which
/// types the listener of the events that carry it, then the method that
/// registers `listener` and returns the `Subscription` that removes it.
pub(super) fn member(
    layer: &Layer,
    handle: &Handle<'_>,
    mode: Mode,
    imports: &mut Imports,
) -> Result<String> {
    let Some(listener) = &handle.listener else {
        return Ok(String::new());
    };
    let operation = listener.operation;
    let params = &operation.params.0;
    let Some(Wrapped { model: event, .. }) = layer
        .definitions
        .get(params)
        .and_then(|schema| wrapped(&schema["properties"][&listener.param], layer, imports))
    else {
        bail!(
            "operation `{}`: the Python handles take the event listened to as one of a choice of strings, but its parameter `{}` is not one (ADR 0081)",
            operation.name,
            listener.param
        );
    };
    let listens = if mode.prefix == ASYNC.prefix {
        "AsyncListener"
    } else {
        "Listener"
    };
    imports.typing.extend(["Any", "Literal", "overload"]);
    imports
        .handle
        .extend(["AsyncListener", "Listener", "Subscription"]);
    let mut text = String::new();
    let decorator = operation
        .deprecated
        .as_ref()
        .map_or_else(String::new, |deprecated| {
            imports.deprecated = true;
            deprecated_decorator(deprecated)
        });
    // PEP 702 marks each overload, after its `@overload`.
    for (payload, names) in by_payload(listener) {
        let _ = write!(
            text,
            "    @overload\n{decorator}    def on(self, event: Literal[{}], listener: {listens}[{}]) -> Subscription: ...\n\n",
            names.join(", "),
            imports.model(payload)
        );
    }
    let _ = writeln!(
        text,
        "    def on(self, event: str, listener: {listens}[Any]) -> Subscription:"
    );
    text.push_str(&docstring(
        "        ",
        &format!("{}\n\n{}", operation.description, event_list(listener)),
        operation.stability,
        operation.deprecated.as_ref(),
    ));
    text.push_str(&warnings(
        &operation.name,
        operation.stability,
        operation.deprecated.as_ref(),
        &mut imports.markers,
    ));
    let _ = writeln!(
        text,
        "        listened, payload = {}[event]\n        return self._host.listen({:?}, {}({}=self._host.id, {}={event}(listened)), f\"{}.{{listened}}\", payload, listener)",
        events_constant(handle),
        operation.name,
        imports.model(params),
        handle.id.param,
        listener.param,
        handle.namespace,
    );
    Ok(text)
}

/// The events of `listener` by payload, in the order each payload first
/// appears, each event under Playwright's name, quoted.
fn by_payload<'a>(listener: &'a Listener<'_>) -> Vec<(&'a str, Vec<String>)> {
    let mut groups: Vec<(&str, Vec<String>)> = Vec::new();
    for listened in &listener.events {
        let name = format!("{:?}", listened.joined());
        match groups
            .iter_mut()
            .find(|(payload, _)| *payload == listened.payload)
        {
            Some((_, names)) => names.push(name),
            None => groups.push((&listened.payload, vec![name])),
        }
    }
    groups
}

/// The docstring paragraph that lists `listener`'s events under
/// Playwright's names, each with its description.
fn event_list(listener: &Listener<'_>) -> String {
    let mut text = String::from("Its events, under Playwright's names (ADR 0081):\n");
    for listened in &listener.events {
        let _ = write!(
            text,
            "\n- `{}`: {}",
            listened.joined(),
            listened.description.trim_end()
        );
    }
    text
}
