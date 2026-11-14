//! A crawl's run file keeps its WAL under its bound through the engine's
//! public interface over the in-memory network (spec #213's Seam 3): a
//! reader that reads its Records all along, each read a snapshot of its
//! own, does not let the WAL grow past twice its checkpoint threshold and
//! one commit (#2323, ADR 0138).

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{Reply, crawl, dependencies, records, run_file, run_folder, seeds};

/// The checkpoint threshold of this test's run file, 16 pages: a crawl of
/// a test's size passes it many times.
const CHECKPOINT_AFTER: u64 = 65_536;

/// How many pages the crawl handles, each with a Record of 8 KiB.
const PAGES: usize = 64;

#[expect(
    clippy::disallowed_methods,
    reason = "the test reads a file's size once its engine stopped, with nothing else to run"
)]
fn wal_size(env: &TestEnv, run_id: &str) -> u64 {
    std::fs::metadata(run_folder(env, run_id).join("run.sqlite-wal"))
        .map_or(0, |metadata| metadata.len())
}

/// Covers: story:3-25
#[test]
fn a_crawl_read_all_along_ends_with_its_wal_under_the_bound() {
    let env = TestEnv::new();
    let options = RunStateOptions {
        wal_checkpoint_after: CHECKPOINT_AFTER,
        ..RunStateOptions::default()
    };
    let mut reader: Option<Connection> = None;
    let mut read = 0;

    crawl(
        || dependencies(&env, options),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "read", "seeds": seeds(PAGES) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler = |_: &Value| {
                let reader = reader.get_or_insert_with(|| run_file(&env, "read"));
                read += records(reader).len();
                Reply::Records(vec![json!({ "text": "x".repeat(8192) })])
            };
            client
                .call("run.finish", json!({ "run_id": "read" }), &mut handler)
                .await
        },
    );
    // Measured while the reader still holds the file open: the last
    // connection to close would copy the WAL into the file.
    let wal = wal_size(&env, "read");
    let kept = reader.as_ref().map_or(0, |reader| records(reader).len());

    assert_eq!(kept, PAGES);
    assert!(read > 0, "the reader read Records while the crawl ran");
    assert!(wal <= 3 * CHECKPOINT_AFTER, "{wal}");
}
