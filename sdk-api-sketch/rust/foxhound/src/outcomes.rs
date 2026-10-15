//! A crawl request's outcomes and Foxhound's own error details of a handler (ADRs 0079, 0093). The
//! error type and every detail type of Collie's codes are the Collie client crate's (ADR 0095).

use crate::BlockCause;
use crate::Error;

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
    /// The handler returned a `model_allowance_reached` error once the Run's Model allowance was
    /// reached (ADR 0107); the Run then ends `incomplete` though it finishes.
    ModelAllowance,
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

