//! The plugin channel through the driver, on the paused `Clock`, with a
//! scripted `notification.send`: what the call carries, and its
//! translation table.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::run_state::{RunState, RunStateOptions};
use goodfetch_engine::secrets::PartyText;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::notifications::NotificationEvent;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use tokio::sync::Mutex;
use url::Url;

use super::{CallFailure, NotificationCall, Params, PluginChannel};
use crate::driver::{Channel, Driver};
use crate::failure::{ChannelText, Failure};
use crate::ledger::{Entry, Ledger, TABLES};
use crate::message::{EventId, Message};
use crate::rate_limit::RateLimit;

/// A `notification.send` whose program answers as scripted, then takes
/// every call, and which notes what each call carried and when.
struct Scripted {
    clock: Clock,
    answers: Mutex<VecDeque<CallFailure>>,
    calls: Mutex<Vec<Call>>,
}

/// What one call carried, and the second it was made.
#[derive(Debug, PartialEq, Eq)]
struct Call {
    second: u64,
    event_id: String,
    event: String,
    text: String,
    link: Option<String>,
}

impl Scripted {
    fn new(clock: Clock, answers: impl IntoIterator<Item = CallFailure>) -> Self {
        Self {
            clock,
            answers: Mutex::new(answers.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl NotificationCall for Scripted {
    async fn send(&self, params: Params<'_>) -> Result<(), CallFailure> {
        let second = self.clock.wall().duration_since(UNIX_EPOCH).unwrap();
        self.calls.lock().await.push(Call {
            second: second.as_secs(),
            event_id: params.event_id.to_owned(),
            event: params.event.to_string(),
            text: params.text.to_owned(),
            link: params.link.map(Url::to_string),
        });
        self.answers.lock().await.pop_front().map_or(Ok(()), Err)
    }
}

/// A `notification.send` that never answers: the translation table sends
/// nothing.
struct NeverCalled;

impl NotificationCall for NeverCalled {
    async fn send(&self, _: Params<'_>) -> Result<(), CallFailure> {
        std::future::pending().await
    }
}

/// A driver over a Run's run file.
async fn driver(env: &TestEnv, owner: &TaskOwner, clock: Clock) -> Driver {
    let state = RunState::open(
        env.folder().join("run.sqlite"),
        &RunStateOptions::default(),
        vec![TABLES],
        1,
        owner,
    )
    .await
    .unwrap();
    Driver::new(Ledger::new(state), Arc::new(RateLimit::new(clock)), clock)
}

fn message() -> Message {
    Message {
        event_id: EventId::new("evt-1").unwrap(),
        event: NotificationEvent::RUN_FAILED,
        run_id: None,
        schedule_id: None,
        product: Product::Collie,
        title: "A Run failed".to_owned(),
        text: "The Run stopped on an error.".to_owned(),
        link: Some(Url::parse("http://127.0.0.1:7777/runs/nightly").unwrap()),
        occurred_at: UNIX_EPOCH,
    }
}

/// Covers: story:11-269
/// Covers: story:11-270
#[test]
fn a_programs_other_error_is_sent_again_on_the_schedule_with_the_same_event() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let env = TestEnv::in_memory();
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let driver = driver(&env, &owner, clock).await;
        let call = Scripted::new(
            clock,
            [CallFailure::Other(ChannelText::new("TypeError: boom"))],
        );
        let channel = PluginChannel::new("pager", "plugin:pager", call);

        let held = driver.deliver(&channel, &message()).await.unwrap();

        assert_eq!(held, Entry::Delivered { attempt: 1 });
        let carried = |second| Call {
            second,
            event_id: "evt-1".to_owned(),
            event: "run_failed".to_owned(),
            text: "The Run stopped on an error.".to_owned(),
            link: Some("http://127.0.0.1:7777/runs/nightly".to_owned()),
        };
        assert_eq!(*channel.call.calls.lock().await, [carried(0), carried(5)]);
    });
}

/// Covers: story:11-269
#[rstest]
#[case::retryable_with_a_wait(
    CallFailure::Goodfetch { code: "rate_limited".to_owned(), retryable: true, retry_after: Some(Duration::from_secs(60)) },
    Failure::RateLimited { retry_after: Some(Duration::from_secs(60)) },
)]
#[case::retryable(
    CallFailure::Goodfetch { code: "upstream_down".to_owned(), retryable: true, retry_after: None },
    Failure::ServerError,
)]
#[case::not_retryable(
    CallFailure::Goodfetch { code: "bad_channel".to_owned(), retryable: false, retry_after: None },
    Failure::Rejected,
)]
#[case::other(
    CallFailure::Other(ChannelText::new("TypeError: boom")),
    Failure::Unrecognized(PartyText::new("TypeError: boom"))
)]
fn each_answer_maps_to_its_failure(#[case] answer: CallFailure, #[case] failure: Failure) {
    let channel = PluginChannel::new("pager", "plugin:pager", NeverCalled);

    let mapped = channel.map_failure(answer);

    assert_eq!(mapped, failure);
    assert_eq!(mapped.is_transient(), !matches!(failure, Failure::Rejected));
}
