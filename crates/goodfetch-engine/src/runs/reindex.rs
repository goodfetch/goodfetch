//! `runs reindex` (ADRs 0031 and 0036): the Run index rebuilt from the run
//! folders of the locations it is given, each Run's row from its own state.
//! A row whose state it does not find is kept as *missing*, never dropped,
//! and a row marked *erased*, a Run discarded on purpose, stays erased. A
//! found Run keeps the owner its row named only while a reader's test finds
//! its writer live: a dead engine holds no lock, so its stale row never
//! blocks anyone.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::{Presence, RunId, RunIndexRow, RunReport};

use super::folder::read_recorded;
use super::writer::{Writer, test_writer};
use super::{RUN_FILE, location};
use crate::home::Home;
use crate::run_index::{self, IndexFile};
use crate::run_state::RunStateError;
use crate::sqlite_store::StoreFile as _;

/// What a rebuild left in the Run index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reindexed {
    /// The Run index's path.
    pub run_index: PathBuf,
    /// Rows of Runs whose state it found.
    pub present: u64,
    /// Rows it kept whose state it did not find.
    pub missing: u64,
    /// Rows of Runs discarded on purpose, kept erased.
    pub erased: u64,
}

/// Rebuilds the Run index of `version`'s major in `home` from the Runs in
/// `locations`: each is a Run's folder, or a folder of Run folders such as
/// the per-major data folder's `runs`. A Run found in two locations keeps
/// the first. It blocks: run it under a blocking policy, or from a
/// command's own thread.
///
/// # Errors
///
/// When the Run index cannot be read or written; it is then left as it
/// was.
pub fn reindex(
    home: &Home,
    version: &ProductVersion,
    locations: &[PathBuf],
) -> Result<Reindexed, RunStateError> {
    let index = IndexFile::of(home.run_index(version), version);
    let connection = index.open()?;
    let per_major = home.per_major(version);
    let listed: BTreeMap<RunId, RunIndexRow> = index
        .rows(&connection)?
        .into_iter()
        .map(|row| (row.run_id.clone(), row))
        .collect();
    let mut rows: BTreeMap<RunId, RunIndexRow> = BTreeMap::new();
    for folder in locations.iter().flat_map(|location| run_folders(location)) {
        let Some(row) = found_row(&folder, &per_major, &listed) else {
            continue;
        };
        rows.entry(row.run_id.clone()).or_insert(row);
    }
    for (run_id, row) in &listed {
        let kept = match (row.presence, rows.contains_key(run_id)) {
            (Presence::Erased, _) => Some(run_index::kept_as(row.clone(), Presence::Erased)),
            (_, false) => Some(run_index::kept_as(row.clone(), Presence::Missing)),
            (_, true) => None,
        };
        if let Some(kept) = kept {
            rows.insert(run_id.clone(), kept);
        }
    }
    let rows: Vec<RunIndexRow> = rows.into_values().collect();
    index.replace(&connection, &rows)?;
    let count = |presence: Presence| {
        u64::try_from(rows.iter().filter(|row| row.presence == presence).count())
            .unwrap_or(u64::MAX)
    };
    Ok(Reindexed {
        run_index: index.path().to_owned(),
        present: count(Presence::Present),
        missing: count(Presence::Missing),
        erased: count(Presence::Erased),
    })
}

/// The Run folders `location` holds: itself when it holds a Run's state,
/// else each of its folders that does.
fn run_folders(location: &Path) -> Vec<PathBuf> {
    if location.join(RUN_FILE).is_file() {
        return vec![location.to_owned()];
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "reindex blocks, and its caller runs it where blocking is allowed"
    )]
    let entries = match std::fs::read_dir(location) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(%error, location = %location.display(), "a run location cannot be read");
            return Vec::new();
        }
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|folder| folder.join(RUN_FILE).is_file())
        .collect();
    folders.sort();
    folders
}

/// The row of the Run whose state `folder` holds, or `None` when that
/// state cannot be read as a Run's.
fn found_row(
    folder: &Path,
    per_major: &Path,
    listed: &BTreeMap<RunId, RunIndexRow>,
) -> Option<RunIndexRow> {
    let recorded = match read_recorded(&folder.join(RUN_FILE), None) {
        Ok(recorded) => recorded?,
        Err(error) => {
            tracing::warn!(%error, folder = %folder.display(), "a run file cannot be read; its row is not rebuilt");
            return None;
        }
    };
    let live = matches!(test_writer(folder), Ok(Writer::Live));
    let owner = listed
        .get(&recorded.run_id)
        .and_then(|row| row.owner.clone())
        .filter(|_| live);
    let report = recorded.report.as_ref();
    Some(RunIndexRow {
        run_id: recorded.run_id.clone(),
        location: location(per_major, folder),
        kind: recorded.kind,
        needs: recorded.needs,
        tags: recorded.tags.clone(),
        lifecycle: recorded.lifecycle,
        verdict: report.map(|report| report.verdict),
        completeness: report.map(|report| report.completeness),
        started_at: recorded.started_at.clone(),
        ended_at: report.map(|report| report.ended_at.clone()),
        owner,
        presence: Presence::Present,
        progress_message: recorded.progress_message.clone(),
        stop_reason: report.and_then(|report| report.stop_reason.clone()),
        replayed: report.is_some_and(RunReport::replayed),
    })
}
