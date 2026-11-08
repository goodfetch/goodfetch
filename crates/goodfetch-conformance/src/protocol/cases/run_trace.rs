//! The one reader of a Run's trace, the JSON lines at
//! `<home>/<major>/runs/<run_id>/trace/`. A case's checks judge what the Run
//! recorded, so a file the reader cannot read or a line it cannot parse is an
//! [`Unread`], which the case reports through [`Checks::run_trace`]: read as
//! nothing, a check that something was never sent would pass on the loss.
//!
//! [`Checks::run_trace`]: super::Checks::run_trace

use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::home::{RUN_TRACE_PROVENANCE, RUN_TRACE_REQUESTS, RUNS};
use serde_json::Value;

use crate::protocol::target::EngineTarget;

/// Why a case has no Run trace to judge.
#[derive(Debug, thiserror::Error)]
pub(super) enum Unread {
    /// The target's environment names no home to find the trace in.
    #[error("the target's environment names no Goodfetch home")]
    NoHome,
    /// A folder or file of the home could not be read.
    #[error("{}: {source}", path.display())]
    Unreadable {
        /// What could not be read.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// A whole line of the file holds no JSON.
    #[error("line {line} of {}: {source}", path.display())]
    BadLine {
        /// The file.
        path: PathBuf,
        /// The line, counted from 1.
        line: usize,
        /// Why.
        source: serde_json::Error,
    },
}

/// The lines of the Run `run_id`'s request log, under the Goodfetch home
/// `target` names.
pub(super) fn requests(target: &EngineTarget, run_id: &str) -> Result<Vec<Value>, Unread> {
    lines_of(target, run_id, RUN_TRACE_REQUESTS)
}

/// The lines of the Run `run_id`'s Provenance log, under the Goodfetch home
/// `target` names.
pub(super) fn provenance(target: &EngineTarget, run_id: &str) -> Result<Vec<Value>, Unread> {
    lines_of(target, run_id, RUN_TRACE_PROVENANCE)
}

/// The lines of the Run trace's file `file`, by its path in the run folder, of
/// the Run `run_id`, under the Goodfetch home `target` names.
pub(super) fn lines_of(
    target: &EngineTarget,
    run_id: &str,
    file: &str,
) -> Result<Vec<Value>, Unread> {
    lines(
        target.environment.get("GOODFETCH_HOME").map(String::as_str),
        run_id,
        file,
    )
}

/// The lines of the Run trace's file `file`, by its path in the run folder, of
/// the Run `run_id`, under the Goodfetch home `home`, in each major folder
/// that holds it. A folder that holds no such file has none: the Run has not
/// written it, or ran under another major.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
pub(super) fn lines(home: Option<&str>, run_id: &str, file: &str) -> Result<Vec<Value>, Unread> {
    let home = Path::new(home.ok_or(Unread::NoHome)?);
    let unreadable = |source| Unread::Unreadable {
        path: home.to_path_buf(),
        source,
    };
    let mut lines = Vec::new();
    for major in std::fs::read_dir(home).map_err(unreadable)? {
        let path = major
            .map_err(unreadable)?
            .path()
            .join(RUNS)
            .join(run_id)
            .join(file);
        match std::fs::read(&path) {
            Ok(written) => lines.extend(whole_lines(&path, &written)?),
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) => {}
            Err(source) => return Err(Unread::Unreadable { path, source }),
        }
    }
    Ok(lines)
}

/// The JSON of each line `written` ends: what follows the last newline is a
/// line a live Run is still writing, which a later read finds whole.
fn whole_lines(path: &Path, written: &[u8]) -> Result<Vec<Value>, Unread> {
    written
        .split_inclusive(|byte| *byte == b'\n')
        .take_while(|line| line.ends_with(b"\n"))
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_slice(line).map_err(|source| Unread::BadLine {
                path: path.to_path_buf(),
                line: index + 1,
                source,
            })
        })
        .collect()
}
