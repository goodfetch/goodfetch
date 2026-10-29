//! Seeds added to a running crawl (ADRs 0024, 0093 and 0102; CO-02, CO-03,
//! RT-12): `run.add_seed` admits one seed through the Frontier, as the
//! start admits its seeds, committed at once in the Run's state, and
//! answers it as an enqueue is answered; named by a handler's attempt, it
//! is staged with that attempt as an enqueue is, committed or discarded
//! with it (ADR 0093), and fails with `attempt_ended` once the attempt
//! ended. Each seed added widens the crawl's scope, which the run file
//! holds, by its host, as the start's seeds do; a seed whose host the
//! Run's domain policy refuses is not added, with `domain_policy`. `run.add_seeds` takes a
//! stream of seeds under credits it grants its Client, at most
//! [`SEED_CREDITS`] seeds unanswered at a time, commits each batch in one
//! write and answers its seeds in order, so a million seeds never sit in
//! the engine's memory. Each seed added wakes the crawl's delivery, which
//! keys it and queues it under its domain, an idle `crawl.keep_alive`
//! crawl included. Once the crawl closed its intake to end its Run, a seed
//! added is refused with `run_not_found`, as on an ended crawl, so it is
//! never answered added and then left pending (#914). A seed whose Payload does not fit its Label's JSON
//! Schema or `crawl.payload.max_size` fails `run.add_seed`, and a batch of
//! `run.add_seeds` holding one fails the call, none of the batch admitted,
//! with `payload_invalid` (ADR 0082).

use std::sync::Arc;

use goodfetch_engine::egress::Policy;
use goodfetch_engine::runs::Run;
use goodfetch_engine::{CallContext, CallError, Handler};
use goodfetch_frontier::Scope;
use goodfetch_frontier::payload::PayloadRules;
use goodfetch_protocol::events::{CREDIT, CreditParams};
use goodfetch_protocol::foxhound::crawl::{ATTEMPT_ENDED, Seed};
use goodfetch_protocol::foxhound::frontier::{
    ENQUEUED, Enqueued, EnqueuedBatch, RUN_ADD_SEED, RUN_ADD_SEEDS, RunAddSeedParams,
    RunAddSeedsParams, RunSeedsAdded, SEED_CREDITS, SEEDS, SeedAttempt, SeedBatch,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::{RUN_NOT_FOUND, RUN_STATE_UNAVAILABLE, RunDetails, RunId};
use rusqlite::Connection;

use crate::intake::Intake;
use crate::serving::{Crawls, LiveCrawl};
use crate::work_items::{self, Positions};

/// The most `$/seeds` batches a `run.add_seeds` call leaves unread: a
/// Client honouring its credits has at most [`SEED_CREDITS`] seeds
/// unanswered, each batch holding one at least, and then the batch that
/// ends the stream, which may hold none.
const SEED_INPUT_ROOM: usize = SEED_CREDITS as usize + 1;

/// The handlers of `run.add_seed` and `run.add_seeds` on the live crawls of
/// `crawls`.
pub(crate) fn handlers(crawls: &Crawls) -> [Handler; 2] {
    let one = crawls.clone();
    let streamed = crawls.clone();
    [
        Handler::new(&RUN_ADD_SEED, move |params, context| {
            add_seed(params, context, one.clone())
        }),
        Handler::new(&RUN_ADD_SEEDS, move |params, context| {
            add_seeds(params, context, streamed.clone())
        })
        .taking_input(SEEDS, SEED_INPUT_ROOM),
    ]
}

/// `run_not_found` for a Run this engine does not run as a live crawl.
fn not_live(run_id: &RunId) -> CallError {
    CallError::new(
        &RUN_NOT_FOUND,
        format!(
            "This engine runs no crawl `{run_id}` to add seeds to: it ended or is ending, or another engine runs it."
        ),
        &RunDetails {
            run_id: run_id.clone(),
            path: None,
        },
    )
}

/// `run_state_unavailable` for a write the Run's state did not take.
fn unwritten(run_id: &RunId, error: &impl std::fmt::Display) -> CallError {
    CallError::new(
        &RUN_STATE_UNAVAILABLE,
        format!("The state of the Run `{run_id}` could not take the seeds: {error}"),
        &RunDetails {
            run_id: run_id.clone(),
            path: None,
        },
    )
}

async fn add_seed(
    params: RunAddSeedParams,
    _context: CallContext,
    crawls: Crawls,
) -> Result<Enqueued, CallError> {
    let Some(LiveCrawl {
        run,
        egress,
        seeded,
        staging,
        intake,
    }) = crawls.live(&params.run_id).await
    else {
        return Err(not_live(&params.run_id));
    };
    let RunAddSeedParams {
        run_id,
        seed,
        attempt,
    } = params;
    let bound = attempt.clone();
    let answered = run
        .state()
        .write(move |connection| {
            intake.admitting(|| match bound {
                Some(attempt) => staging.admit(
                    connection,
                    &attempt.crawl_request_id.clone(),
                    attempt.lease_epoch,
                    move |connection, positions| {
                        admit_within(connection, positions, &egress, &seed, Some(&attempt))
                    },
                ),
                None => admit_within(
                    connection,
                    &mut Positions::of(connection)?,
                    &egress,
                    &seed,
                    None,
                ),
            })
        })
        .await
        .map_err(|error| unwritten(&run_id, &error))?
        .ok_or_else(|| not_live(&run_id))?;
    if let Some(answer) = answered {
        seeded.notify_one();
        return answer;
    }
    let attempt = attempt.expect("invariant: only an attempt ends");
    Err(CallError::new(
        &ATTEMPT_ENDED,
        format!(
            "Attempt {} of crawl request `{}` of the crawl `{run_id}` already ended, so no seed was added.",
            attempt.lease_epoch, attempt.crawl_request_id
        ),
        &(),
    ))
}

/// Admits `seed` under the Run's domain policy `egress`, at its position
/// from `positions`, while `attempt`, when given, holds its lease, once
/// its Payload fits its Label's schema and the cap, else
/// `payload_invalid`; `None` once that attempt ended.
fn admit_within(
    connection: &Connection,
    positions: &mut Positions,
    egress: &Policy,
    seed: &Seed,
    attempt: Option<&SeedAttempt>,
) -> rusqlite::Result<Option<Result<Enqueued, CallError>>> {
    if let Some(attempt) = attempt
        && work_items::attempt(connection, &attempt.crawl_request_id, attempt.lease_epoch)?
            .is_none()
    {
        return Ok(None);
    }
    if let Err(refused) = refused(connection, std::slice::from_ref(seed))? {
        return Ok(Some(Err(refused)));
    }
    let mut answers = work_items::admit_seeds(
        connection,
        positions,
        &Scope::read(connection)?,
        egress,
        std::slice::from_ref(seed),
    )?;
    Ok(Some(Ok(answers
        .pop()
        .expect("invariant: the Frontier answers each seed"))))
}

/// `payload_invalid` for the first of `seeds` whose Payload does not fit
/// its Label's schema or the cap the run file `connection` holds.
fn refused(connection: &Connection, seeds: &[Seed]) -> rusqlite::Result<Result<(), CallError>> {
    let rules = PayloadRules::read(connection)?;
    Ok(seeds
        .iter()
        .try_for_each(|seed| work_items::check_seed(&rules, seed)))
}

async fn add_seeds(
    params: RunAddSeedsParams,
    context: CallContext,
    crawls: Crawls,
) -> Result<RunSeedsAdded, CallError> {
    let Some(LiveCrawl {
        run,
        egress,
        seeded,
        intake,
        ..
    }) = crawls.live(&params.run_id).await
    else {
        return Err(not_live(&params.run_id));
    };
    let input = context.input();
    let mut added = RunSeedsAdded::default();
    let mut credits = SEED_CREDITS;
    if !grant(&context, credits).await {
        return Ok(added);
    }
    loop {
        let Some(batch) = input.next().await? else {
            return Ok(added);
        };
        let batch: SeedBatch = serde_json::from_value(batch).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("A `$/seeds` batch holds no seeds: {error}"),
            )
        })?;
        let taken = u32::try_from(batch.seeds.len()).unwrap_or(u32::MAX);
        if taken > credits {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "A `$/seeds` batch of {taken} seeds came with {credits} credits granted: a Client sends no more seeds than its credits."
                ),
            ));
        }
        credits -= taken;
        let answers = admit_batch(&run, (&egress, &intake), &params.run_id, batch.seeds).await?;
        seeded.notify_one();
        for answer in &answers {
            match answer {
                Enqueued::Added { .. } => added.added += 1,
                Enqueued::NotAdded { .. } => added.not_added += 1,
            }
        }
        let sent = input
            .notify(
                ENQUEUED,
                &EnqueuedBatch {
                    id: input.id().clone(),
                    answers,
                },
            )
            .await;
        if batch.end || !sent {
            return Ok(added);
        }
        if taken > 0 {
            credits += taken;
            if !grant(&context, taken).await {
                return Ok(added);
            }
        }
    }
}

/// Grants the call's Client `credits` more seeds; `false` once the
/// Connection's writer is gone.
async fn grant(context: &CallContext, credits: u32) -> bool {
    let input = context.input();
    input
        .notify(
            CREDIT,
            &CreditParams {
                id: input.id().clone(),
                credits,
            },
        )
        .await
}

/// Admits each of `seeds` under the Run's domain policy `egress` in one
/// write of `run`'s state, and answers each one's answer in order; none
/// is admitted, and the call fails with `payload_invalid`, when one's
/// Payload does not fit its Label's schema or the cap, and with
/// `run_not_found` once the crawl's `intake` closed.
async fn admit_batch(
    run: &Run,
    (egress, intake): (&Arc<Policy>, &Intake),
    run_id: &RunId,
    seeds: Vec<Seed>,
) -> Result<Vec<Enqueued>, CallError> {
    if seeds.is_empty() {
        return Ok(Vec::new());
    }
    let egress = Arc::clone(egress);
    let intake = intake.clone();
    run.state()
        .write(move |connection| {
            intake.admitting(|| {
                if let Err(refused) = refused(connection, &seeds)? {
                    return Ok(Err(refused));
                }
                work_items::admit_seeds(
                    connection,
                    &mut Positions::of(connection)?,
                    &Scope::read(connection)?,
                    &egress,
                    &seeds,
                )
                .map(Ok)
            })
        })
        .await
        .map_err(|error| unwritten(run_id, &error))?
        .ok_or_else(|| not_live(run_id))?
}
