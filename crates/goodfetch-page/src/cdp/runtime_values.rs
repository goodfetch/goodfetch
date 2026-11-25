//! A page's console calls and uncaught exceptions as Playwright's
//! `ConsoleMessage` and `WebError`, from the browser's runtime values.

use goodfetch_cdp::generated::runtime;
use goodfetch_protocol::page::PageId;
use goodfetch_protocol::page::events::{ConsoleLocation, ConsoleMessage, WebError};

use crate::page_functions::error_message;

/// A console call as Playwright's `ConsoleMessage`.
pub(super) fn console_message(
    page_id: &PageId,
    called: &runtime::ConsoleAPICalledEvent,
) -> ConsoleMessage {
    let kind = serde_json::to_value(called.r#type)
        .ok()
        .and_then(|name| name.as_str().map(str::to_owned))
        .unwrap_or_else(|| "log".to_owned());
    let text = called
        .args
        .iter()
        .map(described)
        .collect::<Vec<_>>()
        .join(" ");
    let location = called
        .stack_trace
        .as_ref()
        .and_then(|stack| stack.call_frames.first())
        .map_or_else(
            || ConsoleLocation {
                url: String::new(),
                line_number: 0,
                column_number: 0,
            },
            |frame| ConsoleLocation {
                url: frame.url.clone(),
                line_number: u64::try_from(frame.line_number).unwrap_or_default(),
                column_number: u64::try_from(frame.column_number).unwrap_or_default(),
            },
        );
    ConsoleMessage {
        page_id: page_id.clone(),
        kind,
        text,
        location,
    }
}

/// An argument of a console call as its text: a primitive's value, any
/// other value as the browser describes it.
fn described(argument: &runtime::RemoteObject) -> String {
    match &argument.value {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(value) if !value.is_object() && !value.is_array() => value.to_string(),
        _ => argument
            .unserializable_value
            .clone()
            .or_else(|| argument.description.clone())
            .unwrap_or_else(|| {
                serde_json::to_value(argument.r#type)
                    .ok()
                    .and_then(|kind| kind.as_str().map(str::to_owned))
                    .unwrap_or_default()
            }),
    }
}

/// An uncaught exception as Playwright's `WebError`: an error's name and
/// message from its description's first line, a thrown value that is no
/// error as `Error` with its value as the message.
pub(super) fn web_error(page_id: &PageId, details: &runtime::ExceptionDetails) -> WebError {
    let thrown = details.exception.as_ref();
    let is_error =
        thrown.is_some_and(|thrown| thrown.subtype == Some(runtime::RemoteObjectSubtype::Error));
    let description = thrown.and_then(|thrown| thrown.description.clone());
    if is_error && let Some(description) = description {
        let name = thrown
            .and_then(|thrown| thrown.class_name.clone())
            .unwrap_or_else(|| "Error".to_owned());
        let message = error_message(Some(&name), &description);
        return WebError {
            page_id: page_id.clone(),
            name,
            message,
            stack: Some(description),
        };
    }
    WebError {
        page_id: page_id.clone(),
        name: "Error".to_owned(),
        message: thrown.map_or_else(|| details.text.clone(), described),
        stack: None,
    }
}
