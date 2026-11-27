//! Dead-letter retry, backfill, single-delivery redelivery and async
//! `timeout.sink_write`-bounded delivery on [`Driver`] (ADRs 0026, 0059
//! and 0080).

use std::collections::BTreeMap;
use std::future::Future;

use goodfetch_dataset::Query;
use goodfetch_engine::mask::Scope;
use rusqlite::Connection;
use serde_json::Value;

use crate::contract::{
    Batch, Delivered, DeliveryId, Failure, FailureKind, Primitives, RunCopy, Written,
};
use crate::dead_letters::{self, DeadLetter};
use crate::throughput::write_bounded;

use super::{Driver, RedeliverOutcome, RetryOutcome, Status, key_of, parse_delivery_target};

impl<S: Primitives> Driver<S> {
    /// Delivers pending Records asynchronously, bounding each write with
    /// `timeout.sink_write` on the driver's [`Clock`](goodfetch_engine::clock::Clock)
    /// (ADRs 0026, 0059 and 0080).
    ///
    /// `write` receives each batch sized to `batch.max_records` and
    /// `batch.max_bytes` and gated by the token bucket; if `write` does not
    /// finish before `self.write_timeout()`, its future is dropped, adaptive
    /// concurrency lowers, and delivery returns [`Status::Behind`] with
    /// `deadline_exceeded` so the next call retries on schedule.
    ///
    /// # Errors
    ///
    /// The run file's, when its Records cannot be read.
    pub async fn deliver_with<F, Fut>(
        &mut self,
        run_file: &Connection,
        mut write: F,
    ) -> rusqlite::Result<Status>
    where
        F: FnMut(Batch) -> Fut,
        Fut: Future<Output = Result<Written, Failure>>,
    {
        let mut delivered = match self.begin_delivery_pass() {
            Ok(delivered) => delivered,
            Err(early) => return Ok(early),
        };
        let rejected_before = self.dead_letters.len();
        let run = self.run.clone();
        for index in 0..self.datasets.len() {
            let dataset = self.datasets[index].clone();
            let fields = match self.keying.as_mut() {
                Some(keying) => Some(keying.fields(run_file, &dataset)?),
                None => None,
            };
            let declared_schema = self.declared_schema(run_file, &dataset)?;
            while let Some((batch, last)) =
                self.next_chunk(run_file, &dataset, fields.as_deref(), &mut delivered)?
            {
                if let Some(early) = self
                    .write_bounded_or_split(
                        &run,
                        &dataset,
                        declared_schema.as_ref(),
                        batch,
                        &mut write,
                    )
                    .await
                {
                    if matches!(
                        &early,
                        Status::Behind { failure, .. }
                            if failure.kind() == FailureKind::QuotaExhausted
                    ) {
                        self.delivered = Some(delivered);
                    }
                    return Ok(early);
                }
                delivered.insert(dataset.clone(), last);
            }
        }
        Ok(self.complete_delivery_pass(delivered, rejected_before))
    }

    async fn write_bounded_or_split<F, Fut>(
        &mut self,
        run: &RunCopy,
        dataset: &str,
        declared_schema: Option<&Value>,
        records: Vec<Delivered>,
        write: &mut F,
    ) -> Option<Status>
    where
        F: FnMut(Batch) -> Fut,
        Fut: Future<Output = Result<Written, Failure>>,
    {
        if records.is_empty() {
            return None;
        }
        let mut stack = vec![records];
        while let Some(mut chunk) = stack.pop() {
            if let Some(behind) = self.acquire_write_token() {
                return Some(behind);
            }
            let batch = Batch {
                run: run.clone(),
                dataset: dataset.to_owned(),
                declared_schema: declared_schema.cloned(),
                records: chunk,
            };
            let count = batch.records.len();
            let records_snapshot = batch.records.clone();
            match write_bounded(self.clock, self.write_timeout, write(batch)).await {
                Ok(Written::Delivered) => {
                    self.concurrency.record_success();
                    self.delivered_count = self
                        .delivered_count
                        .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
                    self.count_late_deliveries(&records_snapshot);
                }
                Ok(Written::Superseded { run_epoch }) => {
                    return Some(Status::Superseded { run_epoch });
                }
                Err(failure) => {
                    if failure.kind() != FailureKind::RejectedRecords {
                        return Some(self.handle_failure(failure));
                    }
                    chunk = records_snapshot;
                    if chunk.len() == 1 {
                        let record = chunk.pop().expect("invariant: length checked as 1");
                        let reason = failure.masked_reason(&self.masks);
                        self.push_dead_letter(run, dataset, record, &reason);
                    } else {
                        let mid = chunk.len() / 2;
                        let right = chunk.split_off(mid);
                        stack.push(right);
                        stack.push(chunk);
                    }
                }
            }
        }
        None
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
        if let Some((reason, failure)) = &self.paused {
            return Ok(RetryOutcome::empty(Status::Paused {
                reason: *reason,
                failure: failure.clone(),
            }));
        }
        if entries.is_empty() {
            return Ok(RetryOutcome::empty(Status::Complete));
        }
        if self.delivered.is_none() {
            match self.sink.probe(&self.run) {
                Ok(probe) => {
                    self.class = Some(probe.class);
                    self.delivered = Some(probe.delivered);
                }
                Err(error) => return Ok(RetryOutcome::empty(self.failed(error))),
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
            let declared_schema = self.declared_schema(run_file, &entry.dataset)?;
            let batch = Batch {
                run: run.clone(),
                dataset: entry.dataset.clone(),
                declared_schema,
                records: vec![Delivered {
                    delivery_id: entry.delivery_id.clone(),
                    number: entry.number,
                    committed_at: entry.committed_at.clone(),
                    value: entry.value.clone(),
                    key,
                }],
            };
            match self.sink.write_with_timeout(&batch, self.write_timeout) {
                Ok(Written::Delivered) => {
                    self.concurrency.record_success();
                    self.delivered_count = self.delivered_count.saturating_add(1);
                    self.count_late_deliveries(&batch.records);
                    delivered.push(entry.delivery_id.clone());
                }
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

    /// Backfills every committed Record of `run_file` from Record 1 into this
    /// Sink (including a Sink configured after the Run ended) (ADR 0026).
    ///
    /// # Errors
    ///
    /// The run file's, when its Records cannot be read.
    pub fn backfill(&mut self, run_file: &Connection) -> rusqlite::Result<Status> {
        if self.class.is_none() {
            match self.sink.probe(&self.run) {
                Ok(probe) => self.class = Some(probe.class),
                Err(error) => return Ok(self.failed(error)),
            }
        }
        self.delivered = Some(BTreeMap::new());
        self.deliver(run_file)
    }

    /// Redelivers a single delivery by its `delivery_id`
    /// (`<run_id>/<sink>/<dataset>/<number>`), preserving the exact same
    /// delivery ID so the destination can deduplicate (ADR 0026). Looks up
    /// `sink_dead_letters` first, then falls back to the Run's committed
    /// Dataset row.
    ///
    /// # Errors
    ///
    /// The run file's, when dead letters or Records cannot be read.
    pub fn redeliver(
        &mut self,
        run_file: &Connection,
        delivery_id: &str,
    ) -> rusqlite::Result<RedeliverOutcome> {
        if let Some(entry) = dead_letters::list(run_file, &self.sink_name)?
            .into_iter()
            .find(|entry| entry.delivery_id.as_str() == delivery_id)
        {
            let outcome = self.retry_dead_letters(run_file, &[entry])?;
            return Ok(RedeliverOutcome {
                found: true,
                delivered: !outcome.delivered.is_empty(),
                still_rejected: outcome.still_rejected.into_iter().next(),
                status: outcome.status,
            });
        }
        let Some((dataset, number)) = parse_delivery_target(delivery_id, &self.run.run_id) else {
            return Ok(RedeliverOutcome {
                found: false,
                delivered: false,
                still_rejected: None,
                status: Status::Complete,
            });
        };
        let page = goodfetch_dataset::read(
            run_file,
            Query {
                dataset: &dataset,
                after: number.saturating_sub(1),
                through: Some(number),
                wall: None,
            },
            1,
        )?;
        let Some(record) = page.into_iter().find(|r| r.number == number) else {
            return Ok(RedeliverOutcome {
                found: false,
                delivered: false,
                still_rejected: None,
                status: Status::Complete,
            });
        };
        let mut item = Delivered::new(
            &self.run.run_id,
            &self.sink_name,
            &dataset,
            record.number,
            record.committed_at,
            record.value,
        );
        item.delivery_id = DeliveryId::from_stored(delivery_id.to_owned());
        let entry = DeadLetter {
            sink: self.sink_name.clone(),
            delivery_id: item.delivery_id,
            run: self.run.clone(),
            dataset,
            number: item.number,
            committed_at: item.committed_at,
            value: item.value,
            reason: String::new(),
            attempts: 0,
        };
        let outcome = self.retry_dead_letters(run_file, &[entry])?;
        Ok(RedeliverOutcome {
            found: true,
            delivered: !outcome.delivered.is_empty(),
            still_rejected: outcome.still_rejected.into_iter().next(),
            status: outcome.status,
        })
    }
}
