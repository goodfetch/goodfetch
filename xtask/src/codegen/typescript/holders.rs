//! The handles a TypeScript handle holds and keeps (ADRs 0078, 0092): a
//! value every answer that opens it names another handle in is built with
//! that handle, which a sync method named after it answers without a call,
//! as a page answers its Session; the handle of an opener that is not the
//! held one keeps each held handle its answers name, with its host, built
//! the first time and shared after, as the Run keeps its default Session.

use std::fmt::Write as _;

use super::super::handles::{Handle, Holding, Holds};
use super::super::naming::{camel, pascal};

/// What an opener of a `class` answers, from `opened`, its result, whose
/// field `id` names it: the value, built with the handle it holds.
pub(super) fn opened(holding: Option<&Holding>, class: &str, id: &str) -> String {
    match holding {
        None => format!("    return new {class}(this.#host.child(opened.{id}));\n"),
        Some(Holding::Builder(_)) => {
            format!("    return new {class}(this.#host.child(opened.{id}), this);\n")
        }
        Some(Holding::Kept(holds)) => format!(
            "    const [host, {held}] = this.{kept}(opened.{named});\n    return new {class}(host.child(opened.{id}), {held});\n",
            held = camel(holds.namespace),
            kept = kept_method(holds),
            named = camel(&holds.id),
        ),
    }
}

/// The private fields of `handle` that hold and keep handles, each on a
/// line of its own, whose hosts are `HandleHost`s of `operation_types`.
pub(super) fn fields(handle: &Handle<'_>, operation_types: &str) -> String {
    let mut text = String::new();
    if let Some(holds) = &handle.holds {
        let _ = writeln!(
            text,
            "  readonly #{}: {};",
            camel(holds.namespace),
            pascal(holds.namespace)
        );
    }
    for kept in &handle.keeps {
        let _ = writeln!(
            text,
            "  readonly #{}s = new Map<types.{id}, readonly [HandleHost<types.{id}, {operation_types}>, {}]>();",
            camel(kept.namespace),
            pascal(kept.namespace),
            id = kept.type_name,
        );
    }
    text
}

/// The constructor's parameter of the handle `handle` holds, after its
/// others, if it holds one.
pub(super) fn parameter(handle: &Handle<'_>) -> String {
    handle.holds.as_ref().map_or_else(String::new, |holds| {
        format!(", {}: {}", camel(holds.namespace), pascal(holds.namespace))
    })
}

/// The constructor's statement that keeps that parameter.
pub(super) fn assignment(handle: &Handle<'_>) -> String {
    handle.holds.as_ref().map_or_else(String::new, |holds| {
        let held = camel(holds.namespace);
        format!("    this.#{held} = {held};\n")
    })
}

/// The method that answers the handle `handle` holds, and the private one
/// that keeps each handle its openers keep.
pub(super) fn members(handle: &Handle<'_>, operation_types: &str) -> String {
    let mut text = String::new();
    if let Some(holds) = &handle.holds {
        let held = camel(holds.namespace);
        let _ = write!(
            text,
            "\n  /** Its `{held}`, which every answer that opens a `{}` names, answered without a call: the `{held}` that opened or told of it, or the one its opener keeps (ADRs 0078, 0092). */\n  {held}(): {} {{\n    return this.#{held};\n  }}\n",
            handle.namespace,
            pascal(holds.namespace)
        );
    }
    for kept in &handle.keeps {
        let class = pascal(kept.namespace);
        let id = &kept.type_name;
        let _ = write!(
            text,
            "\n  /** The `{}` of ID `id`, with its host: built the first time an answer names it, shared after. */\n  {}(id: types.{id}): readonly [HandleHost<types.{id}, {operation_types}>, {class}] {{\n    let kept = this.#{}s.get(id);\n    if (kept === undefined) {{\n      const host = this.#host.child(id);\n      kept = [host, new {class}(host)];\n      this.#{}s.set(id, kept);\n    }}\n    return kept;\n  }}\n",
            kept.namespace,
            kept_method(kept),
            camel(kept.namespace),
            camel(kept.namespace),
        );
    }
    text
}

/// The private method that keeps the handles `holds` names, such as
/// `#keptSession`.
fn kept_method(holds: &Holds) -> String {
    format!("#kept{}", pascal(holds.namespace))
}
