//! A discard through its interface (spec #213's Seam 4), on a run folder in
//! the test's folder: it retries what a reader holds for a moment, the
//! writer lock as a reader's test takes it and the run file's WAL as a read
//! keeps it, and never removes a run file whose WAL may still hold its
//! wrapped data key (ADRs 0015 and 0031).
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and a seeded Entropy (ADR 0059); its SQL statements are its own, on no Shared path (ADR 0140)"
)]

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_test_support::TestEnv;
use rusqlite::Connection;

use super::{DiscardError, Discarded, Discarding, discard};
use crate::clock::Clock;
use crate::entropy::Entropy;
use crate::runs::RUN_FILE;
use crate::runs::writer::{LOCK_FILE, take_lock};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// A run folder in the test's folder whose run file, in WAL mode, holds a
/// wrapped data key, and no writer.
fn keyed_run(env: &TestEnv) -> PathBuf {
    let folder = env.folder().join("runs").join("r");
    drop(take_lock(&folder).ok().unwrap());
    let run_file = Connection::open(folder.join(RUN_FILE)).unwrap();
    #[expect(
        clippy::disallowed_methods,
        reason = "the test's own new run file, which no other connection opens yet"
    )]
    run_file
        .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
        .unwrap();
    run_file.execute(crate::run_key::TABLE, []).unwrap();
    run_file
        .execute(
            "INSERT INTO run_key (id, wrapped, key_reference) VALUES (1, x'0102', 'k')",
            [],
        )
        .unwrap();
    folder
}

/// Discards the Run in `folder` while `meanwhile` runs, on a paused clock.
fn discarded_while(
    env: &TestEnv,
    folder: &Path,
    meanwhile: impl AsyncFnOnce(&Clock),
) -> Result<Discarded, DiscardError> {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let clock = Clock::paused(UNIX_EPOCH);
        let mut entropy = Entropy::seeded(env.seed());
        let discarding = Discarding {
            owner: &owner,
            clock: &clock,
            entropy: &mut entropy,
        };
        let (discarded, ()) = tokio::join!(
            discard(discarding, "r", folder, None, None),
            meanwhile(&clock)
        );
        owner.wait().await;
        discarded
    })
}

/// A reader's test holds the writer lock for a moment as the discard first
/// tries it: the discard retries it, as every writer does, and erases the
/// Run.
///
/// Covers: story:3-39
#[test]
fn a_discard_retries_a_lock_a_readers_test_holds_and_erases_the_run() {
    let env = TestEnv::new();
    let folder = keyed_run(&env);
    let test = OpenOptions::new()
        .read(true)
        .open(folder.join(LOCK_FILE))
        .unwrap();
    test.try_lock_shared().unwrap();

    let discarded = discarded_while(&env, &folder, async |clock| {
        clock.sleep(Duration::from_millis(1)).await;
        drop(test);
    });

    assert_eq!(
        (
            discarded.map_err(|error| error.to_string()),
            folder.exists()
        ),
        (Ok(Discarded::Erased), false)
    );
}

/// A read that keeps a snapshot from before the key's deletion keeps the
/// WAL from being truncated through every retry: the discard fails as
/// held, and the run file stays, since its WAL may still hold the wrapped
/// key that unlinking it would leave in freed disk blocks.
///
/// Covers: story:3-39
#[test]
fn a_discard_whose_wal_a_reader_holds_keeps_the_run_file_and_fails_as_held() {
    let env = TestEnv::new();
    let folder = keyed_run(&env);
    let reader = Connection::open(folder.join(RUN_FILE)).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM run_key;")
        .unwrap();

    let discarded = discarded_while(&env, &folder, async |_| {});

    assert_eq!(
        (
            matches!(discarded, Err(DiscardError::Held)),
            folder.join(RUN_FILE).exists()
        ),
        (true, true),
        "{discarded:?}"
    );
}
