//! The driver through its interface, on the paused `Clock`, with a
//! scripted channel and the Run's run file: retries on the schedule,
//! `Retry-After`, permanent failures, resumes of the Run and the rate limit
//! under concurrent events.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use futures_util::future::join_all;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::run_state::{RunState, RunStateOptions};
use goodfetch_engine::secrets::PartyText;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use tokio::sync::Mutex;

use super::{Channel, Driver};
use crate::failure::Failure;
use crate::ledger::{Entry, Ledger, TABLES};
use crate::message::{EventId, Message};
use crate::rate_limit::RateLimit;

/// What the scripted channel's party answers one send.
#[derive(Debug)]
enum Answer {
    Fail(Failure),
    /// No answer ever: the send is in flight until it is dropped.
    Hang,
}

/// A channel whose party answers as scripted, then takes every message,
/// and which notes the second of each send.
struct Scripted {
    clock: Clock,
    answers: Mutex<VecDeque<Answer>>,
    sent: Mutex<Vec<u64>>,
}

impl Scripted {
    fn new(clock: Clock, answers: impl IntoIterator<Item = Answer>) -> Self {
        Self {
            clock,
            answers: Mutex::new(answers.into_iter().collect()),
            sent: Mutex::new(Vec::new()),
        }
    }

    /// The second, from the start, of each send.
    async fn sent(&self) -> Vec<u64> {
        self.sent.lock().await.clone()
    }
}

impl Channel for Scripted {
    type Failure = Failure;

    fn name(&self) -> &'static str {
        "ops"
    }

    fn destination(&self) -> &'static str {
        "hooks.example"
    }

    async fn send(&self, _: &Message) -> Result<(), Failure> {
        let second = self.clock.wall().duration_since(UNIX_EPOCH).unwrap();
        self.sent.lock().await.push(second.as_secs());
        let answer = self.answers.lock().await.pop_front();
        match answer {
            None => Ok(()),
            Some(Answer::Fail(failure)) => Err(failure),
            Some(Answer::Hang) => std::future::pending().await,
        }
    }

    fn map_failure(&self, failure: Failure) -> Failure {
        failure
    }
}

/// A Run's run file, opened by each engine that holds the Run in turn.
struct Run {
    env: TestEnv,
    owner: TaskOwner,
    rate_limit: Arc<RateLimit>,
    clock: Clock,
}

impl Run {
    fn new(clock: Clock) -> Self {
        Self {
            env: TestEnv::in_memory(),
            owner: TaskOwner::root(BlockingPolicy::Inline),
            rate_limit: Arc::new(RateLimit::new(clock)),
            clock,
        }
    }

    /// The driver of the engine that holds the Run at `epoch`.
    async fn driver(&self, epoch: u64) -> Driver {
        let state = RunState::open(
            self.env.folder().join("run.sqlite"),
            &RunStateOptions::default(),
            vec![TABLES],
            epoch,
            &self.owner,
        )
        .await
        .unwrap();
        Driver::new(Ledger::new(state), Arc::clone(&self.rate_limit), self.clock)
    }

    /// Stops the engine that holds the Run, its run file's writer with it.
    async fn stop(&mut self, driver: Driver) {
        drop(driver);
        self.owner.wait().await;
        self.owner = TaskOwner::root(BlockingPolicy::Inline);
    }
}

fn message(event: &str) -> Message {
    Message {
        event_id: EventId::new(event).unwrap(),
        event: "run_failed".to_owned(),
        run_id: None,
        schedule_id: None,
        product: Product::Collie,
        title: "A Run failed".to_owned(),
        text: "The Run stopped on an error.".to_owned(),
        link: None,
        occurred_at: UNIX_EPOCH,
    }
}

/// Covers: story:11-268
/// Covers: story:11-270
#[test]
fn a_transient_failure_is_sent_again_on_the_schedule_then_fails_in_the_ledger() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let run = Run::new(clock);
        let driver = run.driver(1).await;
        let failures = (0..10).map(|_| Answer::Fail(Failure::ServerError));
        let channel = Scripted::new(clock, failures);

        let held = driver.deliver(&channel, &message("evt-1")).await.unwrap();

        let failed = Entry::Failed {
            attempt: 9,
            failure: "server_error".to_owned(),
        };
        assert_eq!(held, failed);
        // Immediately, then 5 s, 5 min, 30 min, 2 h, 5 h, 10 h, 14 h, 20 h
        // and 24 h after each failed attempt.
        assert_eq!(
            channel.sent().await,
            [
                0, 5, 305, 2_105, 9_305, 27_305, 63_305, 113_705, 185_705, 272_105
            ]
        );
        let entry = driver.ledger.entry(&message("evt-1").event_id, "ops").await;
        assert_eq!(entry.unwrap(), Some(failed));
    });
}

/// Covers: story:11-270
#[rstest]
#[case::unreachable(Failure::Unreachable, true)]
#[case::timeout(Failure::Timeout, true)]
#[case::rate_limited(Failure::RateLimited { retry_after: None }, true)]
#[case::server_error(Failure::ServerError, true)]
#[case::unrecognized(Failure::Unrecognized(PartyText::new("I'm a teapot")), true)]
#[case::unauthorized(Failure::Unauthorized, false)]
#[case::gone(Failure::Gone, false)]
#[case::rejected(Failure::Rejected, false)]
fn a_transient_failure_is_sent_again_and_a_permanent_one_fails_at_once(
    #[case] failure: Failure,
    #[case] transient: bool,
) {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let run = Run::new(clock);
        let driver = run.driver(1).await;
        let name = failure.name();
        let channel = Scripted::new(clock, [Answer::Fail(failure)]);

        let held = driver.deliver(&channel, &message("evt-1")).await.unwrap();

        if transient {
            assert_eq!(held, Entry::Delivered { attempt: 1 });
            assert_eq!(channel.sent().await, [0, 5]);
        } else {
            let failed = Entry::Failed {
                attempt: 0,
                failure: name.to_owned(),
            };
            assert_eq!(held, failed);
            assert_eq!(channel.sent().await, [0]);
        }
    });
}

/// Covers: story:11-270
#[rstest]
#[case::longer_than_the_schedule(60, 60)]
#[case::shorter_than_the_schedule(2, 5)]
#[case::longer_than_a_day(48 * 3600, 24 * 3600)]
fn retry_after_holds_the_next_attempt_back_for_up_to_a_day(
    #[case] retry_after: u64,
    #[case] next_second: u64,
) {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let run = Run::new(clock);
        let driver = run.driver(1).await;
        let limited = Failure::RateLimited {
            retry_after: Some(Duration::from_secs(retry_after)),
        };
        let channel = Scripted::new(clock, [Answer::Fail(limited)]);

        let held = driver.deliver(&channel, &message("evt-1")).await.unwrap();

        assert_eq!(held, Entry::Delivered { attempt: 1 });
        assert_eq!(channel.sent().await, [0, next_second]);
    });
}

/// Covers: story:11-268
#[test]
fn a_delivered_event_is_not_sent_again_after_the_run_resumes() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let mut run = Run::new(clock);
        let channel = Scripted::new(clock, []);
        let driver = run.driver(1).await;
        driver.deliver(&channel, &message("evt-1")).await.unwrap();
        run.stop(driver).await;

        let resumed = run.driver(2).await;
        let held = resumed.deliver(&channel, &message("evt-1")).await.unwrap();

        assert_eq!(held, Entry::Delivered { attempt: 0 });
        assert_eq!(channel.sent().await, [0]);
    });
}

/// Covers: story:11-268
#[test]
fn an_attempt_in_flight_when_the_run_stopped_is_never_sent_again() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let mut run = Run::new(clock);
        let channel = Scripted::new(clock, [Answer::Hang]);
        let driver = run.driver(1).await;
        let event = message("evt-1");
        tokio::time::timeout(Duration::from_secs(10), driver.deliver(&channel, &event))
            .await
            .unwrap_err();
        run.stop(driver).await;

        let resumed = run.driver(2).await;
        let held = resumed.deliver(&channel, &message("evt-1")).await.unwrap();

        assert_eq!(held, Entry::Uncertain { attempt: 0 });
        assert_eq!(channel.sent().await, [0]);
    });
}

/// Covers: story:11-268
#[test]
fn an_attempt_waiting_when_the_run_stopped_is_sent_when_due_after_it_resumes() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let mut run = Run::new(clock);
        let channel = Scripted::new(clock, [Answer::Fail(Failure::Timeout)]);
        let driver = run.driver(1).await;
        let event = message("evt-1");
        tokio::time::timeout(Duration::from_secs(2), driver.deliver(&channel, &event))
            .await
            .unwrap_err();
        run.stop(driver).await;

        let resumed = run.driver(2).await;
        let held = resumed.deliver(&channel, &message("evt-1")).await.unwrap();

        assert_eq!(held, Entry::Delivered { attempt: 1 });
        assert_eq!(channel.sent().await, [0, 5]);
    });
}

/// Covers: story:11-270
#[test]
fn concurrent_events_to_one_destination_are_spaced_by_the_rate_limit() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let run = Run::new(clock);
        let driver = run.driver(1).await;
        let channel = Scripted::new(clock, []);
        let messages: Vec<Message> = (0..8).map(|n| message(&format!("evt-{n}"))).collect();

        let held = join_all(messages.iter().map(|m| driver.deliver(&channel, m))).await;

        for entry in held {
            assert_eq!(entry.unwrap(), Entry::Delivered { attempt: 0 });
        }
        let mut sent = channel.sent().await;
        sent.sort_unstable();
        assert_eq!(sent, [0, 0, 0, 0, 0, 1, 2, 3]);
    });
}
