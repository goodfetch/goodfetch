//! One Connection to the Daemon: its `hello`, its subscription and the
//! subscription's credits, the reads it keeps in step with what is wanted,
//! and the answers it reports. The Client opens a new one for each Daemon
//! it reaches (`client::follow`).

use std::collections::BTreeMap;
use std::io;

use goodfetch_protocol::connection::{CANCEL, CancelParams, EngineInstance, HelloResult};
use goodfetch_protocol::daemon::DaemonRuntimeFile;
use goodfetch_protocol::events::{
    CREDIT, CreditParams, EVENTS, EVENTS_SUBSCRIBE, EventBatch, EventEnvelope,
    EventsSubscribeParams, LastRead, Position, RESYNC,
};
use goodfetch_protocol::foxhound::frontier::{ENQUEUED, EnqueuedBatch, SEEDS};
use goodfetch_protocol::foxhound::sql_tab::DATASET_QUERY;
use goodfetch_protocol::governor::{SYSTEM_INFO, SystemInfo};
use goodfetch_protocol::jsonrpc::{ErrorObject, Message, RequestId};
use goodfetch_protocol::needs_you::NEEDS_YOU_CHANGED;
use goodfetch_protocol::run::{
    Lifecycle, RUN_AXES_CHANGED, RUN_LIST, RUN_PROGRESS_READ, RUN_REPORT, RUN_USAGE, RunId,
    RunIndexRow, RunList, RunListParams, RunParams, RunProgress, RunProgressParams, RunReport,
    RunUsage,
};
use goodfetch_protocol::settings::{CONFIG_EXPLAIN, ConfigExplainParams};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::sync::watch;

use super::seeds::SeedStream;
use super::wire::{HELLO_ID, Reader, Writer, connect, hello_request, notification, refusal};
use super::wire::{request, send};
use super::{
    Applied, Batch, CREDITS, Call, CallFailure, DaemonEvent, DaemonIdentity, Generation, Listing,
    Query, Refusal, Report, UsageRead, Wanted,
};
use crate::reports::Reports;

/// The id of its `events.subscribe`, its second.
const SUBSCRIBE_ID: i64 = 2;

/// The id of its first other call, the health poll's included; each next
/// one takes the next.
const FIRST_CALL_ID: i64 = 3;

/// Why opening a Connection failed.
#[derive(Debug)]
pub(super) enum Opening {
    /// Nothing answered at the endpoint, or it closed before answering.
    Unreachable,
    /// The Daemon refused the Kennel.
    Refused(Refusal),
}

/// An open Connection, after its `hello` and its subscription.
pub(super) struct Connection {
    pub(super) lines: Lines<BufReader<Reader>>,
    pub(super) writer: Writer,
    pub(super) identity: DaemonIdentity,
    /// The engine instance that answered `hello`.
    instance: EngineInstance,
    /// The id of the `events.subscribe` call of the subscription.
    subscription: i64,
    /// Whether the subscription takes `engine.system_info`.
    system_info: bool,
    /// The position of the last event the subscription reported, after
    /// which a subscription taken again resumes.
    last_read: Option<Position>,
    /// The subscription's credits.
    pub(super) credits: Credits,
}

/// A subscription's credits: the events applied, given back through
/// [`Applied`], and how many of them were granted again.
pub(super) struct Credits {
    applied: watch::Sender<u64>,
    pub(super) renewals: watch::Receiver<u64>,
    pub(super) credited: u64,
}

impl Credits {
    fn new() -> Self {
        let (applied, renewals) = watch::channel(0_u64);
        Self {
            applied,
            renewals,
            credited: 0,
        }
    }
}

impl Connection {
    /// Opens the Connection to the Daemon of `runtime` with `hello`,
    /// asking it to stay alive when `keep_daemon_alive` is on, then
    /// subscribes to its events, starting from a resync, with the Resource
    /// governor's readings when `system_info` is on.
    pub(super) async fn open(
        runtime: &DaemonRuntimeFile,
        keep_daemon_alive: bool,
        system_info: bool,
    ) -> Result<Self, Opening> {
        let (reader, mut writer) = connect(&runtime.endpoint)
            .await
            .map_err(|_| Opening::Unreachable)?;
        let mut lines = BufReader::new(reader).lines();
        send(&mut writer, &hello_request(runtime, keep_daemon_alive))
            .await
            .map_err(|_| Opening::Unreachable)?;
        let accepted = loop {
            let Ok(Some(line)) = lines.next_line().await else {
                return Err(Opening::Unreachable);
            };
            match serde_json::from_str::<Message>(&line) {
                Ok(Message::Response(response)) if response.id == RequestId::Number(HELLO_ID) => {
                    break serde_json::from_value::<HelloResult>(response.result).map_err(
                        |_| {
                            Opening::Refused(Refusal::Other {
                                code: "invalid_hello_result".to_owned(),
                            })
                        },
                    )?;
                }
                // An error whose id is null answers a message the Daemon
                // could not read, which can only be `hello`.
                Ok(Message::Error(failure))
                    if failure.id.is_none() || failure.id == Some(RequestId::Number(HELLO_ID)) =>
                {
                    return Err(Opening::Refused(refusal(failure.error)));
                }
                // Anything else answers no `hello`: the Kennel waits on.
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "a message the Kennel cannot read before hello's answer");
                }
            }
        };
        let mut connection = Self {
            lines,
            writer,
            identity: DaemonIdentity {
                product: accepted.product,
                product_version: accepted.product_version,
            },
            instance: accepted.engine_instance,
            subscription: SUBSCRIBE_ID,
            system_info,
            last_read: None,
            credits: Credits::new(),
        };
        connection
            .subscribe()
            .await
            .map_err(|_| Opening::Unreachable)?;
        Ok(connection)
    }

    /// Subscribes to the engine's events with the call `subscription`:
    /// after the last event read, or from a resync before any; and grants
    /// the subscription its credits.
    async fn subscribe(&mut self) -> io::Result<()> {
        let params = EventsSubscribeParams {
            // A position ahead of any the engine gave starts the
            // subscription with `events.resync` (ADR 0031).
            after: Some(LastRead {
                engine_instance: self.instance.clone(),
                position: self.last_read.unwrap_or(Position(u64::MAX)),
            }),
            system_info: self.system_info,
        };
        send(
            &mut self.writer,
            &request(self.subscription, EVENTS_SUBSCRIBE.name, &params),
        )
        .await?;
        self.credit(u64::from(CREDITS)).await
    }

    /// Cancels the subscription and takes it again, after the last event
    /// it reported, with the Resource governor's readings when
    /// `system_info` is on. Batches of the cancelled subscription still
    /// on their way are ignored: the new one sends their events again.
    async fn resubscribe(&mut self, reads: &mut Reads, system_info: bool) -> io::Result<()> {
        let cancel = CancelParams {
            id: RequestId::Number(self.subscription),
        };
        send(&mut self.writer, &notification(CANCEL, &cancel)).await?;
        self.subscription = reads.next_id();
        self.system_info = system_info;
        self.credits = Credits::new();
        self.subscribe().await
    }

    /// Grants the subscription `credits` more events.
    pub(super) async fn credit(&mut self, credits: u64) -> io::Result<()> {
        let params = CreditParams {
            id: RequestId::Number(self.subscription),
            credits: u32::try_from(credits).unwrap_or(u32::MAX),
        };
        send(&mut self.writer, &notification(CREDIT, &params)).await
    }

    /// Sends the request of `method` with `params` under the Connection's
    /// next id, and records what its answer is for; gives the id.
    async fn ask(
        &mut self,
        reads: &mut Reads,
        method: &str,
        params: &impl Serialize,
        pending: Pending,
    ) -> io::Result<i64> {
        let id = reads.next_id();
        send(&mut self.writer, &request(id, method, params)).await?;
        reads.pending.insert(id, pending);
        Ok(id)
    }

    /// Cancels the call `id`, whose answer nothing waits for any more.
    async fn cancel(&mut self, reads: &mut Reads, id: i64) -> io::Result<()> {
        reads.pending.remove(&id);
        let params = CancelParams {
            id: RequestId::Number(id),
        };
        send(&mut self.writer, &notification(CANCEL, &params)).await
    }

    /// Sends a person's `call`, whose answer is reported as
    /// [`Report::Answered`].
    pub(super) async fn call(&mut self, reads: &mut Reads, call: Call) -> io::Result<()> {
        let id = reads.next_id();
        tracing::info!(method = call.method(), "a person's call");
        let sent = send(&mut self.writer, &call.request(id)).await;
        let pending = if matches!(call, Call::AddSeeds(_)) {
            Pending::Seeds(SeedStream::new(call))
        } else {
            Pending::Call(call)
        };
        reads.pending.insert(id, pending);
        sent
    }

    /// Handles one message of the Daemon.
    pub(super) async fn read(
        &mut self,
        message: Result<Message, serde_json::Error>,
        reads: &mut Reads,
        report: &impl Reports<Report>,
    ) -> io::Result<()> {
        match message {
            Ok(Message::Notification(notification)) if notification.method == EVENTS => {
                let Some(batch) = notification
                    .params
                    .and_then(|params| serde_json::from_value::<EventBatch>(params).ok())
                else {
                    tracing::warn!("an event batch the Kennel cannot read");
                    return Ok(());
                };
                if batch.id == RequestId::Number(self.subscription) {
                    if let Some(last) = batch.events.last() {
                        self.last_read = Some(last.position);
                    }
                    events(batch.events, &self.credits.applied, report).await;
                }
                Ok(())
            }
            Ok(Message::Notification(incoming)) if incoming.method == CREDIT => {
                let Some(granted) = incoming
                    .params
                    .and_then(|params| serde_json::from_value::<CreditParams>(params).ok())
                else {
                    return Ok(());
                };
                let RequestId::Number(id) = granted.id else {
                    return Ok(());
                };
                let Some(Pending::Seeds(stream)) = reads.pending.get_mut(&id) else {
                    return Ok(());
                };
                match stream.next_batch(id, granted.credits) {
                    Some(batch) => send(&mut self.writer, &notification(SEEDS, &batch)).await,
                    None => Ok(()),
                }
            }
            Ok(Message::Notification(incoming)) if incoming.method == ENQUEUED => {
                let Some(batch) = incoming
                    .params
                    .and_then(|params| serde_json::from_value::<EnqueuedBatch>(params).ok())
                else {
                    tracing::warn!("seeds' answers the Kennel cannot read");
                    return Ok(());
                };
                if let RequestId::Number(id) = batch.id
                    && let Some(Pending::Seeds(stream)) = reads.pending.get_mut(&id)
                {
                    stream.answered(&batch.answers);
                }
                Ok(())
            }
            Ok(Message::Response(response)) => {
                self.answered(reads, response.id, Ok(response.result), report)
                    .await
            }
            Ok(Message::Error(failure)) => match failure.id {
                Some(id) => self.answered(reads, id, Err(failure.error), report).await,
                None => Ok(()),
            },
            Ok(Message::Request(_) | Message::Notification(_)) | Err(_) => Ok(()),
        }
    }

    /// Handles the answer to the call `id`.
    async fn answered(
        &mut self,
        reads: &mut Reads,
        id: RequestId,
        answer: Result<Value, ErrorObject>,
        report: &impl Reports<Report>,
    ) -> io::Result<()> {
        let RequestId::Number(id) = id else {
            return Ok(());
        };
        let reported = match reads.pending.remove(&id) {
            None => None,
            Some(Pending::LiveRuns) => listed(answer).map(Report::LiveRuns),
            Some(Pending::Listed(generation)) => {
                listed(answer).map(|rows| Report::Listed { generation, rows })
            }
            Some(Pending::Report(run)) => {
                if let Some((reported, reading)) = &mut reads.report
                    && *reported == run
                {
                    *reading = None;
                }
                let answer = answer.map_err(|error| error.data.code).and_then(|value| {
                    serde_json::from_value::<Box<RunReport>>(value).map_err(|error| {
                        tracing::warn!(%error, run_id = %run, "a run.report answer the Kennel cannot read");
                        "invalid_run_report".to_owned()
                    })
                });
                Some(Report::RunReport {
                    run_id: run,
                    report: answer,
                })
            }
            Some(Pending::Usage(run)) => match answer.map(serde_json::from_value::<RunUsage>) {
                Ok(Ok(usage)) => Some(Report::Usage { run_id: run, usage }),
                Ok(Err(error)) => {
                    tracing::warn!(%error, run_id = %run, "a run.usage answer the Kennel cannot read");
                    None
                }
                Err(error) => {
                    tracing::warn!(code = error.data.code, run_id = %run, "run.usage failed");
                    None
                }
            },
            Some(Pending::Queried(generation)) => {
                if let Some((query, reading)) = &mut reads.records
                    && query.generation == generation
                {
                    *reading = None;
                }
                let result = decoded_answer(DATASET_QUERY.name, answer);
                Some(Report::Queried { generation, result })
            }
            Some(Pending::Explained(generation)) => {
                let result = decoded_answer(CONFIG_EXPLAIN.name, answer);
                Some(Report::Explained { generation, result })
            }
            Some(Pending::Seeds(stream)) => {
                stream.reported(answer, report).await;
                None
            }
            Some(Pending::Call(call)) => {
                let answer = answer.map(drop).map_err(failure);
                if let Err(failure) = &answer {
                    tracing::warn!(
                        method = call.method(),
                        code = failure.code,
                        "a person's call failed"
                    );
                }
                Some(Report::Answered(call, answer))
            }
            Some(Pending::Progress(run)) => {
                return self.progress_answered(reads, run, answer, report).await;
            }
        };
        if let Some(reported) = reported {
            report.report(reported).await;
        }
        Ok(())
    }

    /// Handles the answer to a `run.progress` read of `run`: reports the
    /// progress and reads the next one, until the last.
    async fn progress_answered(
        &mut self,
        reads: &mut Reads,
        run: RunId,
        answer: Result<Value, ErrorObject>,
        report: &impl Reports<Report>,
    ) -> io::Result<()> {
        let progress = match answer.map(serde_json::from_value::<RunProgress>) {
            Ok(Ok(progress)) => progress,
            Ok(Err(error)) => {
                tracing::warn!(%error, run_id = %run, "a run.progress answer the Kennel cannot read");
                reads.followed.insert(run, Followed::Ended);
                return Ok(());
            }
            Err(error) => {
                tracing::warn!(code = error.data.code, run_id = %run, "run.progress failed");
                reads.followed.insert(run, Followed::Ended);
                return Ok(());
            }
        };
        let next = (!progress.last).then_some(progress.position);
        report.report(Report::Progress(progress)).await;
        if let Some(position) = next {
            self.read_progress(reads, run, Some(position)).await
        } else {
            reads.followed.insert(run, Followed::Ended);
            Ok(())
        }
    }

    /// Sends the reads `wanted` asks for and cancels those it no longer
    /// does.
    pub(super) async fn keep_in_step(
        &mut self,
        reads: &mut Reads,
        wanted: &Wanted,
    ) -> io::Result<()> {
        if wanted.live_runs != reads.live_runs {
            reads.live_runs = wanted.live_runs;
            let params = RunListParams {
                lifecycle: Some(Lifecycle::Running),
                ..RunListParams::default()
            };
            self.ask(reads, RUN_LIST.name, &params, Pending::LiveRuns)
                .await?;
        }
        if wanted.listing != reads.listing {
            reads.listing.clone_from(&wanted.listing);
            if let Some(listing) = &wanted.listing {
                let pending = Pending::Listed(listing.generation);
                self.ask(reads, RUN_LIST.name, &listing.params, pending)
                    .await?;
            }
        }
        if wanted.usage != reads.usage {
            reads.usage.clone_from(&wanted.usage);
            if let Some(usage) = &wanted.usage {
                let params = RunParams {
                    run_id: usage.run_id.clone(),
                };
                let pending = Pending::Usage(usage.run_id.clone());
                self.ask(reads, RUN_USAGE.name, &params, pending).await?;
            }
        }
        if wanted.records.as_ref() != reads.records.as_ref().map(|(query, _)| query) {
            if let Some((_, Some(id))) = reads.records.take() {
                self.cancel(reads, id).await?;
            }
            if let Some(query) = &wanted.records {
                let pending = Pending::Queried(query.generation);
                let id = self
                    .ask(reads, DATASET_QUERY.name, &query.params, pending)
                    .await?;
                reads.records = Some((query.clone(), Some(id)));
            }
        }
        if wanted.explain != reads.explain {
            reads.explain = wanted.explain;
            if let Some(generation) = wanted.explain {
                let params = ConfigExplainParams::default();
                let pending = Pending::Explained(generation);
                self.ask(reads, CONFIG_EXPLAIN.name, &params, pending)
                    .await?;
            }
        }
        if wanted.system_info != self.system_info {
            self.resubscribe(reads, wanted.system_info).await?;
        }
        if wanted.report.as_ref() != reads.report.as_ref().map(|(run, _)| run) {
            if let Some((_, Some(id))) = reads.report.take() {
                self.cancel(reads, id).await?;
            }
            if let Some(run) = &wanted.report {
                let params = RunParams {
                    run_id: run.clone(),
                };
                let pending = Pending::Report(run.clone());
                let id = self.ask(reads, RUN_REPORT.name, &params, pending).await?;
                reads.report = Some((run.clone(), Some(id)));
            }
        }
        let dropped: Vec<RunId> = reads
            .followed
            .keys()
            .filter(|run| !wanted.progress.contains(*run))
            .cloned()
            .collect();
        for run in dropped {
            if let Some(Followed::Reading(id)) = reads.followed.remove(&run) {
                self.cancel(reads, id).await?;
            }
        }
        for run in &wanted.progress {
            if !reads.followed.contains_key(run) {
                self.read_progress(reads, run.clone(), None).await?;
            }
        }
        Ok(())
    }

    /// Reads `run`'s progress: at once, or after the value at `after`.
    async fn read_progress(
        &mut self,
        reads: &mut Reads,
        run: RunId,
        after: Option<u64>,
    ) -> io::Result<()> {
        let params = RunProgressParams {
            run_id: run.clone(),
            after,
        };
        let pending = Pending::Progress(run.clone());
        let id = self
            .ask(reads, RUN_PROGRESS_READ.name, &params, pending)
            .await?;
        reads.followed.insert(run, Followed::Reading(id));
        Ok(())
    }
}

/// The reads of one Connection.
pub(super) struct Reads {
    next_id: i64,
    /// The calls waiting for their answer.
    pub(super) pending: BTreeMap<i64, Pending>,
    /// The Runs whose progress is followed.
    followed: BTreeMap<RunId, Followed>,
    /// The [`Wanted::live_runs`] last read.
    live_runs: Generation,
    /// The [`Wanted::listing`] last read.
    listing: Option<Listing>,
    /// The [`Wanted::usage`] last read.
    usage: Option<UsageRead>,
    /// The Run whose report is read, with the id of the call that reads
    /// it until it answers.
    report: Option<(RunId, Option<i64>)>,
    /// The query of Records read, with the id of the call that reads it
    /// until it answers.
    records: Option<(Query, Option<i64>)>,
    /// The [`Wanted::explain`] last read.
    explain: Option<Generation>,
    /// What was wanted as the Connection opened.
    pub(super) first: Option<Wanted>,
}

impl Reads {
    /// The reads of a new Connection, which reads the live Runs' rows only
    /// once `wanted` asks again.
    pub(super) fn new(wanted: &mut watch::Receiver<Wanted>) -> Self {
        let first = wanted.borrow_and_update().clone();
        Self {
            next_id: FIRST_CALL_ID,
            pending: BTreeMap::new(),
            followed: BTreeMap::new(),
            live_runs: first.live_runs,
            listing: first.listing.clone(),
            usage: first.usage.clone(),
            report: None,
            // A new Connection reads the Records and the settings it is
            // asked for at once.
            records: None,
            explain: None,
            first: Some(first),
        }
    }

    /// The Connection's next request id, which no other request takes.
    pub(super) fn next_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// What a call waiting for its answer reads.
#[derive(Debug)]
pub(super) enum Pending {
    LiveRuns,
    Listed(Generation),
    Report(RunId),
    Usage(RunId),
    Progress(RunId),
    Queried(Generation),
    Explained(Generation),
    /// A person's call.
    Call(Call),
    /// A person's Add Seeds, streaming its seeds.
    Seeds(SeedStream),
}

/// Where a followed Run's progress is.
#[derive(Debug)]
enum Followed {
    /// The call with this id reads it.
    Reading(i64),
    /// Its last value came, or it cannot be read.
    Ended,
}

/// The failure an engine's Error gives a read or a call.
pub(super) fn failure(error: ErrorObject) -> CallFailure {
    CallFailure {
        code: error.data.code,
        message: error.message,
    }
}

/// The rows a `run.list` answered, or none when it failed.
fn listed(answer: Result<Value, ErrorObject>) -> Option<Vec<RunIndexRow>> {
    match answer.map(serde_json::from_value::<RunList>) {
        Ok(Ok(list)) => Some(list.runs),
        Ok(Err(error)) => {
            tracing::warn!(%error, "a run.list answer the Kennel cannot read");
            None
        }
        Err(error) => {
            tracing::warn!(code = error.data.code, "run.list failed");
            None
        }
    }
}

/// The answer to `method`, read as `T`, or why there is none.
fn decoded_answer<T: serde::de::DeserializeOwned>(
    method: &str,
    answer: Result<Value, ErrorObject>,
) -> Result<T, CallFailure> {
    answer.map_err(failure).and_then(|value| {
        serde_json::from_value::<T>(value).map_err(|error| unreadable(method, &error))
    })
}

/// The failure of an answer to `method` the Kennel cannot read.
pub(super) fn unreadable(method: &str, error: &serde_json::Error) -> CallFailure {
    tracing::warn!(%error, method, "an answer the Kennel cannot read");
    CallFailure {
        code: "invalid_answer".to_owned(),
        message: format!("The Daemon's answer to {method} could not be read"),
    }
}

/// Reports the events of a batch the Kennel reads, and gives back at once
/// the credits of a batch with none. The Resource governor's readings are
/// reported apart, the latest of the batch only, their credits given back
/// at once.
async fn events(
    envelopes: Vec<EventEnvelope>,
    applied: &watch::Sender<u64>,
    report: &impl Reports<Report>,
) {
    let credits = u64::try_from(envelopes.len()).unwrap_or(u64::MAX);
    let (readings, envelopes): (Vec<EventEnvelope>, Vec<EventEnvelope>) = envelopes
        .into_iter()
        .partition(|envelope| envelope.event == SYSTEM_INFO.name);
    if let Some(reading) = readings.into_iter().next_back() {
        match serde_json::from_value::<Box<SystemInfo>>(reading.data) {
            Ok(reading) => report.report(Report::SystemInfo(reading)).await,
            Err(error) => tracing::warn!(%error, "a governor reading the Kennel cannot read"),
        }
    }
    let events: Vec<DaemonEvent> = envelopes.into_iter().filter_map(decoded).collect();
    let applied = Applied {
        credits,
        to: applied.clone(),
    };
    if !events.is_empty() {
        report
            .report(Report::Events(Batch { events, applied }))
            .await;
    }
}

/// The event `envelope` carries, if the Kennel reads it.
fn decoded(envelope: EventEnvelope) -> Option<DaemonEvent> {
    let EventEnvelope { event, data, .. } = envelope;
    let decoded = if event == RESYNC.name {
        serde_json::from_value(data).map(DaemonEvent::Resync)
    } else if event == RUN_AXES_CHANGED.name {
        serde_json::from_value(data).map(DaemonEvent::RunAxes)
    } else if event == NEEDS_YOU_CHANGED.name {
        serde_json::from_value(data).map(DaemonEvent::NeedsYou)
    } else {
        return None;
    };
    decoded
        .inspect_err(|error| tracing::warn!(event, %error, "an event the Kennel cannot read"))
        .ok()
}
