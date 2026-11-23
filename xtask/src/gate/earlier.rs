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

use super::plan::{GateRun, Head, Layer, Step, unit};
use super::{plan_list, plan_value, runs};
use crate::affected::{self, Range, WorkspaceMembers};
use crate::git;

/// The nearest run a plan reads, and the nearer records it passed over.
struct Nearest {
    record: Option<GateRun>,
    /// Each nearer record of another host fingerprint, by its head, with
    /// the layers that ran its results.
    other_hosts: Vec<(String, Vec<Layer>)>,
}

/// The record of the nearest run on `head` or one of its ancestors, the
/// fewest commits away, among those of this `host`: every layer of every
/// worktree records into the one folder (ADR 0135's amendment of
/// 2026-11-22), and a result of another host fingerprint is never reused, so
/// a nearer record of another host would leave the plan nothing to reuse.
/// With no record of this host, the nearest record of any, whose plan then
/// says the host changed since it.
fn nearest(root: &Path, records: &Path, head: &str, host: &str) -> Result<Nearest> {
    let mut other_hosts = Vec::new();
    let mut nearest_of_any = None;
    for candidate in recorded_ancestors(root, records, head)? {
        let record = runs::read(records, &candidate)?;
        if record.host == host {
            return Ok(Nearest {
                record: Some(record),
                other_hosts,
            });
        }
        let mut layers: Vec<Layer> = Vec::new();
        for outcome in record.units.values() {
            if !layers.contains(&outcome.by) {
                layers.push(outcome.by);
            }
        }
        other_hosts.push((candidate, layers));
        nearest_of_any.get_or_insert(record);
    }
    Ok(Nearest {
        record: nearest_of_any,
        other_hosts: Vec::new(),
    })
}

/// The earlier run a plan reads.
pub(super) struct Previous {
    pub(super) record: Option<GateRun>,
    /// Why there is none, when there is none.
    pub(super) why_none: String,
    /// The nearer records passed over, of another host fingerprint, with
    /// their layers.
    pub(super) other_hosts: Vec<(String, Vec<Layer>)>,
}

/// The earlier run a plan on `head` reads, or why it reads none: none for a
/// `full` run, the `assumed` green run for a dry run that names one, else
/// the nearest recorded run on the head or an ancestor, this host's before a
/// nearer one of another host.
pub(super) fn previous(
    root: &Path,
    records: &Path,
    head: &str,
    gated: &Head<'_>,
    full: bool,
    assumed: Option<&str>,
) -> Result<Previous> {
    let none = |why: &str| Previous {
        record: None,
        why_none: why.to_owned(),
        other_hosts: Vec::new(),
    };
    if full {
        return Ok(none("a full run was asked for (GOODFETCH_GATE_FULL)"));
    }
    if let Some(assumed) = assumed {
        let assumed = git::commit(root, assumed)?;
        return Ok(Previous {
            record: Some(super::assumed_green(root, &assumed, gated)?),
            ..none("")
        });
    }
    let Nearest {
        record,
        other_hosts,
    } = nearest(root, records, head, gated.host)?;
    Ok(Previous {
        why_none: if record.is_some() {
            String::new()
        } else {
            "no earlier gate run on this head or an ancestor".to_owned()
        },
        record,
        other_hosts,
    })
}

/// The hosted runs' records under `folder`, as `ci/hosted-records.sh` lays
/// them out, one folder a job's artifact, each `<head>.tsv` a run of its own:
/// those whose head is `tip` or one of its ancestors, the fewest commits from
/// it first. Each job records the units it ran beside those its plan carried,
/// so no record is merged with another.
pub(super) fn hosted(root: &Path, folder: &Path, tip: &str) -> Result<Vec<GateRun>> {
    let mut files = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        if !folder.is_dir() {
            continue;
        }
        for entry in
            std::fs::read_dir(&folder).with_context(|| format!("reading {}", folder.display()))?
        {
            let path = entry
                .with_context(|| format!("reading {}", folder.display()))?
                .path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|extension| extension == "tsv")
                && let Some(head) = path.file_stem().and_then(|stem| stem.to_str())
            {
                files.push((head.to_owned(), path));
            }
        }
    }
    files.sort();
    let mut runs = Vec::new();
    for (head, path) in files {
        let Some(distance) = distance(root, &head, tip)? else {
            continue;
        };
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let run =
            runs::parse(&head, &text).with_context(|| format!("reading {}", path.display()))?;
        runs.push((distance, run));
    }
    runs.sort_by_key(|(distance, _)| *distance);
    Ok(runs.into_iter().map(|(_, run)| run).collect())
}

/// How many commits `tip` is from `candidate`, when `candidate` is `tip` or
/// one of its ancestors; none for any other commit, or one this clone lacks.
fn distance(root: &Path, candidate: &str, tip: &str) -> Result<Option<u64>> {
    let ancestor = git::succeeds(
        root,
        &[
            "merge-base",
            "--is-ancestor",
            "--end-of-options",
            candidate,
            tip,
        ],
    )?;
    if !ancestor {
        return Ok(None);
    }
    Ok(Some(
        git::line(
            root,
            &["rev-list", "--count", &format!("{candidate}..{tip}")],
        )?
        .parse()
        .context("reading git rev-list's count")?,
    ))
}

/// The heads with a record that are `tip` or one of its ancestors, the fewest
/// commits from it first.
fn recorded_ancestors(root: &Path, records: &Path, tip: &str) -> Result<Vec<String>> {
    let mut ancestors = Vec::new();
    for candidate in runs::heads(records)? {
        if let Some(distance) = distance(root, &candidate, tip)? {
            ancestors.push((distance, candidate));
        }
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
    let a_test_reads_it = affected::select_among(root, members, &merged_since)?.is_read_by_a_test();
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
