//! The delivery driver (ADR 0129): what every Sink shares. It reads the
//! run file's committed Records in number order per Dataset, sends them in
//! batches with their delivery IDs and the run epoch (ADR 0008), takes up
//! after the last Record the destination holds from this copy of the Run,
//! and turns each failure into what it leads to: a retry on the delivery
//! schedule, or a failed Sink. A copy with a new run epoch, as a resume
//! takes (ADR 0003), holds nothing yet and delivers every Record again,
//! in place of the older copy's. Under a keyed write mode it gives each
//! Record its key's values, from the Sink's own key or its Dataset's
//! (ADR 0103).

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use goodfetch_dataset::{Declarations, Query};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::{Masks, Scope};
use goodfetch_protocol::foxhound::report::SinkStatus;
use rusqlite::Connection;
use serde_json::Value;

use crate::contract::{
    Batch, Delivered, DeliveryId, Failure, FailureKind, GuaranteeClass, Primitives, RunCopy,
    Written,
};
use crate::dead_letters::DeadLetter;
use crate::schedule::{
    self, PauseReason, RetryDecision, RetryState, SinkNotification, SinkNotificationEvent,
};
use crate::throughput::{
    AdaptiveConcurrency, BatchPolicy, DEFAULT_WRITE_TIMEOUT, RateLimit, Throughput, TokenBucket,
    record_wire_bytes,
};

mod bounded;

/// The most Records one write carries, unless set otherwise.
const BATCH_RECORDS: usize = 256;

/// Default maximum serialized Record bytes per write (16 MiB) unless set
/// otherwise.
const BATCH_BYTES: usize = 16 * 1024 * 1024;

/// Default starting adaptive concurrency limit.
const INITIAL_CONCURRENCY: usize = 4;

/// Default upper adaptive concurrency limit.
const MAX_CONCURRENCY: usize = 16;

/// Where a Sink's delivery stands after [`Driver::deliver`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every Record committed when it ran is at the destination.
    Complete,
    /// Every good Record committed when it ran is at the destination, and
    /// one or more bad Records were isolated and dead-lettered (ADR 0026).
    CompleteWithRejections,
    /// An attempt failed and will be retried on the schedule.
    Behind {
        /// Why it failed.
        failure: Failure,
        /// How long until the next attempt.
        retry_in: Duration,
        /// Wall-clock time of the next attempt.
        next_at: SystemTime,
    },
    /// The Sink was paused: either the destination answered HTTP 410 Gone,
    /// or three days of total failure elapsed (ADR 0026).
    Paused {
        /// Why it was paused.
        reason: PauseReason,
        /// The failure that triggered the pause.
        failure: Failure,
    },
    /// The Sink failed: no retry would deliver.
    Failed(Failure),
    /// A later copy of the Run delivers to this Sink: this copy is a
    /// zombie and stops.
    Superseded {
        /// The later copy's run epoch.
        run_epoch: u64,
    },
}

/// What retrying a Sink's dead letters did.
#[derive(Clone, Debug, PartialEq)]
pub struct RetryOutcome {
    /// The dead letters that succeeded and may be removed from the list.
    pub delivered: Vec<DeliveryId>,
    /// The dead letters the destination still rejected, with updated
    /// reasons and incremented attempt counts.
    pub still_rejected: Vec<DeadLetter>,
    /// Where the Sink stands after the retry pass.
    pub status: Status,
}

impl RetryOutcome {
    const fn empty(status: Status) -> Self {
        Self {
            delivered: Vec::new(),
            still_rejected: Vec::new(),
            status,
        }
    }
}

/// What redelivering a single delivery ID did (ADR 0026).
#[derive(Clone, Debug, PartialEq)]
pub struct RedeliverOutcome {
    /// Whether a committed Record or dead letter matched the delivery ID.
    pub found: bool,
    /// Whether the delivery succeeded.
    pub delivered: bool,
    /// Updated dead letter if the destination rejected it again.
    pub still_rejected: Option<DeadLetter>,
    /// Where the Sink stands after the redelivery.
    pub status: Status,
}

/// Delivers one copy of a Run's Datasets to one Sink.
pub struct Driver<S> {
    sink_name: String,
    sink: S,
    run: RunCopy,
    datasets: Vec<String>,
    batch_policy: BatchPolicy,
    bucket: Option<TokenBucket>,
    concurrency: AdaptiveConcurrency,
    write_timeout: Duration,
    clock: Clock,
    class: Option<GuaranteeClass>,
    /// How far the destination holds each Dataset, once probed; none until
    /// the probe, and again after a failure, when the destination may hold
    /// more than was answered.
    delivered: Option<BTreeMap<String, u64>>,
    /// The current failure streak for the Standard Webhooks schedule.
    retry_state: RetryState,
    /// When the next scheduled retry is due, if currently `Behind`.
    next_retry_at: Option<SystemTime>,
    /// Set when the Sink is paused (HTTP 410 or 3 days of total failure).
    paused: Option<(PauseReason, Failure)>,
    /// Whether the Sink was resumed after a pause so its next successful
    /// delivery queues a `sink_recovered` Notification.
    was_paused: bool,
    /// Whether any Record was dead-lettered during this Run.
    had_rejections: bool,
    /// How many Records were written successfully by this driver.
    delivered_count: u64,
    /// How many deliveries landed after the destination's deduplication
    /// window elapsed (ADR 0026).
    outside_dedup_window: u64,
    /// Latest Run-report status of this Sink.
    report_status: SinkStatus,
    /// How it keys its Records; none under `append`.
    keying: Option<Keying>,
    /// Each Dataset's declared JSON Schema from the run file, read once.
    declared_schemas: Option<BTreeMap<String, Option<Value>>>,
    /// The Run's Secret masks, applied to dead-letter Records and reasons.
    masks: Masks,
    /// Records isolated and dead-lettered during delivery, drained by the
    /// caller to write into `sink_dead_letters`.
    dead_letters: Vec<DeadLetter>,
    /// Notifications queued on pause/recovery, drained by the caller.
    notifications: Vec<SinkNotification>,
}

/// How a driver keys its Records under a keyed write mode.
struct Keying {
    /// The Sink's own key; none to take each Dataset's.
    own: Option<Vec<String>>,
    /// Each Dataset's key, as the run file declares it, read once.
    declared: Option<BTreeMap<String, Vec<String>>>,
}

impl Keying {
    /// The fields keying the Records of `dataset`: the Sink's own key, or
    /// else the Dataset's as the run file `run_file` declares it; empty
    /// when neither names any.
    fn fields(&mut self, run_file: &Connection, dataset: &str) -> rusqlite::Result<Vec<String>> {
        if let Some(own) = &self.own {
            return Ok(own.clone());
        }
        if self.declared.is_none() {
            let declarations = Declarations::read(run_file)?;
            self.declared = Some(
                declarations
                    .declarations()
                    .iter()
                    .map(|declaration| (declaration.name.clone(), declaration.key.clone()))
                    .collect(),
            );
        }
        Ok(self
            .declared
            .as_ref()
            .and_then(|declared| declared.get(dataset))
            .cloned()
            .unwrap_or_default())
    }
}

/// Parses `<run_id>/<sink>/<dataset>/<number>` for `run_id` into
/// `(dataset, number)`.
fn parse_delivery_target(delivery_id: &str, run_id: &str) -> Option<(String, u64)> {
    let mut parts = delivery_id.rsplitn(3, '/');
    let number: u64 = parts.next()?.parse().ok()?;
    if number == 0 {
        return None;
    }
    let dataset = parts.next()?;
    let prefix = parts.next()?;
    let rest = prefix.strip_prefix(run_id)?.strip_prefix('/')?;
    if dataset.is_empty() || rest.is_empty() {
        return None;
    }
    Some((dataset.to_owned(), number))
}

/// The values of `fields` in `value`, in order, as a JSON array; the first
/// field `value` lacks otherwise, or an empty one when `fields` is empty.
fn key_of(fields: &[String], value: &Value) -> Result<Value, String> {
    if fields.is_empty() {
        return Err(String::new());
    }
    fields
        .iter()
        .map(|field| value.get(field).cloned().ok_or_else(|| field.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

/// Side effects recorded by [`Driver`] during delivery passes: dead letters
/// isolated from rejected batches and Sink Notifications to persist.
#[derive(Clone, Debug, Default)]
pub struct DeliverySideEffects {
    /// Dead letters isolated since the last drain.
    pub dead_letters: Vec<DeadLetter>,
    /// Sink Notifications queued since the last drain.
    pub notifications: Vec<SinkNotification>,
}

impl<S: Primitives> Driver<S> {
    /// A driver of `sink`, named `sink_name`, delivering the Records of
    /// `datasets` for the copy `run` of a Run.
    #[must_use]
    pub fn new(sink_name: &str, sink: S, run: RunCopy, datasets: Vec<String>) -> Self {
        Self {
            sink_name: sink_name.to_owned(),
            sink,
            run,
            datasets,
            batch_policy: BatchPolicy::new(BATCH_RECORDS, BATCH_BYTES),
            bucket: None,
            concurrency: AdaptiveConcurrency::new(INITIAL_CONCURRENCY, MAX_CONCURRENCY),
            write_timeout: DEFAULT_WRITE_TIMEOUT,
            clock: Clock::system(),
            class: None,
            delivered: None,
            retry_state: RetryState::default(),
            next_retry_at: None,
            paused: None,
            was_paused: false,
            had_rejections: false,
            delivered_count: 0,
            outside_dedup_window: 0,
            report_status: SinkStatus::Complete,
            keying: None,
            declared_schemas: None,
            masks: Masks::new(),
            dead_letters: Vec::new(),
            notifications: Vec::new(),
        }
    }

    /// Reads wall time through `clock` (ADR 0059).
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Masks dead-letter Records and reasons with `masks` (ADR 0067).
    #[must_use]
    pub fn with_masks(mut self, masks: Masks) -> Self {
        self.masks = masks;
        self
    }

    /// Gives each Record its key under a keyed write mode: the values of
    /// the fields `own` names, the Sink's own `key`, or, when it names none,
    /// of its Dataset's key (ADR 0103). A Record that lacks one joins the
    /// Sink's dead letters.
    #[must_use]
    pub fn keyed(mut self, own: Option<Vec<String>>) -> Self {
        self.keying = Some(Keying {
            own: own.filter(|own| !own.is_empty()),
            declared: None,
        });
        self
    }

    /// Sends at most `records` Records in one write.
    #[must_use]
    pub fn with_batch_records(mut self, records: usize) -> Self {
        self.batch_policy = BatchPolicy::new(records, self.batch_policy.max_bytes);
        self
    }

    /// Configures rate limiting, batch sizing, adaptive concurrency and
    /// `timeout.sink_write` from a [`Throughput`] configuration (ADRs 0026
    /// and 0080).
    #[must_use]
    pub fn with_throughput(mut self, throughput: Throughput) -> Self {
        self.batch_policy = throughput.batch;
        self.bucket = Some(TokenBucket::new(throughput.rate));
        self.concurrency =
            AdaptiveConcurrency::new(throughput.initial_concurrency, throughput.max_concurrency);
        self.write_timeout = throughput.write_timeout;
        self
    }

    /// Configures the token bucket rate limit for this Sink (ADR 0026).
    #[must_use]
    pub fn with_rate(mut self, rate: RateLimit) -> Self {
        self.bucket = Some(TokenBucket::new(rate));
        self
    }

    /// Configures both `batch.max_records` and `batch.max_bytes` (ADR 0026).
    #[must_use]
    pub fn with_batch_policy(mut self, policy: BatchPolicy) -> Self {
        self.batch_policy = policy;
        self
    }

    /// Sets `timeout.sink_write`, bounding each write (ADR 0080).
    #[must_use]
    pub fn with_write_timeout(mut self, timeout: Duration) -> Self {
        self.write_timeout = timeout;
        self
    }

    /// Sets the adaptive concurrency starting and maximum limits (ADR 0026).
    #[must_use]
    pub fn with_concurrency(mut self, initial: usize, max: usize) -> Self {
        self.concurrency = AdaptiveConcurrency::new(initial, max);
        self
    }

    /// Current adaptive concurrency limit (lowers on 429/timeout and recovers
    /// on consecutive successes, ADR 0026).
    #[must_use]
    pub fn concurrency_limit(&self) -> usize {
        self.concurrency.limit()
    }

    /// Configured `timeout.sink_write` for this driver.
    #[must_use]
    pub fn write_timeout(&self) -> Duration {
        self.write_timeout
    }

    /// The guarantee class the Sink's last probe reported; none before one
    /// answered.
    #[must_use]
    pub fn class(&self) -> Option<GuaranteeClass> {
        self.class
    }

    /// Why this Sink is currently paused, if paused.
    #[must_use]
    pub fn paused_reason(&self) -> Option<PauseReason> {
        self.paused.as_ref().map(|(reason, _)| *reason)
    }

    /// The Run-report status of this Sink (`complete`,
    /// `complete_with_rejections`, `behind`, or `failed`).
    #[must_use]
    pub fn report_status(&self) -> SinkStatus {
        self.report_status
    }

    /// How many Records were written successfully by this driver.
    #[must_use]
    pub fn delivered_count(&self) -> u64 {
        self.delivered_count
    }

    /// How many deliveries landed after the destination's deduplication
    /// window elapsed, and so were at least once (ADR 0026).
    #[must_use]
    pub fn outside_dedup_window(&self) -> u64 {
        self.outside_dedup_window
    }

    /// Duration until the earliest buffered part expires (Files mode
    /// `max_age`), if any.
    #[must_use]
    pub fn next_flush_after(&self) -> Option<Duration> {
        self.sink.next_flush_after()
    }

    /// Remaining wait before the next scheduled retry, when this driver is
    /// already `Behind` and its retry backoff has not yet elapsed.
    #[must_use]
    pub fn pending_retry(&self) -> Option<Duration> {
        let at = self.next_retry_at?;
        let remaining = at.duration_since(self.clock.wall()).ok()?;
        (!remaining.is_zero()).then_some(remaining)
    }

    /// Resumes a paused Sink so subsequent [`Self::deliver`] calls attempt
    /// delivery again; the next successful delivery queues a
    /// `sink_recovered` Notification (ADR 0026).
    pub fn resume(&mut self) {
        if self.paused.take().is_some() {
            self.was_paused = true;
            self.retry_state.clear();
        }
    }

    /// Drains the dead letters and Sink Notifications recorded since the last
    /// call, for the caller to persist in `sink_dead_letters` and
    /// `sink_notifications`.
    pub fn drain_side_effects(&mut self) -> DeliverySideEffects {
        DeliverySideEffects {
            dead_letters: std::mem::take(&mut self.dead_letters),
            notifications: std::mem::take(&mut self.notifications),
        }
    }

    /// Consumes the driver and returns its underlying Sink.
    #[must_use]
    pub fn into_sink(self) -> S {
        self.sink
    }

    /// Probes the Sink now, as the first delivery would, and answers the
    /// guarantee class its destination proves; the next delivery takes up
    /// after what the probe found the destination holds (ADR 0026).
    ///
    /// # Errors
    ///
    /// The probe's failure, as the Sink's translation point maps it; the
    /// next delivery probes again.
    pub fn probe(&mut self) -> Result<GuaranteeClass, Failure> {
        match self.sink.probe(&self.run) {
            Ok(probe) => {
                self.class = Some(probe.class);
                self.delivered = Some(probe.delivered);
                Ok(probe.class)
            }
            Err(error) => {
                self.delivered = None;
                Err(self.sink.map_failure(error))
            }
        }
    }

    /// Checks pause state and ensures the destination has been probed,
    /// returning the per-Dataset delivered cursor or an early [`Status`].
    pub(super) fn begin_delivery_pass(&mut self) -> Result<BTreeMap<String, u64>, Status> {
        if let Some((reason, failure)) = &self.paused {
            return Err(Status::Paused {
                reason: *reason,
                failure: failure.clone(),
            });
        }
        match self.delivered.take() {
            Some(delivered) => Ok(delivered),
            None => match self.sink.probe(&self.run) {
                Ok(probe) => {
                    self.class = Some(probe.class);
                    Ok(probe.delivered)
                }
                Err(error) => Err(self.failed(error)),
            },
        }
    }

    /// Finalizes a successful delivery pass: stores `delivered`, clears any
    /// failure streak, emits recovery if resumed from pause, and returns
    /// [`Status::Complete`] or [`Status::CompleteWithRejections`].
    pub(super) fn complete_delivery_pass(
        &mut self,
        delivered: BTreeMap<String, u64>,
        rejected_before: usize,
    ) -> Status {
        self.delivered = Some(delivered);
        self.retry_state.clear();
        self.next_retry_at = None;
        if self.was_paused {
            self.was_paused = false;
            self.notifications.push(SinkNotification {
                sink: self.sink_name.clone(),
                run_id: self.run.run_id.clone(),
                event: SinkNotificationEvent::Recovered,
                reason: "Sink delivery recovered".to_owned(),
                occurred_at: self.clock.wall(),
            });
        }
        let had_rejections_now = self.dead_letters.len() > rejected_before;
        self.had_rejections |= had_rejections_now;
        self.report_status = if self.had_rejections {
            SinkStatus::CompleteWithRejections
        } else {
            SinkStatus::Complete
        };
        if had_rejections_now {
            Status::CompleteWithRejections
        } else {
            Status::Complete
        }
    }

    /// Delivers the Records `run_file` has committed past those the
    /// destination holds, in number order per Dataset, and says where the
    /// delivery stands. A rejected batch is split recursively until every
    /// bad Record is isolated onto [`Self::drain_side_effects`] (ADR 0026);
    /// a Record lacking a key field under a keyed mode joins them too
    /// (ADR 0103).
    ///
    /// # Errors
    ///
    /// The run file's, when its Records cannot be read.
    pub fn deliver(&mut self, run_file: &Connection) -> rusqlite::Result<Status> {
        let mut delivered = match self.begin_delivery_pass() {
            Ok(delivered) => delivered,
            Err(early) => return Ok(early),
        };
        let rejected_before = self.dead_letters.len();
        for index in 0..self.datasets.len() {
            let dataset = self.datasets[index].clone();
            let fields = match self.keying.as_mut() {
                Some(keying) => Some(keying.fields(run_file, &dataset)?),
                None => None,
            };
            let declared_schema = self.declared_schema(run_file, &dataset)?;
            let delivered_before = self.delivered_count;
            if let Some(early) = self.deliver_dataset(
                run_file,
                &dataset,
                fields.as_deref(),
                declared_schema.as_ref(),
                &mut delivered,
            )? {
                if matches!(
                    &early,
                    Status::Behind { failure, .. }
                        if failure.kind() == FailureKind::QuotaExhausted
                ) {
                    // Leave `self.delivered = None` if a split partially
                    // succeeded so the next pass re-probes rather than
                    // re-delivering the already-written prefix.
                    if self.delivered_count == delivered_before {
                        self.delivered = Some(delivered);
                    }
                }
                return Ok(early);
            }
        }
        match self.sink.flush_expired(&self.run) {
            Ok(Written::Delivered) => {}
            Ok(Written::Superseded { run_epoch }) => {
                return Ok(Status::Superseded { run_epoch });
            }
            Err(err) => {
                let failure = self.sink.map_failure(err);
                return Ok(self.handle_failure(failure));
            }
        }
        Ok(self.complete_delivery_pass(delivered, rejected_before))
    }

    /// Reads the declared JSON Schema for `dataset` from `run_file`, caching
    /// the run file's Dataset declarations on first call.
    pub(super) fn declared_schema(
        &mut self,
        run_file: &Connection,
        dataset: &str,
    ) -> rusqlite::Result<Option<Value>> {
        if self.declared_schemas.is_none() {
            let declarations = Declarations::read(run_file)?;
            self.declared_schemas = Some(
                declarations
                    .declarations()
                    .iter()
                    .map(|declaration| (declaration.name.clone(), declaration.schema.clone()))
                    .collect(),
            );
        }
        Ok(self
            .declared_schemas
            .as_ref()
            .and_then(|schemas| schemas.get(dataset))
            .cloned()
            .flatten())
    }

    /// Reads the next write-sized chunk of committed Records of `dataset`
    /// past `delivered`, respecting `batch.max_records` and `batch.max_bytes`
    /// and dead-lettering Records missing a required key field.
    pub(super) fn next_chunk(
        &mut self,
        run_file: &Connection,
        dataset: &str,
        fields: Option<&[String]>,
        delivered: &mut BTreeMap<String, u64>,
    ) -> rusqlite::Result<Option<(Vec<Delivered>, u64)>> {
        let run = self.run.clone();
        loop {
            let after = delivered.get(dataset).copied().unwrap_or(0);
            let page = goodfetch_dataset::read(
                run_file,
                Query {
                    dataset,
                    after,
                    through: None,
                    wall: None,
                },
                self.batch_policy.max_records,
            )?;
            if page.is_empty() {
                return Ok(None);
            }
            let mut batch = Vec::with_capacity(page.len());
            let mut used_bytes = 0usize;
            let mut last_in_batch = after;
            let mut skipped_to = after;
            for record in page {
                let mut item = Delivered::new(
                    &self.run.run_id,
                    &self.sink_name,
                    dataset,
                    record.number,
                    record.committed_at,
                    record.value,
                );
                if let Some(fields) = fields {
                    match key_of(fields, &item.value) {
                        Ok(key) => item.key = Some(key),
                        Err(field) => {
                            if !batch.is_empty() {
                                return Ok(Some((batch, last_in_batch)));
                            }
                            let reason = format!(
                                "Record {} of the Dataset `{dataset}` has no key field `{field}`",
                                item.number
                            );
                            tracing::warn!(
                                sink = %self.sink_name,
                                %dataset,
                                number = item.number,
                                %field,
                                "a Record lacks a field of its key: dead-lettered"
                            );
                            skipped_to = item.number;
                            self.push_dead_letter(&run, dataset, item, &reason);
                            continue;
                        }
                    }
                }
                let item_bytes = record_wire_bytes(&item);
                if !batch.is_empty()
                    && used_bytes.saturating_add(item_bytes) > self.batch_policy.max_bytes
                {
                    return Ok(Some((batch, last_in_batch)));
                }
                used_bytes = used_bytes.saturating_add(item_bytes);
                last_in_batch = item.number;
                batch.push(item);
            }
            if !batch.is_empty() {
                return Ok(Some((batch, last_in_batch)));
            }
            delivered.insert(dataset.to_owned(), skipped_to);
        }
    }

    /// Acquires one write token from the Sink's token bucket, or returns a
    /// [`Status::Behind`] (`FailureKind::QuotaExhausted`) with the exact wait
    /// without marking the Sink `failed` (ADR 0026).
    pub(super) fn acquire_write_token(&mut self) -> Option<Status> {
        let bucket = self.bucket.as_mut()?;
        let now = self.clock.wall();
        match bucket.try_acquire(now) {
            Ok(()) => None,
            Err(wait) => {
                let next_at = now + wait;
                self.next_retry_at = Some(next_at);
                self.report_status = SinkStatus::Behind;
                let failure = Failure::new(
                    FailureKind::QuotaExhausted,
                    "Sink token bucket exhausted; waiting for quota refill",
                )
                .with_retry_after(wait);
                Some(Status::Behind {
                    failure,
                    retry_in: wait,
                    next_at,
                })
            }
        }
    }

    /// Delivers every committed page of `dataset` past `delivered`,
    /// returning early on supersession, rate limit or a non-record failure.
    fn deliver_dataset(
        &mut self,
        run_file: &Connection,
        dataset: &str,
        fields: Option<&[String]>,
        declared_schema: Option<&Value>,
        delivered: &mut BTreeMap<String, u64>,
    ) -> rusqlite::Result<Option<Status>> {
        let run = self.run.clone();
        while let Some((batch, last)) = self.next_chunk(run_file, dataset, fields, delivered)? {
            if let Some(early) = self.write_or_split(&run, dataset, declared_schema, batch) {
                return Ok(Some(early));
            }
            delivered.insert(dataset.to_owned(), last);
        }
        Ok(None)
    }

    /// Writes `records` for `dataset`; on `RejectedRecords`, splits the
    /// batch recursively until bad singletons are dead-lettered and every
    /// good Record lands in order.
    fn write_or_split(
        &mut self,
        run: &RunCopy,
        dataset: &str,
        declared_schema: Option<&Value>,
        mut records: Vec<Delivered>,
    ) -> Option<Status> {
        if records.is_empty() {
            return None;
        }
        if let Some(behind) = self.acquire_write_token() {
            return Some(behind);
        }
        let batch = Batch {
            run: run.clone(),
            dataset: dataset.to_owned(),
            declared_schema: declared_schema.cloned(),
            records,
        };
        match self.sink.write_with_timeout(&batch, self.write_timeout) {
            Ok(Written::Delivered) => {
                self.concurrency.record_success();
                self.delivered_count = self
                    .delivered_count
                    .saturating_add(u64::try_from(batch.records.len()).unwrap_or(u64::MAX));
                self.count_late_deliveries(&batch.records);
                None
            }
            Ok(Written::Superseded { run_epoch }) => Some(Status::Superseded { run_epoch }),
            Err(error) => {
                let failure = self.sink.map_failure(error);
                if failure.kind() != FailureKind::RejectedRecords {
                    return Some(self.handle_failure(failure));
                }
                records = batch.records;
                if records.len() == 1 {
                    let record = records.pop().expect("invariant: length checked as 1");
                    let reason = failure.masked_reason(&self.masks);
                    tracing::warn!(
                        sink = %self.sink_name,
                        %dataset,
                        number = record.number,
                        "a Record was rejected by its destination: dead-lettered"
                    );
                    self.push_dead_letter(run, dataset, record, &reason);
                    return None;
                }
                let mid = records.len() / 2;
                let right = records.split_off(mid);
                if let Some(early) = self.write_or_split(run, dataset, declared_schema, records) {
                    return Some(early);
                }
                self.write_or_split(run, dataset, declared_schema, right)
            }
        }
    }

    /// Counts delivered Records whose commit timestamp is older than the
    /// destination's deduplication window relative to `self.clock.wall()`.
    pub(super) fn count_late_deliveries(&mut self, records: &[Delivered]) {
        let Some(window) = self.sink.dedup_window() else {
            return;
        };
        let now = self.clock.wall();
        for record in records {
            let Ok(ts) = record.committed_at.parse::<jiff::Timestamp>() else {
                continue;
            };
            let committed = SystemTime::from(ts);
            if let Ok(age) = now.duration_since(committed)
                && age > window
            {
                self.outside_dedup_window = self.outside_dedup_window.saturating_add(1);
            }
        }
    }

    /// Buffers one isolated Record as a masked [`DeadLetter`].
    pub(super) fn push_dead_letter(
        &mut self,
        run: &RunCopy,
        dataset: &str,
        mut record: Delivered,
        reason: &str,
    ) {
        self.masks.apply_value(&mut record.value, Scope::Kept);
        let reason = self.masks.apply(reason, Scope::Kept).into_owned();
        self.dead_letters.push(DeadLetter {
            sink: self.sink_name.clone(),
            delivery_id: record.delivery_id,
            run: run.clone(),
            dataset: dataset.to_owned(),
            number: record.number,
            committed_at: record.committed_at,
            value: record.value,
            reason,
            attempts: 1,
        });
    }

    /// Ends the delivery of a Run that finished, once [`Self::deliver`]
    /// answered [`Status::Complete`] or [`Status::CompleteWithRejections`]:
    /// a Sink that wrote its Records apart, as `replace` does, swaps them
    /// in (ADR 0026).
    pub fn finish(&mut self) -> Status {
        match self.sink.finish(&self.run) {
            Ok(Written::Delivered) => Status::Complete,
            Ok(Written::Superseded { run_epoch }) => Status::Superseded { run_epoch },
            Err(error) => self.failed(error),
        }
    }

    /// What the failure `error` leads to. The next attempt probes again,
    /// since a write whose answer was lost may have landed.
    pub(super) fn failed(&mut self, error: S::Error) -> Status {
        let failure = self.sink.map_failure(error);
        self.handle_failure(failure)
    }

    pub(super) fn handle_failure(&mut self, failure: Failure) -> Status {
        if failure.is_rate_limited() || failure.is_timeout() {
            self.concurrency.record_throttle();
        }
        match failure.kind() {
            FailureKind::QuotaExhausted => {
                let now = self.clock.wall();
                let wait = failure.retry_after().unwrap_or(Duration::from_secs(1));
                let next_at = now + wait;
                self.next_retry_at = Some(next_at);
                self.report_status = SinkStatus::Behind;
                Status::Behind {
                    failure,
                    retry_in: wait,
                    next_at,
                }
            }
            FailureKind::Transient | FailureKind::Unrecognized => {
                if failure.kind() == FailureKind::Unrecognized {
                    tracing::warn!(sink = %self.sink_name, ?failure, "a Sink's failure was not recognized");
                }
                let now = self.clock.wall();
                let (next_state, decision) =
                    schedule::next_retry(&self.retry_state, now, failure.retry_after());
                self.retry_state = next_state;
                match decision {
                    RetryDecision::Retry { wait, next_at } => {
                        self.next_retry_at = Some(next_at);
                        self.report_status = SinkStatus::Behind;
                        Status::Behind {
                            failure,
                            retry_in: wait,
                            next_at,
                        }
                    }
                    RetryDecision::Pause { reason } => {
                        self.next_retry_at = None;
                        self.paused = Some((reason, failure.clone()));
                        self.report_status = SinkStatus::Failed;
                        self.notifications.push(SinkNotification {
                            sink: self.sink_name.clone(),
                            run_id: self.run.run_id.clone(),
                            event: SinkNotificationEvent::Paused,
                            reason: failure.masked_reason(&self.masks),
                            occurred_at: now,
                        });
                        Status::Paused { reason, failure }
                    }
                }
            }
            FailureKind::Gone => {
                self.paused = Some((PauseReason::Gone, failure.clone()));
                self.report_status = SinkStatus::Failed;
                Status::Paused {
                    reason: PauseReason::Gone,
                    failure,
                }
            }
            FailureKind::RejectedRecords | FailureKind::Auth | FailureKind::Permanent => {
                self.report_status = SinkStatus::Failed;
                Status::Failed(failure)
            }
        }
    }
}
