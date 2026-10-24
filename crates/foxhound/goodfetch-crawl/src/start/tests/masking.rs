//! A handler's failure kept in run state holds no value the Run resolved
//! (spec #564): the details a handler's error carries, here holding the
//! Run's key-encryption key, which `run_state.key_reference` names, are
//! kept with `%run_state.key_reference%` in its place.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::ffi::OsString;

use goodfetch_engine::Dependencies;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use serde_json::json;

use super::{Reply, crawl, default_dependencies, run_file, seeds};

/// The variable the Run's key-encryption key is read from.
const KEY_VARIABLE: &str = "GF_TEST_RUN_KEY";

/// `env`'s dependencies, whose environment holds `key`, which the user
/// configuration names by reference, with no retry.
fn keyed(env: &TestEnv, key: &str) -> Dependencies {
    let folder = env.home().join("1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("configuration.toml"),
        format!(
            "version = \"0.1\"\n\n[settings]\nrun_state.key_reference = \"env:{KEY_VARIABLE}\"\ncrawl.retry.max = 0\n"
        ),
    )
    .unwrap();
    Dependencies {
        environment: [(OsString::from(KEY_VARIABLE), OsString::from(key))].into(),
        ..default_dependencies(env, RunStateOptions::default())
    }
}

/// Covers: story:4-56, error:handler_failed
#[test]
fn a_handlers_failure_is_kept_without_a_value_the_run_resolved() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let key = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    let details = json!({ "page_said": format!("the key is {key}") });

    crawl(
        || keyed(&env, &key),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "reflected", "seeds": seeds(1) }),
                    &mut |_| Reply::Silent,
                )
                .await;
            let mut handler =
                |_: &serde_json::Value| Reply::FailsWith("handler_failed", false, details.clone());
            client
                .call("run.finish", json!({ "run_id": "reflected" }), &mut handler)
                .await
        },
    );

    let kept: String = run_file(&env, "reflected")
        .query_row("SELECT details FROM attempt_errors", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        (
            kept,
            canaries
                .leaks(&[Output::Path(&env.home().join("1").join("runs"))])
                .unwrap()
        ),
        (
            json!({ "page_said": "the key is %run_state.key_reference%" }).to_string(),
            Vec::new()
        )
    );
}
