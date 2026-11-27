//! Run-state persistence for Sink delivery status and queued Sink
//! Notifications, plus the [`SinkReportPart`] that populates
//! `CrawlRunReport.sinks` (ADRs 0026 and 0102).

use goodfetch_engine::clock;
use goodfetch_engine::mask::{Masks, Scope};
use goodfetch_engine::run_state::{RunStateTables, run_row};
use goodfetch_engine::runs::ReportPart;
use goodfetch_protocol::foxhound::report::{SinkReport, SinkStatus};
use goodfetch_protocol::run::{RunKind, RunReport};
use rusqlite::{Connection, OptionalExtension as _};

use crate::contract::GuaranteeClass;
use crate::dead_letters;
use crate::schedule::PauseReason;
pub use crate::schedule::{SinkNotification, SinkNotificationEvent};

/// The run-state tables holding each Sink's delivery status and queued Sink
/// Notifications (ADR 0026), registered in the run-state compatibility corpus
/// (ADR 0066).
pub const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-sink-report",
    statements: &[
        "CREATE TABLE IF NOT EXISTS sink_state (
            sink TEXT NOT NULL PRIMARY KEY,
            guarantee_class TEXT NOT NULL,
            status TEXT NOT NULL,
            paused_reason TEXT,
            outside_dedup_window INTEGER NOT NULL,
            undeclared_datasets TEXT NOT NULL
        ) STRICT, WITHOUT ROWID",
        "CREATE TABLE IF NOT EXISTS sink_notifications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sink TEXT NOT NULL,
            event TEXT NOT NULL,
            reason TEXT NOT NULL,
            occurred_at TEXT NOT NULL
        ) STRICT",
    ],
};

/// One Sink's persisted status row in `sink_state`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SinkStateRow {
    /// The Sink's name.
    pub sink: String,
    /// Its probed Guarantee class (or `AtLeastOnce` if probe never succeeded).
    pub guarantee_class: GuaranteeClass,
    /// Its current delivery status (`complete`, `complete_with_rejections`,
    /// `behind`, `failed`, or `inactive`).
    pub status: SinkStatus,
    /// Why it is paused, when paused.
    pub paused_reason: Option<PauseReason>,
    /// How many deliveries fell outside the destination's deduplication window.
    pub outside_dedup_window: u64,
    /// For an inactive Sink, Dataset names it listed that the crawl did not
    /// declare.
    pub undeclared_datasets: Vec<String>,
}

/// Creates the `sink_state` and `sink_notifications` tables when absent.
///
/// # Errors
///
/// SQLite's, when the statement fails.
pub fn ensure_tables(connection: &Connection) -> rusqlite::Result<()> {
    for statement in TABLES.statements {
        connection.execute_batch(statement)?;
    }
    Ok(())
}

/// Upserts one Sink's delivery status into `sink_state`.
///
/// # Errors
///
/// SQLite's, when the write fails.
pub fn record_state(connection: &Connection, row: &SinkStateRow) -> rusqlite::Result<()> {
    let class = guarantee_str(row.guarantee_class);
    let status = status_str(row.status);
    let paused = row.paused_reason.map(PauseReason::as_str);
    let outside = i64::try_from(row.outside_dedup_window).unwrap_or(i64::MAX);
    let undeclared = serde_json::to_string(&row.undeclared_datasets)
        .expect("invariant: string list encodes as JSON");
    connection
        .prepare_cached(
            "INSERT INTO sink_state (
                 sink, guarantee_class, status, paused_reason,
                 outside_dedup_window, undeclared_datasets
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (sink) DO UPDATE SET
                 guarantee_class = excluded.guarantee_class,
                 status = excluded.status,
                 paused_reason = excluded.paused_reason,
                 outside_dedup_window = excluded.outside_dedup_window,
                 undeclared_datasets = excluded.undeclared_datasets",
        )?
        .execute((&row.sink, class, status, paused, outside, &undeclared))?;
    Ok(())
}

/// Persists a queued Sink Notification in `sink_notifications`, masking its
/// `reason` at `Scope::Kept` (ADR 0067).
///
/// # Errors
///
/// SQLite's, when the insert fails.
pub fn record_notification(
    connection: &Connection,
    notification: &SinkNotification,
    masks: &Masks,
) -> rusqlite::Result<()> {
    let reason = masks.apply(&notification.reason, Scope::Kept);
    let occurred_at = clock::rfc3339(notification.occurred_at);
    connection
        .prepare_cached(
            "INSERT INTO sink_notifications (sink, event, reason, occurred_at)
             VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute((
            &notification.sink,
            notification.event.as_str(),
            reason.as_ref(),
            &occurred_at,
        ))?;
    Ok(())
}

/// Reads all queued Sink Notifications from `connection`, ordered by ID.
///
/// # Errors
///
/// SQLite's, when the query fails.
pub fn list_notifications(
    connection: &Connection,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    if !has_table(connection, "sink_notifications")? {
        return Ok(Vec::new());
    }
    let mut query = connection
        .prepare_cached("SELECT sink, event, reason FROM sink_notifications ORDER BY id")?;
    let rows = query.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.collect()
}

/// Reads every Sink's [`SinkReport`] from `connection`, joining `sink_state`
/// with each Sink's live count in `sink_dead_letters`.
///
/// # Errors
///
/// SQLite's, when the query fails.
pub fn reports(connection: &Connection) -> rusqlite::Result<Vec<SinkReport>> {
    if !has_table(connection, "sink_state")? {
        return Ok(Vec::new());
    }
    let mut query = connection.prepare_cached(
        "SELECT sink, guarantee_class, status, outside_dedup_window, undeclared_datasets
         FROM sink_state
         ORDER BY sink",
    )?;
    let rows = query
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let class_str: String = row.get(1)?;
            let status_str: String = row.get(2)?;
            let outside: i64 = row.get(3)?;
            let undeclared_json: String = row.get(4)?;
            Ok((name, class_str, status_str, outside, undeclared_json))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (name, class_str, status_str, outside, undeclared_json) in rows {
        let dead_letters = dead_letters::count(connection, &name)?;
        let mut status = parse_status(&status_str);
        if status == SinkStatus::Complete && dead_letters > 0 {
            status = SinkStatus::CompleteWithRejections;
        }
        let undeclared_datasets: Vec<String> =
            serde_json::from_str(&undeclared_json).unwrap_or_default();
        out.push(SinkReport {
            name,
            guarantee_class: parse_guarantee(&class_str),
            dead_letters,
            outside_dedup_window: u64::try_from(outside).unwrap_or_default(),
            status,
            undeclared_datasets,
        });
    }
    Ok(out)
}

/// `goodfetch-sink`'s [`ReportPart`] (ADR 0102): adds `sinks` to a crawl's
/// Run report when the Run configured any Sink.
pub struct SinkReportPart;

impl ReportPart for SinkReportPart {
    fn fill(&self, connection: &Connection, report: &mut RunReport) -> rusqlite::Result<()> {
        if !run_row::is_of_kind(connection, RunKind::Crawl)? {
            return Ok(());
        }
        let sinks = reports(connection)?;
        if !sinks.is_empty() {
            report.kind_parts.insert(
                "sinks".to_owned(),
                serde_json::to_value(sinks).expect("invariant: Sink reports encode"),
            );
        }
        Ok(())
    }
}

/// Refreshes the `sinks` section of an ended Run's stored `run.report` JSON
/// from the current `sink_state` and `sink_dead_letters` tables (ADR 0026):
/// called after post-Run delivery (Daemon backlog or `sink.deliver`) updates
/// those tables so `run.report` stays consistent with `sink.list`.
///
/// # Errors
///
/// SQLite's, when the report cannot be read or written.
pub(crate) fn refresh_stored_report(connection: &Connection) -> rusqlite::Result<()> {
    if !has_table(connection, "run")? {
        return Ok(());
    }
    let stored: Option<String> = connection
        .prepare_cached("SELECT report FROM run WHERE ended_at IS NOT NULL")?
        .query_row([], |row| row.get(0))
        .optional()?
        .flatten();
    let Some(stored) = stored else {
        return Ok(());
    };
    let mut report: RunReport = serde_json::from_str(&stored)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    SinkReportPart.fill(connection, &mut report)?;
    let updated = serde_json::to_string(&report)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    connection
        .prepare_cached("UPDATE run SET report = ?1 WHERE ended_at IS NOT NULL")?
        .execute([updated])?;
    Ok(())
}

fn has_table(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
    connection
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        )?
        .query_row([table], |row| row.get(0))
}

const fn guarantee_str(class: GuaranteeClass) -> &'static str {
    match class {
        GuaranteeClass::Fenced => "fenced",
        GuaranteeClass::RecordFenced => "record_fenced",
        GuaranteeClass::Keyed => "keyed",
        GuaranteeClass::AtLeastOnce => "at_least_once",
    }
}

fn parse_guarantee(text: &str) -> GuaranteeClass {
    match text {
        "fenced" => GuaranteeClass::Fenced,
        "record_fenced" => GuaranteeClass::RecordFenced,
        "keyed" => GuaranteeClass::Keyed,
        _ => GuaranteeClass::AtLeastOnce,
    }
}

const fn status_str(status: SinkStatus) -> &'static str {
    match status {
        SinkStatus::Complete => "complete",
        SinkStatus::CompleteWithRejections => "complete_with_rejections",
        SinkStatus::Behind => "behind",
        SinkStatus::Failed => "failed",
        SinkStatus::Inactive => "inactive",
    }
}

fn parse_status(text: &str) -> SinkStatus {
    match text {
        "complete" => SinkStatus::Complete,
        "complete_with_rejections" => SinkStatus::CompleteWithRejections,
        "behind" => SinkStatus::Behind,
        "inactive" => SinkStatus::Inactive,
        _ => SinkStatus::Failed,
    }
}
