//! Run-state import (ADRs 0002 and 0067): a per-run file another machine
//! or the other layout wrote, which `run.resume` takes as a Checkpoint and
//! makes a Run's run file, is input Goodfetch did not write here. Its fuzz
//! target's body runs the import as a resume does: the format and the Run
//! it holds read, the file restored as a run file, the next run epoch
//! taken, then what the run file records read back.
#![expect(
    clippy::disallowed_methods,
    reason = "the fuzz body writes its input to a file and runs the import's blocking calls outside any async worker"
)]

use crate::checkpoint;
use crate::run_state::{self, RunStateOptions};

use super::RUN_FILE;
use super::folder::read_recorded;

/// The fuzz target's body for run-state import (ADR 0067): imports `data`
/// as a per-run file, in a temporary folder, as `run.resume` imports a
/// Checkpoint; it may refuse it, never panic.
pub fn fuzz_run_state_import(data: &[u8]) {
    let Ok(folder) = tempfile::tempdir() else {
        return;
    };
    let imported = folder.path().join("imported.sqlite");
    if std::fs::write(&imported, data).is_err() {
        return;
    }
    let Ok(reader) = checkpoint::open_checkpoint(&imported) else {
        return;
    };
    let held = checkpoint::read(&reader, &imported);
    drop(reader);
    if held.is_err() {
        return;
    }
    let run_file = folder.path().join("run").join(RUN_FILE);
    if checkpoint::install(&imported, &run_file, None).is_err() {
        return;
    }
    if run_state::next_epoch(&run_file, &RunStateOptions::default(), &[], 0).is_err() {
        return;
    }
    let _ = read_recorded(&run_file, None);
}

#[cfg(test)]
mod tests;
