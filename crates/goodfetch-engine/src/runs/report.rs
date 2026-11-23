//! A Run's report, assembled by the engine when the Run ends (ADR 0102):
//! the three axes and their reasons from how it ended, its Usage from what
//! its run state counted and the Limits that apply to it, its warnings and
//! `nothingDone` from what its owners recorded while it ran, then each part
//! an area supplies through its composition value ([`ReportPart`]), and the
//! Verdict `needs_review` when a part left an Effect uncertain, all in
//! the transaction that writes its end, so that the report read back later
//! is the one answered.

use std::sync::Arc;
use std::time::SystemTime;

use goodfetch_protocol::page::proxy::ProxyKind;
use goodfetch_protocol::run::completeness::{FAILED, STOPPED};
use goodfetch_protocol::run::{
    AppliedLimit, AxisReason, Checkpoint, Completeness, EndingError, KindUsage, Lifecycle,
    NothingDone, ProxyKindBytes, RunId, RunReport, RunUsage, RunWarning, StopReason,
    UNCERTAIN_EFFECTS, UsageAmount, Verdict,
};
use goodfetch_protocol::settings::{ExplainedSetting, WeakenedSetting};
use goodfetch_protocol::usage::{
    BROWSER_TIME, MODEL_CALLS, MODEL_TOKENS, PAID_SOLVES, PROXY_BYTES, REQUESTS,
};
use rusqlite::{Connection, OptionalExtension as _};

use super::CompletenessReason;
use crate::mask::{Masks, Scope};

/// The Usage kind of Paid fetches, which only Foxhound charges (ADR 0045),
/// named here since every Run's report carries them (ADR 0101).
const PAID_FETCH: &str = "paid_fetch";

/// What an area adds to a Run's report when the Run ends (ADR 0102), such
/// as a crawl's `requests` and `records`, the Plan's report or the Plugins
/// that ran: given through its composition value, it reads the Run's
/// committed state on `connection`, in the transaction that writes the
/// Run's end, and fills its part of `report`. A part it has nothing for
/// stays absent.
pub trait ReportPart: Send + Sync {
    /// Fills its part of `report`.
    ///
    /// # Errors
    ///
    /// When the Run's state cannot be read: the report then goes without
    /// this part, and the failure is logged.
    fn fill(&self, connection: &Connection, report: &mut RunReport) -> rusqlite::Result<()>;
}

/// What the engine knows of a Run's end beside its state.
pub(super) struct EndFacts {
    pub(super) run_id: RunId,
    pub(super) lifecycle: Lifecycle,
    pub(super) stop_reason: Option<StopReason>,
    pub(super) started_at: SystemTime,
    pub(super) ended_at: SystemTime,
    pub(super) policy_version: Option<String>,
    pub(super) weakened: Vec<WeakenedSetting>,
    pub(super) settings: Option<Vec<ExplainedSetting>>,
    pub(super) checkpoint: Option<Checkpoint>,
    pub(super) limits: Vec<AppliedLimit>,
    pub(super) error: Option<EndingError>,
    /// Why a `finished` Run's driver found its results short of its scope,
    /// if it did (ADRs 0093, 0118 and 0120).
    pub(super) finished: Vec<CompletenessReason>,
}

/// The report of a Run that ended as `ended` says, from its state on
/// `connection` and each of `parts`.
///
/// # Errors
///
/// When the engine's own tables cannot be read.
pub(super) fn assemble(
    connection: &Connection,
    facts: &EndFacts,
    parts: &[Arc<dyn ReportPart>],
) -> rusqlite::Result<RunReport> {
    let usage = usage(connection, facts.limits.clone())?;
    let mut report = facts.bare();
    report.usage = usage;
    report.warnings = warnings(connection)?;
    report.nothing_done = nothing_done(connection)?;
    for part in parts {
        if let Err(error) = part.fill(connection, &mut report) {
            tracing::warn!(%error, run_id = %report.run_id, "a part of the Run's report could not be read; the report goes without it");
        }
    }
    let uncertain = report.uncertain_effects.as_ref().map_or(0, Vec::len);
    if uncertain > 0 {
        report.needs_review(AxisReason {
            code: UNCERTAIN_EFFECTS.to_owned(),
            message: format!(
                "{uncertain} Effect(s) left uncertain: a person says whether each happened."
            ),
            count: Some(u64::try_from(uncertain).expect("invariant: a count fits in u64")),
            call_id: None,
        });
    }
    Ok(report)
}

impl EndFacts {
    /// The report its facts alone give, with nothing read from the Run's
    /// state: what is answered when its end could not be written.
    pub(super) fn bare(&self) -> RunReport {
        let (completeness, completeness_reasons) =
            completeness(self.lifecycle, self.stop_reason.as_ref(), &self.finished);
        RunReport {
            run_id: self.run_id.clone(),
            lifecycle: self.lifecycle,
            stop_reason: self.stop_reason.clone(),
            verdict: Verdict::Verified,
            verdict_reasons: Vec::new(),
            completeness,
            completeness_reasons,
            started_at: self.started_at,
            ended_at: self.ended_at,
            usage: RunUsage {
                limits: self.limits.clone(),
                ..RunUsage::default()
            },
            weakened: self.weakened.clone(),
            plan: None,
            policy_version: self.policy_version.clone(),
            settings: self.settings.clone(),
            plugins: None,
            warnings: Vec::new(),
            recording: None,
            checkpoint: self.checkpoint.clone(),
            nothing_done: None,
            uncertain_effects: None,
            error: self.error.clone(),
            kind_parts: serde_json::Map::new(),
        }
    }
}

/// The Completeness a Run's end gives, with its reasons (ADR 0029): a
/// finished Run's is the program's or the Frontier's to lower, as its
/// driver's reasons do, the worst of them winning, `incomplete` before
/// `unverified`.
fn completeness(
    lifecycle: Lifecycle,
    stop_reason: Option<&StopReason>,
    finished: &[CompletenessReason],
) -> (Completeness, Vec<AxisReason>) {
    match (lifecycle, stop_reason) {
        (Lifecycle::Finished, _) if !finished.is_empty() => (
            worst(finished),
            finished.iter().map(|given| given.reason.clone()).collect(),
        ),
        (Lifecycle::Finished | Lifecycle::Running, _) => (Completeness::Complete, Vec::new()),
        (Lifecycle::Stopped, Some(reason)) => (
            Completeness::Incomplete,
            vec![AxisReason {
                code: STOPPED.to_owned(),
                message: format!("It stopped early: {}.", stopped_for(reason)),
                count: None,
                call_id: None,
            }],
        ),
        (Lifecycle::Stopped, None) => (
            Completeness::Incomplete,
            vec![AxisReason {
                code: STOPPED.to_owned(),
                message: "It stopped early.".to_owned(),
                count: None,
                call_id: None,
            }],
        ),
        (Lifecycle::Failed, _) => (
            Completeness::Incomplete,
            vec![AxisReason {
                code: FAILED.to_owned(),
                message: "An error ended it before its end.".to_owned(),
                count: None,
                call_id: None,
            }],
        ),
    }
}

/// The worst Completeness `reasons` give: `incomplete` before
/// `unverified`, else `complete`.
fn worst(reasons: &[CompletenessReason]) -> Completeness {
    let gives = |completeness| {
        reasons
            .iter()
            .any(|reason| reason.completeness == completeness)
    };
    if gives(Completeness::Incomplete) {
        Completeness::Incomplete
    } else if gives(Completeness::Unverified) {
        Completeness::Unverified
    } else {
        Completeness::Complete
    }
}

/// What a stop reason says, for people.
fn stopped_for(reason: &StopReason) -> String {
    match reason {
        StopReason::Abort { by, .. } => format!("aborted by {}", by_name(*by)),
        StopReason::Pause => "paused".to_owned(),
        StopReason::Migrate => "migrated".to_owned(),
        StopReason::Timeout => "its `timeout.run` passed".to_owned(),
        StopReason::Drain => "its Daemon drained".to_owned(),
        StopReason::Limit(reached) => format!(
            "a Limit on {} {} was reached",
            reached.scope.kind, reached.usage_kind
        ),
        StopReason::LimitNotUnderstood(limit) => format!(
            "a Limit on {} {} this engine cannot evaluate covered it",
            limit.scope.kind, limit.usage_kind
        ),
        StopReason::EngineLost => "its engine was lost".to_owned(),
        StopReason::EnginePanicked => "its engine stopped on a bug".to_owned(),
        StopReason::Resources { resource, .. } => match resource {
            goodfetch_protocol::run::Resource::Memory => "memory ran out".to_owned(),
            goodfetch_protocol::run::Resource::Storage => "storage ran out".to_owned(),
        },
        StopReason::Unknown(member) => format!("stopped for `{}`", member.tag()),
        _ => "stopped".to_owned(),
    }
}

fn by_name(by: goodfetch_protocol::connection::StoppedBy) -> &'static str {
    match by {
        goodfetch_protocol::connection::StoppedBy::Person => "a person",
        goodfetch_protocol::connection::StoppedBy::Client => "its Client",
        goodfetch_protocol::connection::StoppedBy::Program => "its program",
        goodfetch_protocol::connection::StoppedBy::Browser => "its browser",
    }
}

/// What the Run used, from what its run state counted, with `limits`, the
/// Limits that apply to it, and its uncapped mark.
///
/// # Errors
///
/// When its run state cannot be read.
pub(super) fn usage(
    connection: &Connection,
    limits: Vec<AppliedLimit>,
) -> rusqlite::Result<RunUsage> {
    let mut usage = RunUsage {
        limits,
        uncapped: connection
            .prepare_cached("SELECT uncapped FROM run")
            .and_then(|mut cached| cached.query_row([], |row| row.get(0)))?,
        proxy_bytes_by_kind: proxy_bytes_by_kind(connection)?,
        ..RunUsage::default()
    };
    let mut statement = connection.prepare_cached(
        "SELECT usage_kind, currency, amount FROM usage_totals ORDER BY usage_kind, currency",
    )?;
    let counted = statement.query_map([], |row| {
        let currency: String = row.get(1)?;
        let amount: i64 = row.get(2)?;
        Ok((
            row.get::<_, String>(0)?,
            (!currency.is_empty()).then_some(currency),
            u64::try_from(amount).unwrap_or(0),
        ))
    })?;
    for counted in counted {
        let (usage_kind, currency, amount) = counted?;
        match (usage_kind.as_str(), currency) {
            (MODEL_CALLS, None) => usage.model.counts.calls = amount,
            (MODEL_TOKENS, None) => usage.model.counts.tokens = amount,
            (BROWSER_TIME, None) => usage.browser_time = amount,
            (PROXY_BYTES, None) => usage.proxy_bytes = amount,
            (PROXY_BYTES, Some(currency)) => usage.proxy_price.push(UsageAmount {
                currency: Some(currency),
                amount,
            }),
            (REQUESTS, None) => usage.requests = amount,
            (PAID_SOLVES, currency) => usage.paid_solves.push(UsageAmount { currency, amount }),
            (PAID_FETCH, currency) => usage.paid_fetches.push(UsageAmount { currency, amount }),
            (_, currency) => usage.other.push(KindUsage {
                usage_kind,
                currency,
                amount,
            }),
        }
    }
    Ok(usage)
}

/// What the Run sent and received through each proxy kind, cheapest kind
/// first, then any kind this engine does not name.
fn proxy_bytes_by_kind(connection: &Connection) -> rusqlite::Result<Vec<ProxyKindBytes>> {
    let mut statement =
        connection.prepare_cached("SELECT part, amount FROM usage_parts WHERE usage_kind = ?1")?;
    let mut kinds = statement
        .query_map([PROXY_BYTES], |row| {
            let amount: i64 = row.get(1)?;
            Ok(ProxyKindBytes {
                kind: row.get(0)?,
                bytes: u64::try_from(amount).unwrap_or(0),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let order = |kind: &str| {
        ProxyKind::ALL
            .iter()
            .position(|known| known.as_str() == kind)
    };
    kinds.sort_by(|one, other| {
        (order(&one.kind).is_none(), order(&one.kind), &one.kind).cmp(&(
            order(&other.kind).is_none(),
            order(&other.kind),
            &other.kind,
        ))
    });
    Ok(kinds)
}

/// Adds `delta` to what the Run used of `part` of `usage_kind`.
///
/// # Errors
///
/// When its run state cannot be written.
pub(super) fn count_part(
    connection: &Connection,
    usage_kind: &str,
    part: &str,
    delta: i64,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO usage_parts (usage_kind, part, amount) VALUES (?1, ?2, MAX(?3, 0))
         ON CONFLICT (usage_kind, part) DO UPDATE SET amount = MAX(amount + ?3, 0)",
        )
        .and_then(|mut cached| cached.execute((usage_kind, part, delta)))?;
    Ok(())
}

/// Adds `delta` to what the Run used of `usage_kind` in `currency`.
///
/// # Errors
///
/// When its run state cannot be written.
pub(super) fn count(
    connection: &Connection,
    usage_kind: &str,
    currency: Option<&str>,
    delta: i64,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO usage_totals (usage_kind, currency, amount) VALUES (?1, ?2, MAX(?3, 0))
         ON CONFLICT (usage_kind, currency) DO UPDATE SET amount = MAX(amount + ?3, 0)",
        )
        .and_then(|mut cached| cached.execute((usage_kind, currency.unwrap_or_default(), delta)))?;
    Ok(())
}

/// `report` with every value a Run resolved masked, as what the engine
/// keeps: its error, its warnings and every other text of it. A report
/// whose masked form no longer reads as one, which no value of a Run
/// should cause, keeps its texts but its error's and warnings'.
pub(super) fn masked(report: RunReport, masks: &Masks) -> RunReport {
    let Ok(mut value) = serde_json::to_value(&report) else {
        return report;
    };
    let unmasked = value.clone();
    masks.apply_value(&mut value, Scope::Kept);
    if value == unmasked {
        return report;
    }
    serde_json::from_value(value).unwrap_or_else(|_| {
        let mut report = report;
        for warning in &mut report.warnings {
            warning.message = masks.apply(&warning.message, Scope::Kept).into_owned();
        }
        report
    })
}

/// Records `warning` among the Run's warnings, in the order handed over.
///
/// # Errors
///
/// When its run state cannot be written.
pub(super) fn warn(connection: &Connection, warning: &RunWarning) -> rusqlite::Result<()> {
    let sessions =
        serde_json::to_string(&warning.sessions).expect("invariant: Session names encode");
    connection
        .prepare_cached(
            "INSERT INTO warnings (code, message, sessions, folder) VALUES (?1, ?2, ?3, ?4)",
        )
        .and_then(|mut cached| {
            cached.execute((&warning.code, &warning.message, sessions, &warning.folder))
        })?;
    Ok(())
}

/// Whether the Run's report holds a warning of `code` for `folder`, or
/// with no folder when `folder` is none.
pub(super) fn warned(
    connection: &Connection,
    code: &str,
    folder: Option<&str>,
) -> rusqlite::Result<bool> {
    connection
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM warnings WHERE code = ?1 AND folder IS ?2)")
        .and_then(|mut cached| cached.query_row((code, folder), |row| row.get(0)))
}

/// The Run's warnings, in the order handed over.
fn warnings(connection: &Connection) -> rusqlite::Result<Vec<RunWarning>> {
    let mut statement = connection
        .prepare_cached("SELECT code, message, sessions, folder FROM warnings ORDER BY position")?;
    let warnings = statement.query_map([], |row| {
        let sessions: String = row.get(2)?;
        Ok(RunWarning {
            code: row.get(0)?,
            message: row.get(1)?,
            sessions: serde_json::from_str(&sessions)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
            folder: row.get(3)?,
        })
    })?;
    warnings.collect()
}

/// Records why the Run did nothing, as its driver found it.
///
/// # Errors
///
/// When its run state cannot be written.
pub(super) fn record_nothing_done(
    connection: &Connection,
    nothing_done: &NothingDone,
) -> rusqlite::Result<()> {
    let text = serde_json::to_string(nothing_done).expect("invariant: `nothingDone` encodes");
    connection
        .prepare_cached("UPDATE run SET nothing_done = ?1")
        .and_then(|mut cached| cached.execute([text]))?;
    Ok(())
}

fn nothing_done(connection: &Connection) -> rusqlite::Result<Option<NothingDone>> {
    let text: Option<String> = connection
        .prepare_cached("SELECT nothing_done FROM run")
        .and_then(|mut cached| cached.query_row([], |row| row.get(0)))
        .optional()?
        .flatten();
    text.map(|text| {
        serde_json::from_str(&text)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
    })
    .transpose()
}
