//! The handles a Python handle holds and keeps (ADRs 0078, 0092): a value
//! every answer that opens it names another handle in is built with that
//! handle, which a property named after it answers without a call, as
//! Playwright's `page.context`; the handle of an opener that is not the
//! held one keeps each held handle its answers name, with its host, built
//! the first time and shared after, as the Run keeps its default Session.
//! The sync twin repeats each under the same names.

use std::fmt::Write as _;

use super::super::super::handles::{Handle, Holding, Holds};
use super::super::super::naming::pascal;
use super::{Imports, Mode};

/// The `return` of an opener whose result is `opened`: the `returns` its
/// `{namespace}_id` names, built with the handle it holds.
pub(super) fn opened(holding: Option<&Holding>, returns: &str, namespace: &str) -> String {
    match holding {
        None => format!("        return {returns}(self._host.child(opened.{namespace}_id))\n"),
        Some(Holding::Builder(_)) => {
            format!("        return {returns}(self._host.child(opened.{namespace}_id), self)\n")
        }
        Some(Holding::Kept(holds)) => format!(
            "        host, {held} = self.{}(opened.{})\n        return {returns}(host.child(opened.{namespace}_id), {held})\n",
            kept_method(holds),
            holds.id,
            held = holds.namespace,
        ),
    }
}

/// The constructor of `handle`'s own class, which takes its `host`: a
/// handle with reads is built with the answer that opened it, which they
/// answer from, and one that holds a handle with that handle.
pub(super) fn constructor(
    handle: &Handle<'_>,
    host: &str,
    mode: Mode,
    imports: &mut Imports,
) -> String {
    let mut takes = vec!["what it reaches its engine through".to_owned()];
    let mut parameters = String::new();
    let mut kept = String::new();
    if let Some(reads) = &handle.reads {
        let _ = write!(parameters, ", opened: {}", imports.model(&reads.opened));
        takes.push("the answer that opened it".to_owned());
        kept.push_str("\n        self._opened = opened");
    }
    if let Some(holds) = &handle.holds {
        let _ = write!(
            parameters,
            ", {}: {}{}",
            holds.namespace,
            mode.prefix,
            pascal(holds.namespace)
        );
        takes.push(format!("the `{}` it holds", holds.namespace));
    }
    kept.push_str(&assignments(handle, mode, imports));
    let takes = match takes.split_last() {
        Some((last, others)) if !others.is_empty() => format!("{} and {last}", others.join(", ")),
        _ => takes.concat(),
    };
    format!(
        "\n    def __init__(self, host: {host}{parameters}) -> None:\n        \"\"\"Takes {takes}; a program never constructs one.\"\"\"\n        self._host = host{kept}"
    )
}

/// The constructor's statements that keep the handle it holds and start
/// each handle kept empty.
fn assignments(handle: &Handle<'_>, mode: Mode, imports: &mut Imports) -> String {
    let mut text = String::new();
    if let Some(holds) = &handle.holds {
        let _ = write!(
            text,
            "\n        self._{held} = {held}",
            held = holds.namespace
        );
    }
    for kept in &handle.keeps {
        let _ = write!(
            text,
            "\n        self._{}s: dict[str, {}] = {{}}",
            kept.namespace,
            kept_type(kept, mode, imports)
        );
    }
    text
}

/// The property that answers the handle `handle` holds, and the private
/// method that keeps each handle its openers keep, as `mode` names them.
pub(super) fn members(handle: &Handle<'_>, mode: Mode, imports: &mut Imports) -> String {
    let mut text = String::new();
    if let Some(holds) = &handle.holds {
        let _ = write!(
            text,
            "\n    @property\n    def {held}(self) -> {}{}:\n        \"\"\"Its `{held}`, which every answer that opens a `{}` names, answered without a call: the `{held}` that opened or told of it, or the one its opener keeps (ADRs 0078, 0092).\"\"\"\n        return self._{held}\n",
            mode.prefix,
            pascal(holds.namespace),
            handle.namespace,
            held = holds.namespace,
        );
    }
    for kept in &handle.keeps {
        let id = imports.model(&kept.type_name);
        let _ = write!(
            text,
            "\n    def {}(self, {param}: {id}) -> {}:\n        \"\"\"The `{namespace}` of ID `{param}`, with its host: built the first time an answer names it, shared after.\"\"\"\n        kept = self._{namespace}s.get({param}.root)\n        if kept is None:\n            host = self._host.child({param})\n            kept = (host, {}{}(host))\n            self._{namespace}s[{param}.root] = kept\n        return kept\n",
            kept_method(kept),
            kept_type(kept, mode, imports),
            mode.prefix,
            pascal(kept.namespace),
            param = kept.id,
            namespace = kept.namespace,
        );
    }
    text
}

/// The pair a handle keeps of `kept`: its host and its handle.
fn kept_type(kept: &Holds, mode: Mode, imports: &mut Imports) -> String {
    format!(
        "tuple[{}[{}], {}{}]",
        mode.host,
        imports.model(&kept.type_name),
        mode.prefix,
        pascal(kept.namespace)
    )
}

/// The private method that keeps the handles `holds` names, such as
/// `_kept_session`.
fn kept_method(holds: &Holds) -> String {
    format!("_kept_{}", holds.namespace)
}
