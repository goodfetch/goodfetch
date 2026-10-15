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

/// An Error code. A code a newer engine sends and this SDK does not know is kept as received (G3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCode {
    /// `closed`: a call on a closed value (ADR 0078).
    Closed,
    /// `removed`: a removed name was called, and `fix` names its replacement (#467).
    Removed,
    /// `handler_failed`: a crawl handler returned an error that is not a Goodfetch [`Error`].
    HandlerFailed,
    /// `listener_failed`: a listener threw (ADR 0081). Only the TypeScript and Python SDKs raise it:
    /// Rust has no listeners, since its events are streams the program reads.
    ListenerFailed,
    /// `hook_failed` (ADRs 0033, 0104).
    HookFailed,
    /// `header_forbidden`: a before-request Hook set a header the engine owns, raised inside
    /// `hook_failed` (ADR 0104).
    HeaderForbidden,
    /// `hook_ineffective`: a Hook that can never run (ADR 0104).
    HookIneffective,
    /// `hook_duplicate`: a second Hook of one name on one crawl (ADR 0104).
    HookDuplicate,
    /// `fetch_handler_failed`: the program's Fetch backend failed with its own error (ADR 0104).
    FetchHandlerFailed,
    /// `proxy_handler_failed`: the program's Proxy provider failed with its own error (ADR 0104).
    ProxyHandlerFailed,
    /// `sink_handler_failed`: the program's Sink failed with its own error, retried on ADR 0008's
    /// schedule (ADR 0104).
    SinkHandlerFailed,
    /// `limit_reached`.
    LimitReached,
    /// `page_check_failed` (ADR 0006).
    PageCheckFailed,
    /// `target_not_unique`.
    TargetNotUnique,
    /// `blocked`, with a [`BlockCause`].
    Blocked,
    /// `not_verified`, from [`AiWorkResult::require_verified`].
    NotVerified,
    /// `deadline_exceeded`: every timeout, whatever the phase (ADR 0080), except a target still
    /// ambiguous at its deadline, which is `target_not_unique` (ADR 0085).
    DeadlineExceeded,
    /// `call_cancelled`: someone else stopped the call in flight; a dropped future of the caller's
    /// own never sees it (ADR 0080).
    CallCancelled,
    /// `plugin_crashed` (ADR 0033).
    PluginCrashed,
    /// `payload_invalid`: a Payload failed its Label's schema, was not JSON, or was too large (#469).
    PayloadInvalid,
    /// `payload_schema_changed`: a resume found Payloads that fail their Label's changed schema (#469).
    PayloadSchemaChanged,
    /// `setting_unknown`: an unknown key or `GOODFETCH_` variable (ADR 0083).
    SettingUnknown,
    /// `setting_invalid`: a wrong type or out-of-range value (ADR 0083).
    SettingInvalid,
    /// `setting_not_applicable`: a setting no scope beneath can use (ADR 0083).
    SettingNotApplicable,
    /// `setting_locked`, a refusal: a lower layer locked or capped the value (ADR 0083).
    SettingLocked,
    /// `capability_unavailable`: the platform cannot provide a capability (ADR 0083).
    CapabilityUnavailable,
    /// `selector_invalid`: a selector that does not parse, or a Playwright-only engine, `>>` or a
    /// Playwright CSS extension such as `:has-text()` (ADR 0085).
    SelectorInvalid,
    /// `target_unsupported`: the target is of the wrong kind for the action (ADR 0085).
    TargetUnsupported,
    /// `page_function_failed`: the page function threw (ADR 0085).
    PageFunctionFailed,
    /// `page_value_invalid`: a value crossing to or from the page is not JSON (ADR 0085).
    PageValueInvalid,
    /// `page_handler_failed`: a route, dialog or locator handler threw (ADR 0085). Only the
    /// TypeScript and Python SDKs raise it: Rust answers those requests in the program's own code.
    PageHandlerFailed,
    /// `route_already_handled` (ADR 0085). Only the TypeScript and Python SDKs raise it: a Rust
    /// route's answers take the route.
    RouteAlreadyHandled,
    /// `navigation_interrupted`: another navigation superseded this one (ADR 0085).
    NavigationInterrupted,
    /// `response_not_json` (ADR 0085).
    ResponseNotJson,
    /// `navigation_failed`: the site could not be reached (ADR 0085).
    NavigationFailed,
    /// `page_crashed` (ADR 0085).
    PageCrashed,
    /// `body_unavailable`: the browser no longer holds the body (ADR 0085).
    BodyUnavailable,
    /// `download_failed` (ADR 0085).
    DownloadFailed,
    /// `cdp_command_refused`, a refusal (ADR 0085).
    CdpCommandRefused,
    /// `secret_unbound`: no scope binds the Secret placeholder (ADR 0092).
    SecretUnbound,
    /// `secret_unavailable`: the Secret provider gave no value (ADR 0092).
    SecretUnavailable,
    /// `secret_domain_not_allowed`, a refusal (ADR 0092).
    SecretDomainNotAllowed,
    /// `secret_input_refused`, a refusal: other code may read this browser (ADR 0092).
    SecretInputRefused,
    /// `logged_out`: recovery did not bring the login back (ADR 0092).
    LoggedOut,
    /// `human_pause_declined`, a refusal by a person (ADR 0092).
    HumanPauseDeclined,
    /// `session_not_savable`: the Session's cookies live outside run state (ADR 0092).
    SessionNotSavable,
    /// `target_not_found`: `act` found no element for its instruction, or `get_by_ref` named an
    /// element that is gone (ADR 0091).
    TargetNotFound,
    /// `plan_drift`: a step drifted and could not be repaired (ADR 0091).
    PlanDrift,
    /// `plan_step_missing`: `plan.frozen` is set and the Plan holds no entry for the call (ADR 0091).
    PlanStepMissing,
    /// `plan_invalid`: the Plan file cannot be read or does not match its format (ADR 0091).
    PlanInvalid,
    /// `model_not_configured`: a call the Plan holds no entry for needs a model, and none is configured (ADR 0091).
    ModelNotConfigured,
    /// `extraction_incomplete`: required fields were not found on the page (ADR 0091).
    ExtractionIncomplete,
    /// `extraction_invalid`, a model error: the model's answers never passed the checks within the retries (ADR 0091).
    ExtractionInvalid,
    /// `effect_uncertain`: an Effect may have happened and Goodfetch cannot tell: an `act` whose
    /// action was an Effect failed after the action was sent (ADR 0091), or a declared Effect's body
    /// ended after a send (ADR 0106); never retried, a person reconciles it.
    EffectUncertain,
    /// `effect_key_invalid`: an empty key, or one longer than 255 characters (ADR 0106).
    EffectKeyInvalid,
    /// `effect_key_reused`: a key declared again after this attempt already got its answer (ADR 0106).
    EffectKeyReused,
    /// `effect_output_invalid`: a body's value is not JSON or is too large (ADR 0106).
    EffectOutputInvalid,
    /// `effect_not_confirmed`, a refusal: an AI Effect's confirmation was declined or its Human pause
    /// expired (ADR 0091).
    EffectNotConfirmed,
    /// `success_check_failed`: `expect`'s condition was still false at its timeout (ADR 0091).
    SuccessCheckFailed,
    /// `variables_mismatch`: a placeholder nothing fills, a variable no placeholder uses, or a name
    /// that is both a variable and a Secret placeholder (ADR 0091).
    VariablesMismatch,
    /// `label_duplicate`: a second Label of one name, or a second handler for one Label (ADR 0093).
    LabelDuplicate,
    /// `label_unhandled`: the Run's start found a Label with no handler (ADR 0093).
    LabelUnhandled,
    /// `label_unknown`: a Label the crawl does not declare, at an enqueue or a resume (ADR 0093).
    LabelUnknown,
    /// `attempt_ended`: a call on the context of an attempt that has ended; returned by
    /// `ctx.skip()` and `ctx.retry()`, and consumed by the helper (ADR 0093).
    AttemptEnded,
    /// `browser_required`: a call only a browser can serve while `fetch.tier` forces a tier without
    /// one (ADR 0093).
    BrowserRequired,
    /// `retry_requested`: an attempt the handler ended with `ctx.retry()` (ADR 0093).
    RetryRequested,
    /// `response_not_ok`: a final 4xx that is no Block cause and not in `crawl.handle_statuses`;
    /// not retryable (ADR 0093).
    ResponseNotOk,
    /// `response_unavailable`, a page error: a final 5xx or 408 after the request's retries;
    /// retryable (ADR 0093).
    ResponseUnavailable,
    /// `handler_lost`: the program's process or connection was lost mid-attempt (a crash, a signal
    /// it does not handle, or a linked engine that died with it, counted on resume from the Run's
    /// committed state); retryable, and counted against `crawl.retry.max`. The program's own stop
    /// counts nothing (ADR 0093).
    HandlerLost,
    /// `site_preferences_refused`: a handler's own navigation that Site preferences refuse (ADR 0093).
    SitePreferencesRefused,
    /// `run_id_invalid`: an ID the engine refuses (ADR 0102).
    RunIdInvalid,
    /// `run_conflict`: the ID names a Run started with other inputs, or by another caller.
    RunConflict,
    /// `run_ended`: a call that would give an ended Run new work.
    RunEnded,
    /// `run_in_use`, a refusal: the Run is live in another program, or already open in this one.
    RunInUse,
    /// `limit_raise_refused`, a refusal: a resume gave a Limit above the one the Run began with.
    LimitRaiseRefused,
    /// `run_state_unavailable`: the Run index lists the Run, but its state is missing or unreadable.
    RunStateUnavailable,
    /// `run_not_found`: the ID names no Run in the Run index.
    RunNotFound,
    /// `run_interrupted`: a view's `report()` on a Run with no live writer that has not ended.
    RunInterrupted,
    /// `model_not_found`: the provider says the model does not exist or is not available to this
    /// key (ADR 0101).
    ModelNotFound,
    /// `model_capability_missing`: the operation needs a capability the model does not declare.
    ModelCapabilityMissing,
    /// `model_handler_failed`: the program's own Model provider raised an exception that is not a
    /// Goodfetch error; TypeScript's and Python's, since a Rust program fails a call with an [`Error`].
    ModelHandlerFailed,
    /// `model_unauthorized`, a refusal: the provider rejected the key. Not retryable.
    ModelUnauthorized,
    /// `model_funds_exhausted`, a refusal: the provider account has no funds left. Not retryable.
    ModelFundsExhausted,
    /// `model_price_unknown`, a refusal: a currency Limit applies and the model's price cannot be
    /// stated in its currency (no price, or no fixed rate).
    ModelPriceUnknown,
    /// `model_unavailable`: unreachable, a 5xx, or rate limited after the engine's retries. Retryable.
    ModelUnavailable,
    /// `model_declined`, a model error: the model declined to answer.
    ModelDeclined,
    /// `model_context_exceeded`, a model error: the request does not fit the context window.
    ModelContextExceeded,
    /// `model_output_invalid`, a model error: an `act`, `observe` or Agent step's answer never parsed.
    ModelOutputInvalid,
    /// `model_request_rejected`: the provider rejected the request for another reason (a 400 or
    /// 422), such as a forced strategy.
    ModelRequestRejected,
    /// `agent_tool_failed`, a program error: an Agent tool returned an error, and the task ended
    /// (ADR 0100).
    AgentToolFailed,
    /// `agent_tool_invalid`, a program error: two Agent tools share a name, or one takes a built-in
    /// action's (ADR 0100).
    AgentToolInvalid,
    /// `success_check_threw`, a program error: the Agent's Success check returned an error other
    /// than `success_check_failed`, kept as the `source()`: a bug, never a rejection (ADR 0100).
    SuccessCheckThrew,
    /// `response_not_recorded`, a refusal: a replaying Run's recording lacks the request, which was
    /// never sent; not retryable (ADR 0105).
    ResponseNotRecorded,
    /// `recording_invalid`: the Response recording cannot be read or is not WARC (ADR 0105).
    RecordingInvalid,
    /// `scripted_model_exhausted`: a call went beyond a Scripted model's replies (ADR 0105).
    ScriptedModelExhausted,
    /// `scripted_model_unused`: a test engine closed with a Scripted model's replies left (ADR 0105).
    ScriptedModelUnused,
    /// `hook_unknown`: the testing kit was asked for a Hook the crawl does not hold (ADR 0105).
    HookUnknown,
    /// `internal`.
    Internal,
    /// A code this SDK does not know, as received.
    Unknown(String),
}

impl ErrorCode {
    /// The code's snake_case name, the same on every surface.
    pub fn as_str(&self) -> &str {
        unimplemented!()
    }
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
    pub fn code(&self) -> &ErrorCode {
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

    /// Per-code typed fields; `None` for codes without any.
    pub fn details(&self) -> Option<&ErrorDetails> {
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

/// Per-code typed fields, holding `goodfetch-protocol`'s own structs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ErrorDetails {
    /// Of `handler_failed`.
    HandlerFailed(HandlerFailed),
    /// Of `hook_failed`.
    HookFailed(crate::hooks::HookFailed),
    /// Of `header_forbidden`.
    HeaderForbidden(crate::hooks::HeaderForbidden),
    /// Of `hook_ineffective`.
    HookIneffective(crate::hooks::HookIneffective),
    /// Of `hook_duplicate`.
    HookDuplicate(crate::hooks::HookDuplicate),
    /// Of `fetch_handler_failed`, `proxy_handler_failed` and `sink_handler_failed`.
    SdkHandlerFailed(crate::hooks::SdkHandlerFailed),
    /// Of `target_not_unique`.
    TargetNotUnique(TargetNotUnique),
    /// Of `blocked`.
    Blocked(Blocked),
    /// Of `not_verified`.
    NotVerified(NotVerified),
    /// Of `deadline_exceeded`, every timeout but a still-ambiguous target's, whatever the phase (ADR 0080).
    DeadlineExceeded(DeadlineExceeded),
    /// Of `payload_invalid`.
    PayloadInvalid(crate::data::PayloadInvalid),
    /// Of `payload_schema_changed`.
    PayloadSchemaChanged(crate::data::PayloadSchemaChanged),
    /// Of `setting_unknown`.
    SettingUnknown(crate::settings::SettingUnknown),
    /// Of `setting_invalid`.
    SettingInvalid(crate::settings::SettingInvalid),
    /// Of `setting_not_applicable`.
    SettingNotApplicable(crate::settings::SettingNotApplicable),
    /// Of `setting_locked`.
    SettingLocked(crate::settings::SettingLocked),
    /// Of `capability_unavailable`.
    CapabilityUnavailable(crate::settings::CapabilityUnavailable),
    /// Of `secret_unbound`.
    SecretUnbound(crate::sessions::SecretUnbound),
    /// Of `secret_unavailable`.
    SecretUnavailable(crate::sessions::SecretUnavailable),
    /// Of `secret_domain_not_allowed`.
    SecretDomainNotAllowed(crate::sessions::SecretDomainNotAllowed),
    /// Of `secret_input_refused`.
    SecretInputRefused(crate::sessions::SecretInputRefused),
    /// Of `logged_out`.
    LoggedOut(crate::sessions::LoggedOut),
    /// Of `human_pause_declined`.
    HumanPauseDeclined(crate::sessions::HumanPauseDeclined),
    /// Of `session_not_savable`.
    SessionNotSavable(crate::sessions::SessionNotSavable),
    /// Of `target_not_found`.
    TargetNotFound(crate::ai::TargetNotFound),
    /// Of `plan_drift`.
    PlanDrift(crate::ai::PlanDrift),
    /// Of `extraction_incomplete`.
    ExtractionIncomplete(crate::ai::ExtractionIncomplete),
    /// Of `extraction_invalid`.
    ExtractionInvalid(crate::ai::ExtractionInvalid),
    /// Of `success_check_failed`.
    SuccessCheckFailed(crate::ai::SuccessCheckFailed),
    /// Of `variables_mismatch`.
    VariablesMismatch(crate::ai::VariablesMismatch),
    /// Of `plan_step_missing`.
    PlanStepMissing(crate::ai::PlanStepMissing),
    /// Of `plan_invalid`.
    PlanInvalid(crate::ai::PlanInvalid),
    /// Of `model_not_configured`.
    ModelNotConfigured(crate::ai::ModelNotConfigured),
    /// Of `effect_uncertain`.
    EffectUncertain(crate::ai::EffectUncertain),
    /// Of `effect_key_invalid`.
    EffectKeyInvalid(crate::effects::EffectKeyInvalid),
    /// Of `effect_key_reused`.
    EffectKeyReused(crate::effects::EffectKeyReused),
    /// Of `effect_output_invalid`.
    EffectOutputInvalid(crate::effects::EffectOutputInvalid),
    /// Of `effect_not_confirmed`.
    EffectNotConfirmed(crate::ai::EffectNotConfirmed),
    /// Of `model_not_found`, `model_unauthorized` and `model_funds_exhausted` (ADR 0101).
    ModelFailure(crate::models::ModelFailure),
    /// Of `model_declined`.
    ModelDeclined(crate::models::ModelDeclined),
    /// Of `model_context_exceeded`.
    ModelContextExceeded(crate::models::ModelContextExceeded),
    /// Of `model_request_rejected`.
    ModelRequestRejected(crate::models::ModelRequestRejected),
    /// Of `model_price_unknown`.
    ModelPriceUnknown(crate::models::ModelPriceUnknown),
    /// Of `model_capability_missing`.
    ModelCapabilityMissing(crate::models::ModelCapabilityMissing),
    /// Of `model_handler_failed`.
    ModelHandlerFailed(crate::models::ModelHandlerFailed),
    /// Of `model_unavailable`.
    ModelUnavailable(crate::models::ModelUnavailable),
    /// Of `model_output_invalid`.
    ModelOutputInvalid(crate::models::ModelOutputInvalid),
    /// Of `record_invalid`.
    RecordInvalid(crate::output::RecordInvalid),
    /// Of `dataset_duplicate`.
    DatasetDuplicate(crate::output::DatasetDuplicate),
    /// Of `dataset_unknown`.
    DatasetUnknown(crate::output::DatasetUnknown),
    /// Of `dataset_key_invalid`.
    DatasetKeyInvalid(crate::output::DatasetKeyInvalid),
    /// Of `value_invalid`.
    ValueInvalid(crate::output::ValueInvalid),
    /// Of `run_id_invalid`, `run_conflict`, `run_ended`, `run_in_use`, `limit_raise_refused`,
    /// `run_state_unavailable`, `run_not_found` and `run_interrupted` (ADR 0102).
    Run(crate::run::RunFailure),
    /// Of `agent_tool_failed`.
    AgentToolFailed(crate::agent::AgentToolFailed),
    /// Of `agent_tool_invalid`.
    AgentToolInvalid(crate::agent::AgentToolInvalid),
    /// Of `success_check_threw`.
    SuccessCheckThrew(crate::agent::SuccessCheckThrew),
    /// Of `response_not_recorded`.
    ResponseNotRecorded(crate::recording::ResponseNotRecorded),
    /// Of `recording_invalid`.
    RecordingInvalid(crate::recording::RecordingInvalid),
    /// Of `scripted_model_exhausted` and `scripted_model_unused`.
    ScriptedModel(ScriptedModelDetails),
}

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

/// The handler's own error, as text, since the outcome outlives the process.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HandlerFailed {
    /// The error's type name.
    pub error_type: String,
    /// Its message.
    pub error_message: String,
    /// The backtrace frames of the user's code, when captured.
    pub stack: Vec<String>,
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

/// Why a crawl request was skipped (ADR 0093's full list).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// A redirect left the crawl's scope.
    OutOfScope,
    /// A redirect or a Hook's rewrite reached a host the run's domain policy refuses (ADRs 0014,
    /// 0017).
    DomainPolicy,
    /// A URL rule the Frontier learned after admitting it merged it with another (ADR 0005).
    Duplicate,
    /// A Disallow line of the origin's robots.txt, or a robots.txt answered with a 429 or a 5xx, or
    /// unreachable, with no earlier copy, until the request's retries ran out (ADR 0086).
    /// Also a handler's own navigation refused for it whose `site_preferences_refused` escaped the
    /// handler.
    RobotsTxt,
    /// A Crawl-delay above `site_preferences.max_crawl_delay` (ADR 0086), also for an escaped
    /// `site_preferences_refused` of a handler's navigation.
    CrawlDelay,
    /// An AI-use signal that refuses the run's Purpose while
    /// `site_preferences.respect_use_signals` is on (ADR 0086), also for an escaped
    /// `site_preferences_refused` of a handler's navigation.
    UseSignals,
    /// A before-request Hook (ADR 0033).
    Hook,
    /// The handler returned `ctx.skip(…)`.
    Handler,
    /// A reason this SDK does not know, as received.
    Unknown(String),
}

/// Which crawl request an outcome is about. Outcomes span Labels, so the Payload is not typed here.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CrawlRequestRef {
    /// Its ID.
    pub id: String,
    /// Its URL.
    pub url: String,
    /// Its Label's name.
    pub label: String,
    /// Its depth.
    pub depth: u32,
}

/// A crawl request's ending (ADR 0079; the noun is ADR 0093's). A program reads outcomes with
/// `CrawlRun::outcomes` (ADR 0102).
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct CrawlRequestOutcome {
    /// Which request.
    pub request: CrawlRequestRef,
    /// When it ended.
    pub ended_at: std::time::SystemTime,
    /// How it ended.
    pub kind: CrawlRequestOutcomeKind,
    /// Retries are not an outcome: every outcome counts its attempts.
    pub attempts: u32,
    /// The failed attempts.
    pub attempt_errors: Vec<Error>,
}

/// The four endings of a crawl request.
#[derive(Debug)]
#[non_exhaustive]
pub enum CrawlRequestOutcomeKind {
    /// Handled, its outputs committed: per Dataset name, what its commit did to the Records it
    /// pushed (ADR 0103); empty when it pushed none.
    Succeeded {
        /// Per Dataset name.
        records: std::collections::BTreeMap<String, crate::output::RecordCounts>,
    },
    /// Failed, with its error.
    Failed(Error),
    /// Skipped, with the reason and a Hook's or the handler's own message.
    Skipped {
        /// Why.
        reason: SkipReason,
        /// The Hook's or the handler's text.
        message: Option<String>,
    },
    /// Refused by the site, with the Block cause.
    Blocked(BlockCause),
    /// A kind this SDK does not know, as received.
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
    /// Someone other than the caller stopped it: a person in the Console (`person`), another
    /// Client (`client`), or the program closing the page or the Run (`program`).
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
