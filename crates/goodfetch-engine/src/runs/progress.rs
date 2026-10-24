//! A Run's progress (ADR 0102): what a live Run's registry and driver
//! change of it, each change a new value at a higher position that its
//! readers follow and the event stream carries as `run.progress`; the parts
//! an area reads from the Run's committed state when a value is read
//! ([`ProgressPart`]); and the progress a run file gives a Run no engine of
//! this one's works, which is the last that comes from here.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_protocol::run::{
    HumanPause, Lifecycle, PointInTime, ProgressMessage, RUN_PROGRESS, RunId, RunProgress,
    RunStorage, RunWriter, StopReason,
};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Map, Value};
use tokio::sync::watch;

use crate::clock::Clock;
use crate::events::Events;

use super::folder::point_in_time;

/// What an area adds to a Run's progress from its committed state (ADR
/// 0102), such as a crawl's request counts: given through its composition
/// value, it reads the Run's state on `connection`, a read-only connection
/// of the reader's own, each time a value is read, and fills its part of
/// `progress`. A part it has nothing for stays absent.
pub trait ProgressPart: Send + Sync {
    /// Fills its part of `progress`.
    ///
    /// # Errors
    ///
    /// When the Run's state cannot be read: the value then goes without
    /// this part, and the failure is logged.
    fn fill(&self, connection: &Connection, progress: &mut RunProgress) -> rusqlite::Result<()>;
}

/// What a live Run's progress holds as it is now, beside what its parts
/// read of its committed state.
#[derive(Clone, Debug)]
pub(super) struct Progressing {
    /// Its position, which each change raises.
    position: u64,
    pub(super) lifecycle: Lifecycle,
    pub(super) writer: RunWriter,
    pub(super) epoch: u64,
    pub(super) checkpoint_at: Option<PointInTime>,
    pauses: BTreeMap<u64, HumanPause>,
    next_pause: u64,
    pub(super) message: Option<ProgressMessage>,
    pub(super) storage: Option<RunStorage>,
    pub(super) stop_reason: Option<StopReason>,
    pub(super) last: bool,
    kind_parts: Map<String, Value>,
}

impl Progressing {
    /// The progress of a Run that went live at run epoch `epoch`, in
    /// `lifecycle`, with the program's status line `message`. Its positions
    /// start in the epoch's own range, so they keep going up across a
    /// resume.
    pub(super) fn live(epoch: u64, lifecycle: Lifecycle, message: Option<ProgressMessage>) -> Self {
        Self {
            position: epoch_floor(epoch),
            lifecycle,
            writer: RunWriter::Live,
            epoch,
            checkpoint_at: None,
            pauses: BTreeMap::new(),
            next_pause: 0,
            message,
            storage: None,
            stop_reason: None,
            last: false,
            kind_parts: Map::new(),
        }
    }

    /// Its position.
    pub(super) fn position(&self) -> u64 {
        self.position
    }

    /// The value of `run_id` it gives, read at `at`, before its parts fill
    /// theirs.
    pub(super) fn value(&self, run_id: &RunId, at: PointInTime) -> RunProgress {
        RunProgress {
            run_id: run_id.clone(),
            position: self.position,
            at,
            lifecycle: self.lifecycle,
            writer: self.writer,
            epoch: self.epoch,
            checkpoint_at: self.checkpoint_at.clone(),
            pauses: self.pauses.values().cloned().collect(),
            message: self.message.clone(),
            storage: self.storage.clone(),
            stop_reason: self.stop_reason.clone(),
            last: self.last,
            kind_parts: self.kind_parts.clone(),
        }
    }
}

/// The lowest position of run epoch `epoch`'s values.
fn epoch_floor(epoch: u64) -> u64 {
    epoch.saturating_mul(1 << 32)
}

/// A live Run's progress, which its registry and its driver change and its
/// readers follow.
#[derive(Clone, Debug)]
pub(super) struct Progress {
    changes: Arc<watch::Sender<Progressing>>,
}

impl Progress {
    /// A Run's progress, as `start` is.
    pub(super) fn new(start: Progressing) -> Self {
        Self {
            changes: Arc::new(watch::Sender::new(start)),
        }
    }

    /// Changes it as `change` does, a new value at a higher position.
    pub(super) fn change(&self, change: impl FnOnce(&mut Progressing)) {
        self.changes.send_modify(|progressing| {
            change(progressing);
            progressing.position = progressing.position.saturating_add(1);
        });
    }

    /// Takes up a resumed Run's `lifecycle` and the program's status line
    /// `message` its run state kept; a change.
    pub(super) fn resumed(&self, lifecycle: Lifecycle, message: Option<ProgressMessage>) {
        self.change(|progressing| {
            progressing.lifecycle = lifecycle;
            progressing.message = message;
        });
    }

    /// Its values from now on.
    pub(super) fn subscribe(&self) -> watch::Receiver<Progressing> {
        self.changes.subscribe()
    }

    /// Sets the part `name` its kind of Run adds to `part`: a change only
    /// when it differs from the part it held.
    pub(super) fn set_part(&self, name: &str, part: Value) {
        self.changes.send_if_modified(|progressing| {
            if progressing.kind_parts.get(name) == Some(&part) {
                return false;
            }
            progressing.kind_parts.insert(name.to_owned(), part);
            progressing.position = progressing.position.saturating_add(1);
            true
        });
    }

    /// Adds `pause` to the pauses waiting; a change.
    pub(super) fn pause(&self, pause: HumanPause) -> PauseHeld {
        let mut id = 0;
        self.change(|progressing| {
            id = progressing.next_pause;
            progressing.next_pause += 1;
            progressing.pauses.insert(id, pause);
        });
        PauseHeld {
            progress: self.clone(),
            id,
        }
    }
}

/// A Human pause waiting on a live Run (ADR 0092), as its progress shows
/// it until this is ended or dropped, each a change.
#[derive(Debug)]
#[must_use = "the pause ends when this is dropped"]
pub struct PauseHeld {
    progress: Progress,
    id: u64,
}

impl PauseHeld {
    /// Ends the pause.
    pub fn end(self) {
        drop(self);
    }
}

impl Drop for PauseHeld {
    fn drop(&mut self) {
        let id = self.id;
        self.progress.change(|progressing| {
            progressing.pauses.remove(&id);
        });
    }
}

/// Publishes each value of the live Run `run_id`'s progress that `changes`
/// gives on `events` as `run.progress`, read at `clock`'s time, the values
/// a slow stream misses coalesced, until its last; its parts read from
/// committed state are the readers' own.
pub(super) async fn publish(
    run_id: RunId,
    mut changes: watch::Receiver<Progressing>,
    events: Events,
    clock: Clock,
) {
    while changes.changed().await.is_ok() {
        let value = changes
            .borrow_and_update()
            .value(&run_id, point_in_time(&clock));
        events
            .publish(&RUN_PROGRESS, Some(run_id.as_str()), &value)
            .await;
        if value.last {
            return;
        }
    }
}

/// The progress of the Run whose run file `connection` reads, one no
/// engine of this one's works, `writer` as a reader's test found its
/// writer lock, read at `at`; `None` when the file holds no Run. It is the
/// last value from here: nothing more comes until a start with its Run ID
/// resumes the Run, or the engine that works it answers.
pub(super) fn committed(
    connection: &Connection,
    writer: RunWriter,
    at: PointInTime,
) -> rusqlite::Result<Option<RunProgress>> {
    let found = connection
        .query_row(
            "SELECT run_id, epoch, lifecycle, stop_reason, progress_message FROM run",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((run_id, epoch, lifecycle, stop_reason, message)) = found else {
        return Ok(None);
    };
    let decode =
        |error: serde_json::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(error));
    let epoch = u64::try_from(epoch).unwrap_or_default();
    Ok(Some(RunProgress {
        run_id: serde_json::from_value(Value::String(run_id)).map_err(decode)?,
        // Past every value of its epoch a live reader read.
        position: epoch_floor(epoch.saturating_add(1)).saturating_sub(1),
        at,
        lifecycle: serde_json::from_value(Value::String(lifecycle)).map_err(decode)?,
        writer,
        epoch,
        checkpoint_at: None,
        pauses: Vec::new(),
        message: message.as_deref().and_then(ProgressMessage::parse),
        storage: None,
        stop_reason: stop_reason
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(decode)?,
        last: true,
        kind_parts: Map::new(),
    }))
}

/// Fills `progress` with each of `parts`, read on `connection` in one read
/// transaction, so every part reads the same committed state, not one
/// that a commit changed between two of their reads; a part that fails is
/// left out and logged.
pub(super) fn fill(
    parts: &[Arc<dyn ProgressPart>],
    connection: &Connection,
    progress: &mut RunProgress,
) {
    // Dropped at the end, which ends the read transaction.
    let _snapshot = connection
        .unchecked_transaction()
        .inspect_err(|error| {
            tracing::warn!(%error, run_id = %progress.run_id, "the Run's committed state could not be read as one snapshot");
        })
        .ok();
    for part in parts {
        if let Err(error) = part.fill(connection, progress) {
            tracing::warn!(%error, run_id = %progress.run_id, "a part of the Run's progress could not be read");
        }
    }
}
