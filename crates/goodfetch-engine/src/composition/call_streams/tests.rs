//! A call's file sends under the inline blocking policy (ADR 0060).
#![expect(
    clippy::disallowed_methods,
    reason = "the test writes the file it sends itself"
)]

use goodfetch_counted_work::CountedWork;
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_test_support::{TestEnv, with_the_blocking_pool_held};
use tokio::sync::mpsc;

use super::{CallNotifier, CallStream, open_to_send};
use crate::events::Credits;
use crate::rpc::notification_value_line;
use crate::tasks::{BlockingPolicy, TaskOwner};

/// Under `BlockingPolicy::Inline`, the Simulation's and the Compatibility
/// corpus writers', a file goes out in its `$/chunk` notifications with no
/// thread of the runtime's blocking pool, here held by another job: work on
/// such a thread lets the other tasks run for as long as the thread takes,
/// so a run's interleaving would hang on the host's load.
#[test]
fn a_file_sent_under_inline_blocking_takes_no_thread_of_the_runtime() {
    let env = TestEnv::new();
    let path = env.folder().join("body.bin");
    // A whole chunk of 64 KiB and a short one of 10 bytes.
    std::fs::write(&path, vec![7_u8; 65_546]).unwrap();
    let file = open_to_send(&path).unwrap();

    let sent = with_the_blocking_pool_held(|| async move {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let (outgoing, mut lines) = mpsc::channel(4);
        let notifier = CallNotifier::new(
            RequestId::Number(1),
            outgoing,
            notification_value_line,
            CountedWork::default(),
        );
        let credits = Credits::new();
        credits.grant(2);
        let stream = CallStream::new(notifier, credits);
        let sent = stream
            .send_file(&owner, file)
            .await
            .map_err(|error| error.to_string());
        drop(stream);
        let mut chunks = 0;
        while lines.recv().await.is_some() {
            chunks += 1;
        }
        (sent, chunks)
    });

    assert_eq!(
        sent,
        Some((Ok(65_546), 2)),
        "the send waited for the blocking pool"
    );
}
