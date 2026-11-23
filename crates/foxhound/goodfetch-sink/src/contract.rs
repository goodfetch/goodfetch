//! The delivery contract: the batch a Sink is given, what it answers, the
//! failures the driver tells apart, and the primitives each Sink fills
//! (ADRs 0008, 0026 and 0129).

use std::collections::BTreeMap;
use std::fmt;
use std::time::SystemTime;

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
    /// The Records, in number order with no gap.
    pub records: Vec<Delivered>,
}

/// The guarantee class a Sink's probe reports (ADR 0026).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// A stale run epoch is refused in the same transaction as the write.
    Fenced,
    /// A stale run epoch is refused per Record.
    RecordFenced,
    /// A Record's delivery ID makes a second write of it a no-op.
    Keyed,
    /// A Record may arrive more than once.
    AtLeastOnce,
}

/// What a Sink's probe found at its destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// The guarantee class it holds.
    pub class: Class,
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
    /// The destination refused some of the batch's Records.
    RejectedRecords,
    /// The destination refused the engine's credentials or access.
    Auth,
    /// The destination no longer exists.
    Gone,
    /// The Sink's translation point did not recognize it: counted as
    /// transient and logged at `warn` with its text (ADR 0127).
    Unrecognized,
}

/// A Sink's failure, as its translation point (ADR 0127) turned its
/// destination's own: the kind the driver acts on, and the destination's
/// text, kept for logs and never offered to compare.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{kind:?}: {text}")]
#[non_exhaustive]
pub struct Failure {
    kind: FailureKind,
    text: String,
}

impl Failure {
    /// A failure of `kind`, whose destination said `text`.
    #[must_use]
    pub fn new(kind: FailureKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }

    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> FailureKind {
        self.kind
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

    /// The Sink's translation point (ADR 0127): `error` as a [`Failure`].
    fn map_failure(&self, error: Self::Error) -> Failure;
}
