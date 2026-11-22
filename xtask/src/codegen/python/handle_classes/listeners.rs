//! A handle's listener in the Python handles (ADR 0081): the map that
//! names each event by its wire name under Playwright's name, with the
//! model of what its listener gets, and the `on` member, overloaded once
//! per type its listener gets when there are several, that registers a
//! listener and returns its `Subscription`. A page's events come on its
//! `on` call; a Session's, which the engine publishes on its event stream,
//! on the Connection's own Subscription, and a payload that opened a handle
//! reaches the listener as that handle. TypeScript's `listeners.rs` is its
//! twin.

use std::fmt::Write as _;

use anyhow::{Result, bail};
use goodfetch_protocol::connection::Form;

use super::super::super::handles::{Handle, Listened, Listener};
use super::super::super::layer::Layer;
use super::super::super::naming::{pascal, snake};
use super::super::{deprecated_decorator, docstring, warnings};
use super::{ASYNC, Imports, Mode, WARN_ENTRY, Wrapped, wrapped};

/// The name of the map that names each of a handle's events: `_PAGE_EVENTS`
/// for `page`.
fn events_constant(handle: &Handle<'_>) -> String {
    format!("_{}_EVENTS", snake(handle.namespace).to_uppercase())
}

/// What `handle`'s listener writes before its classes, if it has one whose
/// events come on its own call: the map its `on` reads each event's wire
/// name and payload from.
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
            listened.spelled(Form::Python),
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

/// A handle's `on` in `mode` (ADR 0081), if it has a listener: the method
/// that warns of the markers of the event listened to, once per process
/// (ADR 0030), registers `listener` and returns the `Subscription` that
/// removes it, overloaded once per type its listener gets when there are
/// several.
pub(super) fn member(
    layer: &Layer,
    handle: &Handle<'_>,
    mode: Mode,
    imports: &mut Imports,
) -> Result<String> {
    if let Some(listener) = &handle.listener {
        let (mut text, event) = head(layer, listener, mode, imports, |listened, imports| {
            imports.model(&listened.payload)
        })?;
        let _ = writeln!(
            text,
            "        listened, payload = {}[event]\n        warn_entry(f\"{namespace}.{{listened}}\", EXPERIMENTAL_EVENTS, DEPRECATED_EVENTS)\n        return self._host.listen({:?}, {}({}=self._host.id, {}={event}(listened)), f\"{}.{{listened}}\", payload, listener)",
            events_constant(handle),
            listener.operation.name,
            imports.model(&listener.operation.params.0),
            handle.id.param,
            listener.param,
            handle.namespace,
            namespace = handle.namespace,
        );
        return Ok(text);
    }
    if let Some(listener) = &handle.published_listener {
        return published_member(layer, handle, listener, mode, imports);
    }
    Ok(String::new())
}

/// A handle's `on` whose events the engine publishes on its event stream
/// (ADR 0081): it warns of the markers of the event listened to, once per
/// process (ADR 0030), registers `listener` on the Connection's own
/// Subscription for the events whose payload names the handle, sends the
/// call that arms it, and returns the `Subscription` that removes it. A
/// payload an opener
/// of another handle answers reaches the listener as that handle, built in
/// `mode`.
fn published_member(
    layer: &Layer,
    handle: &Handle<'_>,
    listener: &Listener<'_>,
    mode: Mode,
    imports: &mut Imports,
) -> Result<String> {
    let (mut text, event) = head(layer, listener, mode, imports, |listened, imports| {
        gets(listened, mode, imports)
    })?;
    let operation = listener.operation;
    let params = imports.model(&operation.params.0);
    let Some((last, others)) = listener.events.split_last() else {
        bail!(
            "operation `{}`: a listener listens to at least one event (ADR 0081)",
            operation.name
        );
    };
    imports.markers.insert(WARN_ENTRY);
    let listen = |listened: &Listened, indent: &str, imports: &mut Imports| {
        let heard = listened.builds.map_or_else(
            || "listener".to_owned(),
            |namespace| {
                format!(
                    "lambda opened: listener({}{}(self._host.child(opened.{namespace}_id)))",
                    mode.prefix,
                    pascal(namespace)
                )
            },
        );
        format!(
            "{indent}warn_entry(\"{}.{}\", EXPERIMENTAL_EVENTS, DEPRECATED_EVENTS)\n{indent}return self._host.listen_published({:?}, {params}({id}=self._host.id, {}={event}({:?})), \"{}.{}\", {id:?}, {}, {heard})",
            handle.namespace,
            listened.wire,
            operation.name,
            listener.param,
            listened.wire,
            handle.namespace,
            listened.wire,
            imports.model(&listened.payload),
            id = handle.id.param,
        )
    };
    for listened in others {
        let _ = writeln!(
            text,
            "        if event == {:?}:\n{}",
            listened.spelled(Form::Python),
            listen(listened, "            ", imports)
        );
    }
    let _ = writeln!(text, "{}", listen(last, "        ", imports));
    Ok(text)
}

/// What a published listener gets of `listened` in `mode`: the handle its
/// payload opened, or its payload.
fn gets(listened: &Listened, mode: Mode, imports: &mut Imports) -> String {
    listened.builds.map_or_else(
        || imports.model(&listened.payload),
        |namespace| format!("{}{}", mode.prefix, pascal(namespace)),
    )
}

/// The part of a handle's `on` both kinds of listener share: its
/// signature, one overload per type its listener gets when there are
/// several, as `gets` names each, its docstring and its markers' warnings.
/// Also the model of the event parameter's choice of strings.
fn head(
    layer: &Layer,
    listener: &Listener<'_>,
    mode: Mode,
    imports: &mut Imports,
    gets: impl Fn(&Listened, &mut Imports) -> String,
) -> Result<(String, String)> {
    let operation = listener.operation;
    let params = &operation.params.0;
    let Some(Wrapped { model: event, .. }) = layer.definitions.get(params).and_then(|schema| {
        wrapped(
            &schema["properties"][&listener.param],
            layer,
            imports,
            |_| true,
        )
    }) else {
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
    imports.typing.insert("Literal");
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
    let groups = by_gets(listener, |listened| gets(listened, imports));
    if let [(got, names)] = groups.as_slice() {
        let _ = writeln!(
            text,
            "{decorator}    def on(self, event: Literal[{}], listener: {listens}[{got}]) -> Subscription:",
            names.join(", ")
        );
    } else {
        imports.typing.extend(["Any", "overload"]);
        // PEP 702 marks each overload, after its `@overload`.
        for (got, names) in &groups {
            let _ = write!(
                text,
                "    @overload\n{decorator}    def on(self, event: Literal[{}], listener: {listens}[{got}]) -> Subscription: ...\n\n",
                names.join(", ")
            );
        }
        let _ = writeln!(
            text,
            "    def on(self, event: str, listener: {listens}[Any]) -> Subscription:"
        );
    }
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
    Ok((text, event))
}

/// The events of `listener` by what their listener gets, as `gets` names
/// it, in the order each first appears, each event under Playwright's name,
/// quoted.
fn by_gets(
    listener: &Listener<'_>,
    mut gets: impl FnMut(&Listened) -> String,
) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for listened in &listener.events {
        let name = format!("{:?}", listened.spelled(Form::Python));
        let got = gets(listened);
        match groups.iter_mut().find(|(known, _)| *known == got) {
            Some((_, names)) => names.push(name),
            None => groups.push((got, vec![name])),
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
            listened.spelled(Form::Python),
            listened.description.trim_end()
        );
    }
    text
}
