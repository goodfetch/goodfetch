//! The Human pause handlers (`page.pause` and `page.pause_answer`, ADRs 0092,
//! 0102 and 0113): `page.pause` puts its page in `Paused` (waiting for any
//! pending navigation or earlier pause first), holds its browser in the pool
//! so it is recycled only after release, registers the pause in
//! `progress().pauses` (which stops enclosing clocks such as `timeout.run`
//! while Limits keep counting), waits until its `until` condition holds, a
//! person answers `page.pause_answer`, its `timeout.human_pause` expires, or
//! the page/browser/Run ends, records how it ended in `trace/pauses.jsonl`
//! and (for a person's answer or a browser close) in the Audit log, and runs
//! the deferred Page check on the page left when the pause ends.
#![expect(
    clippy::disallowed_types,
    reason = "the active pauses map is locked only for one insert, lookup or removal, never across an `.await` or another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use goodfetch_engine::audit::Origin;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::governor::PageHold;
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::audit::{Actor, HUMAN_PAUSE_ENDED};
use goodfetch_protocol::connection::{
    CALL_CANCELLED, CallCancelledDetails, CancelledBy, Form, WaitedFor,
};
use goodfetch_protocol::page::functions::World;
use goodfetch_protocol::page::pause::{
    HUMAN_PAUSE_DECLINED, HumanPauseDeclinedDetails, HumanPauseResult, PAGE_PAUSE,
    PAGE_PAUSE_ANSWER, PagePauseAnswerParams, PagePauseAnswered, PagePauseParams, PauseAnswer,
    PauseResolvedBy, PauseUntil,
};
use goodfetch_protocol::page::waits::{ElementState, LoadState};
use goodfetch_protocol::page::{PageId, SessionId};
use goodfetch_protocol::run::{HumanPause, PauseOrigin};
use goodfetch_protocol::settings::TIMEOUT_HUMAN_PAUSE;
use jiff::Timestamp;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::{Pages, in_call, pattern_of};
use crate::browser_run::BrowserRun;
use crate::cdp::CdpPage;
use crate::guard::{self, Awaited, Wait, Window};
use crate::page_functions::{self, Returned};
use crate::page_reads;
use crate::sessions::{OpenPage, PageEvent};
use crate::url_pattern::UrlPattern;

/// One person's answer sent to a waiting Human pause.
struct AnswerRequest {
    answer: PauseAnswer,
    person: String,
    note: Option<String>,
    reply: oneshot::Sender<PagePauseAnswered>,
}

/// The Human pauses currently waiting on open pages, by Page ID, so
/// `page.pause_answer` can deliver a person's answer directly to the waiting
/// pause. Clones share the table.
#[derive(Clone, Default)]
pub(super) struct ActivePauses(Arc<Mutex<BTreeMap<PageId, mpsc::Sender<AnswerRequest>>>>);

impl ActivePauses {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<PageId, mpsc::Sender<AnswerRequest>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Registers a waiting pause for `page_id` until the returned guard drops.
    fn register(&self, page_id: PageId) -> (mpsc::Receiver<AnswerRequest>, PauseSlot) {
        let (tx, rx) = mpsc::channel(8);
        self.lock().insert(page_id.clone(), tx);
        (
            rx,
            PauseSlot {
                pauses: self.clone(),
                page_id,
            },
        )
    }

    /// Finds the sender for the pause waiting on `page_id`, if any.
    fn sender(&self, page_id: &PageId) -> Option<mpsc::Sender<AnswerRequest>> {
        self.lock().get(page_id).cloned()
    }
}

/// Removes the page's entry from [`ActivePauses`] when the pause ends or is
/// cancelled.
struct PauseSlot {
    pauses: ActivePauses,
    page_id: PageId,
}

impl Drop for PauseSlot {
    fn drop(&mut self) {
        self.pauses.lock().remove(&self.page_id);
    }
}

/// Adds the handlers of `page.pause` and `page.pause_answer`.
pub(super) fn with_pause_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let (pausing, answering) = (pages.clone(), pages.clone());
    contribution
        .with_handler(Handler::new(
            &PAGE_PAUSE,
            move |params: PagePauseParams, context: CallContext| {
                let pages = pausing.clone();
                in_call((PAGE_PAUSE.name, context.form()), async move {
                    Box::pin(pages.pause(params, context)).await
                })
            },
        ))
        .with_handler(Handler::new(
            &PAGE_PAUSE_ANSWER,
            move |params: PagePauseAnswerParams, context: CallContext| {
                let pages = answering.clone();
                async move { pages.answer_pause(params, context).await }
            },
        ))
}

/// Compiled `until` condition for a Human pause.
enum CompiledUntil {
    Visible { selector: String },
    Url { pattern: UrlPattern, text: String },
}

impl CompiledUntil {
    fn compile(until: PauseUntil) -> Result<Self, CallError> {
        match until {
            PauseUntil::Visible { selector } => Ok(Self::Visible { selector }),
            PauseUntil::Url { url } => {
                let text = format!("{url:?}");
                let pattern = pattern_of(&url)?;
                Ok(Self::Url { pattern, text })
            }
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Visible { selector } => format!("visible: {selector}"),
            Self::Url { text, .. } => format!("url: {text}"),
        }
    }

    /// Returns whether this condition currently holds on `page` right now.
    async fn holds_now(&self, page: &OpenPage, form: Form) -> bool {
        match self {
            Self::Visible { selector } => {
                let wait = Wait {
                    operation: PAGE_PAUSE.name,
                    selector,
                    awaited: Awaited::State(ElementState::Visible),
                    window: Some(Window {
                        limit: page.action_timeout.0.clone(),
                        duration: Duration::from_millis(50),
                        spent: Duration::ZERO,
                    }),
                    form,
                    owner: &page.owner,
                };
                guard::wait(&page.driver, wait, &page.clock).await.is_ok()
            }
            Self::Url { pattern, .. } => {
                let cdp = CdpPage::of(&page.driver);
                let expression = page_functions::expression("window.location.href", None);
                let Ok(returned) =
                    page_reads::evaluate(&cdp, &expression, World::Isolated, PAGE_PAUSE.name).await
                else {
                    return false;
                };
                matches!(
                    returned,
                    Returned::Walked(Value::String(ref current)) if pattern.matches(current)
                )
            }
        }
    }

    /// Waits until this condition holds on `page`, returning `Ok(())` once it
    /// does or `Err(CallError)` if the wait cannot continue.
    async fn wait(&self, page: &OpenPage, form: Form) -> Result<(), CallError> {
        match self {
            Self::Visible { selector } => {
                let wait = Wait {
                    operation: PAGE_PAUSE.name,
                    selector,
                    awaited: Awaited::State(ElementState::Visible),
                    window: None,
                    form,
                    owner: &page.owner,
                };
                guard::wait(&page.driver, wait, &page.clock).await?;
                Ok(())
            }
            Self::Url { pattern, .. } => {
                if self.holds_now(page, form).await {
                    return Ok(());
                }
                let events = page.owner.events();
                let deadline = events.wait_deadline(Some(None))?;
                events
                    .wait_for_url(pattern.clone(), LoadState::DomContentLoaded, deadline)
                    .await?;
                Ok(())
            }
        }
    }
}

/// How the inner pause loop ended.
enum PauseOutcome {
    Condition {
        waited: u64,
    },
    Person {
        answer: PauseAnswer,
        person: String,
        waited: u64,
    },
    Declined {
        person: String,
        note: Option<String>,
        waited: u64,
    },
    Timeout {
        error: CallError,
        waited: u64,
    },
}

/// Writes a `cancelled` entry to `trace/pauses.jsonl` if the `page.pause`
/// future is dropped by `$/cancel` before its outcome is recorded.
struct CancelTrace {
    run: Arc<BrowserRun>,
    session: SessionId,
    page_id: PageId,
    reason: Option<String>,
    until: Option<String>,
    started: tokio::time::Instant,
    recorded: bool,
}

impl Drop for CancelTrace {
    fn drop(&mut self) {
        if self.recorded {
            return;
        }
        let waited = millis(self.run.identity.clock().now() - self.started);
        let entry = trace_entry(
            &self.session,
            &self.page_id,
            self.reason.as_deref(),
            self.until.as_deref(),
            "cancelled",
            waited,
            json!({}),
        );
        let trace = self.run.trace.clone();
        self.run.identity.spawn(async move {
            if let Err(error) = trace.pause(entry).await {
                tracing::warn!(%error, "could not record cancelled Human pause in Run trace");
            }
        });
    }
}

impl Pages {
    /// Waits for a person on `params.page_id` (ADRs 0092 and 0113).
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %params.page_id.0, operation = PAGE_PAUSE.name, run_id = tracing::field::Empty))]
    async fn pause(
        &self,
        params: PagePauseParams,
        context: CallContext,
    ) -> Result<HumanPauseResult, CallError> {
        let compiled_until = params.until.map(CompiledUntil::compile).transpose()?;
        let page = self.sessions.page(&params.page_id).await?;
        tracing::Span::current().record("run_id", page.run.identity.id().as_str());
        let deadline = pause_deadline(&page, params.timeout)?;
        let until_text = compiled_until.as_ref().map(CompiledUntil::description);
        let form = context.form();

        if let Some(reason) = &params.reason {
            tracing::info!(
                run_id = %page.run.identity.id().as_str(),
                page_id = %params.page_id.0,
                %reason,
                "Human pause waiting for a person"
            );
        }

        let started = page.clock.now();
        let mut cancel_trace = CancelTrace {
            run: Arc::clone(&page.run),
            session: page.session.clone(),
            page_id: params.page_id.clone(),
            reason: params.reason.clone(),
            until: until_text.clone(),
            started,
            recorded: false,
        };

        let pauses = self.pauses.clone();
        let page_id = params.page_id.clone();
        let reason = params.reason.clone();
        let until_for_progress = until_text.clone();

        let call_result = Box::pin(page.life.call(
            PageEvent::Pause,
            Box::pin(wait_for_resolution(
                &page,
                &pauses,
                &page_id,
                reason,
                until_for_progress,
                compiled_until.as_ref(),
                deadline.as_ref(),
                form,
            )),
        ))
        .await;

        cancel_trace.recorded = true;
        self.finish_pause(
            &page,
            &params.page_id,
            params.reason.as_deref(),
            until_text.as_deref(),
            started,
            call_result,
        )
        .await
    }

    async fn finish_pause(
        &self,
        page: &OpenPage,
        page_id: &PageId,
        reason: Option<&str>,
        until_text: Option<&str>,
        started: tokio::time::Instant,
        call_result: Result<PauseOutcome, CallError>,
    ) -> Result<HumanPauseResult, CallError> {
        match call_result {
            Ok(PauseOutcome::Condition { waited }) => {
                record_trace(
                    page,
                    page_id,
                    reason,
                    until_text,
                    "condition",
                    waited,
                    json!({}),
                )
                .await;
                self.judge_after_pause(page, page_id).await?;
                Ok(HumanPauseResult {
                    resolved_by: PauseResolvedBy::Condition,
                    person: None,
                    waited,
                })
            }
            Ok(PauseOutcome::Person {
                answer,
                person,
                waited,
            }) => {
                let answer_name = match answer {
                    PauseAnswer::Done => "done",
                    PauseAnswer::HandBack => "hand_back",
                    PauseAnswer::Decline => "decline",
                };
                record_trace(
                    page,
                    page_id,
                    reason,
                    until_text,
                    answer_name,
                    waited,
                    json!({ "person": person }),
                )
                .await;
                let audit_details = json!({ "session": page.session.0, "page": page_id.0, "ended": answer_name, "person": person });
                record_audit(
                    page,
                    page_id,
                    Actor::Person {
                        name: person.clone(),
                    },
                    audit_details,
                )
                .await;
                self.judge_after_pause(page, page_id).await?;
                Ok(HumanPauseResult {
                    resolved_by: PauseResolvedBy::Person,
                    person: Some(person),
                    waited,
                })
            }
            Ok(PauseOutcome::Declined {
                person,
                note,
                waited,
            }) => {
                record_trace(
                    page,
                    page_id,
                    reason,
                    until_text,
                    "declined",
                    waited,
                    json!({ "person": person, "note": note }),
                )
                .await;
                let audit_details = json!({ "session": page.session.0, "page": page_id.0, "ended": "declined", "person": person, "note": note });
                record_audit(
                    page,
                    page_id,
                    Actor::Person {
                        name: person.clone(),
                    },
                    audit_details,
                )
                .await;
                Err(declined(Some(person), note))
            }
            Ok(PauseOutcome::Timeout { error, waited }) => {
                record_trace(
                    page,
                    page_id,
                    reason,
                    until_text,
                    "timeout",
                    waited,
                    json!({}),
                )
                .await;
                Err(error)
            }
            Err(error) => {
                let waited = millis(page.clock.now() - started);
                let (ended, extra, browser_closed) = stopped_details(&error);
                record_trace(
                    page,
                    page_id,
                    reason,
                    until_text,
                    ended,
                    waited,
                    extra.clone(),
                )
                .await;
                if browser_closed {
                    let audit_details = json!({ "session": page.session.0, "page": page_id.0, "ended": "stopped", "by": "browser", "closed": extra.get("closed") });
                    record_audit(page, page_id, Actor::Engine, audit_details).await;
                }
                Err(error)
            }
        }
    }

    /// Runs the deferred Page check on the page left after a Human pause ends
    /// (ADR 0092).
    async fn judge_after_pause(&self, page: &OpenPage, page_id: &PageId) -> Result<(), CallError> {
        self.judge_unjudged(page, page_id, Some(PAGE_PAUSE.name))
            .await
    }

    /// Delivers a person's answer (`Done`, `HandBack`, or `Decline`) to the
    /// Human pause waiting on `params.page_id`.
    async fn answer_pause(
        &self,
        params: PagePauseAnswerParams,
        context: CallContext,
    ) -> Result<PagePauseAnswered, CallError> {
        let Some(sender) = self.pauses.sender(&params.page_id) else {
            return Ok(PagePauseAnswered {
                accepted: false,
                reason: Some(format!(
                    "no Human pause is waiting on page `{}`",
                    params.page_id.0
                )),
            });
        };
        let (reply, answered) = oneshot::channel();
        let request = AnswerRequest {
            answer: params.answer,
            person: context.person(),
            note: params.note,
            reply,
        };
        if sender.send(request).await.is_err() {
            return Ok(PagePauseAnswered {
                accepted: false,
                reason: Some("the Human pause already ended".to_owned()),
            });
        }
        Ok(answered.await.unwrap_or_else(|_| PagePauseAnswered {
            accepted: false,
            reason: Some("the Human pause ended before accepting the answer".to_owned()),
        }))
    }
}

/// Holds the page's browser, lease, and progress entry while waiting for
/// `until`, a person's answer, or `deadline`.
#[expect(
    clippy::too_many_arguments,
    reason = "internal helper splitting the pause wait loop out of `Pages::pause`"
)]
async fn wait_for_resolution(
    page: &OpenPage,
    pauses: &ActivePauses,
    page_id: &PageId,
    reason: Option<String>,
    until_for_progress: Option<String>,
    compiled_until: Option<&CompiledUntil>,
    deadline: Option<&Deadline>,
    form: Form,
) -> Result<PauseOutcome, CallError> {
    let _pool_hold = page.hold(PageHold::Held);
    let _lease_hold = page.lease().map(|lease| lease.pause_wait());
    let since = page.run.identity.point_in_time();
    let pause_started = page.clock.now();
    let deadline_at = deadline.map(|limit| since + limit.duration());
    let _progress_held = page.run.progress.pause(HumanPause {
        session: page.session.0.clone(),
        page: page_id.0.clone(),
        reason,
        until: until_for_progress,
        raised_by: PauseOrigin::Program,
        since,
        deadline: deadline_at,
    });
    let (mut answers, _slot) = pauses.register(page_id.clone());

    let mut until_fut = std::pin::pin!(async {
        if let Some(until) = compiled_until {
            until.wait(page, form).await
        } else {
            std::future::pending::<Result<(), CallError>>().await
        }
    });
    let mut timer_fut = std::pin::pin!(async {
        if let Some(limit) = deadline {
            page.clock.sleep(limit.duration()).await;
        } else {
            std::future::pending::<()>().await;
        }
    });

    loop {
        tokio::select! {
            biased;
            matched = &mut until_fut => {
                matched?;
                let waited = millis(page.clock.now() - pause_started);
                return Ok(PauseOutcome::Condition { waited });
            }
            Some(req) = answers.recv() => {
                match req.answer {
                    PauseAnswer::Decline => {
                        let _ = req.reply.send(PagePauseAnswered {
                            accepted: true,
                            reason: None,
                        });
                        let waited = millis(page.clock.now() - pause_started);
                        return Ok(PauseOutcome::Declined {
                            person: req.person,
                            note: req.note,
                            waited,
                        });
                    }
                    PauseAnswer::Done | PauseAnswer::HandBack => {
                        let holds = match compiled_until {
                            Some(until) => until.holds_now(page, form).await,
                            None => true,
                        };
                        if holds {
                            let _ = req.reply.send(PagePauseAnswered {
                                accepted: true,
                                reason: None,
                            });
                            let waited = millis(page.clock.now() - pause_started);
                            return Ok(PauseOutcome::Person {
                                answer: req.answer,
                                person: req.person,
                                waited,
                            });
                        }
                        let _ = req.reply.send(PagePauseAnswered {
                            accepted: false,
                            reason: Some(
                                "the pause's `until` condition does not hold yet".to_owned(),
                            ),
                        });
                    }
                }
            }
            () = &mut timer_fut => {
                let elapsed = page.clock.now() - pause_started;
                let waited = millis(elapsed);
                let limit = deadline
                    .expect("invariant: timer fired only when deadline is Some");
                return Ok(PauseOutcome::Timeout {
                    error: limit.exceeded(WaitedFor::ANSWER, elapsed),
                    waited,
                });
            }
        }
    }
}

/// Resolves the deadline for `page.pause` from the call's `timeout` argument
/// or the Run's `timeout.human_pause` setting (ADR 0080 and 0092).
#[expect(
    clippy::option_option,
    reason = "three states (ADR 0080): absent for `timeout.human_pause`, `null` for none, a value"
)]
fn pause_deadline(
    page: &OpenPage,
    timeout: Option<Option<u64>>,
) -> Result<Option<Deadline>, CallError> {
    match timeout {
        Some(None) => Ok(None),
        Some(Some(ms)) => Deadline::argument(&TIMEOUT_HUMAN_PAUSE, ms).map(Some),
        None => {
            let resolved = page
                .run
                .identity
                .settings()
                .map(goodfetch_engine::settings::RunSettings::resolved);
            let Some(resolved) = resolved else {
                return Ok(Some(Deadline::declared(&TIMEOUT_HUMAN_PAUSE)));
            };
            // `timeout.human_pause` is clearable: `null` means wait without limit.
            if let Some(effective) = resolved.effective(&TIMEOUT_HUMAN_PAUSE) {
                if effective.value.is_null() {
                    return Ok(None);
                }
                return Ok(Deadline::of(resolved, &TIMEOUT_HUMAN_PAUSE));
            }
            Ok(Some(Deadline::declared(&TIMEOUT_HUMAN_PAUSE)))
        }
    }
}

fn declined(person: Option<String>, note: Option<String>) -> CallError {
    let message = match (&person, &note) {
        (Some(who), Some(why)) => {
            format!("Human pause was declined by `{who}`: {why}")
        }
        (Some(who), None) => format!("Human pause was declined by `{who}`."),
        (None, Some(why)) => format!("Human pause was declined: {why}"),
        (None, None) => "Human pause was declined by a person.".to_owned(),
    };
    CallError::new(
        &HUMAN_PAUSE_DECLINED,
        message,
        &HumanPauseDeclinedDetails { person, note },
    )
}

fn stopped_details(error: &CallError) -> (&'static str, Value, bool) {
    if error.code() == CALL_CANCELLED.name
        && let Some(details) = error.details().and_then(|value| {
            serde_json::from_value::<CallCancelledDetails>(Value::Object(value.clone())).ok()
        })
    {
        let browser_closed = details.by == CancelledBy::Browser;
        return (
            "stopped",
            json!({
                "by": details.by,
                "closed": details.closed,
                "person": details.person,
            }),
            browser_closed,
        );
    }
    ("stopped", json!({ "error": error.code() }), false)
}

fn trace_entry(
    session: &SessionId,
    page_id: &PageId,
    reason: Option<&str>,
    until: Option<&str>,
    ended: &str,
    waited: u64,
    extra: Value,
) -> Value {
    let mut object = serde_json::Map::new();
    object.insert("session".to_owned(), json!(session.0));
    object.insert("page".to_owned(), json!(page_id.0));
    if let Some(reason) = reason {
        object.insert("reason".to_owned(), json!(reason));
    }
    if let Some(until) = until {
        object.insert("until".to_owned(), json!(until));
    }
    object.insert("ended".to_owned(), json!(ended));
    object.insert("waited".to_owned(), json!(waited));
    if let Value::Object(fields) = extra {
        for (key, value) in fields {
            if !value.is_null() {
                object.insert(key, value);
            }
        }
    }
    Value::Object(object)
}

async fn record_trace(
    page: &OpenPage,
    page_id: &PageId,
    reason: Option<&str>,
    until: Option<&str>,
    ended: &str,
    waited: u64,
    extra: Value,
) {
    let entry = trace_entry(&page.session, page_id, reason, until, ended, waited, extra);
    if let Err(error) = page.run.trace.pause(entry).await {
        tracing::warn!(%error, "could not record Human pause in Run trace");
    }
}

async fn record_audit(page: &OpenPage, page_id: &PageId, actor: Actor, details: Value) {
    let origin = Origin {
        actor,
        surface: None,
        layer: None,
        run_id: Some(page.run.identity.id().clone()),
    };
    let now =
        Timestamp::try_from(page.run.identity.point_in_time()).unwrap_or(Timestamp::UNIX_EPOCH);
    if let Err(error) = page
        .run
        .engine
        .audit()
        .record(&origin, &HUMAN_PAUSE_ENDED, details, now)
        .await
    {
        tracing::warn!(%error, page_id = %page_id.0, "could not record Human pause end in Audit log");
    }
}

fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}
