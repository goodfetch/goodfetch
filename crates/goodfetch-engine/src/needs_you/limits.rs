//! The Runs a Limit stopped, in "Needs you" (ADRs 0029, 0031 and 0041),
//! the engine's own source, which its contribution passes in: each Run the
//! Run index holds stopped by a Limit that still holds it, as the user's
//! Usage ledger says: the Limit's period has not started again since the
//! stop, and it was not set again since then with something left. Only a
//! person raises a Limit (`usage.raise`, which the catalog keeps to the
//! Kennel and the interactive CLI prompt), so an agent a Limit stopped
//! cannot lift it. Once raised, a Recipe's Run is resumed in the Daemon
//! (`run.resume`, which the item offers beside it), and an SDK script's Run
//! waits for its script, which resumes it by its Run ID. An item leaves the
//! list once its Limit no longer holds the Run, once the Run runs again, or
//! when a person dismisses it. The source reads both again at each storage
//! sample, as the storage entries do.

use std::time::SystemTime;

use futures_util::StreamExt as _;
use goodfetch_protocol::checkpoint::RUN_RESUME;
use goodfetch_protocol::needs_you::{LIMIT_REACHED, NeedsYouItem, RUN_STOPPED_BY_LIMIT};
use goodfetch_protocol::run::{RECIPE_TAG, ReachedLimit, RunIndexRow, StopReason};
use goodfetch_protocol::usage::USAGE_RAISE;

use jiff::Timestamp;
use serde_json::{Map, Value, json};

use super::{Answering, Changes, NeedsYouSource, Primitives, SourceContext, SourceError};
use crate::clock;
use crate::ledger::{Ledger, LedgerError, LedgerNeedsYou, StoredLimit, point_in_time};

/// Each Run stopped by a Limit that still holds it.
pub fn runs_stopped() -> NeedsYouSource {
    NeedsYouSource::new(RunsStopped)
}

/// Each Limit of the Usage ledger that reached 100 % in its current period,
/// kept in the Usage ledger until a person raises it, its period starts again,
/// or it is dismissed (ADR 0041).
pub fn reached_limits() -> NeedsYouSource {
    NeedsYouSource::new(ReachedLimits)
}

struct ReachedLimits;

impl Primitives for ReachedLimits {
    fn dismissable(&self) -> bool {
        true
    }

    fn items<'a>(
        &'a self,
        context: &'a SourceContext,
    ) -> Answering<'a, Result<Vec<NeedsYouItem>, SourceError>> {
        let ledger = context.ledger.clone();
        let now = context.clock.timestamp();
        Box::pin(async move {
            context
                .blocking(move || {
                    let entries = ledger.needs_you_items(now).map_err(SourceError::new)?;
                    Ok(entries.iter().map(limit_reached_item).collect())
                })
                .await
        })
    }

    fn changes(&self, context: &SourceContext) -> Changes {
        let storage = context.storage();
        let ledger_changes = context.ledger.subscribe_changes();
        let needs_you = context.ledger.subscribe_needs_you();
        Changes(
            futures_util::stream::unfold(
                (storage, ledger_changes, needs_you),
                |(mut storage, mut ledger_changes, mut needs_you)| async move {
                    tokio::select! {
                        changed = storage.changed() => {
                            changed.ok()?;
                            Some(((), (storage, ledger_changes, needs_you)))
                        }
                        changed = ledger_changes.recv() => {
                            match changed {
                                Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    Some(((), (storage, ledger_changes, needs_you)))
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
                            }
                        }
                        changed = needs_you.recv() => {
                            match changed {
                                Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    Some(((), (storage, ledger_changes, needs_you)))
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
                            }
                        }
                    }
                },
            )
            .boxed(),
        )
    }
}

/// The `limit_reached` item for a 100 % Limit entry kept in the Usage ledger.
fn limit_reached_item(entry: &LedgerNeedsYou) -> NeedsYouItem {
    let reached = entry.limit.reached(entry.resets_at);
    let since = point_in_time(entry.reached_at);
    let mut details = Map::new();
    details.insert("limit".to_owned(), json!(reached));
    details.insert("amount".to_owned(), json!(entry.limit.amount));
    let scope = if entry.limit.scope.id.is_empty() {
        entry.limit.scope.kind.clone()
    } else {
        format!("{} ({})", entry.limit.scope.kind, entry.limit.scope.id)
    };
    let summary = format!(
        "The {scope} Limit on {} reached 100 % ({}). A person may raise it.",
        entry.limit.usage_kind, entry.limit.amount
    );
    NeedsYouItem {
        id: format!(
            "{LIMIT_REACHED}:{}:{}:{}",
            entry.limit.id, entry.period_start, entry.limit_amount
        ),
        kind: LIMIT_REACHED.to_owned(),
        since,
        run: None,
        schedule: None,
        summary,
        actions: vec![USAGE_RAISE.name.to_owned()],
        details,
    }
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
        let ledger = context.ledger.clone();
        let now = context.clock.timestamp();
        Box::pin(async move {
            let rows = context
                .run_index
                .stopped_by_limit()
                .await
                .map_err(SourceError::new)?;
            context
                .blocking(move || still_stopped(&rows, &ledger, now).map_err(SourceError::new))
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
        let restarted = reached
            .resets_at
            .is_some_and(|resets_at| SystemTime::from(now) >= resets_at);
        if restarted {
            continue;
        }
        let stopped_at = row.ended_at.unwrap_or(row.started_at);
        let mut holding = None;
        for stored in limits.iter().filter(|stored| names(stored, reached)) {
            let set_since = timestamp(&stored.set_at)
                .is_some_and(|set_at| SystemTime::from(set_at) > stopped_at);
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
fn timestamp(text: &str) -> Option<Timestamp> {
    text.parse().ok()
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
    let since = row.ended_at.unwrap_or(row.started_at);
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
        id: format!("{RUN_STOPPED_BY_LIMIT}:{run_id}:{}", clock::rfc3339(since)),
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
