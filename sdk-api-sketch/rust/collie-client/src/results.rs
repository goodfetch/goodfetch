//! Results, outcomes and errors (ADR 0079, decided in #466).
//!
//! Every call returns `Result<T, Error>`: a failure is an [`Error`], an expected outcome lives in
//! `T`, and no option changes which (G1, S2). The codes below are a representative subset: each
//! area's spec lists its codes, their groups and their details in `goodfetch-protocol`, whose own
//! structs [`ErrorDetails`] holds.

use std::fmt;

/// A group of Error codes sharing one remedy, carried on the wire as the JSON-RPC numeric class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorGroup {
    /// The program is wrong: its call, its settings or its own handler code. Fix the program (G3).
    Program,
    /// A policy, a Limit, a lock or a person refused. Change a setting, raise the Limit or ask the operator.
    Refused,
    /// The page or site never reached the state the call needed. Look at the Run trace.
    Page,
    /// The Model provider's answer was unusable. Change the model or the strategy.
    Model,
    /// The deadline passed. Raise it, or wait for the right thing; deadlines are #467's.
    DeadlineExceeded,
    /// Something Goodfetch depends on is unreachable or gone. Usually retryable.
    Unavailable,
    /// A bug in Goodfetch. Report it.
    Internal,
}

/// Writes an open value type (ADR 0099): a newtype holding its wire string inline, at most 63
/// bytes, so a value read from the wire equals its constant with `==` and in a `match`, whichever
/// crate defines the constant. A value the SDK does not know stays a value, with `as_str()` (G3).
macro_rules! open_value_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name {
            len: u8,
            bytes: [u8; 63],
        }

        impl $name {
            /// Builds a value from its wire string: the generated constants of each Product's
            /// crate use it.
            #[doc(hidden)]
            pub const fn new(value: &'static str) -> Self {
                let source = value.as_bytes();
                assert!(source.len() <= 63, "an open value is at most 63 bytes");
                let mut bytes = [0u8; 63];
                let mut i = 0;
                while i < source.len() {
                    bytes[i] = source[i];
                    i += 1;
                }
                Self {
                    len: source.len() as u8,
                    bytes,
                }
            }

            /// The value's wire string, the same on every surface.
            pub fn as_str(&self) -> &str {
                let _ = (self.len, self.bytes);
                unimplemented!()
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}
pub(crate) use open_value_type;

open_value_type! {
    /// An Error code (ADRs 0079, 0099), an open value type. Collie's codes are associated constants;
    /// Foxhound's are free constants of `goodfetch_foxhound::error_code`, equal to a code read from
    /// the wire with `==` and in a `match`, as `http::StatusCode`'s constants are matched.
    ErrorCode
}

impl ErrorCode {
    /// `closed`: a call on a closed value (ADR 0078).
    pub const CLOSED: ErrorCode = ErrorCode::new("closed");
    /// `removed`: a removed name was called, and `fix` names its replacement (#467).
    pub const REMOVED: ErrorCode = ErrorCode::new("removed");
    /// `listener_failed`: a listener threw (ADR 0081). Only the TypeScript and Python SDKs raise it:
    /// Rust has no listeners, since its events are streams the program reads.
    pub const LISTENER_FAILED: ErrorCode = ErrorCode::new("listener_failed");
    /// `proxy_handler_failed`: the program's Proxy provider failed with its own error (ADR 0104).
    pub const PROXY_HANDLER_FAILED: ErrorCode = ErrorCode::new("proxy_handler_failed");
    /// `limit_reached`.
    pub const LIMIT_REACHED: ErrorCode = ErrorCode::new("limit_reached");
    /// `page_check_failed` (ADR 0006).
    pub const PAGE_CHECK_FAILED: ErrorCode = ErrorCode::new("page_check_failed");
    /// `target_not_unique`.
    pub const TARGET_NOT_UNIQUE: ErrorCode = ErrorCode::new("target_not_unique");
    /// `blocked`, with a [`BlockCause`].
    pub const BLOCKED: ErrorCode = ErrorCode::new("blocked");
    /// `not_verified`, from [`AiWorkResult::require_verified`].
    pub const NOT_VERIFIED: ErrorCode = ErrorCode::new("not_verified");
    /// `deadline_exceeded`: every timeout, whatever the phase (ADR 0080), except a target still
    /// ambiguous at its deadline, which is `target_not_unique` (ADR 0085).
    pub const DEADLINE_EXCEEDED: ErrorCode = ErrorCode::new("deadline_exceeded");
    /// `call_cancelled`: someone else stopped the call in flight, with a [`CallCancelled`] saying
    /// who and what closed, such as a page closed in the browser during a Human pause (ADR 0113);
    /// a dropped future of the caller's own never sees it (ADR 0080).
    pub const CALL_CANCELLED: ErrorCode = ErrorCode::new("call_cancelled");
    /// `plugin_crashed` (ADR 0033).
    pub const PLUGIN_CRASHED: ErrorCode = ErrorCode::new("plugin_crashed");
    /// `setting_unknown`: an unknown key or `GOODFETCH_` variable (ADR 0083).
    pub const SETTING_UNKNOWN: ErrorCode = ErrorCode::new("setting_unknown");
    /// `setting_invalid`: a wrong type or out-of-range value (ADR 0083).
    pub const SETTING_INVALID: ErrorCode = ErrorCode::new("setting_invalid");
    /// `setting_not_applicable`: a setting no scope beneath can use (ADR 0083).
    pub const SETTING_NOT_APPLICABLE: ErrorCode = ErrorCode::new("setting_not_applicable");
    /// `setting_locked`, a refusal: a lower layer locked or capped the value (ADR 0083).
    pub const SETTING_LOCKED: ErrorCode = ErrorCode::new("setting_locked");
    /// `capability_unavailable`: the platform cannot provide a capability (ADR 0083).
    pub const CAPABILITY_UNAVAILABLE: ErrorCode = ErrorCode::new("capability_unavailable");
    /// `selector_invalid`: a selector that does not parse, or a Playwright-only engine, `>>` or a
    /// Playwright CSS extension such as `:has-text()` (ADR 0085).
    pub const SELECTOR_INVALID: ErrorCode = ErrorCode::new("selector_invalid");
    /// `target_unsupported`: the target is of the wrong kind for the action (ADR 0085).
    pub const TARGET_UNSUPPORTED: ErrorCode = ErrorCode::new("target_unsupported");
    /// `page_function_failed`: the page function threw (ADR 0085).
    pub const PAGE_FUNCTION_FAILED: ErrorCode = ErrorCode::new("page_function_failed");
    /// `page_value_invalid`: a value crossing to or from the page is not JSON (ADR 0085).
    pub const PAGE_VALUE_INVALID: ErrorCode = ErrorCode::new("page_value_invalid");
    /// `page_handler_failed`: a route, dialog or locator handler threw (ADR 0085). Only the
    /// TypeScript and Python SDKs raise it: Rust answers those requests in the program's own code.
    pub const PAGE_HANDLER_FAILED: ErrorCode = ErrorCode::new("page_handler_failed");
    /// `route_already_handled` (ADR 0085). Only the TypeScript and Python SDKs raise it: a Rust
    /// route's answers take the route.
    pub const ROUTE_ALREADY_HANDLED: ErrorCode = ErrorCode::new("route_already_handled");
    /// `navigation_interrupted`: another navigation superseded this one (ADR 0085).
    pub const NAVIGATION_INTERRUPTED: ErrorCode = ErrorCode::new("navigation_interrupted");
    /// `response_not_json` (ADR 0085).
    pub const RESPONSE_NOT_JSON: ErrorCode = ErrorCode::new("response_not_json");
    /// `navigation_failed`: the site could not be reached (ADR 0085).
    pub const NAVIGATION_FAILED: ErrorCode = ErrorCode::new("navigation_failed");
    /// `page_crashed` (ADR 0085).
    pub const PAGE_CRASHED: ErrorCode = ErrorCode::new("page_crashed");
    /// `body_unavailable`: the browser no longer holds the body (ADR 0085).
    pub const BODY_UNAVAILABLE: ErrorCode = ErrorCode::new("body_unavailable");
    /// `download_failed` (ADR 0085).
    pub const DOWNLOAD_FAILED: ErrorCode = ErrorCode::new("download_failed");
    /// `cdp_command_refused`, a refusal (ADR 0085).
    pub const CDP_COMMAND_REFUSED: ErrorCode = ErrorCode::new("cdp_command_refused");
    /// `session_reserved`: a second `page.lend()` on a Session while one attach is open, or a lend
    /// of a Session a handler's attach reserves (ADR 0089).
    pub const SESSION_RESERVED: ErrorCode = ErrorCode::new("session_reserved");
    /// `borrow_requires_cdp`: a borrow of a browser that exposes no CDP, such as Firefox or WebKit
    /// (ADRs 0056, 0089).
    pub const BORROW_REQUIRES_CDP: ErrorCode = ErrorCode::new("borrow_requires_cdp");
    /// `browser_borrowed`, a refusal: another Run already borrows this browser (ADR 0056).
    pub const BROWSER_BORROWED: ErrorCode = ErrorCode::new("browser_borrowed");
    /// `attach_ended`: a Lent page's attach was cut (ADRs 0003, 0089).
    pub const ATTACH_ENDED: ErrorCode = ErrorCode::new("attach_ended");
    /// `value_invalid`: a Key-value store value that is not JSON, fails its schema or holds bytes
    /// (ADR 0103).
    pub const VALUE_INVALID: ErrorCode = ErrorCode::new("value_invalid");
    /// `secret_unbound`: no scope binds the Secret placeholder (ADR 0092).
    pub const SECRET_UNBOUND: ErrorCode = ErrorCode::new("secret_unbound");
    /// `secret_unavailable`: the Secret provider gave no value (ADR 0092).
    pub const SECRET_UNAVAILABLE: ErrorCode = ErrorCode::new("secret_unavailable");
    /// `secret_domain_not_allowed`, a refusal (ADR 0092).
    pub const SECRET_DOMAIN_NOT_ALLOWED: ErrorCode = ErrorCode::new("secret_domain_not_allowed");
    /// `secret_input_refused`, a refusal: other code may read this browser (ADR 0092).
    pub const SECRET_INPUT_REFUSED: ErrorCode = ErrorCode::new("secret_input_refused");
    /// `logged_out`: recovery did not bring the login back (ADR 0092).
    pub const LOGGED_OUT: ErrorCode = ErrorCode::new("logged_out");
    /// `human_pause_declined`, a refusal by a person (ADR 0092).
    pub const HUMAN_PAUSE_DECLINED: ErrorCode = ErrorCode::new("human_pause_declined");
    /// `session_not_savable`: the Session's cookies live outside run state (ADR 0092).
    pub const SESSION_NOT_SAVABLE: ErrorCode = ErrorCode::new("session_not_savable");
    /// `target_not_found`: `act` found no element for its instruction, or `get_by_ref` named an
    /// element that is gone (ADR 0091).
    pub const TARGET_NOT_FOUND: ErrorCode = ErrorCode::new("target_not_found");
    /// `plan_drift`: a step drifted and could not be repaired (ADR 0091).
    pub const PLAN_DRIFT: ErrorCode = ErrorCode::new("plan_drift");
    /// `plan_step_missing`: `plan.frozen` is set and the Plan holds no entry for the call (ADR 0091).
    pub const PLAN_STEP_MISSING: ErrorCode = ErrorCode::new("plan_step_missing");
    /// `plan_invalid`: the Plan file cannot be read or does not match its format (ADR 0091).
    pub const PLAN_INVALID: ErrorCode = ErrorCode::new("plan_invalid");
    /// `model_not_configured`: a call the Plan holds no entry for needs a model, and none is configured (ADR 0091).
    pub const MODEL_NOT_CONFIGURED: ErrorCode = ErrorCode::new("model_not_configured");
    /// `extraction_incomplete`: required fields were not found on the page (ADR 0091).
    pub const EXTRACTION_INCOMPLETE: ErrorCode = ErrorCode::new("extraction_incomplete");
    /// `extraction_invalid`, a model error: the model's answers never passed the checks within the retries (ADR 0091).
    pub const EXTRACTION_INVALID: ErrorCode = ErrorCode::new("extraction_invalid");
    /// `effect_uncertain`: an Effect may have happened and Goodfetch cannot tell: an `act` whose
    /// action was an Effect failed after the action was sent (ADR 0091), or a declared Effect's body
    /// ended after a send (ADR 0106); never retried, a person reconciles it.
    pub const EFFECT_UNCERTAIN: ErrorCode = ErrorCode::new("effect_uncertain");
    /// `effect_key_invalid`: an empty key, or one longer than 255 code points (ADR 0106).
    pub const EFFECT_KEY_INVALID: ErrorCode = ErrorCode::new("effect_key_invalid");
    /// `effect_key_reused`: a key declared again while its body runs, or after this attempt already
    /// got its answer (ADR 0106).
    pub const EFFECT_KEY_REUSED: ErrorCode = ErrorCode::new("effect_key_reused");
    /// `effect_output_invalid`: a body's value is not JSON or is too large (ADR 0106).
    pub const EFFECT_OUTPUT_INVALID: ErrorCode = ErrorCode::new("effect_output_invalid");
    /// `effect_staging_refused`: an output staged with an attempt while a declared Effect's body ran
    /// in it (ADR 0106).
    pub const EFFECT_STAGING_REFUSED: ErrorCode = ErrorCode::new("effect_staging_refused");
    /// `effect_not_confirmed`, a refusal: an AI Effect's confirmation was declined or its Human pause
    /// expired (ADR 0091).
    pub const EFFECT_NOT_CONFIRMED: ErrorCode = ErrorCode::new("effect_not_confirmed");
    /// `success_check_failed`: `expect`'s condition was still false at its timeout (ADR 0091).
    pub const SUCCESS_CHECK_FAILED: ErrorCode = ErrorCode::new("success_check_failed");
    /// `variables_mismatch`: a placeholder nothing fills, a variable no placeholder uses, or a name
    /// that is both a variable and a Secret placeholder (ADR 0091).
    pub const VARIABLES_MISMATCH: ErrorCode = ErrorCode::new("variables_mismatch");
    /// `run_id_invalid`: an ID the engine refuses (ADR 0102).
    pub const RUN_ID_INVALID: ErrorCode = ErrorCode::new("run_id_invalid");
    /// `run_conflict`: the ID names a Run started with other inputs, or by another caller.
    pub const RUN_CONFLICT: ErrorCode = ErrorCode::new("run_conflict");
    /// `run_ended`: a call that would give an ended Run new work.
    pub const RUN_ENDED: ErrorCode = ErrorCode::new("run_ended");
    /// `run_in_use`, a refusal: the Run is live in another program, or already open in this one.
    pub const RUN_IN_USE: ErrorCode = ErrorCode::new("run_in_use");
    /// `limit_raise_refused`, a refusal: a resume gave a Limit above the one the Run began with.
    pub const LIMIT_RAISE_REFUSED: ErrorCode = ErrorCode::new("limit_raise_refused");
    /// `run_state_unavailable`: the Run index lists the Run, but its state is missing or unreadable.
    pub const RUN_STATE_UNAVAILABLE: ErrorCode = ErrorCode::new("run_state_unavailable");
    /// `run_not_found`: the ID names no Run in the Run index.
    pub const RUN_NOT_FOUND: ErrorCode = ErrorCode::new("run_not_found");
    /// `run_interrupted`: a view's `report()` on a Run with no live writer that has not ended.
    pub const RUN_INTERRUPTED: ErrorCode = ErrorCode::new("run_interrupted");
    /// `model_not_found`: the provider says the model does not exist or is not available to this
    /// key (ADR 0101).
    pub const MODEL_NOT_FOUND: ErrorCode = ErrorCode::new("model_not_found");
    /// `model_capability_missing`: the operation needs a capability the model does not declare.
    pub const MODEL_CAPABILITY_MISSING: ErrorCode = ErrorCode::new("model_capability_missing");
    /// `model_handler_failed`: the program's own Model provider failed with an error that is not a
    /// Goodfetch [`Error`].
    pub const MODEL_HANDLER_FAILED: ErrorCode = ErrorCode::new("model_handler_failed");
    /// `model_unauthorized`, a refusal: the provider rejected the key. Not retryable.
    pub const MODEL_UNAUTHORIZED: ErrorCode = ErrorCode::new("model_unauthorized");
    /// `model_funds_exhausted`, a refusal: the provider account has no funds left, or a cap set at
    /// the provider (a spend limit, a tier's monthly cap) is reached. Not retryable.
    pub const MODEL_FUNDS_EXHAUSTED: ErrorCode = ErrorCode::new("model_funds_exhausted");
    /// `model_price_unknown`, a refusal: a currency Limit applies and the model's price cannot be
    /// stated in its currency (no price, or no fixed rate).
    pub const MODEL_PRICE_UNKNOWN: ErrorCode = ErrorCode::new("model_price_unknown");
    /// `model_allowance_reached`, a refusal: the Run's Model allowance is reached, so its model calls
    /// are refused while the Run goes on (ADR 0107). Not retryable within the Run.
    pub const MODEL_ALLOWANCE_REACHED: ErrorCode = ErrorCode::new("model_allowance_reached");
    /// `model_unavailable`: unreachable, a 5xx, or rate limited after the engine's retries. Retryable.
    pub const MODEL_UNAVAILABLE: ErrorCode = ErrorCode::new("model_unavailable");
    /// `model_declined`, a model error: the model declined to answer.
    pub const MODEL_DECLINED: ErrorCode = ErrorCode::new("model_declined");
    /// `model_context_exceeded`, a model error: the request does not fit the context window.
    pub const MODEL_CONTEXT_EXCEEDED: ErrorCode = ErrorCode::new("model_context_exceeded");
    /// `model_output_invalid`, a model error: an `act`, `observe` or Agent step's answer never parsed.
    pub const MODEL_OUTPUT_INVALID: ErrorCode = ErrorCode::new("model_output_invalid");
    /// `model_request_rejected`: the provider rejected the request for another reason (a 400 or
    /// 422), such as a forced strategy.
    pub const MODEL_REQUEST_REJECTED: ErrorCode = ErrorCode::new("model_request_rejected");
    /// `agent_tool_failed`, a program error: an Agent tool returned an error, and the task ended
    /// (ADR 0100).
    pub const AGENT_TOOL_FAILED: ErrorCode = ErrorCode::new("agent_tool_failed");
    /// `agent_tool_invalid`, a program error: two Agent tools share a name, or one takes a built-in
    /// action's (ADR 0100).
    pub const AGENT_TOOL_INVALID: ErrorCode = ErrorCode::new("agent_tool_invalid");
    /// `success_check_threw`, a program error: the Agent's Success check returned an error that is
    /// not a Goodfetch one, kept as the `source()`: a bug, never a rejection (ADR 0100).
    pub const SUCCESS_CHECK_THREW: ErrorCode = ErrorCode::new("success_check_threw");
    /// `response_not_recorded`, a refusal: a replaying Run's recording lacks the request, which was
    /// never sent; not retryable (ADR 0105).
    pub const RESPONSE_NOT_RECORDED: ErrorCode = ErrorCode::new("response_not_recorded");
    /// `recording_invalid`: the Response recording cannot be read or is not WARC (ADR 0105).
    pub const RECORDING_INVALID: ErrorCode = ErrorCode::new("recording_invalid");
    /// `scripted_model_exhausted`: a call went beyond a Scripted model's replies (ADR 0105).
    pub const SCRIPTED_MODEL_EXHAUSTED: ErrorCode = ErrorCode::new("scripted_model_exhausted");
    /// `scripted_model_unused`: a test engine closed with a Scripted model's replies left (ADR 0105).
    pub const SCRIPTED_MODEL_UNUSED: ErrorCode = ErrorCode::new("scripted_model_unused");
    /// `internal`.
    pub const INTERNAL: ErrorCode = ErrorCode::new("internal");
}



/// The settings layers of ADR 0028, lowest first. A program that is not a Recipe sets the `Run`
/// layer (ADR 0083).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettingsLayer {
    /// Built-in defaults.
    Default,
    /// An Operator policy.
    OperatorPolicy,
    /// User and project configuration.
    Configuration,
    /// A Recipe's declared settings and code.
    Recipe,
    /// The run itself.
    Run,
}

/// What set a value at the `Run` layer; a call's argument beats a `Settings` value. The
/// environment sets no setting (ADR 0083).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunLayerOrigin {
    /// A call's own argument.
    Argument,
    /// A [`Settings`](crate::settings::Settings) value.
    Settings,
    /// A CLI flag.
    Flag,
    /// An Integration's form.
    Form,
    /// The lowest: the only SDK handler of a kind that one setting chooses (ADR 0104).
    Registered,
}

/// An effective setting's value as the engine reports it.
#[derive(Debug, Clone)]
pub struct SettingValue {
    _private: (),
}

impl SettingValue {
    /// The value as JSON, as configuration files and `config explain` show it.
    pub fn as_json(&self) -> &serde_json::Value {
        unimplemented!()
    }
}

/// Where the limit that ended a call came from (G6).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LimitSource {
    /// The setting's canonical key, such as `timeout.action`.
    pub setting: String,
    /// Its value.
    pub value: SettingValue,
    /// The layer that set the value.
    pub layer: SettingsLayer,
    /// Set when `layer` is [`SettingsLayer::Run`]: a call's own argument, a `Settings` value, a
    /// CLI flag or a form (ADRs 0080, 0083).
    pub origin: Option<RunLayerOrigin>,
}

/// What a call acted on, rendered by the engine in Rust and kept structured too.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ErrorTarget {
    /// As the program wrote it, in Rust; the locator API is #472's.
    pub rendered: String,
    /// The target as data; its shape is #472's.
    pub structured: TargetDescription,
}

/// A target as data; its shape is #472's.
#[derive(Debug, Clone)]
pub struct TargetDescription {
    _private: (),
}

/// Every Goodfetch failure: one opaque type with accessors. It is `Send + Sync + 'static`.
#[derive(Debug)]
pub struct Error {
    _private: (),
}

impl Error {
    /// Builds an error for a program's own tests (fakes and doubles); the SDK never returns one.
    #[cfg(feature = "test-util")]
    pub fn fake(code: ErrorCode, message: impl Into<String>) -> Self {
        let _ = (code, message.into());
        unimplemented!()
    }

    /// The Error code.
    pub fn code(&self) -> ErrorCode {
        unimplemented!()
    }

    /// The code's group, known even for a code this SDK does not know.
    pub fn group(&self) -> ErrorGroup {
        unimplemented!()
    }

    /// Whether trying again is safe and may help. The SDK never retries on its own.
    pub fn retryable(&self) -> bool {
        unimplemented!()
    }

    /// The message, the same as in the other SDKs.
    pub fn message(&self) -> &str {
        unimplemented!()
    }

    /// The SDK call, such as `locator.click`.
    pub fn operation(&self) -> &str {
        unimplemented!()
    }

    /// Finds the call in the Run trace.
    pub fn call_id(&self) -> &str {
        unimplemented!()
    }

    /// The Run the call belongs to, if any.
    pub fn run_id(&self) -> Option<&str> {
        unimplemented!()
    }

    /// What the call acted on.
    pub fn target(&self) -> Option<&ErrorTarget> {
        unimplemented!()
    }

    /// The page's URL.
    pub fn url(&self) -> Option<&str> {
        unimplemented!()
    }

    /// What was there instead.
    pub fn observed(&self) -> Option<&str> {
        unimplemented!()
    }

    /// Where the limit came from.
    pub fn limit(&self) -> Option<&LimitSource> {
        unimplemented!()
    }

    /// What would change the outcome, naming setting keys or operations.
    pub fn fix(&self) -> Option<&str> {
        unimplemented!()
    }

    /// The steps the engine took for this call. Never holds a Secret's value (ADR 0015).
    pub fn call_log(&self) -> &[String] {
        unimplemented!()
    }

    /// The code's typed details, read as `D`, the code's details type (ADR 0099):
    /// `error.details::<TargetNotUnique>()`. `None` when the error carries no details of that type.
    pub fn details<D: ErrorDetails>(&self) -> Option<D> {
        unimplemented!()
    }

    /// What the call's model calls consumed before it failed, when it made any: a failed call's
    /// spend is never hidden (ADR 0101).
    pub fn usage(&self) -> Option<&crate::models::ModelUsage> {
        unimplemented!()
    }

    /// On an error `page.agent` fails with after its task took steps: the steps and the partial
    /// answer (ADR 0100).
    pub fn agent_task(&self) -> Option<&crate::agent::AgentTaskProgress> {
        unimplemented!()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        unimplemented!()
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        unimplemented!()
    }
}

/// The error the program's own code returns to the SDK (ADR 0109): a crawl handler, a crawl Hook, a
/// login step, an Agent tool, the Agent's Success check, an SDK handler's `fail`, and the
/// `finish` of a call answered by the way down. It is std's boxed error, tower's and
/// `lambda_runtime`'s `BoxError`, so `?` converts every `std::error::Error + Send + Sync + 'static`,
/// a `&str` or `String`, a `Box<dyn Error + Send + Sync>` and an `anyhow::Error` or
/// `eyre::Report` into it, and the SDK's own [`Error`] too.
///
/// The SDK reads it as TypeScript and Python read an exception (S1). A box that holds the SDK's
/// own [`Error`] keeps its code and retryable flag (`ctx.skip()`'s and `ctx.retry()`'s included).
/// Any other error, one that wraps a Goodfetch error in its `source()` chain included, is the
/// program's own: `handler_failed`, `hook_failed` with no wrapped code, `agent_tool_failed`,
/// `success_check_threw`, the kind's `<kind>_handler_failed`, or the cause of `logged_out`. Its
/// details' message is its `Display` followed by each `source()`, joined by `": "`; its type, which
/// the box erases, reads `BoxError`; it carries no stack; and in the process it is the
/// failure's `source()`.
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// The typed details of an Error code, read with [`Error::details`] (ADR 0099): each code's details
/// type implements it, Foxhound's in Foxhound's crate, so no enum lists every code's details.
pub trait ErrorDetails: Clone + 'static {}

impl ErrorDetails for crate::plugins::SdkHandlerFailed {}
impl ErrorDetails for TargetNotUnique {}
impl ErrorDetails for Blocked {}
impl ErrorDetails for NotVerified {}
impl ErrorDetails for crate::models::LimitReached {}
impl ErrorDetails for DeadlineExceeded {}
impl ErrorDetails for CallCancelled {}
impl ErrorDetails for crate::settings::SettingUnknown {}
impl ErrorDetails for crate::settings::SettingInvalid {}
impl ErrorDetails for crate::settings::SettingNotApplicable {}
impl ErrorDetails for crate::settings::SettingLocked {}
impl ErrorDetails for crate::settings::CapabilityUnavailable {}
impl ErrorDetails for crate::sessions::SecretUnbound {}
impl ErrorDetails for crate::sessions::SecretUnavailable {}
impl ErrorDetails for crate::sessions::SecretDomainNotAllowed {}
impl ErrorDetails for crate::sessions::SecretInputRefused {}
impl ErrorDetails for crate::sessions::LoggedOut {}
impl ErrorDetails for crate::sessions::HumanPauseDeclined {}
impl ErrorDetails for crate::sessions::SessionNotSavable {}
impl ErrorDetails for crate::ai::TargetNotFound {}
impl ErrorDetails for crate::ai::PlanDrift {}
impl ErrorDetails for crate::ai::ExtractionIncomplete {}
impl ErrorDetails for crate::ai::ExtractionInvalid {}
impl ErrorDetails for crate::ai::SuccessCheckFailed {}
impl ErrorDetails for crate::ai::VariablesMismatch {}
impl ErrorDetails for crate::ai::PlanStepMissing {}
impl ErrorDetails for crate::ai::PlanInvalid {}
impl ErrorDetails for crate::ai::ModelNotConfigured {}
impl ErrorDetails for crate::ai::EffectUncertain {}
impl ErrorDetails for crate::effects::EffectKeyInvalid {}
impl ErrorDetails for crate::effects::EffectKeyReused {}
impl ErrorDetails for crate::effects::EffectOutputInvalid {}
impl ErrorDetails for crate::effects::EffectStagingRefused {}
impl ErrorDetails for crate::ai::EffectNotConfirmed {}
impl ErrorDetails for crate::models::ModelFailure {}
impl ErrorDetails for crate::models::ModelDeclined {}
impl ErrorDetails for crate::models::ModelContextExceeded {}
impl ErrorDetails for crate::models::ModelRequestRejected {}
impl ErrorDetails for crate::models::ModelPriceUnknown {}
impl ErrorDetails for crate::models::ModelAllowanceReached {}
impl ErrorDetails for crate::models::ModelCapabilityMissing {}
impl ErrorDetails for crate::models::ModelHandlerFailed {}
impl ErrorDetails for crate::models::ModelUnavailable {}
impl ErrorDetails for crate::models::ModelOutputInvalid {}
impl ErrorDetails for crate::output::ValueInvalid {}
impl ErrorDetails for crate::run::RunFailure {}
impl ErrorDetails for crate::agent::AgentToolFailed {}
impl ErrorDetails for crate::agent::AgentToolInvalid {}
impl ErrorDetails for crate::agent::SuccessCheckThrew {}
impl ErrorDetails for crate::recording::ResponseNotRecorded {}
impl ErrorDetails for crate::recording::RecordingInvalid {}
impl ErrorDetails for ScriptedModelDetails {}

/// Details of `deadline_exceeded`. The error's `limit` names the phase's setting and its layer; a
/// call's own timeout is that setting at the `run` layer (ADR 0028).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DeadlineExceeded {
    /// The timeout that ran out.
    pub timeout: std::time::Duration,
    /// How long the call ran.
    pub elapsed: std::time::Duration,
    /// What the call waited for, rendered in Rust.
    pub waited_for: String,
    /// The enclosing call whose timeout ended this one, when it was not the call's own.
    pub ended_by: Option<EndedBy>,
}

/// Details of `call_cancelled` (ADRs 0080, 0113): who stopped the call in flight and what closed
/// or stopped under it, so a program branches on a closed page without reading the message (G6).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CallCancelled {
    /// Who stopped it.
    pub by: CancelledBy,
    /// What closed or stopped under the call.
    pub closed: ClosedValue,
    /// Whom the stopping Client acts for (ADR 0031), when known.
    pub person: Option<String>,
}

/// Who stopped a call in flight, the vocabulary of an Agent's `abort` (ADR 0100).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CancelledBy {
    /// A person, in the Console.
    Person,
    /// Another Client.
    Client,
    /// The program closing a parent value from another task.
    Program,
    /// The browser, outside every Client: the page's window or tab closed on the desktop, the
    /// page's own `window.close()`, or the browser quitting (ADR 0113).
    Browser,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// The value that closed or stopped under a call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClosedValue {
    /// The call's page.
    Page,
    /// Its Session.
    Session,
    /// The browser the Session ran in, ending every Session in it.
    Browser,
    /// Its Run.
    Run,
    /// The connection.
    Connection,
    /// A value this SDK does not know, as received.
    Unknown(String),
}

/// The enclosing call whose timeout ended a call.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EndedBy {
    /// Its SDK call, such as `handler`.
    pub operation: String,
    /// Finds it in the Run trace.
    pub call_id: String,
}

/// Details of `scripted_model_exhausted` and `scripted_model_unused`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ScriptedModelDetails {
    /// The name it was registered under.
    pub provider: String,
    /// The calls it answered.
    pub calls: u64,
    /// Its replies left.
    pub remaining: u64,
}

/// The matches of a target that was not unique.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TargetNotUnique {
    /// Each match, rendered in Rust.
    pub matches: Vec<String>,
}

/// A page the site refused.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Blocked {
    /// Why the site refused it.
    pub block_cause: BlockCause,
    /// The URL whose navigation met it (ADR 0092).
    pub url: String,
    /// `None` when the site answered with no response, such as a reset connection.
    pub status: Option<u16>,
}

/// Why AI work was not verified.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NotVerified {
    /// The Verdict given.
    pub verdict: Verdict,
    /// How the work ended.
    pub ended: Ended,
    /// What the model claimed; `None` when it stopped before claiming anything.
    pub reported: Option<Reported>,
    /// Why it is not `Verified`: failed, missing or switched-off checks, uncertain Effects.
    pub reasons: Vec<String>,
}

/// The Block causes of ADRs 0021, 0035, 0045 and 0048.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockCause {
    /// A rate limit.
    RateLimited,
    /// A Challenge.
    Challenge,
    /// An IP block.
    IpBlocked,
    /// A demand for payment (ADR 0045).
    PaymentRequired,
    /// A consent wall (ADR 0048).
    ConsentRequired,
    /// A cause this SDK does not know, as received.
    Unknown(String),
}

/// A Verdict, given by checks under ADR 0029's rules and never by the model's claim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Verdict {
    /// Every check relied on passed.
    Verified,
    /// A declared Success check failed.
    Rejected,
    /// Anything else, including AI work with no Success check.
    NeedsReview,
    /// A value this SDK does not know, as received; never `Verified`.
    Unknown(String),
}

/// How AI work ended. A failed ending is an [`Error`] instead.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Ended {
    /// Reached its end.
    Finished,
    /// Stopped early.
    Stopped(StopReason),
}

/// Why AI work stopped early (ADRs 0079, 0080, 0100). The [`LimitSource`] names the setting that
/// stopped it, its value and its layer (G6).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum StopReason {
    /// A Limit, by name; the Run stops too.
    Limit(String),
    /// A timeout: the Agent's own `timeout.agent`, or the Run's.
    Timeout(LimitSource),
    /// The Agent's step cap, `agent.max_steps`; only the task stops.
    MaxSteps(LimitSource),
    /// The Agent's stall rule, `agent.max_stalled_steps`; only the task stops.
    NoProgress(LimitSource),
    /// The Run's Model allowance, `ai.allowance.<kind>` (ADR 0107): the task stops and the Run goes
    /// on.
    ModelAllowance(LimitSource),
    /// Someone other than the caller stopped it: a person in the Console (`person`), another
    /// Client (`client`), the program closing the page or the Run (`program`), or the browser
    /// closing the page outside every Client (`browser`, ADR 0113).
    Abort(String),
    /// A reason this SDK does not know, as received.
    Unknown(String),
}

/// What the model said about its own work: a signal, never proof (ADR 0013).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Reported {
    /// The claim.
    pub claim: Claim,
    /// The model's message.
    pub message: String,
}

/// The model's claim about its own work.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Claim {
    /// It says the work is done.
    Done,
    /// It gave up.
    GaveUp,
    /// A claim this SDK does not know, as received.
    Unknown(String),
}

/// What AI work gathered before an early end, never typed as complete (ADR 0100): each top-level
/// field of an object, or each element of a list, valid against its own part of the schema and
/// Grounded.
#[derive(Debug, Clone)]
pub struct PartialOutput {
    _private: (),
}

impl PartialOutput {
    /// What was gathered, as JSON: an object of the fields found, or an array of the elements.
    pub fn json(&self) -> &serde_json::Value {
        unimplemented!()
    }

    /// One field, parsed into its own type; `None` when it was not found.
    pub fn field<V: serde::de::DeserializeOwned>(&self, name: &str) -> Option<V> {
        let _ = name;
        unimplemented!()
    }

    /// The elements gathered for a list output, each parsed into its own type.
    pub fn elements<V: serde::de::DeserializeOwned>(&self) -> Vec<V> {
        unimplemented!()
    }
}

/// The result of AI work that can claim success. The Agent's task resolves to
/// [`crate::agent::AgentResult`], which carries these fields and its steps (ADR 0100).
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the Verdict says whether the output is proven"]
pub struct AiWorkResult<T> {
    /// How the work ended.
    pub ended: Ended,
    /// Whether Success checks prove it.
    pub verdict: Verdict,
    /// What the model claimed; `None` when it stopped before claiming anything.
    pub reported: Option<Reported>,
    /// Present only once the work finished with a complete `T`.
    pub output: Option<T>,
    /// What was gathered before an early end.
    pub partial: Option<PartialOutput>,
}

impl<T> AiWorkResult<T> {
    /// The result as the engine sent it, with the fields this SDK does not know (G3): serde
    /// ignores them when it builds the typed view, and they stay reachable here.
    pub fn raw(&self) -> &serde_json::Value {
        unimplemented!()
    }

    /// Returns the output when the Verdict is `Verified`; otherwise the error `not_verified`.
    pub fn require_verified(self) -> Result<T, Error> {
        unimplemented!()
    }
}
