//! Declared Effects (ADR 0106): the Run's Effect journal, a table of run
//! state, keeps each Effect key's entry, and [`Effects`] carries the
//! bodies that run. `effect.open` writes an entry before its body starts
//! and answers `perform`, or the journal's answer when the body must not
//! run; `effect.close` judges the body by whether something was sent for
//! it, which [`Effects::sending`] marks, synced, before the first command
//! the window attributes to it goes out. An entry left `open` or `sent` by
//! a body whose attempt, Connection or engine is gone is judged as cut
//! short at its key's next declaration, so a crash after the sent mark
//! leaves the Effect uncertain, never run again until a person reconciles
//! it ([`Effects::reconcile`]). Every change of an entry's state goes
//! through [`lifecycle::apply`].
//!
//! The idempotency key a body sends is derived from a per-Run secret kept
//! in the journal's own table, so it is the same on every attempt and
//! resume, and a Checkpoint carries it.

mod journal;
pub mod lifecycle;

pub use journal::{Report, TABLES};

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, CancelledBy};
use goodfetch_protocol::effect::{
    EFFECT_KEY_INVALID, EFFECT_KEY_REUSED, EFFECT_OUTPUT_INVALID, EFFECT_OUTPUT_MAX_SIZE,
    EFFECT_RECONCILED, EFFECT_UNCERTAIN, EffectAttempt, EffectCloseParams, EffectClosed,
    EffectOpenParams, EffectOpened, EffectOutcome, KEY_MAX_CODE_POINTS, UncertainReason,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::{RUN_NOT_FOUND, RunDetails, RunId, UncertainEffect};
use serde_json::{Value, json};
use tokio::sync::{Mutex, MutexGuard, Notify};

use self::journal::{
    Answering, Declared, SECRET_BYTES, answer_about, declare, done, drop_entry, idempotency_key,
    mark_sent, row, turn_uncertain,
};
use self::lifecycle::{EffectEvent, EffectState, InvalidEffectTransition, Next, apply};
use crate::audit::{AuditError, Origin};
use crate::call_error::CallError;
use crate::client::Client;
use crate::composition::{CallContext, Handler};
use crate::mask::Scope;
use crate::run::run_error;
use crate::run_state::{RunStateError, Syncing};
use crate::runs::{Run, RunError};
use crate::settings::{RunSettings, get_or_default};
use crate::staging::{AttemptWrites, Unstaged};

/// How often a declaration that waits for another attempt's body checks
/// whether that attempt still holds.
const HOLD_CHECK: Duration = Duration::from_secs(1);

/// The bodies of declared Effects that run, by Run, and the answers each
/// attempt or Connection got: one per engine, shared by the operations
/// and the window that marks what is sent.
#[derive(Clone, Default)]
pub struct Effects {
    shelves: Arc<Mutex<HashMap<RunId, Arc<Shelf>>>>,
}

/// One Run's bodies and answers, and the signal that a body was judged.
#[derive(Default)]
struct Shelf {
    book: Mutex<Book>,
    judged: Notify,
}

#[derive(Default)]
struct Book {
    bodies: Vec<Body>,
    answered: Vec<Answered>,
}

impl Book {
    /// Whether it holds nothing a live caller needs.
    fn idle(&self) -> bool {
        self.bodies.is_empty()
            && self
                .answered
                .iter()
                .all(|answered| !answered.owner.is_alive())
    }
}

/// A body that runs.
struct Body {
    entry: i64,
    namespace: String,
    key: String,
    repeatable: bool,
    /// The first command sent for it, and when, once one was.
    sent: Option<(String, SystemTime)>,
    owner: Owner,
}

impl Body {
    /// Its state in the transition table.
    fn state(&self) -> EffectState {
        match (self.sent.is_some(), self.repeatable) {
            (false, false) => EffectState::Open,
            (false, true) => EffectState::OpenRepeatable,
            (true, false) => EffectState::Sent,
            (true, true) => EffectState::SentRepeatable,
        }
    }
}

/// A key whose owner got its answer: a done body, or the journal's.
struct Answered {
    namespace: String,
    key: String,
    owner: Owner,
}

/// Who declares an Effect: a handler's attempt, or a Connection outside
/// any attempt.
#[derive(Clone)]
struct Owner {
    client: Client,
    attempt: Option<Attempt>,
}

#[derive(Clone)]
struct Attempt {
    writes: Arc<dyn AttemptWrites>,
    id: EffectAttempt,
}

impl Owner {
    /// Whether its Connection and its attempt, if any, still hold.
    fn is_alive(&self) -> bool {
        !self.client.is_gone()
            && self.attempt.as_ref().is_none_or(|attempt| {
                attempt
                    .writes
                    .holds(&attempt.id.crawl_request_id, attempt.id.lease_epoch)
            })
    }

    /// Whether `other` is the same owner: the same attempt, or the same
    /// Connection when neither declares through one.
    fn is(&self, other: &Self) -> bool {
        match (&self.attempt, &other.attempt) {
            (Some(mine), Some(theirs)) => mine.id == theirs.id,
            (None, None) => self.client.same_connection(&other.client),
            (Some(_), None) | (None, Some(_)) => false,
        }
    }

    /// Whether it declares through `attempt`, or outside any attempt when
    /// `attempt` is `None`.
    fn declares_through(&self, attempt: Option<&EffectAttempt>) -> bool {
        self.attempt.as_ref().map(|mine| &mine.id) == attempt
    }

    /// Notes that its body ended in its attempt.
    fn end_effect(&self) {
        if let Some(attempt) = &self.attempt {
            attempt
                .writes
                .end_effect(&attempt.id.crawl_request_id, attempt.id.lease_epoch);
        }
    }
}

/// A person's answer about an uncertain Effect (ADR 0029).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconciliation {
    /// "It happened": the Effect is kept as performed, with no output.
    Happened,
    /// "It did not happen": the entry is dropped, and the next declaration
    /// runs the body.
    DidNotHappen,
    /// "Leave uncertain": nothing changes.
    LeaveUncertain,
}

impl Reconciliation {
    fn event(self) -> EffectEvent {
        match self {
            Self::Happened => EffectEvent::Happened,
            Self::DidNotHappen => EffectEvent::DidNotHappen,
            Self::LeaveUncertain => EffectEvent::LeaveUncertain,
        }
    }

    /// Its name in the Audit log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Happened => "happened",
            Self::DidNotHappen => "did_not_happen",
            Self::LeaveUncertain => "leave_uncertain",
        }
    }
}

/// What a person's answer did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reconciled {
    /// The answer was about this uncertain Effect, and the Audit log
    /// records it.
    Answered(UncertainEffect),
    /// The entry is not uncertain: absent, running, done or reconciled
    /// already. Nothing changed and nothing was recorded.
    NotUncertain,
}

impl Effects {
    /// The shelf of the Run `run_id`, made when it has none; shelves no
    /// live caller needs are dropped.
    async fn shelf(&self, run_id: &RunId) -> Arc<Shelf> {
        let mut shelves = self.shelves.lock().await;
        shelves.retain(|kept, shelf| {
            kept == run_id || !shelf.book.try_lock().is_ok_and(|book| book.idle())
        });
        Arc::clone(shelves.entry(run_id.clone()).or_default())
    }

    /// The shelf of the Run `run_id`, if it has one.
    async fn shelf_of(&self, run_id: &RunId) -> Option<Arc<Shelf>> {
        self.shelves.lock().await.get(run_id).map(Arc::clone)
    }

    /// `effect.open`: writes the Effect's entry and answers `perform`, or
    /// the journal's answer.
    ///
    /// # Errors
    ///
    /// `effect_key_invalid`, `effect_key_reused`, `effect_uncertain`, an
    /// attempt's refusal, a Run that is not live here, or run state that
    /// cannot be written.
    pub async fn open(
        &self,
        params: EffectOpenParams,
        context: &CallContext,
    ) -> Result<EffectOpened, CallError> {
        let EffectOpenParams {
            run_id,
            key,
            repeatable,
            attempt,
        } = params;
        checked_key(&key)?;
        let run = live(&run_id, context).await?;
        let owner = Owner {
            client: context.client().clone(),
            attempt: match attempt {
                Some(id) => Some(Attempt {
                    writes: attempt_writes_of(&run)?,
                    id,
                }),
                None => None,
            },
        };
        let namespace = owner
            .attempt
            .as_ref()
            .map_or_else(String::new, |attempt| attempt.id.crawl_request_id.clone());
        let run_shelf = self.shelf(&run_id).await;
        let mut book = run_shelf
            .free_key(&run, &namespace, &key, &owner, context)
            .await?;
        if let Some(attempt) = &owner.attempt {
            attempt
                .writes
                .begin_effect(&attempt.id.crawl_request_id, attempt.id.lease_epoch)
                .map_err(unstaged)?;
        }
        let answer = match declared(&run, &namespace, &key, repeatable, context).await {
            Ok(Declared::Perform { entry, secret }) => {
                let idempotency_key = idempotency_key(&secret, &namespace, &key);
                book.bodies.push(Body {
                    entry,
                    namespace,
                    key,
                    repeatable,
                    sent: None,
                    owner,
                });
                return Ok(EffectOpened::Perform {
                    entry: entry.to_string(),
                    idempotency_key,
                });
            }
            Ok(other) => journal_answer(other, &key),
            Err(error) => Err(error),
        };
        owner.end_effect();
        if answer.is_ok() {
            book.answered.push(Answered {
                namespace,
                key,
                owner,
            });
        }
        answer
    }

    /// `effect.close`: judges how the body of `entry` ended by whether
    /// something was sent for it.
    ///
    /// # Errors
    ///
    /// `effect_output_invalid` for a value larger than
    /// `effect.output.max_size` with nothing sent, `invalid_params` for an
    /// entry with no body running, a Run that is not live here, or run
    /// state that cannot be written.
    pub async fn close(
        &self,
        params: EffectCloseParams,
        context: &CallContext,
    ) -> Result<EffectClosed, CallError> {
        let EffectCloseParams {
            run_id,
            entry,
            outcome,
        } = params;
        let Ok(number) = entry.parse::<i64>() else {
            return Err(not_running(&entry));
        };
        let run = live(&run_id, context).await?;
        let run_shelf = self.shelf(&run_id).await;
        let mut book = run_shelf.book.lock().await;
        let Some(at) = book.bodies.iter().position(|body| body.entry == number) else {
            drop(book);
            return closed_before(&run, &entry, number, context).await;
        };
        let limit: u64 = get_or_default(
            run.settings().map(RunSettings::resolved),
            &EFFECT_OUTPUT_MAX_SIZE,
        );
        let (event, output, oversized) = ended(outcome, limit);
        let body = book.bodies.swap_remove(at);
        let judged = match apply(body.state(), event) {
            Ok(Next::To(EffectState::Done)) => {
                let performed_at = run.point_in_time();
                let output =
                    output.map(|output| run.secrets().mask().masks().json(&output, Scope::Kept));
                run.state()
                    .write(move |connection| {
                        done(connection, number, output.as_deref(), performed_at)
                    })
                    .await
                    .map(|()| EffectClosed::Done { performed_at })
                    .map_err(|error| unwritten(&run, &error, context))
            }
            Ok(Next::Dropped) => {
                let sent = body.sent.is_some();
                run.state()
                    .write_deferred(move |connection| drop_entry(connection, number))
                    .await
                    .map_err(|error| unwritten(&run, &error, context))
                    .and_then(|()| match oversized {
                        Some(size) if !sent => Err(output_invalid(&body.key, size, limit)),
                        Some(_) | None => Ok(EffectClosed::Dropped { sent }),
                    })
            }
            Ok(Next::To(EffectState::Uncertain)) => {
                let (effect, reason) = body.uncertain(&entry, event);
                run.state()
                    .write(move |connection| turn_uncertain(connection, number, reason))
                    .await
                    .map(|()| EffectClosed::Uncertain {
                        effect,
                        cause: oversized.map(|_| EFFECT_OUTPUT_INVALID.name.to_owned()),
                    })
                    .map_err(|error| unwritten(&run, &error, context))
            }
            Ok(next @ (Next::To(_) | Next::Unchanged)) => Err(CallError::protocol(
                ProtocolFailure::InternalError,
                format!(
                    "the Effect journal judged the body of `{entry}` as {next:?}, which no close makes"
                ),
            )),
            Err(refused) => Err(refusal(refused)),
        };
        body.owner.end_effect();
        if matches!(judged, Ok(EffectClosed::Done { .. })) {
            book.answered.push(Answered {
                namespace: body.namespace,
                key: body.key,
                owner: body.owner,
            });
        }
        run_shelf.judged.notify_waiters();
        judged
    }

    /// Marks sent, synced, every body of `run` declared through `attempt`,
    /// or outside any attempt when it is `None`, that nothing was sent for
    /// yet, before `command`, which may change a page or a site, goes out
    /// (ADR 0106's window). Writes nothing when no such body runs.
    ///
    /// # Errors
    ///
    /// The mark could not be written: the command must not go out.
    pub async fn sending(
        &self,
        run: &Run,
        attempt: Option<&EffectAttempt>,
        command: &str,
    ) -> Result<(), RunStateError> {
        let Some(run_shelf) = self.shelf_of(run.id()).await else {
            return Ok(());
        };
        let mut book = run_shelf.book.lock().await;
        let marked: Vec<usize> = book
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, body)| {
                body.owner.declares_through(attempt)
                    && matches!(apply(body.state(), EffectEvent::Send), Ok(Next::To(_)))
            })
            .map(|(at, _)| at)
            .collect();
        if marked.is_empty() {
            return Ok(());
        }
        let since = run.point_in_time();
        let entries: Vec<i64> = marked.iter().map(|&at| book.bodies[at].entry).collect();
        let first_command = command.to_owned();
        run.state()
            .write_with(Syncing::Sent, move |connection| {
                mark_sent(connection, &entries, &first_command, since)
            })
            .await?;
        for at in marked {
            book.bodies[at].sent = Some((command.to_owned(), since));
        }
        Ok(())
    }

    /// A person's `answer` about the uncertain Effect `entry` of `run`,
    /// recorded in the Audit log as `context`'s person's.
    ///
    /// # Errors
    ///
    /// Run state that cannot be written, or an Audit log that cannot
    /// record the answer, which is then made.
    pub async fn reconcile(
        &self,
        run: &Run,
        entry: &str,
        answer: Reconciliation,
        context: &CallContext,
    ) -> Result<Reconciled, CallError> {
        let Ok(number) = entry.parse::<i64>() else {
            return Ok(Reconciled::NotUncertain);
        };
        let run_shelf = self.shelf(run.id()).await;
        let book = run_shelf.book.lock().await;
        if book.bodies.iter().any(|body| body.entry == number) {
            return Ok(Reconciled::NotUncertain);
        }
        let by = context.person();
        let at = run.point_in_time();
        let answered = {
            let by = by.clone();
            run.state()
                .write(move |connection| answer_about(connection, number, answer, &by, at))
                .await
                .map_err(|error| unwritten(run, &error, context))?
        };
        drop(book);
        let effect = match answered {
            Answering::Answered(effect) => effect,
            Answering::NotUncertain => return Ok(Reconciled::NotUncertain),
            Answering::Refused(refused) => return Err(refusal(refused)),
        };
        let engine = context.engine();
        let origin = Origin {
            actor: Actor::Person { name: by },
            surface: Some(context.surface()),
            layer: None,
            run_id: Some(run.id().clone()),
        };
        engine
            .audit()
            .record(
                &origin,
                &EFFECT_RECONCILED,
                json!({
                    "run_id": run.id(),
                    "entry": effect.entry,
                    "key": effect.key,
                    "crawl_request": effect.crawl_request,
                    "answer": answer.as_str(),
                }),
                engine.now(),
            )
            .await
            .map_err(|error| unrecorded(&error))?;
        Ok(Reconciled::Answered(effect))
    }
}

/// The operations `effect.open` and `effect.close`, on `effects`.
pub(crate) fn handlers(effects: &Effects) -> [Handler; 2] {
    let opening = effects.clone();
    let closing = effects.clone();
    [
        Handler::new(
            &goodfetch_protocol::effect::EFFECT_OPEN,
            move |params: EffectOpenParams, context: CallContext| {
                let effects = opening.clone();
                async move { effects.open(params, &context).await }
            },
        ),
        Handler::new(
            &goodfetch_protocol::effect::EFFECT_CLOSE,
            move |params: EffectCloseParams, context: CallContext| {
                let effects = closing.clone();
                async move { effects.close(params, &context).await }
            },
        ),
    ]
}

impl Shelf {
    /// Its book once no other body of `key` in `namespace` runs: waits for
    /// a live owner's body to be judged, and takes out a body whose owner
    /// is gone, whose row the declaration then judges as cut short.
    ///
    /// # Errors
    ///
    /// `effect_key_reused` when `owner` declared the key already, and the
    /// call's cancellation.
    async fn free_key(
        &self,
        run: &Run,
        namespace: &str,
        key: &str,
        owner: &Owner,
        context: &CallContext,
    ) -> Result<MutexGuard<'_, Book>, CallError> {
        let reused = || {
            CallError::new(
                &EFFECT_KEY_REUSED,
                format!(
                    "The Effect key {key:?} was declared again while its body runs or after its answer: put what varies in the key."
                ),
                &json!({ "key": key }),
            )
        };
        loop {
            let mut book = self.book.lock().await;
            book.answered.retain(|answered| answered.owner.is_alive());
            if book.answered.iter().any(|answered| {
                answered.namespace == namespace && answered.key == key && answered.owner.is(owner)
            }) {
                return Err(reused());
            }
            let Some(at) = book
                .bodies
                .iter()
                .position(|body| body.namespace == namespace && body.key == key)
            else {
                return Ok(book);
            };
            let running = &book.bodies[at];
            if running.owner.is(owner) {
                return Err(reused());
            }
            if !running.owner.is_alive() {
                let gone = book.bodies.swap_remove(at);
                gone.owner.end_effect();
                self.judged.notify_waiters();
                return Ok(book);
            }
            let other = running.owner.client.clone();
            let judged = self.judged.notified();
            tokio::pin!(judged);
            judged.as_mut().enable();
            drop(book);
            let clock = run.clock();
            tokio::select! {
                () = judged => {}
                () = other.gone() => {}
                () = clock.sleep(HOLD_CHECK) => {}
                () = context.cancellation().cancelled() => return Err(cancelled()),
                () = context.client().gone() => return Err(cancelled()),
            }
        }
    }
}

impl Body {
    /// The uncertain Effect it leaves as `entry` when `event` ends it
    /// after something was sent, and why.
    fn uncertain(&self, entry: &str, event: EffectEvent) -> (UncertainEffect, UncertainReason) {
        let reason = if event == EffectEvent::Interrupt {
            UncertainReason::Interrupted
        } else {
            UncertainReason::FailedAfterSend
        };
        let (first_command, since) = self
            .sent
            .clone()
            .expect("invariant: only a body something was sent for turns uncertain");
        let effect = UncertainEffect {
            entry: entry.to_owned(),
            key: Some(self.key.clone()),
            instruction: None,
            crawl_request: (!self.namespace.is_empty()).then(|| self.namespace.clone()),
            reason: reason.as_str().to_owned(),
            first_command: Some(first_command),
            since,
        };
        (effect, reason)
    }
}

/// `effect_key_invalid` unless `key` holds 1 to 255 Unicode code points.
fn checked_key(key: &str) -> Result<(), CallError> {
    let length = key.chars().count();
    if length == 0 || length > KEY_MAX_CODE_POINTS {
        return Err(CallError::new(
            &EFFECT_KEY_INVALID,
            format!(
                "The Effect key {key:?} holds {length} Unicode code points: a key holds 1 to {KEY_MAX_CODE_POINTS}."
            ),
            &json!({ "key": key }),
        ));
    }
    Ok(())
}

/// The journal's answer to a declaration of `key` in `namespace` of
/// `run`, its entry written when the body is to run.
async fn declared(
    run: &Run,
    namespace: &str,
    key: &str,
    repeatable: bool,
    context: &CallContext,
) -> Result<Declared, CallError> {
    let mut candidate = [0_u8; SECRET_BYTES];
    getrandom::fill(&mut candidate).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the operating system gave no randomness for the Run's Effect secret: {error}"),
        )
    })?;
    let (namespace, key) = (namespace.to_owned(), key.to_owned());
    run.state()
        .write_deferred(move |connection| {
            declare(connection, &namespace, &key, repeatable, &candidate)
        })
        .await
        .map_err(|error| unwritten(run, &error, context))
}

/// The answer to a declaration of `key` the journal answers itself.
fn journal_answer(declared: Declared, key: &str) -> Result<EffectOpened, CallError> {
    match declared {
        Declared::Journal {
            output,
            performed_at,
        } => output
            .map(|output| serde_json::from_str::<Value>(&output))
            .transpose()
            .map(|output| EffectOpened::Journal {
                output,
                performed_at,
            })
            .map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InternalError,
                    format!("the Effect journal's output of {key:?} is no JSON: {error}"),
                )
            }),
        Declared::Reconciled { by, at } => Ok(EffectOpened::Reconciled {
            reconciled_by: by,
            reconciled_at: at,
        }),
        Declared::Uncertain(effect) => Err(uncertain(&effect)),
        Declared::Refused(refused) => Err(refusal(refused)),
        Declared::Perform { entry, .. } => Err(CallError::protocol(
            ProtocolFailure::InternalError,
            format!(
                "the Effect journal wrote the entry `{entry}` of {key:?} with no body to run it"
            ),
        )),
    }
}

/// The table's event for a body that ended with `outcome`, the output the
/// journal keeps, and the size of a returned value over `limit`.
fn ended(outcome: EffectOutcome, limit: u64) -> (EffectEvent, Option<Value>, Option<u64>) {
    match outcome {
        EffectOutcome::Returned { output } => {
            let size = output.as_ref().map_or(0, |output| output.to_string().len());
            let size = u64::try_from(size).unwrap_or(u64::MAX);
            if size > limit {
                (EffectEvent::Fail, None, Some(size))
            } else {
                (EffectEvent::Return, output, None)
            }
        }
        EffectOutcome::Threw => (EffectEvent::Fail, None, None),
        EffectOutcome::CutShort => (EffectEvent::Interrupt, None, None),
    }
}

/// The answer to `effect.close` of `entry`, whose body no longer runs: the
/// journal's judgment when it is done or uncertain, as for a close
/// answered again.
async fn closed_before(
    run: &Run,
    entry: &str,
    number: i64,
    context: &CallContext,
) -> Result<EffectClosed, CallError> {
    let found = run
        .state()
        .write_deferred(move |connection| row(connection, number))
        .await
        .map_err(|error| unwritten(run, &error, context))?;
    match found {
        Some(row) if row.state == "done" => Ok(EffectClosed::Done {
            performed_at: row
                .performed_at
                .expect("invariant: the journal's CHECK keeps a done entry's time"),
        }),
        Some(row) if row.state == "uncertain" => Ok(EffectClosed::Uncertain {
            effect: row.uncertain(),
            cause: None,
        }),
        Some(_) | None => Err(not_running(entry)),
    }
}

/// `invalid_params` for an `effect.close` of `entry`, which has no body
/// running.
fn not_running(entry: &str) -> CallError {
    CallError::protocol(
        ProtocolFailure::InvalidParams,
        format!(
            "The Effect journal entry `{entry}` has no body running: `effect.close` closes the entry `effect.open` answered with `perform`, once."
        ),
    )
}

/// `effect_output_invalid` for the body of `key`, which returned `size`
/// bytes of JSON, over `limit`, with nothing sent.
fn output_invalid(key: &str, size: u64, limit: u64) -> CallError {
    CallError::new(
        &EFFECT_OUTPUT_INVALID,
        format!(
            "The body of the Effect {key:?} returned {size} bytes of JSON, more than `effect.output.max_size`, {limit}: the entry is dropped."
        ),
        &json!({ "key": key, "size": size, "limit": limit }),
    )
}

/// The live Run `run_id`.
async fn live(run_id: &RunId, context: &CallContext) -> Result<Run, CallError> {
    context
        .runs()
        .live(run_id.clone())
        .await
        .map_err(|error| match error {
            RunError::NotFound(_) => CallError::new(
                &RUN_NOT_FOUND,
                format!("No live Run has the Run ID `{run_id}` here: start or resume it to declare an Effect."),
                &RunDetails {
                    run_id: run_id.clone(),
                    path: None,
                },
            ),
            other => run_error(other, context),
        })
}

/// Where `run`'s crawl attempts stage; `invalid_params` on a Run with no
/// crawl, which has no attempt.
fn attempt_writes_of(run: &Run) -> Result<Arc<dyn AttemptWrites>, CallError> {
    run.attempt_writes().ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "The Run `{}` is no crawl, so an Effect it declares takes no `attempt`: declare it without one.",
                run.id()
            ),
        )
    })
}

fn unstaged(unstaged: Unstaged) -> CallError {
    CallError::new(unstaged.code, unstaged.message, &unstaged.details)
}

/// The failure of a write to `run`'s state.
fn unwritten(run: &Run, error: &RunStateError, context: &CallContext) -> CallError {
    run_error(
        RunError::Unavailable {
            run_id: run.id().clone(),
            folder: run.folder().to_owned(),
            error: error.to_string(),
        },
        context,
    )
}

/// `effect_uncertain` for `effect`.
fn uncertain(effect: &UncertainEffect) -> CallError {
    CallError::new(
        &EFFECT_UNCERTAIN,
        format!(
            "The Effect {:?} may or may not have happened ({}): it is never run again until a person reconciles it.",
            effect.key.as_deref().unwrap_or_default(),
            effect.reason
        ),
        effect,
    )
}

/// The call's failure for a transition the table refused: a bug in
/// Goodfetch, logged.
fn refusal(refused: InvalidEffectTransition) -> CallError {
    let message = refused.to_string();
    let _ = refused.logged();
    CallError::protocol(
        ProtocolFailure::InternalError,
        format!("the Effect journal refused a transition: {message}"),
    )
}

fn cancelled() -> CallError {
    CallError::new(
        &CALL_CANCELLED,
        "The call was cancelled by its Client.",
        &CallCancelledDetails {
            by: CancelledBy::Client,
            closed: None,
            person: None,
        },
    )
}

/// The failure of an answer the Audit log could not record.
fn unrecorded(error: &AuditError) -> CallError {
    CallError::protocol(
        ProtocolFailure::InternalError,
        format!("the answer was made, but the Audit log cannot record it: {error}"),
    )
}
