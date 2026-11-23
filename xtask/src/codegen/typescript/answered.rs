//! What a round trip, a query or a waiter of the TypeScript handles
//! answers, and the lines that return it: the result, or its one property
//! (`Member::unwrapped`, issue 2883), and a page's response wrapped in the
//! hand-written `Response`, built with the page's host, which reads its
//! body (`Member::responds`, issue 3394), as Python's and Rust's handles
//! answer it.

use super::super::handles::{Handle, Member};
use super::super::naming::camel;

/// The hand-written module of a page's `Response`.
const RESPONSE_MODULE: &str = "../response.ts";

/// The hand-written class of a page's response.
const RESPONSE: &str = "Response";

/// The type `member` answers: the operation's result type, or the type of
/// its one property, which an optional one gives with `null` for absent;
/// a `Response` for a page's response.
pub(super) fn answered(member: &Member<'_>, result: &str) -> String {
    match (&member.unwrapped, member.responds) {
        (None, true) => RESPONSE.to_owned(),
        (None, false) => format!("types.{result}"),
        (Some(unwrapped), true) if unwrapped.required => RESPONSE.to_owned(),
        (Some(_), true) => format!("{RESPONSE} | null"),
        (Some(unwrapped), false) if unwrapped.required => {
            format!("types.{result}[{:?}]", camel(&unwrapped.property))
        }
        (Some(unwrapped), false) => {
            format!(
                "NonNullable<types.{result}[{:?}]> | null",
                camel(&unwrapped.property)
            )
        }
    }
}

/// The lines of `member`'s body that return what it answers of `call`:
/// its result, or its result's one property, `null` when an optional one
/// is absent, a page's response wrapped with the handle's host.
pub(super) fn answer(member: &Member<'_>, call: &str) -> String {
    let wrapped = |value: &str| {
        if member.responds {
            format!("new {RESPONSE}(this.#host, {value})")
        } else {
            value.to_owned()
        }
    };
    match &member.unwrapped {
        None if member.responds => format!("    return {};\n", wrapped(&format!("await {call}"))),
        None => format!("    return {call};\n"),
        Some(unwrapped) if unwrapped.required => format!(
            "    return {};\n",
            wrapped(&format!("(await {call}).{}", camel(&unwrapped.property)))
        ),
        Some(unwrapped) if member.responds => format!(
            "    const response = (await {call}).{} ?? null;\n    return response === null ? null : {};\n",
            camel(&unwrapped.property),
            wrapped("response")
        ),
        Some(unwrapped) => format!(
            "    return (await {call}).{} ?? null;\n",
            camel(&unwrapped.property)
        ),
    }
}

/// The import of the hand-written `Response`, when a member of `handles`
/// answers a page's response.
pub(super) fn response_import(handles: &[Handle<'_>]) -> String {
    let responds = handles.iter().any(|handle| {
        handle
            .members
            .iter()
            .chain(handle.sub_objects.iter().flat_map(|held| &held.members))
            .any(|member| member.responds)
    });
    if responds {
        format!("import {{ {RESPONSE} }} from \"{RESPONSE_MODULE}\";\n")
    } else {
        String::new()
    }
}
