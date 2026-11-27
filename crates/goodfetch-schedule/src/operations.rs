//! The `schedule.*` handlers (ADR 0036): `schedule.create` checks a
//! definition and records it in the Schedule store, `schedule.list` and
//! `schedule.get` read the store, and a Schedule's own `schedule.info` and
//! `schedule.delete` act on it. Each waits for the store under its call's
//! `timeout`, `timeout.wait` when absent (ADR 0080). The store is the
//! Daemon's: an Attached engine that can start its major's Daemon forwards
//! each call to it, and only a Daemon, or an engine that cannot start one,
//! such as a Linked engine, runs these handlers.

use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::user_store::{UserStore, UserStoreError};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler, kept_by_the_daemon};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::schedule::{
    DEFAULT_PREVIEW_COUNT, MAX_PREVIEW_COUNT, SCHEDULE_CREATE, SCHEDULE_DELETE, SCHEDULE_EXISTS,
    SCHEDULE_GET, SCHEDULE_INFO, SCHEDULE_LIST, SCHEDULE_NOT_FOUND, SCHEDULE_PREVIEW,
    SCHEDULE_STORE_ANSWER, SCHEDULE_TRIGGER_INVALID, ScheduleCreateParams, ScheduleCreated,
    ScheduleDeleted, ScheduleExistsDetails, ScheduleGetParams, ScheduleId, ScheduleInfo,
    ScheduleList, ScheduleListParams, ScheduleNotFoundDetails, ScheduleParams, SchedulePreview,
    SchedulePreviewParams, ScheduleState,
};
use goodfetch_protocol::settings::TIMEOUT_WAIT;
use jiff::tz::TimeZone;

use crate::{definition, store, trigger};

/// What the schedule crate adds to an engine: the Schedule store and the
/// `schedule.*` handlers, each call sent to the major's Daemon when the
/// engine is an Attached engine that reaches it.
pub fn contribution() -> Contribution {
    Contribution::new("schedule")
        .with_user_store(&store::STORE)
        .with_handler(Handler::new(&SCHEDULE_CREATE, create))
        .with_handler(Handler::new(&SCHEDULE_LIST, list))
        .with_handler(Handler::new(&SCHEDULE_GET, get))
        .with_handler(Handler::new(&SCHEDULE_PREVIEW, preview))
        .with_handler(Handler::new(&SCHEDULE_INFO, info))
        .with_handler(Handler::new(&SCHEDULE_DELETE, delete))
        .guarded(&kept_by_the_daemon())
}

async fn create(
    params: ScheduleCreateParams,
    context: CallContext,
) -> Result<ScheduleCreated, CallError> {
    let deadline = deadline(&context, params.timeout).await?;
    let definition = params.definition;
    trigger::check(&definition.trigger).map_err(|refused| {
        CallError::new(&SCHEDULE_TRIGGER_INVALID, refused.message, &refused.details)
    })?;
    let engine = context.engine();
    let time_zone = definition::time_zone(definition.time_zone.as_deref(), engine.host_zone())
        .map_err(|unknown| {
            CallError::protocol(ProtocolFailure::InvalidParams, unknown.to_string())
        })?;
    let clock = engine.clock();
    let schedule_id = definition
        .id
        .unwrap_or_else(|| definition::generated_id(clock.wall(), || engine.draw_u64()));
    let schedule = ScheduleInfo {
        schedule_id: schedule_id.clone(),
        target: definition.target,
        trigger: definition.trigger,
        time_zone,
        version: 1,
        state: ScheduleState::On,
        creator: engine.person().to_owned(),
        created_at: clock.wall(),
    };
    let jitter_ms = trigger::draw_jitter(0, || engine.draw_u64());
    let draws = engine.clone();
    let created = within(
        deadline.as_ref(),
        &clock,
        store::create(&store_of(&context), schedule, jitter_ms, move || {
            store::made_store_id(draws.draw_u64(), draws.draw_u64())
        }),
    )
    .await?
    .map_err(|error| store_failed(&error))?;
    if !created {
        return Err(CallError::new(
            &SCHEDULE_EXISTS,
            format!(
                "A Schedule has the ID `{schedule_id}` already; create it under another ID, or omit the ID for a new one."
            ),
            &ScheduleExistsDetails { schedule_id },
        ));
    }
    Ok(ScheduleCreated {
        schedule_id,
        warnings: Vec::new(),
    })
}

async fn list(params: ScheduleListParams, context: CallContext) -> Result<ScheduleList, CallError> {
    let deadline = deadline(&context, params.timeout).await?;
    let schedules = within(
        deadline.as_ref(),
        &context.engine().clock(),
        store::list(&store_of(&context)),
    )
    .await?
    .map_err(|error| store_failed(&error))?;
    Ok(ScheduleList { schedules })
}

async fn get(params: ScheduleGetParams, context: CallContext) -> Result<ScheduleInfo, CallError> {
    read_one(params.id, params.timeout, &context).await
}

async fn preview(
    params: SchedulePreviewParams,
    context: CallContext,
) -> Result<SchedulePreview, CallError> {
    let _deadline = deadline(&context, params.timeout).await?;
    let count = params.count.unwrap_or(DEFAULT_PREVIEW_COUNT);
    if count > MAX_PREVIEW_COUNT {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`count` is {count}; ask for at most {MAX_PREVIEW_COUNT} Occurrences."),
        ));
    }
    let parsed = trigger::parse(&params.trigger).map_err(|refused| {
        CallError::new(&SCHEDULE_TRIGGER_INVALID, refused.message, &refused.details)
    })?;
    let engine = context.engine();
    let time_zone = definition::time_zone(params.time_zone.as_deref(), engine.host_zone())
        .map_err(|unknown| {
            CallError::protocol(ProtocolFailure::InvalidParams, unknown.to_string())
        })?;
    let zone = TimeZone::get(&time_zone).expect("invariant: `definition::time_zone` validated it");
    let after = params.after.unwrap_or_else(|| engine.clock().wall());
    let count = usize::try_from(count).expect("invariant: count <= MAX_PREVIEW_COUNT");
    let occurrences = trigger::occurrences(&parsed, &zone, after, Duration::ZERO, after, count)
        .map_err(|refused| {
            CallError::new(&SCHEDULE_TRIGGER_INVALID, refused.message, &refused.details)
        })?;
    Ok(SchedulePreview {
        description: trigger::describe(&parsed),
        time_zone,
        occurrences,
    })
}

async fn info(params: ScheduleParams, context: CallContext) -> Result<ScheduleInfo, CallError> {
    read_one(params.schedule_id, params.timeout, &context).await
}

async fn delete(
    params: ScheduleParams,
    context: CallContext,
) -> Result<ScheduleDeleted, CallError> {
    let deadline = deadline(&context, params.timeout).await?;
    let deleted = within(
        deadline.as_ref(),
        &context.engine().clock(),
        store::delete(&store_of(&context), &params.schedule_id),
    )
    .await?
    .map_err(|error| store_failed(&error))?;
    Ok(ScheduleDeleted { deleted })
}

/// The Schedule of `id`, or `schedule_not_found`.
#[expect(
    clippy::option_option,
    reason = "the param's three states as the protocol carries them (ADR 0080): absent, `null` or a duration"
)]
async fn read_one(
    id: ScheduleId,
    timeout: Option<Option<u64>>,
    context: &CallContext,
) -> Result<ScheduleInfo, CallError> {
    let deadline = deadline(context, timeout).await?;
    let found = within(
        deadline.as_ref(),
        &context.engine().clock(),
        store::get(&store_of(context), &id),
    )
    .await?
    .map_err(|error| store_failed(&error))?;
    found.ok_or_else(|| {
        CallError::new(
            &SCHEDULE_NOT_FOUND,
            format!("No Schedule has the ID `{id}`; `schedule.list` lists the store's Schedules."),
            &ScheduleNotFoundDetails { schedule_id: id },
        )
    })
}

/// The call's deadline: its own `timeout`, else `timeout.wait` as the
/// machine's settings resolve it, else that setting's default.
#[expect(
    clippy::option_option,
    reason = "the param's three states as the protocol carries them (ADR 0080): absent, `null` or a duration"
)]
async fn deadline(
    context: &CallContext,
    timeout: Option<Option<u64>>,
) -> Result<Option<Deadline>, CallError> {
    let snapshot = context.engine().settings().snapshot().await;
    let scope = Deadline::of_resolved(snapshot.resolved().ok(), &TIMEOUT_WAIT);
    Deadline::of_call(&scope, &TIMEOUT_WAIT, timeout)
}

/// What `work` answers within `deadline`, if any.
async fn within<T>(
    deadline: Option<&Deadline>,
    clock: &Clock,
    work: impl Future<Output = T>,
) -> Result<T, CallError> {
    match deadline {
        Some(deadline) => deadline.within(clock, SCHEDULE_STORE_ANSWER, work).await,
        None => Ok(work.await),
    }
}

/// The engine's Schedule store, which [`contribution`] declares.
fn store_of(context: &CallContext) -> UserStore {
    context
        .engine()
        .user_store(&store::STORE)
        .expect(
            "invariant: the contribution that registers these handlers declares the Schedule store",
        )
        .clone()
}

/// The error a call answers when the store fails it.
fn store_failed(error: &UserStoreError) -> CallError {
    match error {
        UserStoreError::Format {
            found,
            compatibility,
            ..
        } => goodfetch_engine::run::format_refusal(&store::SCHEDULE_STORE, found, *compatibility),
        _ => CallError::protocol(ProtocolFailure::InternalError, error.to_string()),
    }
}
