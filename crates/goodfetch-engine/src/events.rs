//! The engine's event stream (ADRs 0024, 0031 and 0081): one actor, a task
//! of the engine's owner, that gives every event the next position of the
//! engine instance, keeps the last lossless ones, and holds each subscription's
//! queue, from which the subscription takes as many events as its Client's
//! credits allow.
//!
//! No producer ever waits for a subscriber. A lossless event (an axis
//! change, a "Needs you" item) is never dropped without notice: each
//! subscription takes them in the order they were published, and one whose
//! queue is full falls behind instead, its queued lossless events dropped,
//! and next takes `events.resync`, the axes of every live Run the stream
//! has reported and every item of "Needs you" it lists, then the stream
//! (ADRs 0024 and 0031). A telemetry event
//! (progress, logs) replaces the unsent one of the same kind and scope in
//! each queue, which the one sent counts as `dropped`. A subscription that
//! names the position its Client last read resumes after it from the kept
//! events, and one the engine no longer holds, or another engine instance
//! gave, as before a Daemon restart, starts behind, with `events.resync`.
//!
//! Each event's payload is encoded once, as it is published, and every
//! subscription that takes it shares it (ADR 0140's E1).
//!
//! Also the credits a Client grants a streaming call, which the protocol
//! server adds and the call takes.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::catalog::{Event, EventClass};
use goodfetch_protocol::connection::EngineInstance;
use goodfetch_protocol::events::{LastRead, Position, RESYNC, Resync};
use goodfetch_protocol::governor::SYSTEM_INFO;
use goodfetch_protocol::needs_you::{NEEDS_YOU_CHANGED, NeedsYouChange, NeedsYouItem};
use goodfetch_protocol::run::{Lifecycle, RUN_AXES_CHANGED, RunAxes, RunId};
use serde::{Serialize, Serializer};
use serde_json::value::RawValue;
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::tasks::TaskOwner;

/// How many commands wait for the stream's actor before their senders wait.
const COMMANDS: usize = 1024;

/// The most credits a streaming call holds unspent; a grant past it is
/// capped.
const MAX_CREDITS: usize = 1 << 20;

/// How big the stream's buffers are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventsOptions {
    /// How many lossless events a subscription's queue holds before it
    /// falls behind to an `events.resync`.
    pub queue: usize,
    /// How many of the last lossless events the engine keeps for a
    /// subscription that resumes.
    pub retained: usize,
}

impl Default for EventsOptions {
    fn default() -> Self {
        Self {
            queue: 64,
            retained: 4096,
        }
    }
}

/// The engine's event stream: a handle to its actor, which any task may
/// clone.
#[derive(Clone, Debug)]
pub struct Events {
    commands: mpsc::Sender<Command>,
    instance: EngineInstance,
    work: CountedWork,
}

/// One event as a stream holds and sends it, an `EventEnvelope` on the
/// wire.
#[derive(Clone, Debug, Serialize)]
pub struct Envelope {
    /// Its position.
    pub position: Position,
    /// The catalog's name of the event.
    pub event: &'static str,
    /// Its payload.
    pub data: EventData,
    /// For a telemetry event, how many of the same kind and scope it
    /// replaced unsent.
    #[serde(skip_serializing_if = "is_zero")]
    pub dropped: u64,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes a reference"
)]
fn is_zero(count: &u64) -> bool {
    *count == 0
}

/// An event's payload, encoded once: each copy shares it, and each line
/// that carries it copies it without encoding it again.
#[derive(Clone, Debug)]
pub struct EventData(Arc<RawValue>);

impl EventData {
    /// `data`, encoded, the event's one encode counted on `work` (ADR
    /// 0140's E1).
    pub fn encode(data: &impl Serialize, work: &CountedWork) -> Self {
        work.count(Row::MessageEncodes);
        let data = serde_json::value::to_raw_value(data)
            .expect("invariant: an event's payload encodes as JSON");
        Self(Arc::from(data))
    }

    /// Its JSON text.
    pub fn json(&self) -> &str {
        self.0.get()
    }
}

impl Serialize for EventData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// One subscription to the stream, as the call that reads it holds it;
/// dropping it ends the subscription.
#[derive(Debug)]
pub(crate) struct Subscription {
    id: u64,
    events: Events,
    _alive: oneshot::Sender<()>,
}

/// The credits a Client grants one streaming call with `$/credit`.
#[derive(Clone, Debug)]
pub(crate) struct Credits(Arc<Semaphore>);

enum Command {
    Publish(Publish),
    Subscribe {
        after: Option<LastRead>,
        system_info: bool,
        alive: oneshot::Receiver<()>,
        answer: oneshot::Sender<u64>,
    },
    Take {
        subscription: u64,
        most: usize,
        answer: oneshot::Sender<Vec<Envelope>>,
    },
    Unsubscribe(u64),
}

struct Publish {
    event: &'static Event,
    scope: Option<String>,
    data: EventData,
    fold: Option<Fold>,
    accepted: oneshot::Sender<()>,
}

/// What a lossless event changes of the state a resync gives.
enum Fold {
    Axes(RunAxes),
    NeedsYou(Box<NeedsYouChange>),
}

/// The state a resync gives: the axes of every live Run the stream has
/// reported, and every item of "Needs you" it lists. A Run leaves it as it
/// ends, its axes in the Run index, so an engine that runs Run after Run
/// keeps none of theirs for its life (#1768).
#[derive(Default)]
struct Snapshot {
    runs: BTreeMap<RunId, RunAxes>,
    needs_you: BTreeMap<String, NeedsYouItem>,
}

impl Snapshot {
    fn fold(&mut self, fold: Fold) {
        match fold {
            Fold::Axes(axes) if axes.lifecycle == Lifecycle::Running => {
                self.runs.insert(axes.run_id.clone(), axes);
            }
            Fold::Axes(axes) => {
                self.runs.remove(&axes.run_id);
            }
            Fold::NeedsYou(change) => match *change {
                NeedsYouChange::Listed { item } => {
                    self.needs_you.insert(item.id.clone(), item);
                }
                NeedsYouChange::Removed { id, .. } => {
                    self.needs_you.remove(&id);
                }
            },
        }
    }

    fn resync(&self) -> Resync {
        let mut needs_you: Vec<NeedsYouItem> = self.needs_you.values().cloned().collect();
        needs_you.sort_by(|left, right| {
            (left.since.as_str(), &left.id).cmp(&(right.since.as_str(), &right.id))
        });
        Resync {
            runs: self.runs.values().cloned().collect(),
            needs_you,
        }
    }
}

impl Events {
    /// Starts the stream's actor as a task of `owner`, its positions those
    /// of `instance`, each encode of an event counted on `work`.
    pub(crate) fn start(
        options: EventsOptions,
        instance: EngineInstance,
        owner: &TaskOwner,
        work: CountedWork,
    ) -> Self {
        let (commands, mailbox) = mpsc::channel(COMMANDS);
        let stream = Stream {
            options,
            instance: instance.clone(),
            last: 0,
            floor: 0,
            retained: VecDeque::new(),
            snapshot: Snapshot::default(),
            subscriptions: BTreeMap::new(),
            next_subscription: 0,
            work: work.clone(),
        };
        let token = owner.token().clone();
        owner.spawn(async move {
            tokio::select! {
                () = stream.run(mailbox) => {}
                () = token.cancelled() => {}
            }
        });
        Self {
            commands,
            instance,
            work,
        }
    }

    /// The engine instance whose positions it gives.
    pub(crate) fn instance(&self) -> &EngineInstance {
        &self.instance
    }

    /// Publishes `event`, with `data` as its payload; `scope` (such as a
    /// Run ID) tells telemetry events of the same kind apart, which
    /// coalesce only within one scope.
    ///
    /// It answers once the stream has given the event its position, which
    /// no subscriber's queue delays: a full queue falls behind instead.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the event was sent still publishes it.
    pub async fn publish(&self, event: &'static Event, scope: Option<&str>, data: &impl Serialize) {
        self.send(
            event,
            scope.map(str::to_owned),
            EventData::encode(data, &self.work),
            None,
        )
        .await;
    }

    /// Publishes a Run's axes as `run.axes_changed`, which a resync then
    /// reports.
    ///
    /// # Cancel safety
    ///
    /// As [`Events::publish`]'s.
    pub(crate) async fn publish_axes(&self, axes: RunAxes) {
        let data = EventData::encode(&axes, &self.work);
        let scope = Some(axes.run_id.as_str().to_owned());
        self.send(&RUN_AXES_CHANGED, scope, data, Some(Fold::Axes(axes)))
            .await;
    }

    /// Publishes a change of "Needs you" as `needs_you.changed`, which a
    /// resync then reports.
    ///
    /// # Cancel safety
    ///
    /// As [`Events::publish`]'s.
    pub(crate) async fn publish_needs_you(&self, change: NeedsYouChange) {
        let data = EventData::encode(&change, &self.work);
        self.send(
            &NEEDS_YOU_CHANGED,
            None,
            data,
            Some(Fold::NeedsYou(Box::new(change))),
        )
        .await;
    }

    async fn send(
        &self,
        event: &'static Event,
        scope: Option<String>,
        data: EventData,
        fold: Option<Fold>,
    ) {
        let (accepted, acceptance) = oneshot::channel();
        let publish = Publish {
            event,
            scope,
            data,
            fold,
            accepted,
        };
        if self.commands.send(Command::Publish(publish)).await.is_ok() {
            let _ = self.answered(acceptance).await;
        }
    }

    /// The stream's answer to a command sent to it, `None` once the stream
    /// has stopped. Its end, not only the answer, ends the wait: a command
    /// sent under a permit taken just before the stream stopped lands after
    /// its mailbox was emptied, where the stream never reads it and the
    /// channel keeps it, its answer's sender with it, while any handle
    /// lives, such as the waiting caller's own.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses the answer.
    async fn answered<T>(&self, answer: oneshot::Receiver<T>) -> Option<T> {
        tokio::select! {
            biased;
            answered = answer => answered.ok(),
            () = self.commands.closed() => None,
        }
    }

    /// Opens a subscription that resumes after `after`, or starts with the
    /// next event, taking `engine.system_info` too when `system_info`;
    /// `None` once the stream has stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after it was sent opens a subscription the
    /// stream ends at its next event.
    pub(crate) async fn subscribe(
        &self,
        after: Option<LastRead>,
        system_info: bool,
    ) -> Option<Subscription> {
        let (alive, watched) = oneshot::channel();
        let (answer, answered) = oneshot::channel();
        self.commands
            .send(Command::Subscribe {
                after,
                system_info,
                alive: watched,
                answer,
            })
            .await
            .ok()?;
        let id = self.answered(answered).await?;
        Some(Subscription {
            id,
            events: self.clone(),
            _alive: alive,
        })
    }
}

impl Subscription {
    /// Waits for at least one event, and takes at most `most`, in the
    /// order of their positions; `None` once the stream has stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses the events it took; drop it only with the
    /// subscription.
    pub(crate) async fn next(&mut self, most: usize) -> Option<Vec<Envelope>> {
        let (answer, answered) = oneshot::channel();
        self.events
            .commands
            .send(Command::Take {
                subscription: self.id,
                most: most.max(1),
                answer,
            })
            .await
            .ok()?;
        self.events.answered(answered).await
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        // A full mailbox has commands the actor reads, and each one makes
        // it find this subscription gone, so a failed send loses nothing.
        let _ = self.events.commands.try_send(Command::Unsubscribe(self.id));
    }
}

impl Credits {
    /// No credits yet.
    pub(crate) fn new() -> Self {
        Self(Arc::new(Semaphore::new(0)))
    }

    /// Adds `credits`, capped so that at most [`MAX_CREDITS`] stay unspent.
    pub(crate) fn grant(&self, credits: u32) {
        let room = MAX_CREDITS.saturating_sub(self.0.available_permits());
        let credits = usize::try_from(credits).unwrap_or(usize::MAX).min(room);
        self.0.add_permits(credits);
    }

    /// Waits for at least one credit, and spends at most `most`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future spends nothing.
    pub(crate) async fn spend(&self, most: usize) -> usize {
        let Ok(first) = self.0.acquire().await else {
            return 0;
        };
        first.forget();
        let more = self.0.available_permits().min(most.saturating_sub(1));
        let more = u32::try_from(more).unwrap_or(u32::MAX);
        if more > 0
            && let Ok(permits) = self.0.try_acquire_many(more)
        {
            permits.forget();
            return 1 + usize::try_from(more).unwrap_or(0);
        }
        1
    }

    /// Gives back `credits` spent on nothing.
    pub(crate) fn refund(&self, credits: usize) {
        self.0.add_permits(credits);
    }
}

/// One subscription's queue.
struct Queue {
    lossless: VecDeque<Envelope>,
    /// Whether it fell behind, or started behind: its next take starts with
    /// `events.resync`, and it queues no lossless event until then.
    behind: bool,
    telemetry: BTreeMap<(&'static str, Option<String>), Envelope>,
    waiting: Option<(usize, oneshot::Sender<Vec<Envelope>>)>,
    alive: oneshot::Receiver<()>,
    /// Whether it takes `engine.system_info`.
    system_info: bool,
}

impl Queue {
    fn is_gone(&mut self) -> bool {
        matches!(
            self.alive.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        )
    }

    fn is_empty(&self) -> bool {
        !self.behind && self.lossless.is_empty() && self.telemetry.is_empty()
    }

    /// Takes at most `most` events, in the order of their positions.
    fn take(&mut self, most: usize) -> Vec<Envelope> {
        let mut telemetry: Vec<(Position, (&'static str, Option<String>))> = self
            .telemetry
            .iter()
            .map(|(key, envelope)| (envelope.position, key.clone()))
            .collect();
        telemetry.sort_unstable();
        let mut telemetry = VecDeque::from(telemetry);
        let mut taken = Vec::new();
        while taken.len() < most {
            let lossless = self.lossless.front().map(|envelope| envelope.position);
            let coalesced = telemetry.front().map(|(position, _)| *position);
            let next = match (lossless, coalesced) {
                (Some(lossless), Some(coalesced)) if coalesced < lossless => None,
                (Some(_), _) => self.lossless.pop_front(),
                (None, _) => None,
            };
            if let Some(envelope) = next {
                taken.push(envelope);
                continue;
            }
            let Some((_, key)) = telemetry.pop_front() else {
                break;
            };
            if let Some(envelope) = self.telemetry.remove(&key) {
                taken.push(envelope);
            }
        }
        taken
    }
}

/// The actor's state.
struct Stream {
    options: EventsOptions,
    /// The engine instance whose positions it gives.
    instance: EngineInstance,
    /// The last position given.
    last: u64,
    /// The last position no longer kept: a subscription may resume after
    /// any position from it to `last`.
    floor: u64,
    retained: VecDeque<Envelope>,
    snapshot: Snapshot,
    subscriptions: BTreeMap<u64, Queue>,
    next_subscription: u64,
    /// Where each `events.resync` it encodes counts.
    work: CountedWork,
}

impl Stream {
    async fn run(mut self, mut mailbox: mpsc::Receiver<Command>) {
        while let Some(command) = mailbox.recv().await {
            match command {
                Command::Publish(publish) => self.accept(publish),
                Command::Subscribe {
                    after,
                    system_info,
                    alive,
                    answer,
                } => {
                    let id = self.subscribe(after, system_info, alive);
                    if answer.send(id).is_err() {
                        self.subscriptions.remove(&id);
                    }
                }
                Command::Take {
                    subscription,
                    most,
                    answer,
                } => {
                    if let Some(queue) = self.subscriptions.get_mut(&subscription) {
                        queue.waiting = Some((most, answer));
                        serve(queue, &mut self.last, &self.snapshot, &self.work);
                    }
                }
                Command::Unsubscribe(subscription) => {
                    self.subscriptions.remove(&subscription);
                }
            }
            self.subscriptions.retain(|_, queue| !queue.is_gone());
        }
    }

    fn accept(&mut self, publish: Publish) {
        let Publish {
            event,
            scope,
            data,
            fold,
            accepted,
        } = publish;
        let opted_in = event.name == SYSTEM_INFO.name;
        if opted_in && !self.subscriptions.values().any(|queue| queue.system_info) {
            // Nobody takes it: it gets no position.
            let _ = accepted.send(());
            return;
        }
        let envelope = Envelope {
            position: next_position(&mut self.last),
            event: event.name,
            data,
            dropped: 0,
        };
        match event.class {
            EventClass::Lossless => {
                if let Some(fold) = fold {
                    self.snapshot.fold(fold);
                }
                self.retained.push_back(envelope.clone());
                while self.retained.len() > self.options.retained {
                    if let Some(gone) = self.retained.pop_front() {
                        self.floor = gone.position.0;
                    }
                }
                for (id, queue) in &mut self.subscriptions {
                    if queue.behind {
                        continue;
                    }
                    if queue.lossless.len() >= self.options.queue {
                        tracing::debug!(
                            subscription = id,
                            "a subscription fell behind: its next take is a resync"
                        );
                        queue.lossless.clear();
                        queue.behind = true;
                        continue;
                    }
                    queue.lossless.push_back(envelope.clone());
                    serve(queue, &mut self.last, &self.snapshot, &self.work);
                }
            }
            EventClass::Telemetry => {
                for queue in self
                    .subscriptions
                    .values_mut()
                    .filter(|queue| !opted_in || queue.system_info)
                {
                    let key = (event.name, scope.clone());
                    let dropped = queue
                        .telemetry
                        .get(&key)
                        .map_or(0, |unsent| unsent.dropped + 1);
                    queue.telemetry.insert(
                        key,
                        Envelope {
                            dropped,
                            ..envelope.clone()
                        },
                    );
                    serve(queue, &mut self.last, &self.snapshot, &self.work);
                }
            }
        }
        let _ = accepted.send(());
    }

    fn subscribe(
        &mut self,
        after: Option<LastRead>,
        system_info: bool,
        alive: oneshot::Receiver<()>,
    ) -> u64 {
        let id = self.next_subscription;
        self.next_subscription += 1;
        let mut queue = Queue {
            lossless: VecDeque::new(),
            behind: false,
            telemetry: BTreeMap::new(),
            waiting: None,
            alive,
            system_info,
        };
        match after {
            None => {}
            Some(LastRead {
                engine_instance,
                position: Position(after),
            }) if engine_instance == self.instance && (self.floor..=self.last).contains(&after) => {
                queue.lossless.extend(
                    self.retained
                        .iter()
                        .filter(|envelope| envelope.position.0 > after)
                        .cloned(),
                );
            }
            Some(_) => queue.behind = true,
        }
        self.subscriptions.insert(id, queue);
        id
    }
}

/// The position after `last`, which it becomes.
fn next_position(last: &mut u64) -> Position {
    *last += 1;
    Position(*last)
}

/// Answers the queue's waiting take, if any, once it has an event; a queue
/// behind first takes `events.resync`, at the next position, with the axes
/// of every live Run and every item of "Needs you" in `snapshot`.
fn serve(queue: &mut Queue, last: &mut u64, snapshot: &Snapshot, work: &CountedWork) {
    if queue.is_empty() {
        return;
    }
    if let Some((most, answer)) = queue.waiting.take() {
        if queue.behind {
            let resync = snapshot.resync();
            queue.lossless.push_back(Envelope {
                position: next_position(last),
                event: RESYNC.name,
                data: EventData::encode(&resync, work),
                dropped: 0,
            });
            queue.behind = false;
        }
        let taken = queue.take(most);
        // A take whose reader is gone ends with its subscription.
        let _ = answer.send(taken);
    }
}

#[cfg(test)]
mod tests;
