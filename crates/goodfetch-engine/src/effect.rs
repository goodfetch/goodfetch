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

pub mod lifecycle;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, StoppedBy};
use goodfetch_protocol::effect::{
    EFFECT_KEY_INVALID, EFFECT_KEY_REUSED, EFFECT_OUTPUT_INVALID, EFFECT_OUTPUT_MAX_SIZE,
    EFFECT_RECONCILED, EFFECT_UNCERTAIN, EffectAttempt, EffectCloseParams, EffectClosed,
    EffectOpenParams, EffectOpened, EffectOutcome, KEY_MAX_CODE_POINTS, UncertainReason,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::{
    PointInTime, RUN_NOT_FOUND, RunDetails, RunId, RunReport, UncertainEffect,
};
use hkdf::Hkdf;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::sync::{Mutex, Notify};

use self::lifecycle::{EffectEvent, EffectState, InvalidEffectTransition, Next, apply};
use crate::audit::{AuditError, Origin};
use crate::call_error::CallError;
use crate::client::Client;
use crate::composition::{CallContext, Handler};
use crate::mask::Scope;
use crate::run::run_error;
use crate::run_state::{RunStateError, RunStateTables, Syncing};
use crate::runs::{ReportPart, Run, RunError};
use crate::settings::{RunSettings, get_or_default};
use crate::staging::{AttemptWrites, Unstaged};

/// The Effect journal's tables in every run file: an entry per Effect key
/// of a namespace, the crawl request of a handler's attempt or `""` for
/// the Run, and the Run's secret its idempotency keys derive from.
pub const TABLES: RunStateTables = RunStateTables {
    name: "effect-journal",
    statements: &[
        "CREATE TABLE IF NOT EXISTS effect_journal (entry INTEGER PRIMARY KEY AUTOINCREMENT, \
         namespace TEXT NOT NULL, key TEXT NOT NULL, repeatable INTEGER NOT NULL, \
         state TEXT NOT NULL CHECK (state IN ('open', 'sent', 'done', 'uncertain', 'reconciled')), \
         output TEXT, reason TEXT, first_command TEXT, since TEXT, performed_at TEXT, \
         reconciled_by TEXT, reconciled_at TEXT, \
         UNIQUE (namespace, key), \
         CHECK (state NOT IN ('sent', 'uncertain') OR since IS NOT NULL), \
         CHECK (state <> 'done' OR performed_at IS NOT NULL), \
         CHECK (state <> 'uncertain' OR reason IS NOT NULL), \
         CHECK (state <> 'reconciled' OR (reconciled_by IS NOT NULL AND reconciled_at IS NOT NULL))) STRICT",
        "CREATE TABLE IF NOT EXISTS effect_secret (id INTEGER PRIMARY KEY CHECK (id = 1), secret BLOB NOT NULL) STRICT",
    ],
};

/// The bytes of the Run's secret and of an idempotency key.
const SECRET_BYTES: usize = 32;

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
    sent: Option<(String, PointInTime)>,
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
        let shelf = self.shelf(&run_id).await;
        let mut book = loop {
            let mut book = shelf.book.lock().await;
            book.answered.retain(|answered| answered.owner.is_alive());
            let reused = || {
                CallError::new(
                    &EFFECT_KEY_REUSED,
                    format!(
                        "The Effect key {key:?} was declared again while its body runs or after its answer: put what varies in the key."
                    ),
                    &json!({ "key": key }),
                )
            };
            if book.answered.iter().any(|answered| {
                answered.namespace == namespace && answered.key == key && answered.owner.is(&owner)
            }) {
                return Err(reused());
            }
            let Some(at) = book
                .bodies
                .iter()
                .position(|body| body.namespace == namespace && body.key == key)
            else {
                break book;
            };
            let running = &book.bodies[at];
            if running.owner.is(&owner) {
                return Err(reused());
            }
            if !running.owner.is_alive() {
                // Its row is judged as cut short below.
                let gone = book.bodies.swap_remove(at);
                gone.owner.end_effect();
                shelf.judged.notify_waiters();
                break book;
            }
            let other = running.owner.client.clone();
            let judged = shelf.judged.notified();
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
        };
        if let Some(attempt) = &owner.attempt {
            attempt
                .writes
                .begin_effect(&attempt.id.crawl_request_id, attempt.id.lease_epoch)
                .map_err(unstaged)?;
        }
        let mut candidate = [0_u8; SECRET_BYTES];
        let declared = match getrandom::fill(&mut candidate) {
            Ok(()) => {
                let (namespace, key) = (namespace.clone(), key.clone());
                run.state()
                    .write_deferred(move |connection| {
                        declare(connection, &namespace, &key, repeatable, &candidate)
                    })
                    .await
                    .map_err(|error| unwritten(&run, &error, context))
            }
            Err(error) => Err(CallError::protocol(
                ProtocolFailure::InternalError,
                format!(
                    "the operating system gave no randomness for the Run's Effect secret: {error}"
                ),
            )),
        };
        let answer = match declared {
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
            Ok(Declared::Journal {
                output,
                performed_at,
            }) => output
                .map(|output| serde_json::from_str::<Value>(&output))
                .transpose()
                .map(|output| EffectOpened::Journal {
                    output,
                    performed_at: PointInTime::from_rfc3339(performed_at),
                })
                .map_err(|error| {
                    CallError::protocol(
                        ProtocolFailure::InternalError,
                        format!("the Effect journal's output of {key:?} is no JSON: {error}"),
                    )
                }),
            Ok(Declared::Reconciled { by, at }) => Ok(EffectOpened::Reconciled {
                reconciled_by: by,
                reconciled_at: PointInTime::from_rfc3339(at),
            }),
            Ok(Declared::Uncertain(effect)) => Err(uncertain(&effect)),
            Ok(Declared::Refused(refused)) => Err(refusal(refused)),
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
        let not_running = || {
            CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "The Effect journal entry `{entry}` has no body running: `effect.close` closes the entry `effect.open` answered with `perform`, once."
                ),
            )
        };
        let Ok(number) = entry.parse::<i64>() else {
            return Err(not_running());
        };
        let run = live(&run_id, context).await?;
        let shelf = self.shelf(&run_id).await;
        let mut book = shelf.book.lock().await;
        let Some(at) = book.bodies.iter().position(|body| body.entry == number) else {
            drop(book);
            let found = run
                .state()
                .write_deferred(move |connection| row(connection, number))
                .await
                .map_err(|error| unwritten(&run, &error, context))?;
            return match found {
                Some(row) if row.state == "done" => Ok(EffectClosed::Done {
                    performed_at: PointInTime::from_rfc3339(row.performed_at.unwrap_or_default()),
                }),
                Some(row) if row.state == "uncertain" => Ok(EffectClosed::Uncertain {
                    effect: row.uncertain(),
                    cause: None,
                }),
                Some(_) | None => Err(not_running()),
            };
        };
        let limit: u64 = get_or_default(
            run.settings().map(RunSettings::resolved),
            &EFFECT_OUTPUT_MAX_SIZE,
        );
        let mut oversized = None;
        let (event, output) = match outcome {
            EffectOutcome::Returned { output } => {
                let size = output.as_ref().map_or(0, |output| output.to_string().len());
                let size = u64::try_from(size).unwrap_or(u64::MAX);
                if size > limit {
                    oversized = Some(size);
                    (EffectEvent::Fail, None)
                } else {
                    (EffectEvent::Return, output)
                }
            }
            EffectOutcome::Threw => (EffectEvent::Fail, None),
            EffectOutcome::CutShort => (EffectEvent::Interrupt, None),
        };
        let body = book.bodies.swap_remove(at);
        let judged = match apply(body.state(), event) {
            Ok(Next::To(EffectState::Done)) => {
                let performed_at = run.point_in_time();
                let output =
                    output.map(|output| run.secrets().mask().masks().json(&output, Scope::Kept));
                let at = performed_at.as_str().to_owned();
                run.state()
                    .write(move |connection| done(connection, number, output.as_deref(), &at))
                    .await
                    .map(|()| EffectClosed::Done { performed_at })
                    .map_err(|error| unwritten(&run, &error, context))
            }
            Ok(Next::Dropped) => {
                let sent = body.sent.is_some();
                let dropped = run
                    .state()
                    .write_deferred(move |connection| drop_entry(connection, number))
                    .await
                    .map_err(|error| unwritten(&run, &error, context));
                match (dropped, oversized) {
                    (Err(error), _) => Err(error),
                    (Ok(()), Some(size)) if !sent => Err(CallError::new(
                        &EFFECT_OUTPUT_INVALID,
                        format!(
                            "The body of the Effect {:?} returned {size} bytes of JSON, more than `effect.output.max_size`, {limit}: the entry is dropped.",
                            body.key
                        ),
                        &json!({ "key": body.key, "size": size, "limit": limit }),
                    )),
                    (Ok(()), _) => Ok(EffectClosed::Dropped { sent }),
                }
            }
            Ok(Next::To(EffectState::Uncertain)) => {
                let reason = if event == EffectEvent::Interrupt {
                    UncertainReason::Interrupted
                } else {
                    UncertainReason::FailedAfterSend
                };
                let (first_command, since) = body
                    .sent
                    .clone()
                    .expect("invariant: only a body something was sent for turns uncertain");
                let effect = UncertainEffect {
                    entry: entry.clone(),
                    key: Some(body.key.clone()),
                    instruction: None,
                    crawl_request: (!body.namespace.is_empty()).then(|| body.namespace.clone()),
                    reason: reason.as_str().to_owned(),
                    first_command: Some(first_command),
                    since,
                };
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
        shelf.judged.notify_waiters();
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
        let Some(shelf) = self.shelf_of(run.id()).await else {
            return Ok(());
        };
        let mut book = shelf.book.lock().await;
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
        let (first_command, at) = (command.to_owned(), since.as_str().to_owned());
        run.state()
            .write_with(Syncing::Sent, move |connection| {
                mark_sent(connection, &entries, &first_command, &at)
            })
            .await?;
        for at in marked {
            book.bodies[at].sent = Some((command.to_owned(), since.clone()));
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
        let shelf = self.shelf(run.id()).await;
        let book = shelf.book.lock().await;
        if book.bodies.iter().any(|body| body.entry == number) {
            return Ok(Reconciled::NotUncertain);
        }
        let by = context.person();
        let at = run.point_in_time().as_str().to_owned();
        let answered = {
            let by = by.clone();
            run.state()
                .write(move |connection| answer_about(connection, number, answer, &by, &at))
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

/// The Run report's uncertain Effects (ADR 0106): each entry left
/// `uncertain`, and each left `sent` by a body that is not repeatable and
/// whose engine is gone, which its next declaration judges uncertain.
pub struct Report;

impl ReportPart for Report {
    fn fill(&self, connection: &Connection, report: &mut RunReport) -> rusqlite::Result<()> {
        let mut statement = connection.prepare_cached(&format!(
            "SELECT {ROW} FROM effect_journal \
             WHERE state = 'uncertain' OR (state = 'sent' AND repeatable = 0) ORDER BY entry"
        ))?;
        let found = statement
            .query_map([], Row::read)?
            .map(|row| row.map(|row| row.uncertain()))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !found.is_empty() {
            report
                .uncertain_effects
                .get_or_insert_with(Vec::new)
                .extend(found);
        }
        Ok(())
    }
}

/// What a declaration's write found.
enum Declared {
    /// The key's entry was written: run the body.
    Perform {
        entry: i64,
        /// The Run's secret.
        secret: Vec<u8>,
    },
    /// A body returned this output, at this point in time.
    Journal {
        output: Option<String>,
        performed_at: String,
    },
    /// A person answered "it happened".
    Reconciled { by: String, at: String },
    /// The Effect is uncertain.
    Uncertain(UncertainEffect),
    /// The transition table refused the declaration.
    Refused(InvalidEffectTransition),
}

/// What a person's answer found.
enum Answering {
    Answered(UncertainEffect),
    NotUncertain,
    Refused(InvalidEffectTransition),
}

/// The columns [`Row::read`] reads.
const ROW: &str = "entry, namespace, key, repeatable, state, output, reason, first_command, since, \
                   performed_at, reconciled_by, reconciled_at";

/// An entry of the Effect journal.
struct Row {
    entry: i64,
    namespace: String,
    key: String,
    repeatable: bool,
    state: String,
    output: Option<String>,
    reason: Option<String>,
    first_command: Option<String>,
    since: Option<String>,
    performed_at: Option<String>,
    reconciled_by: Option<String>,
    reconciled_at: Option<String>,
}

impl Row {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            entry: row.get(0)?,
            namespace: row.get(1)?,
            key: row.get(2)?,
            repeatable: row.get(3)?,
            state: row.get(4)?,
            output: row.get(5)?,
            reason: row.get(6)?,
            first_command: row.get(7)?,
            since: row.get(8)?,
            performed_at: row.get(9)?,
            reconciled_by: row.get(10)?,
            reconciled_at: row.get(11)?,
        })
    }

    /// Its state in the transition table.
    fn effect_state(&self) -> rusqlite::Result<EffectState> {
        EffectState::of(&self.state, self.repeatable).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                Type::Text,
                format!("`{}` is no state of the Effect journal", self.state).into(),
            )
        })
    }

    /// The uncertain Effect it is, as the Run report lists it: a `sent`
    /// entry is one whose body was cut short.
    fn uncertain(&self) -> UncertainEffect {
        UncertainEffect {
            entry: self.entry.to_string(),
            key: Some(self.key.clone()),
            instruction: None,
            crawl_request: (!self.namespace.is_empty()).then(|| self.namespace.clone()),
            reason: self
                .reason
                .clone()
                .unwrap_or_else(|| UncertainReason::Interrupted.as_str().to_owned()),
            first_command: self.first_command.clone(),
            since: PointInTime::from_rfc3339(self.since.clone().unwrap_or_default()),
        }
    }
}

/// The entry `entry`, if the journal holds it.
fn row(connection: &Connection, entry: i64) -> rusqlite::Result<Option<Row>> {
    connection
        .prepare_cached(&format!(
            "SELECT {ROW} FROM effect_journal WHERE entry = ?1"
        ))?
        .query_row([entry], Row::read)
        .optional()
}

/// The entry of `key` in `namespace`, if the journal holds it.
fn keyed(connection: &Connection, namespace: &str, key: &str) -> rusqlite::Result<Option<Row>> {
    connection
        .prepare_cached(&format!(
            "SELECT {ROW} FROM effect_journal WHERE namespace = ?1 AND key = ?2"
        ))?
        .query_row((namespace, key), Row::read)
        .optional()
}

/// Judges `row`, left `open` or `sent` by a body no engine runs, as cut
/// short, and answers its state after: the entry `uncertain`, or absent.
/// Any other entry's state is answered unchanged.
fn settled(connection: &Connection, row: &Row) -> rusqlite::Result<EffectState> {
    let state = row.effect_state()?;
    if !matches!(
        state,
        EffectState::Open
            | EffectState::OpenRepeatable
            | EffectState::Sent
            | EffectState::SentRepeatable
    ) {
        return Ok(state);
    }
    match apply(state, EffectEvent::Interrupt) {
        Ok(Next::To(EffectState::Uncertain)) => {
            turn_uncertain(connection, row.entry, UncertainReason::Interrupted)?;
            Ok(EffectState::Uncertain)
        }
        Ok(Next::Dropped) => {
            drop_entry(connection, row.entry)?;
            Ok(EffectState::Absent)
        }
        Ok(Next::To(other)) => Ok(other),
        Ok(Next::Unchanged) | Err(_) => Ok(state),
    }
}

/// Declares `key` in `namespace`: judges an entry left by a body no engine
/// runs, then writes the entry, or answers the journal's answer.
fn declare(
    connection: &Connection,
    namespace: &str,
    key: &str,
    repeatable: bool,
    candidate: &[u8; SECRET_BYTES],
) -> rusqlite::Result<Declared> {
    let found = keyed(connection, namespace, key)?;
    let state = match &found {
        Some(row) => settled(connection, row)?,
        None => EffectState::Absent,
    };
    let event = if repeatable {
        EffectEvent::DeclareRepeatable
    } else {
        EffectEvent::Declare
    };
    match apply(state, event) {
        Ok(Next::To(_)) => {
            let entry = connection
                .prepare_cached(
                    "INSERT INTO effect_journal (namespace, key, repeatable, state) \
                     VALUES (?1, ?2, ?3, 'open') RETURNING entry",
                )?
                .query_row((namespace, key, repeatable), |row| row.get(0))?;
            connection
                .prepare_cached(
                    "INSERT INTO effect_secret (id, secret) VALUES (1, ?1) ON CONFLICT (id) DO NOTHING",
                )?
                .execute([candidate.as_slice()])?;
            let secret = connection
                .prepare_cached("SELECT secret FROM effect_secret WHERE id = 1")?
                .query_row([], |row| row.get(0))?;
            Ok(Declared::Perform { entry, secret })
        }
        Ok(Next::Unchanged | Next::Dropped) => {
            let row = found.expect("invariant: only an entry the journal holds answers");
            Ok(match state {
                EffectState::Reconciled => Declared::Reconciled {
                    by: row.reconciled_by.unwrap_or_default(),
                    at: row.reconciled_at.unwrap_or_default(),
                },
                _ => Declared::Journal {
                    output: row.output,
                    performed_at: row.performed_at.unwrap_or_default(),
                },
            })
        }
        Err(_) if state == EffectState::Uncertain => {
            let row = keyed(connection, namespace, key)?
                .expect("invariant: an uncertain entry is in the journal");
            Ok(Declared::Uncertain(row.uncertain()))
        }
        Err(refused) => Ok(Declared::Refused(refused)),
    }
}

/// Marks each of `entries` sent, before `first_command` at `since`.
fn mark_sent(
    connection: &Connection,
    entries: &[i64],
    first_command: &str,
    since: &str,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(
        "UPDATE effect_journal SET state = 'sent', first_command = ?2, since = ?3 \
         WHERE entry = ?1 AND state = 'open'",
    )?;
    for entry in entries {
        statement.execute((entry, first_command, since))?;
    }
    Ok(())
}

/// Keeps `output` as what the body of `entry` returned, performed at
/// `performed_at`.
fn done(
    connection: &Connection,
    entry: i64,
    output: Option<&str>,
    performed_at: &str,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "UPDATE effect_journal SET state = 'done', output = ?2, performed_at = ?3 WHERE entry = ?1",
        )?
        .execute((entry, output, performed_at))?;
    Ok(())
}

/// Leaves `entry` uncertain for `reason`.
fn turn_uncertain(
    connection: &Connection,
    entry: i64,
    reason: UncertainReason,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "UPDATE effect_journal SET state = 'uncertain', reason = ?2 WHERE entry = ?1",
        )?
        .execute((entry, reason.as_str()))?;
    Ok(())
}

/// Takes `entry` out of the journal.
fn drop_entry(connection: &Connection, entry: i64) -> rusqlite::Result<()> {
    connection
        .prepare_cached("DELETE FROM effect_journal WHERE entry = ?1")?
        .execute([entry])?;
    Ok(())
}

/// Applies a person's `answer` to `entry`, given `by` at `at`.
fn answer_about(
    connection: &Connection,
    entry: i64,
    answer: Reconciliation,
    by: &str,
    at: &str,
) -> rusqlite::Result<Answering> {
    let Some(found) = row(connection, entry)? else {
        return Ok(Answering::NotUncertain);
    };
    if settled(connection, &found)? != EffectState::Uncertain {
        return Ok(Answering::NotUncertain);
    }
    let found = row(connection, entry)?.expect("invariant: an uncertain entry is in the journal");
    let effect = found.uncertain();
    match apply(EffectState::Uncertain, answer.event()) {
        Ok(Next::To(EffectState::Reconciled)) => {
            connection
                .prepare_cached(
                    "UPDATE effect_journal SET state = 'reconciled', reconciled_by = ?2, reconciled_at = ?3 \
                     WHERE entry = ?1",
                )?
                .execute((entry, by, at))?;
        }
        Ok(Next::Dropped) => drop_entry(connection, entry)?,
        Ok(Next::To(_) | Next::Unchanged) => {}
        Err(refused) => return Ok(Answering::Refused(refused)),
    }
    Ok(Answering::Answered(effect))
}

/// The idempotency key of `key` in `namespace`: HKDF-SHA256 of the Run's
/// secret, so it reveals neither the Run nor the key.
fn idempotency_key(secret: &[u8], namespace: &str, key: &str) -> String {
    let mut info = b"goodfetch effect idempotency key\0".to_vec();
    info.extend_from_slice(namespace.as_bytes());
    info.push(0);
    info.extend_from_slice(key.as_bytes());
    let mut okm = [0_u8; SECRET_BYTES];
    Hkdf::<Sha256>::new(None, secret)
        .expand(&info, &mut okm)
        .expect("invariant: 32 bytes is a valid HKDF-SHA256 output length");
    data_encoding::BASE64URL_NOPAD.encode(&okm)
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
            by: StoppedBy::Client,
            closed: None,
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
