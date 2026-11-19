//! A Run's progress (ADR 0102): what a live Run's registry and driver
//! change of it, each change a new value at a higher position that its
//! readers follow and the event stream carries as `run.progress`; the parts
//! an area reads from the Run's committed state when a value is read
//! ([`ProgressPart`]); the parts its kind of Run keeps typed, which are
//! encoded only for a value a reader takes ([`TypedPart`]); and the progress
//! a run file gives a Run no engine of this one's works, which is the last
//! that comes from here.

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::SystemTime;

use goodfetch_protocol::run::{
    HumanPause, Lifecycle, ProgressMessage, RUN_PROGRESS, RunId, RunProgress, RunStorage,
    RunWriter, StopReason,
};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Map, Value};
use tokio::sync::watch;

use crate::clock::Clock;
use crate::events::Events;

/// What an area adds to a Run's progress from its committed state (ADR
/// 0102), such as a crawl's request counts: given through its composition
/// value, it reads the Run's state on `connection`, a read-only connection
/// of the reader's own, each time a value is read, and fills its part of
/// `progress`. A part it has nothing for stays absent.
pub trait ProgressPart: Send + Sync {
    /// Fills its part of `progress`, whose live Run keeps `typed` unencoded:
    /// a typed part it encodes into `progress` itself, as the committed
    /// state has it, is not encoded again.
    ///
    /// # Errors
    ///
    /// When the Run's state cannot be read: the value then goes without
    /// this part, and the failure is logged.
    fn fill(
        &self,
        connection: &Connection,
        typed: &TypedParts,
        progress: &mut RunProgress,
    ) -> rusqlite::Result<()>;
}

/// What a kind of Run keeps of one part of its progress, in types the
/// engine does not know, such as a crawl's requests in flight.
pub trait PartValue: Any + Send + Sync + Debug {
    /// Its JSON value, as it stands.
    fn encoded(&self) -> Value;
}

/// A part of a live Run's progress that its kind of Run keeps typed: it is
/// encoded when a reader takes a value, once for that value, and never
/// when it changes.
#[derive(Clone, Debug)]
pub struct TypedPart(Arc<dyn PartValue>);

impl TypedPart {
    /// The part that holds `value`.
    pub fn new(value: impl PartValue) -> Self {
        Self(Arc::new(value))
    }

    /// What it holds, when that is a `T`.
    #[must_use]
    pub fn value<T: PartValue>(&self) -> Option<&T> {
        let value: &dyn Any = self.0.as_ref();
        value.downcast_ref()
    }
}

/// The typed parts of one value of a live Run's progress, by name; none for
/// a Run no engine of this one's works.
#[derive(Clone, Debug, Default)]
pub struct TypedParts(BTreeMap<&'static str, TypedPart>);

impl TypedParts {
    /// The part `name`, when the Run keeps one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&TypedPart> {
        self.0.get(name)
    }

    /// Encodes into `progress` each part it does not hold yet: those no
    /// [`ProgressPart`] encoded from the committed state.
    pub(super) fn encode_into(&self, progress: &mut RunProgress) {
        for (name, part) in &self.0 {
            if !progress.kind_parts.contains_key(*name) {
                progress
                    .kind_parts
                    .insert((*name).to_owned(), part.0.encoded());
            }
        }
    }
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
    pub(super) checkpoint_at: Option<SystemTime>,
    pauses: BTreeMap<u64, HumanPause>,
    next_pause: u64,
    pub(super) message: Option<ProgressMessage>,
    pub(super) storage: Option<RunStorage>,
    pub(super) stop_reason: Option<StopReason>,
    pub(super) last: bool,
    kind_parts: Map<String, Value>,
    typed_parts: TypedParts,
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
            typed_parts: TypedParts::default(),
        }
    }

    /// Its position.
    pub(super) fn position(&self) -> u64 {
        self.position
    }

    /// The value of `run_id` it gives, read at `at`, before its parts fill
    /// theirs: its typed parts encoded as they stand.
    pub(super) fn value(&self, run_id: &RunId, at: SystemTime) -> RunProgress {
        self.unencoded(run_id, at).encoded()
    }

    /// The same with its typed parts not encoded yet, for a reader whose
    /// parts fill theirs first.
    pub(super) fn unencoded(&self, run_id: &RunId, at: SystemTime) -> Unencoded {
        let value = RunProgress {
            run_id: run_id.clone(),
            position: self.position,
            at,
            lifecycle: self.lifecycle,
            writer: self.writer,
            epoch: self.epoch,
            checkpoint_at: self.checkpoint_at,
            pauses: self.pauses.values().cloned().collect(),
            message: self.message.clone(),
            storage: self.storage.clone(),
            stop_reason: self.stop_reason.clone(),
            last: self.last,
            kind_parts: self.kind_parts.clone(),
        };
        Unencoded {
            value,
            typed: self.typed_parts.clone(),
        }
    }
}

/// A value of a live Run's progress whose typed parts are not encoded yet.
#[derive(Clone, Debug)]
pub(super) struct Unencoded {
    value: RunProgress,
    typed: TypedParts,
}

impl Unencoded {
    /// Says the value is the last that comes from here.
    pub(super) fn last(mut self) -> Self {
        self.value.last = true;
        self
    }

    /// The value, its typed parts encoded as they stand.
    pub(super) fn encoded(mut self) -> RunProgress {
        self.typed.encode_into(&mut self.value);
        self.value
    }

    /// The value once each of `parts` filled its own from the committed
    /// state `connection` reads, as [`fill`] does, and each typed part none
    /// of them encoded is encoded as it stands.
    pub(super) fn filled(
        mut self,
        parts: &[Arc<dyn ProgressPart>],
        connection: &Connection,
    ) -> RunProgress {
        fill(parts, connection, &self.typed, &mut self.value);
        self.value
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

    /// Sets the typed part `name` its kind of Run adds to `part`; a change.
    pub(super) fn set_typed_part(&self, name: &'static str, part: TypedPart) {
        self.change(|progressing| {
            progressing.typed_parts.0.insert(name, part);
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
/// committed state are the readers' own. While `events` holds no
/// subscription it takes no value, since none would reach a reader: a
/// change then encodes nothing.
pub(super) async fn publish(
    run_id: RunId,
    mut changes: watch::Receiver<Progressing>,
    events: Events,
    clock: Clock,
) {
    while changes.changed().await.is_ok() {
        let (value, last) = {
            let current = changes.borrow_and_update();
            let value = events
                .subscribed()
                .then(|| current.value(&run_id, clock.point_in_time()));
            (value, current.last)
        };
        if let Some(value) = value {
            events
                .publish(&RUN_PROGRESS, Some(run_id.as_str()), &value)
                .await;
        }
        if last {
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
    at: SystemTime,
) -> rusqlite::Result<Option<RunProgress>> {
    let found = connection
        .prepare_cached("SELECT run_id, epoch, lifecycle, stop_reason, progress_message FROM run")?
        .query_row([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
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
/// left out and logged. Each of `typed` that no part encoded is then
/// encoded as it stands.
pub(super) fn fill(
    parts: &[Arc<dyn ProgressPart>],
    connection: &Connection,
    typed: &TypedParts,
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
        if let Err(error) = part.fill(connection, typed, progress) {
            tracing::warn!(%error, run_id = %progress.run_id, "a part of the Run's progress could not be read");
        }
    }
    typed.encode_into(progress);
}
