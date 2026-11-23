//! One call of a Hook in code on an attempt (ADR 0104), at any stage: one
//! `crawl_hook.handle` request under the attempt's lease, waiting under
//! `timeout.hook`, its Label's before the Run's, the lease renewed before
//! it for that wait plus the lease's margin. Past it the call is cancelled
//! and the attempt fails with `deadline_exceeded`, which names the setting
//! and the stage. A Hook that raises fails the attempt with `hook_failed`,
//! which names the stage, the Hook and the Label: the program's own
//! exception as the program answered it, any other code wrapped. A call the
//! program gives back, answering `call_cancelled` with nothing closed,
//! releases the request, as its handler's would.
//!
//! [`ended_in_stage`] runs a request or page stage's chain, the Record
//! stage's `record_stage`.

use goodfetch_engine::client::ClientRequestError;
use goodfetch_protocol::foxhound::crawl::{CrawlRequest, HANDLER_FAILED, PushedRecord};
use goodfetch_protocol::foxhound::hook::{
    CRAWL_HOOK_HANDLE, CrawlHookDeclaration, CrawlHookHandled, CrawlHookStage,
};
use goodfetch_protocol::hook::{HOOK_FAILED, HookFailedDetails};
use goodfetch_protocol::run::RunId;
use serde::Serialize;
use serde_json::{Map, Value};

use super::{Answer, On, given_back};
use crate::delivery::failure::AttemptFailure;
use crate::policy::lease_for;

/// What one call of a Hook sends, borrowed from the attempt: written as
/// `crawl_hook.handle`'s params are.
#[derive(Serialize)]
struct Calling<'a> {
    run_id: &'a RunId,
    hook: &'a str,
    stage: &'a CrawlHookStage,
    crawl_request: &'a CrawlRequest,
    attempt: u32,
    lease_epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    record: Option<&'a PushedRecord>,
}

/// The attempt through the Hooks of the request or page stage `stage` its
/// request's Label reaches, in the order declared, each after the one
/// before returned: `None` once every one returned, none when no Hook is
/// there, else how the first that did not ended the attempt.
pub(super) async fn ended_in_stage(on: &On<'_>, stage: &CrawlHookStage) -> Option<Answer> {
    let label = on.leased.crawl_request.label.as_str();
    for hook in on.hooks.on_request(stage, label) {
        if let Some(answer) = called(on, hook, None).await {
            return Some(answer);
        }
    }
    None
}

/// One call of `hook`, on `record` at the Record stage: `None` once it
/// returned, else how the attempt ends.
pub(super) async fn called(
    on: &On<'_>,
    hook: &CrawlHookDeclaration,
    record: Option<&PushedRecord>,
) -> Option<Answer> {
    let label = on.leased.crawl_request.label.as_str();
    let stage = &hook.stage;
    let calling = Calling {
        run_id: on.run.identity.id(),
        hook: &hook.name,
        stage,
        crawl_request: &on.leased.crawl_request,
        attempt: on.leased.attempt,
        lease_epoch: on.leased.lease_epoch,
        record,
    };
    let deadline = on.hooks.deadline(label);
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
    let raised = |code, message: &str, details: &Map<String, Value>| {
        Answer::failed(raised(stage, (&hook.name, label), code, message, details))
    };
    tokio::select! {
        biased;
        answer = deadline.within(&clock, stage.waited_for(), sent.answer()) => match answer {
            Ok(Ok(CrawlHookHandled {})) => None,
            Ok(Err(ClientRequestError::Failed { code, details, .. })) if given_back(&code, &details) => {
                Some(Answer::Released)
            }
            Ok(Err(ClientRequestError::Failed { code, retryable, message, details })) => {
                Some(raised((code, retryable), &message, &details))
            }
            Ok(Err(ClientRequestError::Malformed(message))) => Some(raised(
                (HOOK_FAILED.name.to_owned(), true),
                &message,
                &Map::new(),
            )),
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

/// The `hook_failed` of the Hook `hook` at `stage` on a request of
/// `label`, whose call failed with `code`, retryable or not, `message` and
/// `details`. The program's own exception, a `hook_failed` or a
/// `handler_failed` as an SDK answers it, keeps what its details say of the
/// exception and the code it wraps, if any; any other code is the code the
/// Hook raised, wrapped, and retried as its own flag says.
fn raised(
    stage: &CrawlHookStage,
    (hook, label): (&str, &str),
    (code, retryable): (String, bool),
    message: &str,
    details: &Map<String, Value>,
) -> AttemptFailure {
    let text = |name: &str| details.get(name).and_then(Value::as_str).map(str::to_owned);
    let own = code == HOOK_FAILED.name || code == HANDLER_FAILED.name;
    let exception_message = text("exception_message").unwrap_or_else(|| message.to_owned());
    let failed = HookFailedDetails {
        stage: stage.as_str().to_owned(),
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
        stage.as_str()
    );
    let Ok(Value::Object(details)) = serde_json::to_value(&failed) else {
        unreachable!("invariant: `hook_failed`'s details write as an object")
    };
    AttemptFailure::entered(HOOK_FAILED.name.to_owned(), retryable, message, details)
}

#[cfg(test)]
mod tests;
