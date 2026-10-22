//! The invariant checkers (ADR 0060): functions over an Observation log,
//! each returning the first violation with the records that show it, which
//! the Simulation, real-process tests and conformance suites share. Here:
//! fencing, judged by Guarantee class, and Limits.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

use crate::observation::{Epoch, ObservationLog, Operation, Record, ValueDigest};

/// How strongly a Sink keeps a delivery from being duplicated or
/// overwritten by a stale copy of the run (ADR 0026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuaranteeClass {
    /// The epoch check and the whole batch are atomic: no stale-epoch
    /// record is visible, and each committed record is visible exactly once.
    Fenced,
    /// A stale epoch cannot overwrite a record but can add one.
    RecordFenced,
    /// A retry lands on the same key: no key is visible twice.
    Keyed,
    /// The receiver dedups on the delivery ID: each committed record is
    /// visible at least once.
    AtLeastOnce,
}

/// What a violation broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationKind {
    /// A record a stale copy wrote is visible (Fenced).
    StaleRecordVisible,
    /// A stale copy overwrote a record a current copy wrote (Record-fenced).
    StaleOverwrite,
    /// One key is visible twice (Fenced, Keyed).
    DuplicateKey,
    /// A committed record is not visible (every class).
    CommitMissing,
    /// Charged plus reserved Usage went past a Limit and the allowed loss.
    LimitExceeded,
    /// A settle returned more Usage than its scope had reserved.
    OverSettled,
}

/// The first violation a checker found, with the records that show it, in
/// log order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation<R> {
    /// What it broke.
    pub kind: ViolationKind,
    /// The records that show it.
    pub records: Vec<Record<R>>,
}

impl<R: Serialize> fmt::Display for Violation<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}, shown by:", self.kind)?;
        for record in &self.records {
            let line = serde_json::to_string(record).map_err(|_| fmt::Error)?;
            write!(formatter, "\n{line}")?;
        }
        Ok(())
    }
}

/// Judges a log's delivery by Guarantee class. The log's `Fence`, `Write`,
/// `Commit` and `Visible` records share one epoch sequence; a write is stale
/// when a greater epoch was fenced before it. Visible records are judged in
/// log order, then commits. A visible copy no logged `Write` explains, as a
/// real-process test logs a copy in a process it does not instrument, is
/// explained by a logged `Commit` of its key and value, and is otherwise
/// stale when its epoch is below the greatest epoch fenced.
pub fn fencing<R: Serialize + Clone>(
    log: &ObservationLog<R>,
    class: GuaranteeClass,
) -> Result<(), Violation<R>> {
    let records = log.records();
    let delivery = Delivery::read(records);
    let violation = |kind, positions| violation(records, kind, positions);
    let mut first_copy: BTreeMap<&str, usize> = BTreeMap::new();
    for shown in &delivery.visible {
        let sources: Vec<&Written<'_>> = delivery
            .written
            .iter()
            .filter(|written| written.copy == shown.copy)
            .collect();
        if sources.is_empty()
            && let Some((kind, positions)) = unexplained(&delivery, shown, class)
        {
            return Err(violation(kind, positions));
        }
        let stale = (!sources.is_empty()
            && sources.iter().all(|written| written.stale_since.is_some()))
        .then(|| sources[0]);
        match (class, stale) {
            (GuaranteeClass::Fenced, Some(written)) => {
                let fence = written
                    .stale_since
                    .expect("invariant: a stale write has its fence");
                return Err(violation(
                    ViolationKind::StaleRecordVisible,
                    vec![fence, written.position, shown.position],
                ));
            }
            (GuaranteeClass::RecordFenced, Some(written)) => {
                let current = delivery
                    .written
                    .iter()
                    .find(|other| other.copy.key == shown.copy.key && other.stale_since.is_none());
                if let Some(current) = current {
                    return Err(violation(
                        ViolationKind::StaleOverwrite,
                        vec![current.position, written.position, shown.position],
                    ));
                }
            }
            _ => {}
        }
        if matches!(class, GuaranteeClass::Fenced | GuaranteeClass::Keyed)
            && let Some(&first) = first_copy.get(shown.copy.key)
        {
            return Err(violation(
                ViolationKind::DuplicateKey,
                vec![first, shown.position],
            ));
        }
        first_copy.entry(shown.copy.key).or_insert(shown.position);
    }
    for committed in &delivery.committed {
        let shown = delivery
            .visible
            .iter()
            .any(|shown| shown.copy.key == committed.key && shown.copy.digest == committed.digest);
        if !shown {
            return Err(violation(
                ViolationKind::CommitMissing,
                vec![committed.position],
            ));
        }
    }
    Ok(())
}

/// The violation, if any, of a visible copy no logged `Write` explains: a
/// stale one under Fenced, or a stale one over a key a current copy wrote or
/// a commit holds another value of under Record-fenced. A logged `Commit` of
/// its key and value explains it.
fn unexplained(
    delivery: &Delivery<'_>,
    shown: &Shown<'_>,
    class: GuaranteeClass,
) -> Option<(ViolationKind, Vec<usize>)> {
    let committed = delivery
        .committed
        .iter()
        .any(|committed| committed.key == shown.copy.key && committed.digest == shown.copy.digest);
    let (fence, epoch) = delivery.greatest_fence?;
    if committed || shown.copy.epoch >= epoch {
        return None;
    }
    match class {
        GuaranteeClass::Fenced => Some((
            ViolationKind::StaleRecordVisible,
            vec![fence, shown.position],
        )),
        GuaranteeClass::RecordFenced => delivery
            .written
            .iter()
            .find(|other| other.copy.key == shown.copy.key && other.stale_since.is_none())
            .map(|current| current.position)
            .or_else(|| {
                delivery
                    .committed
                    .iter()
                    .find(|committed| committed.key == shown.copy.key)
                    .map(|committed| committed.position)
            })
            .map(|current| (ViolationKind::StaleOverwrite, vec![current, shown.position])),
        GuaranteeClass::Keyed | GuaranteeClass::AtLeastOnce => None,
    }
}

/// A log's delivery records, read once.
struct Delivery<'a> {
    written: Vec<Written<'a>>,
    committed: Vec<Committed<'a>>,
    visible: Vec<Shown<'a>>,
    /// The position and epoch of the first fence of the greatest epoch.
    greatest_fence: Option<(usize, Epoch)>,
}

/// One copy of a record: its epoch, key and digest.
#[derive(PartialEq, Eq)]
struct RecordCopy<'a> {
    epoch: Epoch,
    key: &'a str,
    digest: &'a ValueDigest,
}

/// A `Write`, and the position of the fence that made it stale, if any.
struct Written<'a> {
    position: usize,
    copy: RecordCopy<'a>,
    stale_since: Option<usize>,
}

struct Committed<'a> {
    position: usize,
    key: &'a str,
    digest: &'a ValueDigest,
}

struct Shown<'a> {
    position: usize,
    copy: RecordCopy<'a>,
}

impl<'a> Delivery<'a> {
    fn read<R>(records: &'a [Record<R>]) -> Self {
        let mut delivery = Self {
            written: Vec::new(),
            committed: Vec::new(),
            visible: Vec::new(),
            greatest_fence: None,
        };
        let mut current: Option<(usize, Epoch)> = None;
        for (position, record) in records.iter().enumerate() {
            match &record.operation {
                Operation::Fence { epoch } => {
                    if current.is_none_or(|(_, current)| *epoch > current) {
                        current = Some((position, *epoch));
                    }
                }
                Operation::Write { epoch, key, digest } => delivery.written.push(Written {
                    position,
                    copy: RecordCopy {
                        epoch: *epoch,
                        key,
                        digest,
                    },
                    stale_since: current
                        .filter(|(_, current)| epoch < current)
                        .map(|(fence, _)| fence),
                }),
                Operation::Commit { key, digest } => delivery.committed.push(Committed {
                    position,
                    key,
                    digest,
                }),
                Operation::Visible { epoch, key, digest } => delivery.visible.push(Shown {
                    position,
                    copy: RecordCopy {
                        epoch: *epoch,
                        key,
                        digest,
                    },
                }),
                _ => {}
            }
        }
        delivery.greatest_fence = current;
        delivery
    }
}

/// Judges a log's Usage: on each scope, charged plus reserved Usage never
/// exceeds its Limit by more than `allowed_loss`, the Usage ADR 0055 allows
/// to be lost with the Lead Daemon after its last Group backup. A scope with
/// no `Limit` record is not judged.
pub fn limits<R: Serialize + Clone>(
    log: &ObservationLog<R>,
    allowed_loss: u64,
) -> Result<(), Violation<R>> {
    let records = log.records();
    let mut scopes: BTreeMap<&str, Scope> = BTreeMap::new();
    for (position, record) in records.iter().enumerate() {
        let scope = match &record.operation {
            Operation::Limit { scope, usage } => {
                let entry = scopes.entry(scope).or_default();
                entry.limit = Some((position, *usage));
                entry
            }
            Operation::Reserve { scope, usage } => {
                let entry = scopes.entry(scope).or_default();
                entry.reserved = entry.reserved.saturating_add(*usage);
                entry.positions.push(position);
                entry
            }
            Operation::Settle {
                scope,
                reserved,
                charged,
            } => {
                let entry = scopes.entry(scope).or_default();
                entry.positions.push(position);
                let Some(left) = entry.reserved.checked_sub(*reserved) else {
                    return Err(violation(
                        records,
                        ViolationKind::OverSettled,
                        entry.positions.clone(),
                    ));
                };
                entry.reserved = left;
                entry.charged = entry.charged.saturating_add(*charged);
                entry
            }
            _ => continue,
        };
        let Some((limit_position, limit)) = scope.limit else {
            continue;
        };
        if scope.charged.saturating_add(scope.reserved) > limit.saturating_add(allowed_loss) {
            let mut positions = scope.positions.clone();
            positions.push(limit_position);
            return Err(violation(records, ViolationKind::LimitExceeded, positions));
        }
    }
    Ok(())
}

/// A violation shown by the records at these positions, put in log order.
fn violation<R: Clone>(
    records: &[Record<R>],
    kind: ViolationKind,
    mut positions: Vec<usize>,
) -> Violation<R> {
    positions.sort_unstable();
    Violation {
        kind,
        records: positions
            .into_iter()
            .map(|position| records[position].clone())
            .collect(),
    }
}

/// One scope's Usage so far, and the records that changed it.
#[derive(Default)]
struct Scope {
    limit: Option<(usize, u64)>,
    reserved: u64,
    charged: u64,
    positions: Vec<usize>,
}

#[cfg(test)]
mod tests;
