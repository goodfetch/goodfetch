//! Why a crawl does not cover its scope, its Completeness reasons (ADRs
//! 0005, 0045, 0048, 0107, 0118 and 0120): what its bounds, caps and path
//! budgets refused and what its learned traps refused, its listings paged
//! only by an offset, the requests the Model allowance skipped, the pages
//! left `blocked` at `payment_required` or `consent_required`, and the
//! requests failed with `response_unavailable`, each with its count; and,
//! through the composition's report part, the same for a crawl that
//! stopped, beside the `stopped` reason, which counts the requests it left.

use goodfetch_engine::runs::{CompletenessReason, ReportPart};
use goodfetch_frontier::{completeness_reasons, learned_completeness_reasons};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::RESPONSE_UNAVAILABLE;
use goodfetch_protocol::foxhound::report::{CountOf, MODEL_ALLOWANCE};
use goodfetch_protocol::run::completeness::STOPPED;
use goodfetch_protocol::run::{AxisReason, Completeness, Lifecycle, RunKind, RunReport};
use rusqlite::Connection;

use crate::report;
use crate::work_items;

/// Why the crawl the run file `connection` holds does not cover its scope,
/// in the order of the reasons' codes.
///
/// # Errors
///
/// When the run file cannot be read.
pub(crate) fn short_of_scope(connection: &Connection) -> rusqlite::Result<Vec<CompletenessReason>> {
    let mut reasons = completeness_reasons(connection)?;
    reasons.extend(learned_completeness_reasons(connection)?);
    let (requests, _) = report::ended(connection)?;
    let skipped = count_of(&requests.by_skip_reason, MODEL_ALLOWANCE);
    if skipped > 0 {
        reasons.push(CompletenessReason::incomplete(counted(
            MODEL_ALLOWANCE,
            format!(
                "The Model allowance skipped {skipped} {} the crawl would otherwise have handled; its `skipped` outcomes with `{MODEL_ALLOWANCE}` list {}.",
                plural(skipped, "request", "requests"),
                plural(skipped, "it", "them"),
            ),
            skipped,
        )));
    }
    for cause in [BlockCause::PaymentRequired, BlockCause::ConsentRequired] {
        let left = count_of(&requests.by_block_cause, cause.as_str());
        if left > 0 {
            reasons.push(CompletenessReason {
                completeness: Completeness::Unverified,
                reason: counted(
                    cause.as_str(),
                    format!(
                        "{left} {} in the crawl's scope {} left `blocked` with `{}`; its `blocked` outcomes list {}.",
                        plural(left, "page", "pages"),
                        plural(left, "was", "were"),
                        cause.as_str(),
                        plural(left, "it", "them"),
                    ),
                    left,
                ),
            });
        }
    }
    let unavailable = count_of(&requests.by_error_code, RESPONSE_UNAVAILABLE.name);
    if unavailable > 0 {
        reasons.push(CompletenessReason::incomplete(counted(
            RESPONSE_UNAVAILABLE.name,
            "A site asked to wait longer than `politeness.retry_after.max` allows, so some of its requests failed with `response_unavailable`; `retryFailed()` re-admits them.".to_owned(),
            unavailable,
        )));
    }
    reasons.sort_by(|one, other| one.reason.code.cmp(&other.reason.code));
    Ok(reasons)
}

/// The count `counts` give `name`; 0 when they name it not.
fn count_of(counts: &[CountOf], name: &str) -> u64 {
    counts
        .iter()
        .find(|count| count.name == name)
        .map_or(0, |count| count.count)
}

fn counted(code: &str, message: String, count: u64) -> AxisReason {
    AxisReason {
        code: code.to_owned(),
        message,
        count: Some(count),
        call_id: None,
    }
}

fn plural<'a>(count: u64, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// The crawl's part of a stopped Run's Completeness reasons: its `stopped`
/// reason counts the requests the crawl left, and what else cut it follows.
pub(crate) struct StoppedParts;

impl ReportPart for StoppedParts {
    fn fill(&self, connection: &Connection, report: &mut RunReport) -> rusqlite::Result<()> {
        let kind: String = connection.query_row("SELECT kind FROM run", [], |row| row.get(0))?;
        if kind != RunKind::Crawl.as_str() || report.lifecycle != Lifecycle::Stopped {
            return Ok(());
        }
        let left = work_items::left(connection)?;
        for reason in &mut report.completeness_reasons {
            if reason.code == STOPPED {
                reason.count = Some(left);
            }
        }
        report.completeness_reasons.extend(
            short_of_scope(connection)?
                .into_iter()
                .map(|given| given.reason),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
