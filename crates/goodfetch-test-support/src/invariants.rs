//! The invariant checkers (ADR 0060): functions over an Observation log,
//! each returning the first violation with the records that show it, which
//! the Simulation, real-process tests and conformance suites share: fencing,
//! judged by Guarantee class, one lead, Limits, Partitions, the Audit log's
//! chain, checked by the product's verifier passed in as a value, and
//! bounded liveness.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Bound;

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
    /// Two hosts acted as Lead Daemon under one lead epoch.
    TwoLeads,
    /// A host acted as Lead Daemon under a lead epoch below one another
    /// host had already acted under.
    StaleLead,
    /// A Schedule fired twice for one occurrence.
    ScheduleFiredTwice,
    /// A URL joined a Partition's Frontier while another Partition's held
    /// it.
    UrlInTwoFrontiers,
    /// A discovered URL is in no Partition's Frontier at the log's end.
    UrlLost,
    /// A Partition admitted a request to a domain another Partition held.
    DomainHeldTwice,
    /// An Audit log entry does not verify from the chain's start: the
    /// verifier refuses it, or it chains to, or merges, no entry of the log.
    AuditChainBroken,
    /// The Audit log has more than one head: a branch no merge entry
    /// carries.
    AuditBranchUnmerged,
    /// A run neither finished nor became resumable within its bound.
    ProgressMissing,
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

/// Judges a log's leads (ADR 0055): no two hosts act as Lead Daemon under
/// one lead epoch, none acts under an epoch below one another host already
/// acted under, and a Schedule fires once for each occurrence.
pub fn one_lead<R: Serialize + Clone>(log: &ObservationLog<R>) -> Result<(), Violation<R>> {
    let records = log.records();
    let violation = |kind, positions| Err(violation(records, kind, positions));
    // The first record of each lead epoch, and of each Schedule occurrence.
    let mut leads: BTreeMap<Epoch, usize> = BTreeMap::new();
    let mut fired: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for (position, record) in records.iter().enumerate() {
        match &record.operation {
            Operation::Lead { epoch } => {
                let first = *leads.entry(*epoch).or_insert(position);
                if records[first].host != record.host {
                    return violation(ViolationKind::TwoLeads, vec![first, position]);
                }
                let newest = leads
                    .range((Bound::Excluded(*epoch), Bound::Unbounded))
                    .next_back();
                if let Some((_, &newest)) = newest
                    && records[newest].host != record.host
                {
                    return violation(ViolationKind::StaleLead, vec![newest, position]);
                }
            }
            Operation::Fired {
                schedule,
                occurrence,
            } => {
                let first = *fired.entry((schedule, occurrence)).or_insert(position);
                if first != position {
                    return violation(ViolationKind::ScheduleFiredTwice, vec![first, position]);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Judges a Partitioned crawl's logs (ADR 0054): a URL is in one
/// Partition's Frontier at a time and every discovered URL is in one at the
/// log's end, and a domain a Partition admitted is held by it alone until it
/// hands it off, its URLs leaving its Frontier with it.
pub fn partitions<R: Serialize + Clone>(log: &ObservationLog<R>) -> Result<(), Violation<R>> {
    let records = log.records();
    let violation = |kind, positions| Err(violation(records, kind, positions));
    // Each URL's first discovery, in log order.
    let mut discovered: Vec<(usize, &str)> = Vec::new();
    // The Frontier each URL is in: the record that put it there, its
    // Partition and its domain.
    let mut frontiers: BTreeMap<&str, (usize, &str, &str)> = BTreeMap::new();
    // The Partition holding each domain, and the admission it began with.
    let mut held: BTreeMap<&str, (usize, &str)> = BTreeMap::new();
    for (position, record) in records.iter().enumerate() {
        match &record.operation {
            Operation::Discovered { url } => {
                if discovered.iter().all(|(_, known)| known != url) {
                    discovered.push((position, url));
                }
            }
            Operation::Frontier {
                partition,
                domain,
                url,
            } => {
                let (first, holder, _) = *frontiers
                    .entry(url)
                    .or_insert((position, partition, domain));
                if holder != partition {
                    return violation(ViolationKind::UrlInTwoFrontiers, vec![first, position]);
                }
            }
            Operation::Admitted { partition, domain } => {
                let (first, holder) = *held.entry(domain).or_insert((position, partition));
                if holder != partition {
                    return violation(ViolationKind::DomainHeldTwice, vec![first, position]);
                }
            }
            Operation::HandedOff { partition, domain } => {
                if held
                    .get(domain.as_str())
                    .is_some_and(|(_, holder)| holder == partition)
                {
                    held.remove(domain.as_str());
                }
                frontiers.retain(|_, (_, holder, of)| !(holder == partition && of == domain));
            }
            _ => {}
        }
    }
    match discovered
        .into_iter()
        .find(|(_, url)| !frontiers.contains_key(url))
    {
        Some((position, _)) => violation(ViolationKind::UrlLost, vec![position]),
        None => Ok(()),
    }
}

/// Judges the Audit log's chain (ADRs 0016 and 0055): every entry stays
/// verifiable from the chain's start. The entries form a tree, each chained
/// to an earlier one by its hash, a Group member's branch forking from the
/// last lead hash it saw; `verify`, the product's chain verifier passed in
/// as a value, must accept the entries of each path from the first entry to
/// a head, and every head but one must be merged by a merge entry. On a
/// path it refuses, the entry its shortest refused start ends with is shown,
/// with the one it chains to.
pub fn audit_chain<R: Serialize + Clone>(
    log: &ObservationLog<R>,
    verify: impl Fn(&[&str]) -> bool,
) -> Result<(), Violation<R>> {
    let records = log.records();
    let violation = |kind, positions| Err(violation(records, kind, positions));
    let mut links: Vec<Link<'_>> = Vec::new();
    // The link of each hash.
    let mut by_hash: BTreeMap<&str, usize> = BTreeMap::new();
    let mut merged: BTreeSet<usize> = BTreeSet::new();
    for (position, record) in records.iter().enumerate() {
        let Operation::Audit {
            hash,
            previous,
            entry,
            merges,
        } = &record.operation
        else {
            continue;
        };
        let parent = by_hash.get(previous.as_str()).copied();
        if parent.is_none() && !links.is_empty() {
            return violation(ViolationKind::AuditChainBroken, vec![position]);
        }
        if let Some(head) = merges {
            let Some(&head) = by_hash.get(head.as_str()) else {
                return violation(ViolationKind::AuditChainBroken, vec![position]);
            };
            merged.insert(head);
        }
        by_hash.insert(hash, links.len());
        links.push(Link {
            position,
            parent,
            entry,
        });
    }
    let mut chained_to: BTreeSet<usize> = BTreeSet::new();
    chained_to.extend(links.iter().filter_map(|link| link.parent));
    let heads: Vec<usize> = (0..links.len())
        .filter(|link| !chained_to.contains(link))
        .collect();
    for &head in &heads {
        let mut path = vec![head];
        while let Some(parent) = links[path[path.len() - 1]].parent {
            path.push(parent);
        }
        path.reverse();
        let entries: Vec<&str> = path.iter().map(|&link| links[link].entry).collect();
        if verify(&entries) {
            continue;
        }
        // The shortest start of the path the verifier refuses: every
        // start of a chain it accepts verifies too.
        let (mut accepted, mut refused) = (0, entries.len());
        while refused - accepted > 1 {
            let middle = accepted + (refused - accepted) / 2;
            if verify(&entries[..middle]) {
                accepted = middle;
            } else {
                refused = middle;
            }
        }
        let shown = path[refused.saturating_sub(2)..refused]
            .iter()
            .map(|&link| links[link].position)
            .collect();
        return violation(ViolationKind::AuditChainBroken, shown);
    }
    let unmerged: Vec<usize> = heads
        .into_iter()
        .filter(|head| !merged.contains(head))
        .map(|head| links[head].position)
        .collect();
    if unmerged.len() > 1 {
        return violation(ViolationKind::AuditBranchUnmerged, unmerged);
    }
    Ok(())
}

/// An Audit log entry of the log: its position, the link it chains to and
/// its text.
struct Link<'a> {
    position: usize,
    parent: Option<usize>,
    entry: &'a str,
}

/// Judges bounded liveness (ADR 0060): once faults stop at the simulated
/// millisecond `after`, every run finishes or becomes resumable within
/// `within` of `after`, or of its last start when that is later. A run
/// still unsettled at the log's end fails. A record with no simulated time,
/// as a real-process test's, counts as in time: such a test bounds its own
/// wait.
pub fn liveness<R: Serialize + Clone>(
    log: &ObservationLog<R>,
    after: u64,
    within: u64,
) -> Result<(), Violation<R>> {
    let records = log.records();
    // Each run's last start, and the first finish or checkpoint after it.
    let mut runs: BTreeMap<&str, (usize, Option<usize>)> = BTreeMap::new();
    for (position, record) in records.iter().enumerate() {
        match &record.operation {
            Operation::Started { run } => {
                runs.insert(run, (position, None));
            }
            Operation::Finished { run } | Operation::Resumable { run } => {
                if let Some((_, settled @ None)) = runs.get_mut(run.as_str()) {
                    *settled = Some(position);
                }
            }
            _ => {}
        }
    }
    let mut runs: Vec<(usize, Option<usize>)> = runs.into_values().collect();
    runs.sort_unstable();
    for (start, settled) in runs {
        let bound = records[start]
            .at
            .map_or(after, |at| at.max(after))
            .saturating_add(within);
        let shown = match settled {
            None => vec![start],
            Some(settled) if records[settled].at.is_some_and(|at| at > bound) => {
                vec![start, settled]
            }
            Some(_) => continue,
        };
        return Err(violation(records, ViolationKind::ProgressMissing, shown));
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
