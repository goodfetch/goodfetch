//! A page's fields found in one engine read (ADR 0147): `page.read` on a
//! browser page, and a code-free crawl's fields on the browser tier (ADR
//! 0123). [`READ`] finds each declared field's elements in the page's
//! document as a served document's tree finds them, and the extractor's
//! `fields` module reads their values.

use std::collections::BTreeMap;

use goodfetch_engine::CallError;
use goodfetch_extractor::fields::{FieldElement, Found};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::read::PageField;
use serde_json::{Value, json};

/// The page function that finds each declared field's elements, called
/// with [`argument`]'s; [`answered`] reads what it answers.
pub const READ: &str = include_str!("fields/read.js");

/// What [`READ`] is called with for `fields`: each, in the order of their
/// names, `{ name, kind, locator }` once per rung of its locator ladder
/// (issue 1972), `kind` being `one`, `list` or `rows`, and rows' `fields`
/// each `{ name, locator }` once per rung of its ladder, in the order of
/// their names: one find per rung, as the extractor's `Fields` reads them.
#[must_use]
pub fn argument(fields: &BTreeMap<String, PageField>) -> Value {
    let mut declared = Vec::with_capacity(fields.len());
    for (name, field) in fields {
        match field {
            PageField::Value(field) => declared.extend(
                field
                    .rungs()
                    .map(|rung| json!({ "name": name, "kind": "one", "locator": rung })),
            ),
            PageField::List(list) => declared.extend(
                list.list
                    .rungs()
                    .map(|rung| json!({ "name": name, "kind": "list", "locator": rung })),
            ),
            PageField::Rows(rows) => declared.push(json!({
                "name": name,
                "kind": "rows",
                "locator": rows.rows,
                "fields": rows
                    .fields
                    .iter()
                    .flat_map(|(name, field)| {
                        field
                            .rungs()
                            .map(move |rung| json!({ "name": name, "locator": rung }))
                    })
                    .collect::<Vec<_>>(),
            })),
        }
    }
    json!({ "fields": declared })
}

/// What a page answered [`READ`].
#[derive(Clone, Debug, PartialEq, Eq)]
enum Answer {
    /// What each field found, in the order of their names, in the document
    /// whose base URL is `base_url`.
    Found { base_url: String, found: Vec<Found> },
    /// The sentence refusing the first locator the page does not read, as
    /// a served document's refuses it.
    Invalid(String),
}

/// What each field a page's [`READ`] found, in the order of their names,
/// in the document whose base URL is `base_url`.
pub struct FoundFields {
    pub base_url: String,
    pub found: Vec<Found>,
}

/// What the page answered [`READ`] for `fields`, `answer`, as the call
/// `call` read them: `page.read`, or a code-free crawl's start.
///
/// # Errors
///
/// `invalid_params` naming the first locator the page does not read, as a
/// served document's refuses it; `internal_error` for an answer the page
/// never gives.
pub fn answered(
    answer: &Value,
    fields: &BTreeMap<String, PageField>,
    call: &str,
) -> Result<FoundFields, CallError> {
    match answer_of(answer, fields) {
        Some(Answer::Found { base_url, found }) => Ok(FoundFields { base_url, found }),
        Some(Answer::Invalid(sentence)) => Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            sentence,
        )),
        None => Err(CallError::protocol(
            ProtocolFailure::InternalError,
            format!("The page answered `{call}` with what it never answers."),
        )),
    }
}

/// What the page answered [`READ`] for `fields`, `answer`; `None` for an
/// answer it never gives.
fn answer_of(answer: &Value, fields: &BTreeMap<String, PageField>) -> Option<Answer> {
    let text = |value: &Value, name: &str| value.get(name)?.as_str().map(str::to_owned);
    if let Some(invalid) = answer.get("invalid") {
        let selector = text(invalid, "selector")?;
        let refused = match text(invalid, "kind")?.as_str() {
            "css" => format!("`{selector}` is no CSS selector"),
            "xpath" => format!(
                "`{selector}` is no XPath 1.0 expression that finds nodes: {message}",
                message = text(invalid, "message")?.trim_end_matches('.')
            ),
            _ => return None,
        };
        let field = text(invalid, "field")?;
        return Some(Answer::Invalid(format!("The field `{field}`'s {refused}.")));
    }
    let mut answers = answer.get("found")?.as_array()?.iter();
    let mut found = Vec::with_capacity(fields.len());
    for field in fields.values() {
        match field {
            PageField::Value(field) => {
                for _ in field.rungs() {
                    found.push(Found::One(element(answers.next()?).ok()?));
                }
            }
            PageField::List(list) => {
                for _ in list.list.rungs() {
                    found.push(Found::List(elements(answers.next()?)?));
                }
            }
            PageField::Rows(_) => found.push(Found::Rows(
                answers
                    .next()?
                    .as_array()?
                    .iter()
                    .map(|row| {
                        row.as_array()?
                            .iter()
                            .map(|each| element(each).ok())
                            .collect()
                    })
                    .collect::<Option<_>>()?,
            )),
        }
    }
    if answers.next().is_some() {
        return None;
    }
    Some(Answer::Found {
        base_url: text(answer, "baseUrl")?,
        found,
    })
}

/// Every element of `value`, a list's answer.
fn elements(value: &Value) -> Option<Vec<FieldElement>> {
    value
        .as_array()?
        .iter()
        .map(|each| element(each).ok()?)
        .collect()
}

/// An element's answer that is not the shape `READ` answers with.
struct Malformed;

/// The element `value` is, `null` being none.
fn element(value: &Value) -> Result<Option<FieldElement>, Malformed> {
    if value.is_null() {
        return Ok(None);
    }
    let text = |name: &str| value.get(name)?.as_str().map(str::to_owned);
    let read = || {
        Some(FieldElement {
            tag: text("tag")?,
            text: text("text")?,
            attributes: value
                .get("attributes")?
                .as_array()?
                .iter()
                .map(|pair| {
                    let at = |index: usize| pair.get(index)?.as_str().map(str::to_owned);
                    Some((at(0)?, at(1)?))
                })
                .collect::<Option<_>>()?,
        })
    };
    read().map(Some).ok_or(Malformed)
}

#[cfg(test)]
mod tests;
