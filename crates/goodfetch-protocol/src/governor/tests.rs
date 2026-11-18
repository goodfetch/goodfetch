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

/// `governor.worker_threads` and `governor.blocking_threads` each take a
/// count of at least 1 or `per_core`: 0 starts no thread, so neither takes
/// it (#2200; the blocking threads' `per_core`, owner, 2026-11-17).
#[test]
fn the_thread_counts_take_one_or_more_or_per_core() {
    use std::num::NonZeroUsize;

    use serde_json::json;

    use super::ThreadCount;

    let three = NonZeroUsize::new(3).expect("3 is not 0");
    let read = (
        ThreadCount::read(&json!(3)),
        ThreadCount::read(&json!("per_core")),
        ThreadCount::read(&json!(0)).is_err(),
        ThreadCount::read(&json!("per_host")).is_err(),
    );

    assert_eq!(
        read,
        (
            Ok(ThreadCount::Count(three)),
            Ok(ThreadCount::PerCore),
            true,
            true,
        )
    );
}
