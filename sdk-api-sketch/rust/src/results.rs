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
    /// `handler_failed`: a handler returned an error that is not a Goodfetch [`Error`].
    HandlerFailed,
    /// `hook_failed` (ADR 0033).
    HookFailed,
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
    /// `deadline_exceeded`.
    DeadlineExceeded,
    /// `plugin_crashed` (ADR 0033).
    PluginCrashed,
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

/// The settings layers of ADR 0028, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SettingsLayer {
    /// Built-in defaults.
    Default,
    /// An Operator policy.
    OperatorPolicy,
    /// User and project configuration.
    Configuration,
    /// A Recipe or the developer's code.
    Recipe,
    /// The run itself.
    Run,
}

/// A setting's value; its shape is #470's.
#[derive(Debug, Clone)]
pub struct SettingValue {
    _private: (),
}

/// Where the limit that ended a call came from (G6).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LimitSource {
    /// The setting key, such as `timeouts.action`.
    pub setting: String,
    /// Its value.
    pub value: SettingValue,
    /// The layer that set the value.
    pub layer: SettingsLayer,
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
    /// Of `target_not_unique`.
    TargetNotUnique(TargetNotUnique),
    /// Of `blocked`.
    Blocked(Blocked),
    /// Of `not_verified`.
    NotVerified(NotVerified),
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
    /// What the model claimed.
    pub reported: Reported,
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

/// Why a request was skipped; the full list is #477's and #480's.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SkipReason {
    /// Outside the crawl's scope.
    OutOfScope,
    /// Already seen.
    Duplicate,
    /// Excluded by Site preferences.
    SitePreferences,
    /// Skipped by a Hook.
    SkippedByHook,
    /// Skipped by the handler.
    SkippedByHandler,
    /// A reason this SDK does not know, as received.
    Unknown(String),
}

/// A crawl request's ending. The noun "request" is #477's to settle.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct RequestOutcome {
    /// How it ended.
    pub kind: RequestOutcomeKind,
    /// Retries are not an outcome: every outcome counts its attempts.
    pub attempts: u32,
    /// The failed attempts.
    pub attempt_errors: Vec<Error>,
}

/// The four endings of a crawl request.
#[derive(Debug)]
#[non_exhaustive]
pub enum RequestOutcomeKind {
    /// Handled, its outputs committed.
    Succeeded,
    /// Failed, with its error.
    Failed(Error),
    /// Skipped, with the reason.
    Skipped(SkipReason),
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ended {
    /// Reached its end.
    Finished,
    /// Stopped early.
    Stopped(StopReason),
}

/// Why AI work stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopReason {
    /// A Limit, by name.
    Limit(String),
    /// An abort.
    Abort,
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

/// What AI work gathered before an early end, never typed as complete; its shape is #474's and #473's.
#[derive(Debug, Clone)]
pub struct PartialOutput {
    _private: (),
}

/// The result of AI work that can claim success: an Agent's task, or a Plan replayed toward a goal.
/// #474 and #473 build their result types on it.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the Verdict says whether the output is proven"]
pub struct AiWorkResult<T> {
    /// How the work ended.
    pub ended: Ended,
    /// Whether Success checks prove it.
    pub verdict: Verdict,
    /// What the model claimed.
    pub reported: Reported,
    /// Present only once the work finished with a complete `T`.
    pub output: Option<T>,
    /// What was gathered before an early end.
    pub partial: Option<PartialOutput>,
}

impl<T> AiWorkResult<T> {
    /// Returns the output when the Verdict is `Verified`; otherwise the error `not_verified`.
    pub fn require_verified(self) -> Result<T, Error> {
        unimplemented!()
    }
}
