//! What the runs registry keeps of the Runs this engine ended (#1729,
//! #1768): nothing of a Run whose report its run file holds, which a read
//! takes from there, so a Daemon that runs Run after Run holds nothing of
//! each for its life. It keeps the report of a Run whose end could not be
//! written to its run file, being the only one there is, and the last
//! Checkpoint this engine wrote of each of the latest [`CHECKPOINTS_KEPT`]
//! Runs that ended with one, which a run file cannot name. A later read of
//! an older one lacks its Checkpoint, as a read after a restart does (ADR
//! 0102's note).

use std::collections::{BTreeMap, VecDeque};

use goodfetch_protocol::run::{Checkpoint, RunId, RunReport};

/// How many ended Runs' Checkpoints the registry keeps: a later
/// `run.finish` or `run.report` of one of them carries it, as the
/// governor's last-resort stop is read (ADR 0040). About 300 bytes each.
pub(super) const CHECKPOINTS_KEPT: usize = 1024;

/// What the registry keeps of one Run this engine ended.
#[derive(Debug)]
pub(super) enum EndedHere {
    /// Its report is in its run file; a read of it adds this, the last
    /// Checkpoint this engine wrote of it.
    Checkpointed(Checkpoint),
    /// Its end could not be written to its run file: this is its report.
    Unwritten(Box<RunReport>),
}

/// The Runs this engine ended that it keeps something of.
#[derive(Debug)]
pub(super) struct EndedRuns {
    kept: BTreeMap<RunId, EndedHere>,
    /// The Checkpointed ones, the latest last.
    checkpointed: VecDeque<RunId>,
    /// How many Checkpointed ones it keeps.
    most: usize,
}

impl EndedRuns {
    /// None yet, keeping [`CHECKPOINTS_KEPT`] Checkpoints.
    pub(super) fn new() -> Self {
        Self::keeping(CHECKPOINTS_KEPT)
    }

    fn keeping(most: usize) -> Self {
        Self {
            kept: BTreeMap::new(),
            checkpointed: VecDeque::new(),
            most,
        }
    }

    /// The Run `run_id` ended, its report in its run file, `checkpoint` the
    /// last Checkpoint this engine wrote of it: it keeps that alone, and
    /// forgets the oldest kept past its bound.
    pub(super) fn in_run_file(&mut self, run_id: RunId, checkpoint: Option<Checkpoint>) {
        self.forget(&run_id);
        let Some(checkpoint) = checkpoint else {
            return;
        };
        self.checkpointed.push_back(run_id.clone());
        self.kept
            .insert(run_id, EndedHere::Checkpointed(checkpoint));
        while self.checkpointed.len() > self.most {
            if let Some(oldest) = self.checkpointed.pop_front() {
                self.kept.remove(&oldest);
            }
        }
    }

    /// The Run `run_id` ended, and its end could not be written to its run
    /// file: `report` is its report.
    pub(super) fn unwritten(&mut self, run_id: RunId, report: Box<RunReport>) {
        self.forget(&run_id);
        self.kept.insert(run_id, EndedHere::Unwritten(report));
    }

    /// What it keeps of `run_id`.
    pub(super) fn get(&self, run_id: &RunId) -> Option<&EndedHere> {
        self.kept.get(run_id)
    }

    /// Whether it keeps anything of `run_id`.
    pub(super) fn contains(&self, run_id: &RunId) -> bool {
        self.kept.contains_key(run_id)
    }

    /// Forgets `run_id`, as once it resumes or is discarded.
    pub(super) fn forget(&mut self, run_id: &RunId) {
        if let Some(EndedHere::Checkpointed(_)) = self.kept.remove(run_id) {
            self.checkpointed.retain(|kept| kept != run_id);
        }
    }
}

#[cfg(test)]
mod tests;
