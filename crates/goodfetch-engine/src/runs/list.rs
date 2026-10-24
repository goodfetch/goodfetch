//! Which rows of the Run index `run.list` answers, and in which order (ADRs
//! 0031, 0036 and 0117): the engine filters and sorts, so every Client sees
//! the same view of the same filter. A Run's Recipe, Schedule, occurrence,
//! trigger and role are its reserved `goodfetch.` tags; a Watch run is left
//! out unless the filter asks for Watch runs. A row's presence never
//! removes it: missing and erased rows are listed as such.

use std::cmp::Ordering;

use goodfetch_protocol::run::{
    Completeness, Lifecycle, OCCURRENCE_TAG, PointInTime, RECIPE_TAG, ROLE_TAG, RunIndexRow,
    RunListParams, RunSortKey, RunTrigger, SCHEDULE_TAG, SortOrder, TRIGGER_TAG, TimeRange,
    Verdict, WATCH_ROLE,
};
use jiff::Timestamp;

/// The rows of `rows` that `params` names, in its order.
pub(super) fn select(rows: Vec<RunIndexRow>, params: &RunListParams) -> Vec<RunIndexRow> {
    let text = params.text.as_deref().map(str::to_lowercase);
    let mut selected: Vec<RunIndexRow> = rows
        .into_iter()
        .filter(|row| matches(row, params, text.as_deref()))
        .collect();
    let key = params.sort.unwrap_or_default();
    let order = params.order.unwrap_or_default();
    selected.sort_by(|left, right| {
        let compared = compare(key, left, right);
        let ordered = match order {
            SortOrder::Ascending => compared,
            SortOrder::Descending => compared.reverse(),
        };
        ordered.then_with(|| left.run_id.cmp(&right.run_id))
    });
    selected
}

/// The trigger a row's reserved tag names, if it is one this engine knows.
fn trigger(row: &RunIndexRow) -> Option<RunTrigger> {
    RunTrigger::parse(row.tags.reserved(TRIGGER_TAG)?)
}

fn matches(row: &RunIndexRow, params: &RunListParams, text: Option<&str>) -> bool {
    let reserved = |key| row.tags.reserved(key);
    (params.watch_runs || reserved(ROLE_TAG) != Some(WATCH_ROLE))
        && params
            .tags
            .iter()
            .all(|wanted| row.tags.iter().any(|tag| tag == wanted))
        && equal(params.recipe.as_deref(), reserved(RECIPE_TAG))
        && equal(params.schedule.as_deref(), reserved(SCHEDULE_TAG))
        && equal(params.occurrence.as_deref(), reserved(OCCURRENCE_TAG))
        && equal(params.trigger, trigger(row))
        && equal(params.product, Some(row.needs))
        && equal(params.lifecycle, Some(row.lifecycle))
        && equal(params.verdict, row.verdict)
        && equal(params.completeness, row.completeness)
        && within(params.started.as_ref(), Some(&row.started_at))
        && within(params.ended.as_ref(), row.ended_at.as_ref())
        && text.is_none_or(|text| holds_text(row, text))
}

/// Whether a field the filter may name has the value it names.
fn equal<T: PartialEq>(wanted: Option<T>, found: Option<T>) -> bool {
    wanted.is_none_or(|wanted| found.is_some_and(|found| found == wanted))
}

fn instant(point: &PointInTime) -> Option<Timestamp> {
    point.as_str().parse().ok()
}

/// Whether `point` is in `range`; no point, or one that is not a time, is
/// in no range.
fn within(range: Option<&TimeRange>, point: Option<&PointInTime>) -> bool {
    let Some(range) = range else {
        return true;
    };
    let Some(point) = point.and_then(instant) else {
        return false;
    };
    let after_from = range
        .from
        .as_ref()
        .is_none_or(|from| instant(from).is_some_and(|from| point >= from));
    let before_until = range
        .until
        .as_ref()
        .is_none_or(|until| instant(until).is_some_and(|until| point < until));
    after_from && before_until
}

/// Whether `text`, in lowercase, is in the row's Run ID or a tag, its
/// Recipe's among them, ignoring case.
fn holds_text(row: &RunIndexRow, text: &str) -> bool {
    let holds = |field: &str| field.to_lowercase().contains(text);
    holds(row.run_id.as_str()) || row.tags.iter().any(|tag| holds(tag.as_str()))
}

/// Compares two values a row may lack, a lacking one after every other.
fn present_first<T: Ord>(left: Option<T>, right: Option<T>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Each value's place in its type's declaration order.
fn lifecycle_rank(lifecycle: Lifecycle) -> u8 {
    match lifecycle {
        Lifecycle::Running => 0,
        Lifecycle::Stopped => 1,
        Lifecycle::Finished => 2,
        Lifecycle::Failed => 3,
    }
}

fn verdict_rank(verdict: Verdict) -> u8 {
    match verdict {
        Verdict::Verified => 0,
        Verdict::Rejected => 1,
        Verdict::NeedsReview => 2,
    }
}

fn completeness_rank(completeness: Completeness) -> u8 {
    match completeness {
        Completeness::Complete => 0,
        Completeness::Incomplete => 1,
        Completeness::Unverified => 2,
    }
}

fn trigger_rank(trigger: RunTrigger) -> u8 {
    match trigger {
        RunTrigger::Time => 0,
        RunTrigger::Watch => 1,
        RunTrigger::AfterRun => 2,
        RunTrigger::RunNow => 3,
    }
}

fn compare(key: RunSortKey, left: &RunIndexRow, right: &RunIndexRow) -> Ordering {
    match key {
        RunSortKey::StartedAt => {
            present_first(instant(&left.started_at), instant(&right.started_at))
        }
        RunSortKey::EndedAt => present_first(
            left.ended_at.as_ref().and_then(instant),
            right.ended_at.as_ref().and_then(instant),
        ),
        RunSortKey::RunId => left.run_id.cmp(&right.run_id),
        RunSortKey::Recipe => present_first(
            left.tags.reserved(RECIPE_TAG),
            right.tags.reserved(RECIPE_TAG),
        ),
        RunSortKey::Schedule => present_first(
            left.tags.reserved(SCHEDULE_TAG),
            right.tags.reserved(SCHEDULE_TAG),
        ),
        RunSortKey::Occurrence => present_first(
            left.tags.reserved(OCCURRENCE_TAG),
            right.tags.reserved(OCCURRENCE_TAG),
        ),
        RunSortKey::Trigger => present_first(
            trigger(left).map(trigger_rank),
            trigger(right).map(trigger_rank),
        ),
        RunSortKey::Product => left.needs.cmp(&right.needs),
        RunSortKey::Lifecycle => {
            lifecycle_rank(left.lifecycle).cmp(&lifecycle_rank(right.lifecycle))
        }
        RunSortKey::Verdict => present_first(
            left.verdict.map(verdict_rank),
            right.verdict.map(verdict_rank),
        ),
        RunSortKey::Completeness => present_first(
            left.completeness.map(completeness_rank),
            right.completeness.map(completeness_rank),
        ),
    }
}
