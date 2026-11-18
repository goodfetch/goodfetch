//! The start table's cases (ADR 0102): a start under a Run ID reaches the
//! Run it names, by what the Run index holds of it. Each case starts a Run
//! under an ID of its own, brings it to the row it checks (live, stopped,
//! finished, or listed with its run file removed from the target's
//! Goodfetch home) and starts it again under that ID.

use std::path::{Path, PathBuf};

use goodfetch_protocol::home::{RUN_FILE, RUNS};
use goodfetch_protocol::run::{RUN_FINISH, RUN_START, RUN_STOP};
use serde_json::{Value, json};

use super::Checks;
use super::consent::walls::major_of;
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// A start table case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    New,
    Stopped,
    Finished,
    Live,
    StateGone,
}

impl Case {
    /// The case `id` names, if it is one of these.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-start-under-a-new-run-id-begins-it" => Self::New,
            "a-start-under-a-stopped-runs-id-resumes-it" => Self::Stopped,
            "a-start-under-a-finished-runs-id-answers-it-as-it-ended" => Self::Finished,
            "a-start-under-a-live-runs-id-is-run-in-use" => Self::Live,
            "a-start-under-a-listed-runs-id-whose-state-is-gone-is-run-state-unavailable" => {
                Self::StateGone
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to `target`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::New => new(connection, checks),
            Self::Stopped => stopped(connection, checks),
            Self::Finished => finished(connection, checks),
            Self::Live => live(connection, checks),
            Self::StateGone => {
                let home = target.environment.get("GOODFETCH_HOME").cloned();
                checks.check(
                    "the target runs in a Goodfetch home of its own",
                    home.is_some(),
                );
                match home {
                    Some(home) => state_gone(connection, &home, checks),
                    None => Ok(()),
                }
            }
        }
    }
}

/// The answer of `run.start` under `run_id`.
fn start(connection: &mut Connection, run_id: &str) -> Result<Value, Gone> {
    let id = connection.request(RUN_START.name, &json!({ "run_id": run_id }))?;
    Ok(connection.answer(id)?.message)
}

/// The answer of `operation` on the Run `run_id`.
fn end(connection: &mut Connection, operation: &str, run_id: &str) -> Result<Value, Gone> {
    let id = connection.request(operation, &json!({ "run_id": run_id }))?;
    Ok(connection.answer(id)?.message)
}

/// Checks that `answer` is `run.start`'s, started as `started_as`.
fn started_as(answer: &Value, started_as: &str, check: &str, checks: &mut Checks) {
    checks.check(
        format!("{check}, `{started_as}` (answered: {answer})"),
        answer["result"]["started_as"] == json!(started_as),
    );
}

/// Checks that `answer` failed with the Error code `code`.
fn failed_with(answer: &Value, code: &str, check: &str, checks: &mut Checks) {
    checks.check(
        format!("{check}, `{code}` (answered: {answer})"),
        answer["error"]["data"]["code"] == json!(code),
    );
}

/// A start under a Run ID no Run has begins the Run.
fn new(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-new";
    let answer = start(connection, run_id)?;
    started_as(
        &answer,
        "new",
        "a start under a new Run ID begins it",
        checks,
    );
    end(connection, RUN_FINISH.name, run_id)?;
    Ok(())
}

/// A start under the ID of a Run `run.stop` stopped resumes it, live again
/// until it finishes.
fn stopped(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-stopped";
    start(connection, run_id)?;
    let stop = end(connection, RUN_STOP.name, run_id)?;
    checks.check(
        "`run.stop` stops the Run",
        stop["result"]["lifecycle"] == json!("stopped"),
    );
    let again = start(connection, run_id)?;
    started_as(
        &again,
        "resumed",
        "a start under the stopped Run's ID resumes it",
        checks,
    );
    let finish = end(connection, RUN_FINISH.name, run_id)?;
    checks.check(
        "the resumed Run finishes",
        finish["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A start under the ID of a finished Run answers it as it ended, and
/// leaves it finished.
fn finished(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-finished";
    start(connection, run_id)?;
    end(connection, RUN_FINISH.name, run_id)?;
    let again = start(connection, run_id)?;
    started_as(
        &again,
        "ended",
        "a start under the finished Run's ID answers it as it ended",
        checks,
    );
    checks.check(
        "the Run it answers is finished",
        again["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A start under the ID of a Run the engine runs fails with `run_in_use`,
/// and leaves that Run live.
fn live(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-live";
    start(connection, run_id)?;
    let again = start(connection, run_id)?;
    failed_with(
        &again,
        "run_in_use",
        "a start under the ID of a Run the engine runs fails",
        checks,
    );
    let finish = end(connection, RUN_FINISH.name, run_id)?;
    checks.check(
        "the live Run still finishes",
        finish["result"]["lifecycle"] == json!("finished"),
    );
    Ok(())
}

/// A start under the ID of a finished Run whose run file was removed, the
/// Run index still listing it, fails with `run_state_unavailable` and
/// begins no Run.
fn state_gone(connection: &mut Connection, home: &str, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-start-state-gone";
    start(connection, run_id)?;
    end(connection, RUN_FINISH.name, run_id)?;
    let folder = major_of(home, run_id).map(|major| major.join(RUNS).join(run_id));
    checks.check(
        "the Run's folder is found in the Goodfetch home",
        folder.is_some(),
    );
    let Some(folder) = folder else {
        return Ok(());
    };
    let removed = remove_run_file(&folder);
    checks.check(
        format!("the Run's run file is removed ({removed:?})"),
        removed.is_ok(),
    );
    let again = start(connection, run_id)?;
    failed_with(
        &again,
        "run_state_unavailable",
        "a start under the ID of a listed Run whose run file is gone fails",
        checks,
    );
    let after = start(connection, run_id)?;
    failed_with(
        &after,
        "run_state_unavailable",
        "a second start fails the same, no Run having begun",
        checks,
    );
    Ok(())
}

/// Removes the run file in `folder` and its write-ahead log and shared
/// memory files, those that are there.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn remove_run_file(folder: &Path) -> std::io::Result<()> {
    let file: PathBuf = folder.join(RUN_FILE);
    std::fs::remove_file(&file)?;
    for suffix in ["-wal", "-shm"] {
        let mut companion = file.clone().into_os_string();
        companion.push(suffix);
        match std::fs::remove_file(&companion) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}
