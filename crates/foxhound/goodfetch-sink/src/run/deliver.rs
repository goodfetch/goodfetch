//! Post-Run delivery outliving the Run (ADR 0026): Daemon backlog delivery
//! after a Run finishes, and `sink.deliver` (`redeliver`, `recover`, `replay`
//! and `backfill`) against past Runs and Sinks configured later.

use std::sync::{Arc, PoisonError};
use std::time::{Duration, SystemTime};

use goodfetch_engine::run::run_error;
use goodfetch_engine::run_state::run_row;
use goodfetch_engine::runs::RunError;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::{CallContext, CallError, named};
use goodfetch_protocol::foxhound::report::SinkStatus;
use goodfetch_protocol::foxhound::sink::{
    SINKS, SinkDeliver, SinkDeliverAction, SinkDeliverParams,
};
use goodfetch_protocol::point_in_time;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_protocol::settings::{SETTING_INVALID, SettingInvalidDetails};
use rusqlite::Connection;

use crate::contract::{DeliveryId, GuaranteeClass, RunCopy};
use crate::dead_letters::{self, DeadLetter};
use crate::driver::{Driver, Status};
use crate::report::{self, SinkStateRow};
use crate::schedule::SinkNotification;
use crate::sqlite::SqliteSink;
use crate::sync::Mutex;

use super::ops::open_settings;
use super::{Destination, Sink, deliverable, entries, layer};

/// Output of one synchronous delivery pass against a run file.
pub(super) struct PassOutput {
    pub(super) status: Status,
    pub(super) rejected: Vec<DeadLetter>,
    pub(super) notifications: Vec<SinkNotification>,
    pub(super) state_row: SinkStateRow,
}

/// Runs one delivery pass on `driver` against `run_file` and drains the
/// resulting dead letters, Notifications and state snapshot.
pub(super) fn deliver_pass(
    driver: &Mutex<Driver<SqliteSink>>,
    sink_name: &str,
    run_file: &Connection,
) -> rusqlite::Result<PassOutput> {
    let mut guard = driver.lock().unwrap_or_else(PoisonError::into_inner);
    let status = guard.deliver(run_file)?;
    let rejected = guard.take_dead_letters();
    let notifications = guard.take_notifications();
    let state_row = SinkStateRow {
        sink: sink_name.to_owned(),
        guarantee_class: guard.class().unwrap_or(GuaranteeClass::AtLeastOnce),
        status: guard.report_status(),
        paused_reason: guard.paused_reason(),
        outside_dedup_window: guard.outside_dedup_window(),
        undeclared_datasets: Vec::new(),
    };
    Ok(PassOutput {
        status,
        rejected,
        notifications,
        state_row,
    })
}

/// Spawns a task on `engine_owner` (the Daemon's task owner, which outlives
/// the Run) to keep retrying a behind Sink's backlog after its Run finishes
/// (ADR 0026).
pub(super) fn spawn_backlog(
    sink: Arc<Sink>,
    engine_owner: &Arc<TaskOwner>,
    initial_wait: Duration,
    finished: bool,
) {
    let owner = Arc::clone(engine_owner);
    engine_owner.spawn(async move {
        deliver_backlog(sink, owner, initial_wait, finished).await;
    });
}

/// Retries `sink`'s backlog against the ended Run's file until every Record
/// lands, the Sink pauses/fails/is superseded, or the Daemon shuts down.
async fn deliver_backlog(
    sink: Arc<Sink>,
    engine_owner: Arc<TaskOwner>,
    mut wait: Duration,
    finished: bool,
) {
    let Some(Ok(driver)) = sink.driver.get().cloned() else {
        return;
    };
    let run_id = sink.identity.id().clone();
    let clock = sink.identity.clock();
    let masks = sink.keys.secrets().mask().masks().clone();
    loop {
        tokio::select! {
            biased;
            () = engine_owner.cancelled() => return,
            () = clock.sleep(wait) => {}
        }
        let Ok(mut reader) = sink.runs.reader(run_id.clone()).await else {
            return;
        };
        let driver_for_read = Arc::clone(&driver);
        let sink_name = sink.name.clone();
        let delivered = reader
            .read(move |run_file| deliver_pass(&driver_for_read, &sink_name, run_file))
            .await;
        drop(reader);
        let Ok(PassOutput {
            status,
            rejected,
            notifications,
            state_row,
        }) = delivered
        else {
            return;
        };
        if finished && matches!(status, Status::Complete | Status::CompleteWithRejections) {
            let driver_for_finish = Arc::clone(&driver);
            let _ = engine_owner
                .blocking(move || {
                    driver_for_finish
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .finish()
                })
                .await;
        }
        let sink_name = sink.name.clone();
        let masks_for_write = masks.clone();
        if let Err(error) = sink
            .runs
            .update_ended(run_id.clone(), move |connection| {
                for entry in &rejected {
                    dead_letters::record(connection, entry, &masks_for_write)?;
                }
                for notification in &notifications {
                    report::record_notification(connection, notification, &masks_for_write)?;
                }
                report::record_state(connection, &state_row)?;
                report::refresh_stored_report(connection)
            })
            .await
        {
            tracing::warn!(
                sink = %sink_name,
                %run_id,
                %error,
                "Daemon backlog delivery could not persist updated Sink state"
            );
        }
        match status {
            Status::Behind { retry_in, .. } => {
                wait = retry_in;
            }
            Status::Complete
            | Status::CompleteWithRejections
            | Status::Paused { .. }
            | Status::Failed(_)
            | Status::Superseded { .. } => return,
        }
    }
}

/// What one `sink.deliver` execution against a read connection produced,
/// before persisting dead-letter and state updates to the ended run file.
struct ActionOutput {
    delivered: u64,
    removed_dead_letters: Vec<DeliveryId>,
    recorded_dead_letters: Vec<DeadLetter>,
    notifications: Vec<SinkNotification>,
    status: Status,
    state_row: SinkStateRow,
}

/// Resolves `params`'s Sink entry and verifies it is deliverable by this
/// engine.
async fn resolve_deliverable_sink(
    params: &mut SinkDeliverParams,
    context: &CallContext,
) -> Result<
    (
        goodfetch_protocol::foxhound::sink::SinkSettings,
        goodfetch_protocol::settings::SettingsLayer,
    ),
    CallError,
> {
    let settings = open_settings(&params.settings, params.project.take(), context).await?;
    let layer = layer(&settings);
    let sink_name = &params.sink;
    let Some((_, entry)) = entries(&settings).find(|(name, _)| name == sink_name) else {
        let key = format!("{}.{sink_name}", SINKS.name);
        let reason = format!("the settings name no Sink `{sink_name}`");
        let fix = format!("configure `sinks.{sink_name}` in settings or project configuration");
        return Err(CallError::new(
            &SETTING_INVALID,
            format!(
                "`{}` is invalid: {reason}; {fix}.",
                named::setting_in(&key, layer)
            ),
            &SettingInvalidDetails {
                key,
                layer: Some(layer),
                reason,
                limit: None,
                fix: Some(fix),
            },
        ));
    };
    deliverable(sink_name, &entry, layer)?;
    Ok((entry, layer))
}

/// `sink.deliver`: redelivers one delivery, recovers failed deliveries since a
/// timestamp, replays undelivered Records, or backfills a past Run to any Sink
/// (ADRs 0026 and 0099).
pub(super) async fn deliver_op(
    mut params: SinkDeliverParams,
    context: CallContext,
) -> Result<SinkDeliver, CallError> {
    let (entry, layer) = resolve_deliverable_sink(&mut params, &context).await?;
    let sink_name = params.sink.clone();

    let failed = |error: RunError| run_error(error, &context);
    let mut reader = context
        .runs()
        .reader(params.run_id.clone())
        .await
        .map_err(failed)?;
    let (run_epoch, run_lifecycle, initial_dead_letters) = reader
        .read({
            let sink_name = sink_name.clone();
            move |connection| {
                let epoch = run_row::epoch(connection)?;
                let lifecycle = run_row::lifecycle(connection)?;
                let count = u64::try_from(dead_letters::list(connection, &sink_name)?.len())
                    .unwrap_or(u64::MAX);
                Ok((epoch, lifecycle, count))
            }
        })
        .await
        .map_err(failed)?;

    let (_, secrets) = context.runs().outside_a_run();
    let masks = secrets.mask().masks().clone();
    let destination = Destination {
        name: &sink_name,
        settings: &entry,
        layer,
        run_id: Some(&params.run_id),
    };
    let run_copy = RunCopy {
        run_id: params.run_id.as_str().to_owned(),
        run_epoch,
    };
    let driver = match destination
        .open_with_epoch(
            &secrets,
            context.engine().owner(),
            context.engine().clock(),
            run_copy,
            true,
        )
        .await
    {
        Ok(driver) => Arc::new(Mutex::new(driver)),
        Err(cause) => {
            tracing::warn!(sink = %sink_name, run_id = %params.run_id, %cause, "`sink.deliver` could not open destination");
            return Ok(SinkDeliver {
                run_id: params.run_id,
                sink: sink_name,
                delivered: 0,
                dead_letters: initial_dead_letters,
                status: SinkStatus::Failed,
            });
        }
    };

    let should_finish = matches!(
        params.action,
        SinkDeliverAction::Replay | SinkDeliverAction::Backfill
    ) && run_lifecycle == Lifecycle::Finished;
    let action = params.action;
    let driver_for_read = Arc::clone(&driver);
    let sink_for_read = sink_name.clone();
    let output = reader
        .read(move |run_file| run_action(run_file, &sink_for_read, &driver_for_read, action))
        .await
        .map_err(failed)?;
    drop(reader);

    if should_finish
        && matches!(
            output.status,
            Status::Complete | Status::CompleteWithRejections
        )
    {
        let driver_for_finish = Arc::clone(&driver);
        let _ = context
            .engine()
            .owner()
            .blocking(move || {
                driver_for_finish
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .finish()
            })
            .await;
    }

    let delivered = output.delivered;
    let (remaining_dead_letters, final_status) =
        persist_action_output(&context, &params.run_id, &sink_name, output, masks).await?;

    Ok(SinkDeliver {
        run_id: params.run_id,
        sink: sink_name,
        delivered,
        dead_letters: remaining_dead_letters,
        status: final_status,
    })
}

/// Persists `output`'s dead-letter updates, Notifications, Sink state and
/// refreshed stored Run report into the ended Run's file, returning the
/// remaining dead-letter count and final [`SinkStatus`].
async fn persist_action_output(
    context: &CallContext,
    run_id: &goodfetch_protocol::run::RunId,
    sink_name: &str,
    output: ActionOutput,
    masks: goodfetch_engine::mask::Masks,
) -> Result<(u64, SinkStatus), CallError> {
    let sink_for_write = sink_name.to_owned();
    context
        .runs()
        .update_ended(run_id.clone(), move |connection| {
            for id in &output.removed_dead_letters {
                dead_letters::remove(connection, &sink_for_write, id)?;
            }
            for entry in &output.recorded_dead_letters {
                dead_letters::record(connection, entry, &masks)?;
            }
            for notification in &output.notifications {
                report::record_notification(connection, notification, &masks)?;
            }
            let remaining = u64::try_from(dead_letters::list(connection, &sink_for_write)?.len())
                .unwrap_or(u64::MAX);
            let final_status = match output.state_row.status {
                SinkStatus::Failed | SinkStatus::Behind | SinkStatus::Inactive => {
                    output.state_row.status
                }
                SinkStatus::Complete | SinkStatus::CompleteWithRejections => {
                    if remaining > 0 {
                        SinkStatus::CompleteWithRejections
                    } else {
                        SinkStatus::Complete
                    }
                }
            };
            let mut row = output.state_row;
            row.sink = sink_for_write;
            row.status = final_status;
            report::record_state(connection, &row)?;
            report::refresh_stored_report(connection)?;
            Ok((remaining, final_status))
        })
        .await
        .map_err(|error| run_error(error, context))
}

/// Executes one `SinkDeliverAction` against `run_file` using `driver`.
fn run_action(
    run_file: &Connection,
    sink_name: &str,
    driver: &Mutex<Driver<SqliteSink>>,
    action: SinkDeliverAction,
) -> rusqlite::Result<ActionOutput> {
    let mut guard = driver.lock().unwrap_or_else(PoisonError::into_inner);
    let (delivered, removed_dead_letters, recorded_dead_letters, status) = match action {
        SinkDeliverAction::Redeliver { delivery_id } => {
            let outcome = guard.redeliver(run_file, &delivery_id)?;
            let removed = outcome
                .delivered
                .then(|| DeliveryId::from_stored(delivery_id))
                .into_iter()
                .collect();
            let recorded = outcome.still_rejected.into_iter().collect();
            (
                u64::from(outcome.delivered),
                removed,
                recorded,
                outcome.status,
            )
        }
        SinkDeliverAction::Recover { since } => {
            let matching = dead_letters_since(run_file, sink_name, since)?;
            let outcome = guard.retry_dead_letters(run_file, &matching)?;
            let count = u64::try_from(outcome.delivered.len()).unwrap_or(u64::MAX);
            (
                count,
                outcome.delivered,
                outcome.still_rejected,
                outcome.status,
            )
        }
        SinkDeliverAction::Replay => {
            let status = guard.deliver(run_file)?;
            let rejected = guard.take_dead_letters();
            (guard.delivered_count(), Vec::new(), rejected, status)
        }
        SinkDeliverAction::Backfill => {
            let status = guard.backfill(run_file)?;
            let rejected = guard.take_dead_letters();
            (guard.delivered_count(), Vec::new(), rejected, status)
        }
    };
    let notifications = guard.take_notifications();
    let state_row = SinkStateRow {
        sink: sink_name.to_owned(),
        guarantee_class: guard.class().unwrap_or(GuaranteeClass::AtLeastOnce),
        status: guard.report_status(),
        paused_reason: guard.paused_reason(),
        outside_dedup_window: guard.outside_dedup_window(),
        undeclared_datasets: Vec::new(),
    };
    Ok(ActionOutput {
        delivered,
        removed_dead_letters,
        recorded_dead_letters,
        notifications,
        status,
        state_row,
    })
}

/// Lists dead letters for `sink_name` in `run_file` committed at or after
/// `since` (inclusive).
fn dead_letters_since(
    run_file: &Connection,
    sink_name: &str,
    since: SystemTime,
) -> rusqlite::Result<Vec<DeadLetter>> {
    Ok(dead_letters::list(run_file, sink_name)?
        .into_iter()
        .filter(|entry| {
            point_in_time::from_rfc3339(&entry.committed_at)
                .is_ok_and(|committed| committed >= since)
        })
        .collect())
}
