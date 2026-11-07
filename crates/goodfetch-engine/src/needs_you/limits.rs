//! The Runs a Limit stopped, in "Needs you" (ADRs 0029, 0031 and 0041),
//! the engine's own source, which its contribution passes in: each Run the
//! Run index holds stopped by a Limit that still holds it, as the user's
//! Usage ledger says: the Limit's period has not started again since the
//! stop, and it was not set again since then with something left. Only a
//! person raises a Limit (`usage.raise`, which the catalog keeps to the
//! Console and the interactive CLI prompt), so an agent a Limit stopped
//! cannot lift it. Once raised, a Recipe's Run is resumed in the Daemon
//! (`run.resume`, which the item offers beside it), and an SDK script's Run
//! waits for its script, which resumes it by its Run ID. An item leaves the
//! list once its Limit no longer holds the Run, once the Run runs again, or
//! when a person dismisses it. The source reads both again at each storage
//! sample, as the storage entries do.

use goodfetch_protocol::checkpoint::RUN_RESUME;
use goodfetch_protocol::needs_you::{NeedsYouItem, RUN_STOPPED_BY_LIMIT};
use goodfetch_protocol::run::{PointInTime, RECIPE_TAG, ReachedLimit, RunIndexRow, StopReason};
use goodfetch_protocol::usage::USAGE_RAISE;
use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::{Answering, Changes, NeedsYouSource, Primitives, SourceContext, SourceError};
use crate::ledger::{Ledger, LedgerError, StoredLimit};
use crate::run_index;

/// Each Run stopped by a Limit that still holds it.
pub fn runs_stopped() -> NeedsYouSource {
    NeedsYouSource::new(RunsStopped)
}

struct RunsStopped;

impl Primitives for RunsStopped {
    fn dismissable(&self) -> bool {
        true
    }

    fn items<'a>(
        &'a self,
        context: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        let path = context.run_index.clone();
        let version = context.version.clone();
        let ledger = context.ledger.clone();
        let now = context.clock.timestamp();
        Box::pin(async move {
            context
                .blocking(move || {
                    let rows =
                        run_index::stopped_by_limit(&path, &version).map_err(SourceError::new)?;
                    still_stopped(&rows, &ledger, now).map_err(SourceError::new)
                })
                .await
        })
    }

    fn changes(&self, context: &SourceContext) -> Changes {
        Changes::of(context.storage())
    }
}

/// The items of the Runs of `rows` whose Limit `ledger` says still holds
/// them at `now`. A ledger no engine made yet holds no Limit, and is not
/// made.
fn still_stopped(
    rows: &[RunIndexRow],
    ledger: &Ledger,
    now: Timestamp,
) -> Result<Vec<NeedsYouItem>, LedgerError> {
    if rows.is_empty() || !ledger.path().exists() {
        return Ok(Vec::new());
    }
    let limits = ledger.limits()?;
    let mut items = Vec::new();
    for row in rows {
        let Some(StopReason::Limit(reached)) = &row.stop_reason else {
            continue;
        };
        let restarted = timestamp(reached.resets_at.as_ref().map(PointInTime::as_str))
            .is_some_and(|resets_at| now >= resets_at);
        if restarted {
            continue;
        }
        let stopped_at = timestamp(Some(
            row.ended_at.as_ref().unwrap_or(&row.started_at).as_str(),
        ));
        let mut holding = None;
        for stored in limits.iter().filter(|stored| names(stored, reached)) {
            let set_since = timestamp(Some(&stored.set_at))
                .zip(stopped_at)
                .is_some_and(|(set_at, stopped_at)| set_at > stopped_at);
            if !set_since || ledger.left(stored, now)? == 0 {
                holding = Some(stored);
                break;
            }
        }
        if let Some(stored) = holding {
            items.push(stopped(row, reached, stored.amount));
        }
    }
    Ok(items)
}

/// The point in time `text` names, if it names one.
fn timestamp(text: Option<&str>) -> Option<Timestamp> {
    text?.parse().ok()
}

/// Whether `stored` is the Limit `reached` names: its scope, Usage kind,
/// currency and period.
fn names(stored: &StoredLimit, reached: &ReachedLimit) -> bool {
    stored.scope == reached.scope
        && stored.usage_kind == reached.usage_kind
        && stored.currency == reached.currency
        && stored.known_period() == reached.period
}

/// The item of the Run `row` names, stopped by `reached`, which allows
/// `amount` per period now.
fn stopped(row: &RunIndexRow, reached: &ReachedLimit, amount: u64) -> NeedsYouItem {
    let since = row
        .ended_at
        .clone()
        .unwrap_or_else(|| row.started_at.clone());
    let run_id = row.run_id.as_str();
    let recipe = row.tags.reserved(RECIPE_TAG);
    let mut details = Map::new();
    details.insert("limit".to_owned(), json!(reached));
    details.insert("amount".to_owned(), json!(amount));
    details.insert("needs".to_owned(), json!(row.needs));
    let mut actions = vec![USAGE_RAISE.name.to_owned()];
    let summary = match recipe {
        Some(recipe) => {
            details.insert("recipe".to_owned(), Value::from(recipe));
            actions.push(RUN_RESUME.name.to_owned());
            format!(
                "The Run {run_id} of the Recipe {recipe} stopped: a Limit on {} has nothing left. A person may raise it, then resume the Run in the Daemon.",
                reached.usage_kind
            )
        }
        None => format!(
            "The Run {run_id} stopped: a Limit on {} has nothing left. A person may raise it; the Run then waits for its script, which resumes it when started again with its Run ID.",
            reached.usage_kind
        ),
    };
    NeedsYouItem {
        id: format!("{RUN_STOPPED_BY_LIMIT}:{run_id}:{}", since.as_str()),
        kind: RUN_STOPPED_BY_LIMIT.to_owned(),
        since,
        run: Some(row.run_id.clone()),
        schedule: None,
        summary,
        actions,
        details,
    }
}

#[cfg(test)]
mod tests;
