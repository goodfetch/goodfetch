//! The earlier runs a plan reads, found among the records by ancestry (ADR
//! 0135): the nearest run on the head or one of its ancestors, whose green
//! results the plan may reuse, and the last full gate `main` holds, where the
//! stress run's range starts once quick merges came after it.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};

use super::plan::{GateRun, Step, unit};
use super::{plan_list, plan_value, runs};
use crate::affected::{self, Range, WorkspaceMembers};
use crate::git;

/// The record of the nearest run on `head` or one of its ancestors: the
/// fewest commits away.
pub(super) fn nearest(root: &Path, records: &Path, head: &str) -> Result<Option<GateRun>> {
    recorded_ancestors(root, records, head)?
        .first()
        .map(|candidate| runs::read(records, candidate))
        .transpose()
}

/// The heads with a record that are `tip` or one of its ancestors, the fewest
/// commits from it first.
fn recorded_ancestors(root: &Path, records: &Path, tip: &str) -> Result<Vec<String>> {
    let mut ancestors = Vec::new();
    for candidate in runs::heads(records)? {
        let ancestor = git::succeeds(
            root,
            &[
                "merge-base",
                "--is-ancestor",
                "--end-of-options",
                &candidate,
                tip,
            ],
        )?;
        if !ancestor {
            continue;
        }
        let distance: u64 = git::line(
            root,
            &["rev-list", "--count", &format!("{candidate}..{tip}")],
        )?
        .parse()
        .context("reading git rev-list's count")?;
        ancestors.push((distance, candidate));
    }
    ancestors.sort();
    Ok(ancestors.into_iter().map(|(_, head)| head).collect())
}

/// What the stress run of a run compares its head with: the head of the last
/// full gate, or the change's base and why.
pub(super) enum StressBase {
    /// The head of the last full gate whose stress run passed: a test reads
    /// what the change's base gained since.
    LastFullGate(String),
    /// The change's base, which is the run's own head, as the backstop's on
    /// `main`'s head: it gates no change, each merge it holds was judged by
    /// its own gate, and its stress run selects nothing.
    RunOnTheBase,
    /// The change's base, for want of a record: none on a head the base holds
    /// shows a stress run that passed, so the tests merged on the quick gate
    /// since the last full gate are not in the range.
    NoFullGateRecorded,
    /// The change's base, since no test reads what it gained since the last
    /// full gate (`xtask affected`'s `gate=docs` or `gate=quick`, by which
    /// `just merge` gives such a change no stress run): the selection would be
    /// the same, and the base's saved listings, which `just merge` carries
    /// across such merges, serve it.
    NoTestReadsWhatMerged,
}

impl StressBase {
    /// The head that replaces the change's base, when one does.
    pub(super) fn last_full_gate(&self) -> Option<&str> {
        match self {
            Self::LastFullGate(head) => Some(head),
            Self::RunOnTheBase | Self::NoFullGateRecorded | Self::NoTestReadsWhatMerged => None,
        }
    }
}

/// The head the stress run of the run on `head` compares it with in place of
/// the change's `base`, so that it covers what merged on the quick gate since
/// the last full gate (ADR 0135's amendment of 2026-11-10): the nearest head
/// `base` holds, which `main` therefore holds too, whose record shows its
/// stress run passed, so every test up to it has had its runs. A record on a
/// head of the change itself is none of them: the change's own tests are new
/// since its base, whatever an earlier run on it found.
///
/// Else the change's `base`, with which of [`StressBase`]'s reasons keeps it.
pub(super) fn stress_base(
    root: &Path,
    members: &WorkspaceMembers,
    records: &Path,
    head: &str,
    base: &str,
) -> Result<StressBase> {
    if head == base {
        return Ok(StressBase::RunOnTheBase);
    }
    let Some(last_full_gate) = recorded_ancestors(root, records, base)?
        .into_iter()
        .find(|candidate| stress_passed(records, candidate))
    else {
        return Ok(StressBase::NoFullGateRecorded);
    };
    let merged_since = Range::Commits {
        base: last_full_gate.clone(),
        head: base.to_owned(),
    };
    let a_test_reads_it =
        affected::select_among(root, members, &merged_since)?.takes_the_full_gate();
    Ok(if a_test_reads_it {
        StressBase::LastFullGate(last_full_gate)
    } else {
        StressBase::NoTestReadsWhatMerged
    })
}

/// Whether the record of the run on `head` shows that its stress run passed:
/// each member its plan lists has a green stress result, run there or carried.
/// A run stopped before its stress run ended has none for the members it was
/// rerunning, and a plan or record that is missing or cannot be read shows
/// nothing.
fn stress_passed(records: &Path, head: &str) -> bool {
    let Ok(plan) = std::fs::read_to_string(runs::plan_file(records, head)) else {
        return false;
    };
    let (Ok(members), Ok(run)) = (plan_value(&plan, "members"), runs::read(records, head)) else {
        return false;
    };
    let members = plan_list(members);
    !members.is_empty()
        && members.iter().all(|member| {
            run.units
                .get(&unit(Step::Stress, Some(member)))
                .is_some_and(|outcome| outcome.green)
        })
}
