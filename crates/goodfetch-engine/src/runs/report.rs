//! A Run's report, assembled by the engine when the Run ends (ADR 0102):
//! the three axes and their reasons from how it ended, its Usage from what
//! its run state counted and the Limits that apply to it, its warnings and
//! `nothingDone` from what its owners recorded while it ran, then each part
//! an area supplies through its composition value ([`ReportPart`]), and the
//! Verdict `needs_review` when a part left an Effect uncertain, all in
//! the transaction that writes its end, so that the report read back later
//! is the one answered. A live Run's owners record its warnings and why it
//! did nothing through [`ReportWarnings`], the value a function takes in
//! place of the Run's handle (ADR 0145).

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use goodfetch_protocol::page::proxy::ProxyKind;
use goodfetch_protocol::plugin::approvals::Origin;
use goodfetch_protocol::run::completeness::{FAILED, STOPPED};
use goodfetch_protocol::run::warning::PLUGIN_FIGURES;
use goodfetch_protocol::run::{
    AppliedLimit, AxisReason, Checkpoint, Completeness, EndingError, KindUsage, Lifecycle,
    NothingDone, PluginConfinement, PluginForm, PluginRun, PluginStreamUsage, PluginUnitUsage,
    PluginUsage, ProxyKindBytes, RunId, RunReport, RunUsage, RunWarning, StopReason,
    UNCERTAIN_EFFECTS, UsageAmount, Verdict,
};
use goodfetch_protocol::settings::{ExplainedSetting, WeakenedSetting};
use goodfetch_protocol::usage::{
    BROWSER_TIME, MODEL_CALLS, MODEL_TOKENS, PAID_SOLVES, PLUGIN_CALLS, PLUGIN_STREAM_BYTES,
    PLUGIN_STREAM_TIME, PLUGIN_TIME, PLUGIN_UNITS, PROXY_BYTES, REQUESTS,
};
use rusqlite::{Connection, OptionalExtension as _};

use super::CompletenessReason;
use crate::mask::{Mask, Masks, Scope};
use crate::run_state::{RunState, RunStateError, Syncing};

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
    report.plugins = run_plugins(connection)?;
    if !report.usage.plugin_units.is_empty()
        && !report
            .warnings
            .iter()
            .any(|warning| warning.code == PLUGIN_FIGURES)
    {
        report.warnings.push(RunWarning {
            code: PLUGIN_FIGURES.to_owned(),
            message: "Usage figures come from a Plugin.".to_owned(),
            sessions: Vec::new(),
            folder: None,
        });
    }
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
        plugins: plugins(connection)?,
        plugin_units: plugin_units(connection)?,
        plugin_streams: plugin_streams(connection)?,
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
            (BROWSER_TIME, None) => usage.browser_time = Duration::from_millis(amount),
            (PROXY_BYTES, None) => usage.proxy_bytes = amount,
            (PROXY_BYTES, Some(currency)) => usage.proxy_price.push(UsageAmount {
                currency: Some(currency),
                amount,
            }),
            (REQUESTS, None) => usage.requests = amount,
            (PAID_SOLVES, currency) => usage.paid_solves.push(UsageAmount { currency, amount }),
            (PAID_FETCH, currency) => usage.paid_fetches.push(UsageAmount { currency, amount }),
            // `plugins`, `plugin_units` and `plugin_streams` hold them.
            (PLUGIN_CALLS | PLUGIN_TIME | PLUGIN_STREAM_BYTES | PLUGIN_STREAM_TIME, None)
            | (PLUGIN_UNITS, _) => {}
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

/// What the Run's calls into each implementation of a Plugin kind used, in
/// name order: the parts of `plugin_calls` and `plugin_time`.
fn plugins(connection: &Connection) -> rusqlite::Result<Vec<PluginUsage>> {
    let mut statement = connection.prepare_cached(
        "SELECT part, usage_kind, amount FROM usage_parts WHERE usage_kind IN (?1, ?2) ORDER BY part",
    )?;
    let parts = statement.query_map([PLUGIN_CALLS, PLUGIN_TIME], |row| {
        let amount: i64 = row.get(2)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            u64::try_from(amount).unwrap_or(0),
        ))
    })?;
    let mut plugins: Vec<PluginUsage> = Vec::new();
    for part in parts {
        let (plugin, usage_kind, amount) = part?;
        if plugins.last().is_none_or(|last| last.plugin != plugin) {
            plugins.push(PluginUsage {
                plugin,
                calls: 0,
                time: 0,
            });
        }
        let usage = plugins
            .last_mut()
            .expect("invariant: the implementation's entry is the last");
        if usage_kind == PLUGIN_CALLS {
            usage.calls = amount;
        } else {
            usage.time = amount;
        }
    }
    Ok(plugins)
}

/// Opaque stream bytes and duration per destination (`host:port`), in
/// destination order: the parts of `plugin_stream_bytes` and
/// `plugin_stream_time` (ADR 0033, issue 833).
fn plugin_streams(connection: &Connection) -> rusqlite::Result<Vec<PluginStreamUsage>> {
    let mut statement = connection.prepare_cached(
        "SELECT part, usage_kind, amount FROM usage_parts WHERE usage_kind IN (?1, ?2) ORDER BY part",
    )?;
    let parts = statement.query_map([PLUGIN_STREAM_BYTES, PLUGIN_STREAM_TIME], |row| {
        let amount: i64 = row.get(2)?;
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            u64::try_from(amount).unwrap_or(0),
        ))
    })?;
    let mut streams: Vec<PluginStreamUsage> = Vec::new();
    for part in parts {
        let (destination, usage_kind, amount) = part?;
        if streams
            .last()
            .is_none_or(|last| last.destination != destination)
        {
            streams.push(PluginStreamUsage {
                destination,
                bytes: 0,
                time: 0,
            });
        }
        let usage = streams
            .last_mut()
            .expect("invariant: the destination's entry is the last");
        if usage_kind == PLUGIN_STREAM_BYTES {
            usage.bytes = amount;
        } else {
            usage.time = amount;
        }
    }
    Ok(streams)
}

/// What the Run used of each Plugin's declared spend units, in Plugin then
/// unit order: the parts of `plugin_units`.
fn plugin_units(connection: &Connection) -> rusqlite::Result<Vec<PluginUnitUsage>> {
    let mut statement = connection.prepare_cached(
        "SELECT part, amount FROM usage_parts WHERE usage_kind = ?1 ORDER BY part",
    )?;
    let rows = statement.query_map([PLUGIN_UNITS], |row| {
        let amount: i64 = row.get(1)?;
        Ok((row.get::<_, String>(0)?, u64::try_from(amount).unwrap_or(0)))
    })?;
    let mut by_key: BTreeMap<(String, String), (u64, Option<UsageAmount>)> = BTreeMap::new();
    for row in rows {
        let (part, amount) = row?;
        if let Some(rest) = part.strip_prefix("qty:")
            && let Some((plugin, unit)) = rest.split_once(':')
        {
            let entry = by_key
                .entry((plugin.to_owned(), unit.to_owned()))
                .or_insert((0, None));
            entry.0 = entry.0.saturating_add(amount);
        } else if let Some(rest) = part.strip_prefix("price:")
            && let Some((plugin, rest)) = rest.split_once(':')
            && let Some((currency, unit)) = rest.split_once(':')
        {
            let entry = by_key
                .entry((plugin.to_owned(), unit.to_owned()))
                .or_insert((0, None));
            entry.1 = Some(UsageAmount {
                currency: Some(currency.to_owned()),
                amount,
            });
        }
    }
    Ok(by_key
        .into_iter()
        .filter(|(_, (quantity, price))| {
            *quantity > 0 || price.as_ref().is_some_and(|priced| priced.amount > 0)
        })
        .map(|((plugin, unit), (quantity, price))| PluginUnitUsage {
            plugin,
            unit,
            quantity,
            price,
            from_plugin: true,
        })
        .collect())
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

/// A live Run's warnings for its report, and the record of why it did
/// nothing (ADR 0102), each written to its run state: a value a function
/// takes in place of the Run's handle (ADR 0145). Clones share it.
#[derive(Clone, Debug)]
pub struct ReportWarnings {
    state: RunState,
    /// The Run's mask, which every warning's message is masked by first.
    mask: Mask,
}

impl ReportWarnings {
    /// The warnings of the Run whose run state `state` writes, masked by
    /// its `mask`.
    pub(super) fn new(state: RunState, mask: Mask) -> Self {
        Self { state, mask }
    }

    /// Hands `warning` over to the Run's report (ADR 0102): kept in its run
    /// state, in the order handed over, so a resume keeps it. A code of the
    /// warning table, or any other, is kept as it is.
    ///
    /// # Errors
    ///
    /// When its run state cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the write was queued still writes it.
    pub async fn warn(&self, warning: RunWarning) -> Result<(), RunStateError> {
        let warning = self.masked(warning);
        self.state
            .write(move |connection| warn(connection, &warning))
            .await
    }

    /// `warning` with every value a Run resolved masked, as what the engine
    /// keeps.
    fn masked(&self, mut warning: RunWarning) -> RunWarning {
        if let Cow::Owned(message) = self.mask.masks().apply(&warning.message, Scope::Kept) {
            warning.message = message;
        }
        warning
    }

    /// Hands `warning` over to the Run's report unless it holds a warning
    /// of the same code for the same folder, or with no folder, already,
    /// such as one an earlier run epoch handed over before a resume.
    ///
    /// # Errors
    ///
    /// When its run state cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future does not withdraw a write already queued: the
    /// warning may still be handed over.
    pub async fn warn_once(&self, warning: RunWarning) -> Result<(), RunStateError> {
        self.warn_once_with(Syncing::Required, warning).await
    }

    /// Hands `warning` over as [`ReportWarnings::warn_once`] does, in the
    /// open batch, with no sync of its own: for a warning the Run hands
    /// over again each time it opens, such as one its settings decide,
    /// which a crash that loses it loses only until the resume (ADR 0142).
    ///
    /// # Errors
    ///
    /// When its run state cannot be written.
    ///
    /// # Cancel safety
    ///
    /// As [`ReportWarnings::warn_once`]'s.
    pub async fn warn_once_deferred(&self, warning: RunWarning) -> Result<(), RunStateError> {
        self.warn_once_with(Syncing::Deferred { charged: 0 }, warning)
            .await
    }

    async fn warn_once_with(
        &self,
        syncing: Syncing,
        warning: RunWarning,
    ) -> Result<(), RunStateError> {
        let warning = self.masked(warning);
        self.state
            .write_with(syncing, move |connection| {
                if warned(connection, &warning.code, warning.folder.as_deref())? {
                    return Ok(());
                }
                warn(connection, &warning)
            })
            .await
    }

    /// Records why the Run did nothing (ADR 0102), as its driver found it:
    /// no seed reached a handler, or a resume found nothing left. Its
    /// report says so.
    ///
    /// # Errors
    ///
    /// When its run state cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future after the write was queued still writes it.
    pub async fn nothing_done(&self, nothing_done: NothingDone) -> Result<(), RunStateError> {
        self.state
            .write(move |connection| record_nothing_done(connection, &nothing_done))
            .await
    }
}

/// Records `warning` among the Run's warnings, in the order handed over.
///
/// # Errors
///
/// When its run state cannot be written.
fn warn(connection: &Connection, warning: &RunWarning) -> rusqlite::Result<()> {
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
fn warned(connection: &Connection, code: &str, folder: Option<&str>) -> rusqlite::Result<bool> {
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
fn record_nothing_done(
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

/// Records a Plugin or SDK handler that ran in the Run's `run_plugins` table
/// (ADRs 0016, 0033 and 0104).
fn record_plugin(connection: &Connection, plugin: &PluginRun) -> rusqlite::Result<()> {
    let missing = serde_json::to_string(&plugin.confinement_missing)
        .expect("invariant: confinement_missing encodes");
    let origin = plugin.origin.map(Origin::as_str);
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO run_plugins \
             (name, form, origin, digest, network_level, confinement, confinement_missing) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .and_then(|mut cached| {
            cached.execute((
                &plugin.name,
                plugin.form.as_str(),
                origin,
                &plugin.digest,
                &plugin.network_level,
                plugin.confinement.as_str(),
                missing,
            ))
        })?;
    Ok(())
}

/// Every Plugin recorded in the Run's `run_plugins` table, in name order;
/// `None` when none ran.
pub(super) fn run_plugins(connection: &Connection) -> rusqlite::Result<Option<Vec<PluginRun>>> {
    let mut statement = connection.prepare_cached(
        "SELECT name, form, origin, digest, network_level, confinement, confinement_missing \
         FROM run_plugins ORDER BY name",
    )?;
    let rows = statement
        .query_map([], |row| {
            let form: String = row.get(1)?;
            let origin: Option<String> = row.get(2)?;
            let confinement: String = row.get(5)?;
            let missing: String = row.get(6)?;
            Ok(PluginRun {
                name: row.get(0)?,
                form: PluginForm::from_stored(&form),
                origin: origin.as_deref().and_then(Origin::from_stored),
                digest: row.get(3)?,
                network_level: row.get(4)?,
                confinement: PluginConfinement::from_stored(&confinement),
                confinement_missing: serde_json::from_str(&missing)
                    .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((!rows.is_empty()).then_some(rows))
}

/// Records an accepted third-party Plugin digest change in the Run's state
/// and updates `run_plugins` to the accepted digest (ADR 0033).
pub(super) fn record_plugin_change_acceptance(
    connection: &Connection,
    run_epoch: u64,
    plugin: &str,
    previous_digest: &str,
    new_digest: Option<&str>,
    accepted_at: &str,
) -> rusqlite::Result<()> {
    let epoch = i64::try_from(run_epoch).unwrap_or(i64::MAX);
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO plugin_change_acceptances \
             (run_epoch, plugin, previous_digest, new_digest, accepted_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .and_then(|mut cached| {
            cached.execute((epoch, plugin, previous_digest, new_digest, accepted_at))
        })?;
    connection
        .prepare_cached("UPDATE run_plugins SET digest = ?1 WHERE name = ?2")
        .and_then(|mut cached| cached.execute((new_digest, plugin)))?;
    Ok(())
}

/// Persists started Plugins into a live Run's [`RunState`] so
/// `RunReport::plugins` lists them when the Run ends (ADRs 0016 and 0033).
pub(super) struct RunPluginRecorder {
    state: RunState,
}

impl RunPluginRecorder {
    /// A recorder writing to `state`.
    pub(super) fn new(state: RunState) -> Arc<Self> {
        Arc::new(Self { state })
    }
}

impl crate::plugin_kinds::run_plugins::PluginRunRecorder for RunPluginRecorder {
    fn record(&self, plugin: PluginRun) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Err(error) = self
                .state
                .write(move |connection| record_plugin(connection, &plugin))
                .await
            {
                tracing::warn!(%error, "could not record started Plugin in run state");
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::plugin::approvals::Origin;
    use goodfetch_protocol::run::{PluginConfinement, PluginForm, PluginRun};

    use super::{record_plugin, run_plugins};

    /// Recording a Plugin at `direct` in `run_plugins` populates
    /// `RunReport::plugins` in name order when the Run report is assembled.
    ///
    /// Covers: story:7-56, story:7-25
    #[test]
    fn recorded_direct_plugin_appears_in_run_report_plugins() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE run_plugins (
                    name TEXT NOT NULL PRIMARY KEY,
                    form TEXT NOT NULL DEFAULT 'process',
                    origin TEXT,
                    digest TEXT,
                    network_level TEXT NOT NULL,
                    confinement TEXT NOT NULL DEFAULT 'full',
                    confinement_missing TEXT NOT NULL DEFAULT '[]'
                ) STRICT",
            )
            .unwrap();
        assert_eq!(run_plugins(&connection).unwrap(), None);

        let direct = PluginRun {
            name: "pager".to_owned(),
            form: PluginForm::Process,
            origin: Some(Origin::ThirdParty),
            digest: Some(
                "sha256:0000000000000000000000000000000000000000000000000000000000000001"
                    .to_owned(),
            ),
            network_level: "direct".to_owned(),
            confinement: PluginConfinement::Full,
            confinement_missing: Vec::new(),
        };
        record_plugin(&connection, &direct).unwrap();

        assert_eq!(run_plugins(&connection).unwrap(), Some(vec![direct]));
    }
}
