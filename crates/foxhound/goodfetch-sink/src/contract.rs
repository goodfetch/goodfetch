//! The delivery contract: the batch a Sink is given, what it answers, the
//! failures the driver tells apart, and the primitives each Sink fills
//! (ADRs 0008, 0026 and 0129).

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, SystemTime};

use goodfetch_engine::mask::{Masks, Scope};
use goodfetch_engine::retry_after::{self, RetryAfter};
use serde_json::Value;

/// One copy of a Run: the Run, and the run epoch its lease gave this copy
/// (ADR 0003). A copy with a lower epoch than another is a zombie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunCopy {
    /// The Run's ID.
    pub run_id: String,
    /// The run epoch of this copy.
    pub run_epoch: u64,
}

/// The ID of one Record's delivery to one Sink, the same on every attempt
/// and every copy of the Run, so a destination can tell a retry from a new
/// Record (ADR 0008).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeliveryId(String);

impl DeliveryId {
    /// The delivery ID of Record `number` of `dataset` to `sink` in the Run
    /// `run_id`.
    #[must_use]
    pub fn new(run_id: &str, sink: &str, dataset: &str, number: u64) -> Self {
        Self(format!("{run_id}/{sink}/{dataset}/{number}"))
    }

    /// Its text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reconstructs a stored delivery ID from its text.
    #[must_use]
    pub(crate) fn from_stored(text: String) -> Self {
        Self(text)
    }
}

impl fmt::Display for DeliveryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One Record as a Sink is given it.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivered {
    /// Its delivery ID.
    pub delivery_id: DeliveryId,
    /// Its number in its Dataset.
    pub number: u64,
    /// When it was committed, RFC 3339 in UTC.
    pub committed_at: String,
    /// The Record.
    pub value: Value,
    /// Under a keyed write mode, the values of its key's fields, in the
    /// key's order, as a JSON array: what identifies it among its
    /// Dataset's Records (ADR 0026). None under `append`.
    pub key: Option<Value>,
}

impl Delivered {
    /// The Record numbered `number` of `dataset`, committed at
    /// `committed_at`, as delivered to `sink` by the Run `run_id`.
    pub(crate) fn new(
        run_id: &str,
        sink: &str,
        dataset: &str,
        number: u64,
        committed_at: SystemTime,
        value: Value,
    ) -> Self {
        let committed_at = jiff::Timestamp::try_from(committed_at)
            .map_or_else(|_| String::new(), |at| at.to_string());
        Self {
            delivery_id: DeliveryId::new(run_id, sink, dataset, number),
            number,
            committed_at,
            value,
            key: None,
        }
    }
}

/// Records of one Dataset, in number order, that one write delivers.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    /// The copy of the Run delivering them.
    pub run: RunCopy,
    /// Their Dataset.
    pub dataset: String,
    /// The Dataset's declared JSON Schema, when the crawl declared one
    /// (ADR 0026: declared schema comes first, else inferred).
    pub declared_schema: Option<Value>,
    /// The Records, in number order with no gap.
    pub records: Vec<Delivered>,
}

/// The guarantee class a Sink's probe reports (ADR 0026), the protocol's,
/// which a Run report's Sink carries.
pub use goodfetch_protocol::foxhound::sink::GuaranteeClass;

/// What a Sink's probe found at its destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// The guarantee class it holds.
    pub class: GuaranteeClass,
    /// For each Dataset, the number of the last Record the destination
    /// holds from this copy of the Run; a Dataset it holds none of is
    /// missing.
    pub delivered: BTreeMap<String, u64>,
}

/// What a write did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Written {
    /// The batch is at the destination.
    Delivered,
    /// The destination holds a later copy of the Run, whose epoch it
    /// names, and refused the batch.
    Superseded {
        /// The later copy's run epoch.
        run_epoch: u64,
    },
}

/// What a failed probe or write leads to, which the driver decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// It may pass later: retried on the delivery schedule.
    Transient,
    /// The destination's rate or quota limit was reached: reported as
    /// `behind`, never `failed` (ADR 0026).
    QuotaExhausted,
    /// The destination refused some of the batch's Records.
    RejectedRecords,
    /// The destination refused the engine's credentials or access.
    Auth,
    /// The destination no longer exists (e.g. HTTP 410): pauses the Sink.
    Gone,
    /// A non-retried permanent failure (e.g. HTTP 404, 405): fails the Sink
    /// without retrying.
    Permanent,
    /// The Sink's translation point did not recognize it: counted as
    /// transient and logged at `warn` (ADR 0127).
    Unrecognized,
}

/// A Sink's failure, as its translation point (ADR 0127) turned its
/// destination's own: the kind the driver acts on, an optional `Retry-After`
/// wait the destination asked for, and the destination's text, kept for
/// diagnosis. Two failures are equal when their kinds are: no code compares
/// the text.
#[derive(Clone, Debug, thiserror::Error)]
#[error("a Sink's write failed: {kind:?}")]
#[non_exhaustive]
pub struct Failure {
    kind: FailureKind,
    retry_after: Option<Duration>,
    rate_limited: bool,
    timed_out: bool,
    text: PartyText,
}

impl PartialEq for Failure {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl Eq for Failure {}

/// A destination's own text about a failure: read by no code, and redacted
/// wherever it is formatted, since a destination may echo what it was
/// given, a Record's values or a credential included.
#[derive(Clone)]
struct PartyText(String);

impl fmt::Debug for PartyText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "PartyText(<{} bytes, redacted>)", self.0.len())
    }
}

impl Failure {
    /// A failure of `kind`, whose destination said `text`.
    #[must_use]
    pub fn new(kind: FailureKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            retry_after: None,
            rate_limited: kind == FailureKind::QuotaExhausted,
            timed_out: false,
            text: PartyText(text.into()),
        }
    }

    /// A write that exceeded `timeout.sink_write` (`deadline_exceeded`,
    /// ADRs 0026 and 0080): retried as transient and lowers adaptive
    /// concurrency.
    #[must_use]
    pub fn deadline_exceeded(timeout: Duration) -> Self {
        Self {
            kind: FailureKind::Transient,
            retry_after: None,
            rate_limited: false,
            timed_out: true,
            text: PartyText(format!(
                "Sink write exceeded `timeout.sink_write` ({} ms): deadline_exceeded",
                timeout.as_millis()
            )),
        }
    }

    /// Attaches a `Retry-After` wait asked by the destination (ADR 0026).
    #[must_use]
    pub fn with_retry_after(mut self, wait: Duration) -> Self {
        self.retry_after = Some(wait);
        self
    }

    /// Classifies an HTTP status code from a destination (ADR 0026): only
    /// 408, 429 and 5xx are retried (`Transient`, honouring `retry_after`);
    /// 410 pauses (`Gone`); 401/403 fail with `Auth`; 400/422 split and
    /// dead-letter (`RejectedRecords`); other 4xx fail as `Permanent`.
    #[must_use]
    pub fn from_http_status(
        status: u16,
        retry_after: Option<Duration>,
        detail: impl Into<String>,
    ) -> Self {
        let kind = match status {
            408 | 429 | 500..=599 => FailureKind::Transient,
            410 => FailureKind::Gone,
            401 | 403 => FailureKind::Auth,
            400 | 422 => FailureKind::RejectedRecords,
            _ => FailureKind::Permanent,
        };
        let mut failure = Self::new(kind, detail);
        if status == 429 {
            failure.rate_limited = true;
        }
        if status == 408 {
            failure.timed_out = true;
        }
        match (kind, retry_after) {
            (FailureKind::Transient, Some(wait)) => failure.with_retry_after(wait),
            _ => failure,
        }
    }

    /// Classifies an HTTP status code along with its raw `Retry-After` and
    /// `Date` headers, parsed through [`goodfetch_engine::retry_after::read`]
    /// relative to `received` (ADRs 0026 and 0118).
    #[must_use]
    pub fn from_http_response(
        status: u16,
        retry_after_header: Option<&str>,
        date_header: Option<&str>,
        received: SystemTime,
        detail: impl Into<String>,
    ) -> Self {
        let wait = retry_after_header.and_then(|value| {
            match retry_after::read(value, date_header, received) {
                RetryAfter::Until(until) => {
                    Some(until.duration_since(received).unwrap_or_default())
                }
                RetryAfter::NoWait => Some(Duration::ZERO),
                RetryAfter::Unparsable => None,
            }
        });
        Self::from_http_status(status, wait, detail)
    }

    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> FailureKind {
        self.kind
    }

    /// The `Retry-After` wait asked by the destination, when it sent one.
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// Whether the failure signals rate limiting or quota exhaustion (HTTP 429
    /// or [`FailureKind::QuotaExhausted`]), lowering adaptive concurrency.
    #[must_use]
    pub fn is_rate_limited(&self) -> bool {
        self.rate_limited
    }

    /// Whether the failure was a write timeout (`timeout.sink_write` or HTTP
    /// 408), lowering adaptive concurrency.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        self.timed_out
    }

    /// The destination's text, masked at [`Scope::Kept`] for a dead-letter
    /// reason: no code reads the raw text (ADRs 0067 and 0127).
    #[must_use]
    pub(crate) fn masked_reason(&self, masks: &Masks) -> String {
        masks.apply(&self.text.0, Scope::Kept).into_owned()
    }
}

/// The steps that differ between Sinks, which the delivery driver calls
/// (ADR 0129). Each does one step; the order, the retries and the fencing
/// rule above them are the driver's.
pub trait Primitives {
    /// The destination's own failure, before translation.
    type Error;

    /// Readies the destination for `run` and reads its guarantee class and
    /// how far it holds this copy's Records.
    ///
    /// # Errors
    ///
    /// The destination's, when it cannot be reached or readied.
    fn probe(&mut self, run: &RunCopy) -> Result<Probe, Self::Error>;

    /// Writes `batch`, unless the destination holds a later copy of the
    /// Run. A Record it already holds from this copy is not written twice.
    ///
    /// # Errors
    ///
    /// The destination's, when the batch did not land or its answer was
    /// lost.
    fn write(&mut self, batch: &Batch) -> Result<Written, Self::Error>;

    /// Writes `batch` bounded by `timeout` (`timeout.sink_write`, ADRs 0026
    /// and 0080). Sinks whose underlying client or connection accepts a
    /// per-write timeout override this; others fall back to [`Self::write`].
    ///
    /// # Errors
    ///
    /// The destination's, when the batch did not land or timed out.
    fn write_with_timeout(
        &mut self,
        batch: &Batch,
        timeout: Duration,
    ) -> Result<Written, Self::Error> {
        let _ = timeout;
        self.write(batch)
    }

    /// Ends the delivery of the copy `run` of a Run that finished, once its
    /// last batch is written: a Sink that writes a finished Run's Records
    /// apart, as `replace` does, swaps them in here, unless the destination
    /// holds a later copy of the Run. Nothing otherwise.
    ///
    /// # Errors
    ///
    /// The destination's, when the swap did not land or its answer was
    /// lost.
    fn finish(&mut self, run: &RunCopy) -> Result<Written, Self::Error> {
        let _ = run;
        Ok(Written::Delivered)
    }

    /// The destination's deduplication window, when its guarantee is keyed
    /// only within a finite window (e.g. a 5-minute FIFO queue); a delivery
    /// arriving later than this window after commit is counted in
    /// `outside_dedup_window` (ADR 0026). `None` when deduplication has no
    /// time window (e.g. fenced row stores).
    fn dedup_window(&self) -> Option<Duration> {
        None
    }

    /// The Sink's translation point (ADR 0127): `error` as a [`Failure`].
    fn map_failure(&self, error: Self::Error) -> Failure;
}
