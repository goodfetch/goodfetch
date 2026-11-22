//! The Record stage of an attempt (ADR 0104): once its handler answered,
//! each Record the attempt is about to commit, those it pushed in order and
//! then those its answer gives, goes to each `before_record_commit` Hook in
//! code its Label and Dataset reach, in the order the crawl declared them,
//! one `crawl_hook.handle` call at a time, under the attempt's lease. Each
//! call waits under `timeout.hook`, its Label's before the Run's, the lease
//! renewed before it for that wait plus the lease's margin; past it
//! the call is cancelled and the attempt fails with `deadline_exceeded`,
//! which names the setting and the stage. A Hook that raises fails the
//! attempt with `hook_failed`, which names the stage, the Hook and the
//! Label: the program's own exception as the program answered it, any other
//! code wrapped. A failed attempt commits none of what it staged, its
//! enqueues, Records and Key-value writes discarded together (ADR 0003),
//! and is retried as its failure says.
//!
//! A Hook reads each Record where the attempt staged it, never copied: what
//! the attempt pushed is set aside while its Hooks run and put back after,
//! so what it stages meanwhile, after its handler answered, is discarded.
//! A Run with no Hook at the Record stage sends no message here.

use goodfetch_protocol::foxhound::crawl::{
    CrawlRequest, CrawlRequestHandled, HANDLER_FAILED, PushedRecord,
};
use goodfetch_protocol::foxhound::hook::{
    CRAWL_HOOK_HANDLE, CrawlHookDeclaration, CrawlHookHandled, CrawlHookStage,
};
use goodfetch_protocol::hook::{HOOK_FAILED, HookFailedDetails};
use goodfetch_protocol::run::RunId;
use serde::Serialize;
use serde_json::{Map, Value};

use super::{Answer, On, given_back};
use crate::delivery::failure::AttemptFailure;
use crate::hooks::Hooks;
use crate::policy::lease_for;
use crate::staging::SetAside;
use goodfetch_engine::client::ClientRequestError;

/// The stage this module runs.
const STAGE: CrawlHookStage = CrawlHookStage::BeforeRecordCommit;

/// What one call of a Record-stage Hook sends, borrowed from the attempt:
/// written as `crawl_hook.handle`'s params are.
#[derive(Serialize)]
struct Calling<'a> {
    run_id: &'a RunId,
    hook: &'a str,
    stage: CrawlHookStage,
    crawl_request: &'a CrawlRequest,
    attempt: u32,
    lease_epoch: u64,
    record: &'a PushedRecord,
}

/// The answer of an attempt whose handler answered `handled`, once each
/// Record it is about to commit went through the Record stage's Hooks of
/// `hooks`: `handled` when every call returned, else how the first that
/// did not ended the attempt.
pub(super) async fn passed(on: &On<'_>, hooks: &Hooks, handled: CrawlRequestHandled) -> Answer {
    let label = on.leased.crawl_request.label.as_str();
    let pushed = on.lease.set_aside();
    let records = pushed
        .iter()
        .flat_map(SetAside::records)
        .chain(&handled.records);
    let mut ended = None;
    'records: for record in records {
        for hook in hooks.on_record(label, &record.dataset) {
            if let Some(answer) = called(on, hooks, hook, record).await {
                ended = Some(answer);
                break 'records;
            }
        }
    }
    if let Some(pushed) = pushed {
        on.lease.put_back(pushed);
    }
    ended.unwrap_or(Answer::Handled(handled))
}

/// One call of `hook` on `record`: `None` once it returned, else how the
/// attempt ends.
async fn called(
    on: &On<'_>,
    hooks: &Hooks,
    hook: &CrawlHookDeclaration,
    record: &PushedRecord,
) -> Option<Answer> {
    let label = on.leased.crawl_request.label.as_str();
    let calling = Calling {
        run_id: on.run.identity.id(),
        hook: &hook.name,
        stage: STAGE,
        crawl_request: &on.leased.crawl_request,
        attempt: on.leased.attempt,
        lease_epoch: on.leased.lease_epoch,
        record,
    };
    let deadline = hooks.deadline(label);
    on.lease
        .renew(on.lease.length().max(lease_for(deadline.duration())));
    let Ok(mut sent) = on
        .context
        .client()
        .send::<CrawlHookHandled>(&CRAWL_HOOK_HANDLE, &calling)
        .await
    else {
        return Some(Answer::ClientGone);
    };
    let clock = on.run.identity.clock();
    tokio::select! {
        biased;
        answer = deadline.within(&clock, STAGE.waited_for(), sent.answer()) => match answer {
            Ok(Ok(CrawlHookHandled {})) => None,
            Ok(Err(ClientRequestError::Failed { code, details, .. })) if given_back(&code, &details) => {
                Some(Answer::Released)
            }
            Ok(Err(ClientRequestError::Failed { code, retryable, message, details })) => {
                Some(Answer::failed(raised(&hook.name, label, (code, retryable), &message, &details)))
            }
            Ok(Err(ClientRequestError::Malformed(message))) => Some(Answer::failed(raised(
                &hook.name,
                label,
                (HOOK_FAILED.name.to_owned(), true),
                &message,
                &Map::new(),
            ))),
            Ok(Err(_)) => Some(Answer::ClientGone),
            Err(exceeded) => {
                sent.cancel().await;
                Some(Answer::failed(AttemptFailure::entered(
                    exceeded.code().to_owned(),
                    exceeded.retryable(),
                    exceeded.message().to_owned(),
                    exceeded.details().cloned().unwrap_or_default(),
                )))
            }
        },
        _ = on.lease.ended() => {
            sent.cancel().await;
            Some(Answer::LeaseEnded)
        }
    }
}

/// The `hook_failed` of the Record-stage Hook `hook` on a request of
/// `label`, whose call failed with `code`, retryable or not, `message` and
/// `details`. The program's own exception, a `hook_failed` or a
/// `handler_failed` as an SDK answers it, keeps what its details say of the
/// exception and the code it wraps, if any; any other code is the code the
/// Hook raised, wrapped, and retried as its own flag says.
fn raised(
    hook: &str,
    label: &str,
    (code, retryable): (String, bool),
    message: &str,
    details: &Map<String, Value>,
) -> AttemptFailure {
    let text = |name: &str| details.get(name).and_then(Value::as_str).map(str::to_owned);
    let own = code == HOOK_FAILED.name || code == HANDLER_FAILED.name;
    let exception_message = text("exception_message").unwrap_or_else(|| message.to_owned());
    let failed = HookFailedDetails {
        stage: STAGE.as_str().to_owned(),
        hook: hook.to_owned(),
        label: Some(label.to_owned()),
        exception_type: text("exception_type").unwrap_or_else(|| {
            if own {
                "Error".to_owned()
            } else {
                code.clone()
            }
        }),
        exception_message: exception_message.clone(),
        exception_stack: text("exception_stack").unwrap_or_default(),
        wrapped_code: if own {
            text("wrapped_code").filter(|_| code == HOOK_FAILED.name)
        } else {
            Some(code)
        },
    };
    let message = format!(
        "The `{}` Hook `{hook}` raised: {exception_message}",
        STAGE.as_str()
    );
    let Ok(Value::Object(details)) = serde_json::to_value(&failed) else {
        unreachable!("invariant: `hook_failed`'s details write as an object")
    };
    AttemptFailure::entered(HOOK_FAILED.name.to_owned(), retryable, message, details)
}

#[cfg(test)]
mod tests;
