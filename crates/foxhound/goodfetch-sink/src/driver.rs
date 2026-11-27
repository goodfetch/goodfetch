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
use std::time::Duration;

use goodfetch_dataset::{Declarations, Query};
use goodfetch_engine::mask::{Masks, Scope};
use rusqlite::Connection;
use serde_json::Value;

use crate::contract::{
    Batch, Delivered, DeliveryId, Failure, FailureKind, GuaranteeClass, Primitives, RunCopy,
    Written,
};
use crate::dead_letters::DeadLetter;

/// How long after each failed attempt the next one waits: the Standard
/// Webhooks schedule, whose first attempt is immediate. A Sink still
/// failing after the last is failed.
const SCHEDULE: [Duration; 9] = [
    Duration::from_secs(5),
    Duration::from_mins(5),
    Duration::from_mins(30),
    Duration::from_hours(2),
    Duration::from_hours(5),
    Duration::from_hours(10),
    Duration::from_hours(14),
    Duration::from_hours(20),
    Duration::from_hours(24),
];

/// The most Records one write carries, unless set otherwise.
const BATCH_RECORDS: usize = 256;

/// Where a Sink's delivery stands after [`Driver::deliver`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Every Record committed when it ran is at the destination.
    Complete,
    /// Every good Record committed when it ran is at the destination, and
    /// one or more bad Records were isolated and dead-lettered (ADR 0026).
    CompleteWithRejections,
    /// An attempt failed and will be retried.
    Behind {
        /// Why it failed.
        failure: Failure,
        /// How long until the next attempt.
        retry_in: Duration,
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

/// Delivers one copy of a Run's Datasets to one Sink.
pub struct Driver<S> {
    sink_name: String,
    sink: S,
    run: RunCopy,
    datasets: Vec<String>,
    batch_records: usize,
    class: Option<GuaranteeClass>,
    /// How far the destination holds each Dataset, once probed; none until
    /// the probe, and again after a failure, when the destination may hold
    /// more than was answered.
    delivered: Option<BTreeMap<String, u64>>,
    /// Attempts failed in a row.
    failures: usize,
    /// How it keys its Records; none under `append`.
    keying: Option<Keying>,
    /// The Run's Secret masks, applied to dead-letter Records and reasons.
    masks: Masks,
    /// Records isolated and dead-lettered during delivery, drained by the
    /// caller to write into `sink_dead_letters`.
    dead_letters: Vec<DeadLetter>,
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
            batch_records: BATCH_RECORDS,
            class: None,
            delivered: None,
            failures: 0,
            keying: None,
            masks: Masks::new(),
            dead_letters: Vec::new(),
        }
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
        self.batch_records = records.max(1);
        self
    }

    /// The guarantee class the Sink's last probe reported; none before one
    /// answered.
    #[must_use]
    pub fn class(&self) -> Option<GuaranteeClass> {
        self.class
    }

    /// Drains the dead letters isolated since the last call, for the caller
    /// to persist in `sink_dead_letters`.
    pub fn take_dead_letters(&mut self) -> Vec<DeadLetter> {
        std::mem::take(&mut self.dead_letters)
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

    /// Delivers the Records `run_file` has committed past those the
    /// destination holds, in number order per Dataset, and says where the
    /// delivery stands. A rejected batch is split recursively until every
    /// bad Record is isolated onto [`Self::take_dead_letters`] (ADR 0026);
    /// a Record lacking a key field under a keyed mode joins them too
    /// (ADR 0103).
    ///
    /// # Errors
    ///
    /// The run file's, when its Records cannot be read.
    pub fn deliver(&mut self, run_file: &Connection) -> rusqlite::Result<Status> {
        let mut delivered = match self.delivered.take() {
            Some(delivered) => delivered,
            None => match self.sink.probe(&self.run) {
                Ok(probe) => {
                    self.class = Some(probe.class);
                    probe.delivered
                }
                Err(error) => return Ok(self.failed(error)),
            },
        };
        let rejected_before = self.dead_letters.len();
        for index in 0..self.datasets.len() {
            let dataset = self.datasets[index].clone();
            let fields = match self.keying.as_mut() {
                Some(keying) => Some(keying.fields(run_file, &dataset)?),
                None => None,
            };
            if let Some(early) =
                self.deliver_dataset(run_file, &dataset, fields.as_deref(), &mut delivered)?
            {
                return Ok(early);
            }
        }
        self.delivered = Some(delivered);
        self.failures = 0;
        Ok(if self.dead_letters.len() > rejected_before {
            Status::CompleteWithRejections
        } else {
            Status::Complete
        })
    }

    /// Delivers every committed page of `dataset` past `delivered`,
    /// returning early on supersession or a non-record failure.
    fn deliver_dataset(
        &mut self,
        run_file: &Connection,
        dataset: &str,
        fields: Option<&[String]>,
        delivered: &mut BTreeMap<String, u64>,
    ) -> rusqlite::Result<Option<Status>> {
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
                self.batch_records,
            )?;
            let Some(last) = page.last().map(|record| record.number) else {
                return Ok(None);
            };
            let mut batch = Vec::with_capacity(page.len());
            for record in page {
                let mut item = Delivered::new(
                    &self.run.run_id,
                    &self.sink_name,
                    dataset,
                    record.number,
                    record.committed_at,
                    record.value,
                );
                match fields {
                    Some(fields) => match key_of(fields, &item.value) {
                        Ok(key) => {
                            item.key = Some(key);
                            batch.push(item);
                        }
                        Err(field) => {
                            if let Some(early) =
                                self.write_or_split(&run, dataset, std::mem::take(&mut batch))
                            {
                                return Ok(Some(early));
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
                            self.push_dead_letter(&run, dataset, item, &reason);
                        }
                    },
                    None => batch.push(item),
                }
            }
            if let Some(early) = self.write_or_split(&run, dataset, batch) {
                return Ok(Some(early));
            }
            delivered.insert(dataset.to_owned(), last);
        }
    }

    /// Writes `records` for `dataset`; on `RejectedRecords`, splits the
    /// batch recursively until bad singletons are dead-lettered and every
    /// good Record lands in order.
    fn write_or_split(
        &mut self,
        run: &RunCopy,
        dataset: &str,
        mut records: Vec<Delivered>,
    ) -> Option<Status> {
        if records.is_empty() {
            return None;
        }
        let batch = Batch {
            run: run.clone(),
            dataset: dataset.to_owned(),
            records,
        };
        match self.sink.write(&batch) {
            Ok(Written::Delivered) => None,
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
                if let Some(early) = self.write_or_split(run, dataset, records) {
                    return Some(early);
                }
                self.write_or_split(run, dataset, right)
            }
        }
    }

    /// Buffers one isolated Record as a masked [`DeadLetter`].
    fn push_dead_letter(
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

    /// Computes `entry`'s key for a retry, or returns the updated rejected
    /// [`DeadLetter`] when a keyed Sink's Record still lacks a key field.
    fn retry_key(
        &mut self,
        run_file: &Connection,
        entry: &DeadLetter,
    ) -> rusqlite::Result<Result<Option<Value>, DeadLetter>> {
        let Some(keying) = self.keying.as_mut() else {
            return Ok(Ok(None));
        };
        let fields = keying.fields(run_file, &entry.dataset)?;
        Ok(match key_of(&fields, &entry.value) {
            Ok(key) => Ok(Some(key)),
            Err(field) => {
                let reason = format!(
                    "Record {} of the Dataset `{}` has no key field `{field}`",
                    entry.number, entry.dataset
                );
                Err(DeadLetter {
                    reason: self.masks.apply(&reason, Scope::Kept).into_owned(),
                    attempts: entry.attempts.saturating_add(1),
                    ..entry.clone()
                })
            }
        })
    }

    /// Retries each dead letter in `entries` one by one against the
    /// destination (ADR 0026), reading any Dataset key from `run_file`
    /// under a keyed mode. Reports which succeeded (to remove) and which
    /// were rejected again (to keep with incremented attempt counts).
    ///
    /// # Errors
    ///
    /// The run file's, when a Dataset's declared key cannot be read.
    pub fn retry_dead_letters(
        &mut self,
        run_file: &Connection,
        entries: &[DeadLetter],
    ) -> rusqlite::Result<RetryOutcome> {
        if entries.is_empty() {
            return Ok(RetryOutcome {
                delivered: Vec::new(),
                still_rejected: Vec::new(),
                status: Status::Complete,
            });
        }
        if self.delivered.is_none() {
            match self.sink.probe(&self.run) {
                Ok(probe) => {
                    self.class = Some(probe.class);
                    self.delivered = Some(probe.delivered);
                }
                Err(error) => {
                    return Ok(RetryOutcome {
                        delivered: Vec::new(),
                        still_rejected: Vec::new(),
                        status: self.failed(error),
                    });
                }
            }
        }
        let mut delivered = Vec::new();
        let mut still_rejected = Vec::new();
        for entry in entries {
            let key = match self.retry_key(run_file, entry)? {
                Ok(key) => key,
                Err(rejected) => {
                    still_rejected.push(rejected);
                    continue;
                }
            };
            let run = if entry.run.run_id == self.run.run_id {
                self.run.clone()
            } else {
                entry.run.clone()
            };
            let batch = Batch {
                run: run.clone(),
                dataset: entry.dataset.clone(),
                records: vec![Delivered {
                    delivery_id: entry.delivery_id.clone(),
                    number: entry.number,
                    committed_at: entry.committed_at.clone(),
                    value: entry.value.clone(),
                    key,
                }],
            };
            match self.sink.write(&batch) {
                Ok(Written::Delivered) => delivered.push(entry.delivery_id.clone()),
                Ok(Written::Superseded { run_epoch }) => {
                    return Ok(RetryOutcome {
                        delivered,
                        still_rejected,
                        status: Status::Superseded { run_epoch },
                    });
                }
                Err(error) => {
                    let failure = self.sink.map_failure(error);
                    if failure.kind() == FailureKind::RejectedRecords {
                        still_rejected.push(DeadLetter {
                            run,
                            reason: failure.masked_reason(&self.masks),
                            attempts: entry.attempts.saturating_add(1),
                            ..entry.clone()
                        });
                    } else {
                        self.delivered = None;
                        return Ok(RetryOutcome {
                            delivered,
                            still_rejected,
                            status: self.handle_failure(failure),
                        });
                    }
                }
            }
        }
        let status = if still_rejected.is_empty() {
            Status::Complete
        } else {
            Status::CompleteWithRejections
        };
        Ok(RetryOutcome {
            delivered,
            still_rejected,
            status,
        })
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
    fn failed(&mut self, error: S::Error) -> Status {
        let failure = self.sink.map_failure(error);
        self.handle_failure(failure)
    }

    fn handle_failure(&mut self, failure: Failure) -> Status {
        match failure.kind() {
            FailureKind::Transient | FailureKind::Unrecognized => {
                if failure.kind() == FailureKind::Unrecognized {
                    tracing::warn!(sink = %self.sink_name, ?failure, "a Sink's failure was not recognized");
                }
                let retry_in = SCHEDULE.get(self.failures).copied();
                self.failures += 1;
                match retry_in {
                    Some(retry_in) => Status::Behind { failure, retry_in },
                    None => Status::Failed(failure),
                }
            }
            FailureKind::RejectedRecords | FailureKind::Auth | FailureKind::Gone => {
                Status::Failed(failure)
            }
        }
    }
}
