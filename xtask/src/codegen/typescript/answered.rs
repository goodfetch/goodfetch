//! What a round trip, a query or a waiter of the TypeScript handles
//! answers, and the lines that return it: the result, or its one property
//! (`Member::unwrapped`, issue 2883), and a page's response or Run copy
//! wrapped in its hand-written class (`Response`, `Checkpoint`, `RunTrace`).

use std::collections::BTreeSet;

use super::super::handles::{Handle, Member, Wrapped};
use super::super::naming::camel;

/// The hand-written module of a page's `Response`.
const RESPONSE_MODULE: &str = "../response.ts";
/// The hand-written module of a Run's `Checkpoint` and `RunTrace`.
const RUN_COPIES_MODULE: &str = "../run-copies.ts";

fn wrapped_name(wrapped: Wrapped) -> &'static str {
    match wrapped {
        Wrapped::Response => "Response",
        Wrapped::Checkpoint => "Checkpoint",
        Wrapped::RunTrace => "RunTrace",
    }
}

/// The type `member` answers: the operation's result type, or the type of
/// its one property, which an optional one gives with `null` for absent;
/// or the wrapped hand-written class.
pub(super) fn answered(member: &Member<'_>, result: &str) -> String {
    match (&member.unwrapped, member.wrapped) {
        (None, Some(wrapped)) => wrapped_name(wrapped).to_owned(),
        (None, None) => format!("types.{result}"),
        (Some(unwrapped), Some(wrapped)) if unwrapped.required => wrapped_name(wrapped).to_owned(),
        (Some(_), Some(wrapped)) => format!("{} | null", wrapped_name(wrapped)),
        (Some(unwrapped), None) if unwrapped.required => {
            format!("types.{result}[{:?}]", camel(&unwrapped.property))
        }
        (Some(unwrapped), None) => {
            format!(
                "NonNullable<types.{result}[{:?}]> | null",
                camel(&unwrapped.property)
            )
        }
    }
}

/// The lines of `member`'s body that return what it answers of `call`.
pub(super) fn answer(member: &Member<'_>, call: &str) -> String {
    let wrap = |value: &str| match member.wrapped {
        Some(wrapped) => format!("new {}(this.#host, {value})", wrapped_name(wrapped)),
        None => value.to_owned(),
    };
    match &member.unwrapped {
        None if member.wrapped.is_some() => {
            format!("    return {};\n", wrap(&format!("await {call}")))
        }
        None => format!("    return {call};\n"),
        Some(unwrapped) if unwrapped.required => format!(
            "    return {};\n",
            wrap(&format!("(await {call}).{}", camel(&unwrapped.property)))
        ),
        Some(unwrapped) if member.wrapped.is_some() => format!(
            "    const value = (await {call}).{} ?? null;\n    return value === null ? null : {};\n",
            camel(&unwrapped.property),
            wrap("value")
        ),
        Some(unwrapped) => format!(
            "    return (await {call}).{} ?? null;\n",
            camel(&unwrapped.property)
        ),
    }
}

/// The imports of hand-written wrapped classes (`Response`, `Checkpoint`,
/// `RunTrace`) used by members of `handles`.
pub(super) fn response_import(handles: &[Handle<'_>]) -> String {
    let mut response = false;
    let mut copies = BTreeSet::new();
    for member in handles.iter().flat_map(|handle| {
        handle
            .members
            .iter()
            .chain(handle.sub_objects.iter().flat_map(|held| &held.members))
    }) {
        match member.wrapped {
            Some(Wrapped::Response) => response = true,
            Some(Wrapped::Checkpoint) => {
                copies.insert("Checkpoint");
            }
            Some(Wrapped::RunTrace) => {
                copies.insert("RunTrace");
            }
            None => {}
        }
    }
    let mut imports = String::new();
    if response {
        let _ = std::fmt::Write::write_fmt(
            &mut imports,
            format_args!("import {{ Response }} from \"{RESPONSE_MODULE}\";\n"),
        );
    }
    if !copies.is_empty() {
        let names = copies.into_iter().collect::<Vec<_>>().join(", ");
        let _ = std::fmt::Write::write_fmt(
            &mut imports,
            format_args!("import {{ {names} }} from \"{RUN_COPIES_MODULE}\";\n"),
        );
    }
    imports
}
