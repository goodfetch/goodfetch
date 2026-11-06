//! Add Seeds on the wire: a `run.add_seeds` call streams its seeds within
//! the credits the call grants, counts the seeds the crawl did not add, and
//! reports the counts before the call's answer.

use std::collections::BTreeMap;

use goodfetch_protocol::foxhound::frontier::{Enqueued, RUN_ADD_SEEDS, RunSeedsAdded, SeedBatch};
use goodfetch_protocol::jsonrpc::{ErrorObject, RequestId};
use serde_json::Value;

use super::connection::{failure, unreadable};
use super::{Call, Report, SeedsAdded};
use crate::reports::Reports;

/// An Add Seeds call streaming its seeds within the credits it grants.
#[derive(Debug)]
pub(super) struct SeedStream {
    /// The call, a [`Call::AddSeeds`].
    pub(super) call: Call,
    /// How many of its seeds were sent.
    sent: usize,
    /// Whether the batch marked `end` was sent.
    ended: bool,
    /// The credits granted and not yet spent.
    credits: u64,
    /// The seeds not added so far, by reason.
    not_added: BTreeMap<String, u64>,
}

impl SeedStream {
    pub(super) fn new(call: Call) -> Self {
        Self {
            call,
            sent: 0,
            ended: false,
            credits: 0,
            not_added: BTreeMap::new(),
        }
    }

    /// The next batch of the call `id` once it granted `credits` more:
    /// as many seeds as the credits allow, the last batch marked `end`;
    /// none once that batch was sent.
    pub(super) fn next_batch(&mut self, id: i64, credits: u32) -> Option<SeedBatch> {
        let Call::AddSeeds(to_add) = &self.call else {
            return None;
        };
        if self.ended {
            return None;
        }
        self.credits += u64::from(credits);
        let left = &to_add.seeds[self.sent..];
        let taken = left
            .len()
            .min(usize::try_from(self.credits).unwrap_or(usize::MAX));
        if taken == 0 && !left.is_empty() {
            return None;
        }
        self.sent += taken;
        self.credits -= u64::try_from(taken).unwrap_or(u64::MAX);
        self.ended = self.sent == to_add.seeds.len();
        Some(SeedBatch {
            id: RequestId::Number(id),
            seeds: left[..taken].to_vec(),
            end: self.ended,
        })
    }

    /// Counts the seeds of `answers` the crawl did not add.
    pub(super) fn answered(&mut self, answers: &[Enqueued]) {
        for answer in answers {
            if let Enqueued::NotAdded { reason, .. } = answer {
                let reason = serde_json::to_value(reason)
                    .ok()
                    .and_then(|reason| reason.as_str().map(str::to_owned))
                    .unwrap_or_default();
                *self.not_added.entry(reason).or_default() += 1;
            }
        }
    }

    /// Reports the call's answer: its counts, then its success; or its
    /// failure.
    pub(super) async fn reported(
        self,
        answer: Result<Value, ErrorObject>,
        report: &impl Reports<Report>,
    ) {
        let Self {
            call, not_added, ..
        } = self;
        let answer = answer.map_err(failure).and_then(|value| {
            serde_json::from_value::<RunSeedsAdded>(value)
                .map_err(|error| unreadable(RUN_ADD_SEEDS.name, &error))
        });
        match answer {
            Ok(counts) => {
                if let Call::AddSeeds(seeds) = &call {
                    report
                        .report(Report::SeedsAdded {
                            run_id: seeds.run_id.clone(),
                            added: SeedsAdded {
                                added: counts.added,
                                not_added,
                            },
                        })
                        .await;
                }
                report.report(Report::Answered(call, Ok(()))).await;
            }
            Err(failure) => {
                tracing::warn!(code = failure.code, "adding seeds failed");
                report.report(Report::Answered(call, Err(failure))).await;
            }
        }
    }
}
