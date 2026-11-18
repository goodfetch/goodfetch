//! A [`named`] runtime's threads carry their kind's name.
//! Covers: story:11-60
#![expect(
    clippy::disallowed_methods,
    reason = "the test spawns on the runtime it built to read its threads' names, and no owner's tracker exists here"
)]

use std::num::NonZeroUsize;

use super::{BLOCKING_THREAD, Threads, WORKER_THREAD, named};

fn thread_name() -> Option<String> {
    std::thread::current().name().map(str::to_owned)
}

#[test]
fn a_named_runtime_names_its_workers_and_its_blocking_threads() {
    let one = NonZeroUsize::MIN;
    let runtime = named(Threads {
        workers: Some(one),
        blocking: one,
    })
    .unwrap();

    let (worker, blocking) = runtime.block_on(async {
        let worker = tokio::spawn(async { thread_name() }).await.unwrap();
        let blocking = tokio::task::spawn_blocking(thread_name).await.unwrap();
        (worker, blocking)
    });

    assert_eq!(worker.as_deref(), Some(WORKER_THREAD));
    assert_eq!(blocking.as_deref(), Some(BLOCKING_THREAD));
}
