use super::ENTRIES;

/// Every key of the governor is under `governor.`, its floors, ceilings
/// and thresholds included, so `config explain` shows them as one family
/// (ADR 0083).
#[test]
fn every_governor_key_is_under_governor() {
    let outside: Vec<&str> = ENTRIES
        .settings
        .iter()
        .map(|setting| setting.name)
        .filter(|name| !name.starts_with("governor."))
        .collect();

    assert_eq!(outside, Vec::<&str>::new());
}

/// `governor.worker_threads` takes a count of at least 1 or `per_core`,
/// and `governor.blocking_threads` a count of at least 1: 0 starts no
/// thread, so neither takes it (#2200).
#[test]
fn the_thread_counts_take_one_or_more_and_the_workers_per_core() {
    use std::num::NonZeroUsize;

    use serde_json::json;

    use super::{BlockingThreadCap, WorkerThreads};

    let three = NonZeroUsize::new(3).expect("3 is not 0");
    let read = (
        WorkerThreads::read(&json!(3)),
        WorkerThreads::read(&json!("per_core")),
        WorkerThreads::read(&json!(0)).is_err(),
        WorkerThreads::read(&json!("per_host")).is_err(),
        BlockingThreadCap::read(&json!(3)).map(BlockingThreadCap::get),
        BlockingThreadCap::read(&json!(0)).is_err(),
        BlockingThreadCap::read(&json!("per_core")).is_err(),
    );

    assert_eq!(
        read,
        (
            Ok(WorkerThreads::Count(three)),
            Ok(WorkerThreads::PerCore),
            true,
            true,
            Ok(3),
            true,
            true,
        )
    );
}
