//! Versions through the engine's public interface over the in-memory
//! network (spec #213's Seam 3): a Run's start refused by a shared store's
//! lowest minor (ADR 0098) and by an Operator policy found only for an
//! older major (ADR 0030), each before the Run begins, its Error code and
//! message snapshotted. The engine here is Collie 1.4.0, a version these
//! tests give it as a value (ADR 0059).
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the Goodfetch home's files themselves; its SQL statements are its own, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::formats::RUN_INDEX;
use goodfetch_test_support::{SnapshotValues, TestEnv, snapshot_settings};
use serde_json::{Value, json};

use super::run::call;
use super::{attached_on, engine_with, hello};
use crate::lowest_minor::{FORMAT_TABLE, Marker, write_sqlite};

/// Starts the Run `kept` on Collie 1.4.0, and answers the start's reply.
fn start(env: &TestEnv) -> Value {
    attached_on(
        &engine_with(Product::Collie, None),
        env,
        |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            client.next().await;
            call(&mut client, 1, "run.start", json!({ "run_id": "kept" })).await
        },
    )
}

/// An engine that finds the Run index's lowest minor newer than itself
/// takes no new Run, and names the version to install (ADR 0098).
///
/// Covers: story:3-144, format:run-index, error:format_version_unsupported
#[test]
fn a_run_index_marked_for_a_newer_minor_refuses_a_new_run() {
    let env = TestEnv::new();
    let major = env.home().join("1");
    std::fs::create_dir_all(&major).unwrap();
    let index = rusqlite::Connection::open(major.join("run-index.sqlite")).unwrap();
    index.execute(FORMAT_TABLE, []).unwrap();
    write_sqlite(
        &index,
        RUN_INDEX.name,
        Marker {
            version: RUN_INDEX.version,
            lowest_minor: 5,
        },
    )
    .unwrap();

    let started = start(&env);

    assert!(!major.join("runs").join("kept").exists());
    insta::assert_snapshot!(serde_json::to_string_pretty(&started["error"]).unwrap());
}

/// A new major that finds an Operator policy only for an older major
/// refuses to start a Run and names `upgrade-files`, so the operator's
/// caps and locks are never lost silently (ADR 0030).
///
/// Covers: story:3-94, format:operator-policy, error:format_version_unsupported
#[test]
fn an_operator_policy_only_for_an_older_major_refuses_a_run_naming_upgrade_files() {
    let env = TestEnv::new();
    let older = env.home().join("0.9");
    std::fs::create_dir_all(&older).unwrap();
    std::fs::write(
        older.join("operator-policy.toml"),
        "version = \"0.1\"\n\n[lock.timeout]\nnavigation = 10000\n",
    )
    .unwrap();

    let started = start(&env);

    assert!(!env.home().join("1").join("runs").join("kept").exists());
    snapshot_settings(&SnapshotValues::of(&env)).bind(|| {
        insta::assert_snapshot!(serde_json::to_string_pretty(&started["error"]).unwrap());
    });
}
