//! A page's JavaScript dialogs (ADR 0085): an `alert`, `confirm`, `prompt`
//! or `beforeunload` dialog is answered under the routes' rules (ADR
//! 0081). `page.on_dialog` is a subscription, a call that stays in flight
//! until its Client cancels it or its handler fails, armed when its request
//! arrives. For each dialog its page opens, the engine sends the
//! Connection the handler request `dialog.handle`, naming the dialog by a
//! `dialog_id`, to the latest handler first; the handler answers it with
//! `dialog.accept` or `dialog.dismiss` and replies to `dialog.handle` once
//! it ends. The first answer wins; a handler that ends without one falls
//! through to the handler armed before, and past the last one, or with
//! none, the dialog gets Playwright's default: an `alert`, `confirm` or
//! `prompt` is dismissed, since accepting a `confirm()` may confirm an
//! Effect (ADR 0014), and a `beforeunload` dialog is accepted, so the
//! navigation that raised it goes on. A handler that fails (its reply is an
//! error) gives the dialog its default unless it answered already, its
//! handler is removed, its subscription ends with `page_handler_failed`,
//! and the call the dialog blocked, or else the next call on its page,
//! raises it. A second answer, or one after the dialog closed, raises
//! `dialog_already_handled`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ARRIVED_IN, PageId};
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, HandlerRequest, Operation, OperationClass, OperationKind,
    Reach, Stability, schema_of,
};
use crate::jsonrpc::RequestId;

/// What `page.on_dialog` takes: the page whose dialogs its handler
/// answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageOnDialogParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
}

/// What `page.on_dialog` answers when its page closes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DialogsEnded {}

/// One dialog a handler answers, which `dialog.*` names.
#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct DialogId(pub String);

/// A dialog's kind, as Playwright's `dialog.type()` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DialogType {
    /// `alert()`.
    Alert,
    /// `confirm()`.
    Confirm,
    /// `prompt()`.
    Prompt,
    /// A `beforeunload` handler that asks to stay: raised by a navigation,
    /// or by `page.close` with `run_before_unload`.
    #[serde(rename = "beforeunload")]
    BeforeUnload,
}

/// A dialog as its handler sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Dialog {
    /// Its kind.
    #[serde(rename = "type")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: DialogType,
    /// Its message.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// A prompt's default value; empty for any other kind.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub default_value: String,
}

/// What `dialog.handle` gives the handler: the dialog, named by the ID its
/// answer passes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DialogHandleParams {
    /// The id of the `page.on_dialog` call whose handler is asked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// The dialog, which an answer names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dialog_id: DialogId,
    /// The page that opened it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The dialog.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dialog: Dialog,
}

/// What the handler replies to `dialog.handle` once it ended; a failure
/// reply says it failed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DialogHandled {}

/// What `dialog.accept` takes: the dialog, and a prompt's text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DialogAcceptParams {
    /// The dialog.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dialog_id: DialogId,
    /// The text a prompt returns; its default value when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub prompt_text: Option<String>,
}

/// What `dialog.dismiss` takes: the dialog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DialogDismissParams {
    /// The dialog.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dialog_id: DialogId,
}

/// What an answer to a dialog answers once the page closed the dialog.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DialogAnswered {}

/// What `dialog_already_handled` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DialogAlreadyHandledDetails {
    /// The dialog answered already.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dialog_id: DialogId,
}

/// A dialog answered twice (ADR 0085).
pub const DIALOG_ALREADY_HANDLED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "dialog_already_handled",
    description: "The dialog was answered already, by an earlier `accept` or `dismiss`, or it closed, its handler having ended or the dialog having got its default: the first answer stands and this one changed nothing.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `page.on_dialog`.
pub const PAGE_ON_DIALOG: Operation = Operation {
    name: "page.on_dialog",
    description: "Hands the page's JavaScript dialogs to the program, armed when the call arrives: Playwright's `page.on('dialog')`. The call stays in flight, and for each dialog the engine sends `dialog.handle`, which the handler answers with `dialog.accept` or `dialog.dismiss`. Later handlers come first and the first answer wins; a handler that ends without one falls through, and past the last one, or with none, an `alert`, `confirm` or `prompt` is dismissed and a `beforeunload` dialog accepted, so a handler that answers nothing never stalls the page. Cancelling the call removes the handler; a handler that fails ends it with `page_handler_failed`, the dialog getting its default.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::Sequence,
    reach: Reach::SdkOnly,
    params: schema_of::<PageOnDialogParams>,
    result: schema_of::<DialogsEnded>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `dialog.accept`.
pub const DIALOG_ACCEPT: Operation = Operation {
    name: "dialog.accept",
    description: "Answers a dialog by accepting it, a prompt with `prompt_text`, or else its default value; it answers once the page closed the dialog.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<DialogAcceptParams>,
    result: schema_of::<DialogAnswered>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `dialog.dismiss`.
pub const DIALOG_DISMISS: Operation = Operation {
    name: "dialog.dismiss",
    description: "Answers a dialog by dismissing it, as its Cancel button does; it answers once the page closed the dialog.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<DialogDismissParams>,
    result: schema_of::<DialogAnswered>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `dialog.handle`.
pub const DIALOG_HANDLE: HandlerRequest = HandlerRequest {
    name: "dialog.handle",
    description: "Asks a dialog handler to answer one dialog its page opened, named by its `dialog_id`, with `dialog.accept` or `dialog.dismiss`. The handler replies once it ends: a dialog it did not answer falls through to the handler before, and past the last one gets its default. A failure reply gives the dialog its default unless it was answered, removes the handler and raises `page_handler_failed`.",
    params: schema_of::<DialogHandleParams>,
    result: schema_of::<DialogHandled>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
