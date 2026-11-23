//! What a member of the Python handles answers, and the last lines of its
//! body that call its host and return it: the result's model, the handle a
//! call opens, nothing for a call that closes, or, for a round trip, a query
//! or a waiter whose result has one property, that property's value
//! (`Member::unwrapped`, issue 2883), as TypeScript's and Rust's handles
//! answer it.

use anyhow::{Context as _, Result, bail};

use super::super::super::handles::{Form, Member};
use super::super::super::layer::Layer;
use super::{Imports, annotation};

/// The member's call on its host, `awaits` before it, sending `sent`.
///
/// A call's own timeout also bounds its wait in the Client's queue under
/// the engine's cap on calls in flight, failing naming its setting
/// (ADR 0038; issue 3188).
fn host_call(member: &Member<'_>, awaits: &str, sent: &str) -> String {
    let deadline = member
        .timeout_setting
        .as_ref()
        .map(|setting| format!(", timeout_setting={setting:?}"))
        .unwrap_or_default();
    format!(
        "{awaits}self._host.call({:?}, {sent}{deadline})",
        member.operation.name
    )
}

/// What `member` answers in place of its result when the result has one
/// property (`Member::unwrapped`, issue 2883): that property's annotation,
/// with `None` for an optional one, which is absent, and the property.
pub(super) fn answered(
    layer: &Layer,
    member: &Member<'_>,
    imports: &mut Imports,
) -> Result<Option<(String, String)>> {
    let Some(unwrapped) = &member.unwrapped else {
        return Ok(None);
    };
    let operation = member.operation;
    let property = layer
        .definitions
        .get(&operation.result.0)
        .map(|result| &result["properties"][&unwrapped.property])
        .with_context(|| format!("operation `{}` has no result type", operation.name))?;
    let Some(spelled) = annotation(property, imports) else {
        bail!(
            "operation `{}`: no Python annotation for its result's `{}`, which it answers in place of the result",
            operation.name,
            unwrapped.property
        );
    };
    let returns = if unwrapped.required || spelled.ends_with("None") {
        spelled
    } else {
        format!("{spelled} | None")
    };
    Ok(Some((returns, unwrapped.property.clone())))
}

/// What a member's body returns, beside how it calls: the result's model,
/// the annotation it returns, the one property it answers in place of
/// the result, if any, and the hand-written `Response` class that wraps a
/// page's response with the page's host, for a member that answers one
/// (`Member::responds`).
#[derive(Clone, Copy)]
pub(super) struct Returned<'text> {
    pub(super) result: &'text str,
    pub(super) returns: &'text str,
    pub(super) property: Option<&'text str>,
    pub(super) response: Option<&'text str>,
}

/// The last lines of `member`'s body, which send `sent` and return what
/// its form answers.
pub(super) fn returned(
    member: &Member<'_>,
    returned: Returned<'_>,
    awaits: &str,
    sent: &str,
) -> String {
    let Returned {
        result,
        returns,
        property,
        response,
    } = returned;
    let call = host_call(member, awaits, sent);
    match &member.form {
        Form::RoundTrip | Form::Query | Form::Waiter => {
            let answer = format!(
                "result_of({result}, {call}){}",
                property
                    .map(|property| format!(".{property}"))
                    .unwrap_or_default()
            );
            match response {
                None => format!("        return {answer}\n"),
                Some(class) if returns.ends_with("None") => format!(
                    "        response = {answer}\n        return None if response is None else {class}(self._host, response)\n"
                ),
                Some(class) => format!("        return {class}(self._host, {answer})\n"),
            }
        }
        Form::Opens(opened) => format!(
            "        opened = result_of({result}, {call})\n        return {returns}(self._host.child(opened.{opened}_id))\n"
        ),
        Form::Closes => format!(
            "        {awaits}self._host.close({:?}, {sent})\n",
            member.operation.name
        ),
    }
}
