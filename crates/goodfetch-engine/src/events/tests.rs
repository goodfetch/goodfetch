use futures_util::FutureExt as _;
use goodfetch_protocol::connection::EngineInstance;
use goodfetch_protocol::governor::SYSTEM_INFO;
use serde_json::Value;
use tokio::sync::oneshot;

use super::{Command, Events, EventsOptions, Publish};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A publish whose command reached the stream's mailbox under a permit
/// taken just before the stream stopped, as a publisher on another worker
/// sends one while the engine's owner stops, lands after the mailbox was
/// emptied: the stream never reads it, and the channel keeps it while any
/// handle lives. Waiting for its acceptance still ends with the stream, so
/// a task that publishes, such as the governor's sample reports, lets the
/// engine's owner stop.
#[test]
fn waiting_for_a_command_the_stopped_stream_never_read_ends_with_the_stream() {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let events = Events::start(
            EventsOptions::default(),
            EngineInstance::new("0123456789abcdef"),
            &owner,
        );
        let permit = events.commands.reserve().await.unwrap();
        owner.stop().await;
        let (accepted, acceptance) = oneshot::channel();
        permit.send(Command::Publish(Publish {
            event: &SYSTEM_INFO,
            scope: None,
            data: Value::Null,
            fold: None,
            accepted,
        }));

        assert_eq!(events.answered(acceptance).now_or_never(), Some(None));
    });
}
