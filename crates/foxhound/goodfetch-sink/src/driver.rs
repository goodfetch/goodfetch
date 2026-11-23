//! The delivery driver (ADR 0129): what every Sink shares. It reads the
//! run file's committed Records in number order per Dataset, sends them in
//! batches with their delivery IDs and the run epoch (ADR 0008), takes up
//! after the last Record the destination holds from this copy of the Run,
//! and turns each failure into what it leads to: a retry on the delivery
//! schedule, or a failed Sink. A copy with a new run epoch, as a resume
//! takes (ADR 0003), holds nothing yet and delivers every Record again,
//! in place of the older copy's.

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_dataset::Query;
use rusqlite::Connection;

use crate::contract::{
    Batch, Delivered, Failure, FailureKind, GuaranteeClass, Primitives, RunCopy, Written,
};

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
        }
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

    /// Delivers the Records `run_file` has committed past those the
    /// destination holds, in number order per Dataset, and says where the
    /// delivery stands.
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
        for dataset in &self.datasets {
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
                    break;
                };
                let batch = Batch {
                    run: self.run.clone(),
                    dataset: dataset.clone(),
                    records: page
                        .into_iter()
                        .map(|record| {
                            Delivered::new(
                                &self.run.run_id,
                                &self.sink_name,
                                dataset,
                                record.number,
                                record.committed_at,
                                record.value,
                            )
                        })
                        .collect(),
                };
                match self.sink.write(&batch) {
                    Ok(Written::Delivered) => {
                        delivered.insert(dataset.clone(), last);
                    }
                    Ok(Written::Superseded { run_epoch }) => {
                        return Ok(Status::Superseded { run_epoch });
                    }
                    Err(error) => return Ok(self.failed(error)),
                }
            }
        }
        self.delivered = Some(delivered);
        self.failures = 0;
        Ok(Status::Complete)
    }

    /// What the failure `error` leads to. The next attempt probes again,
    /// since a write whose answer was lost may have landed.
    fn failed(&mut self, error: S::Error) -> Status {
        let failure = self.sink.map_failure(error);
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
