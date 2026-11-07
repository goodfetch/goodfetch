//! A power cut of Collie's executable on Linux (ADR 0060): its Goodfetch
//! home, which holds the per-major folder and the run folder in it, is
//! mounted through LazyFS; the engine is killed once `run.start` answered,
//! LazyFS drops every write that was never synced, and the home is mounted
//! again, as after a reboot. The run file then passes `integrity_check` and
//! holds what the start acknowledged, and a new engine takes the same Run ID
//! back as an existing Run (ADR 0002). Only the Release suite runs it, on
//! Linux x64 (`just lazyfs-test`); nextest's profiles leave it out elsewhere.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{LazyFs, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::run::call;

#[test]
fn a_run_acknowledged_before_a_power_cut_is_intact_and_taken_back_after_it() {
    let env = TestEnv::for_lazyfs();
    let home = LazyFs::mount(&env, env.home()).unwrap();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let start = json!({ "run_id": "cut", "tags": ["acknowledged"] });
    let started = call(&mut engine, 1, "run.start", &start);

    engine.kill();
    home.clear_cache().unwrap();
    drop(home);
    let _home = LazyFs::mount(&env, env.home()).unwrap();

    let file = HomeLayout::of(env.home()).run_file("cut");
    let (integrity, run, tags) = {
        let connection = Connection::open(&file).unwrap();
        let integrity: String = connection
            .pragma_query_value(None, "integrity_check", |row| row.get(0))
            .unwrap();
        let run: (String, String) = connection
            .query_row("SELECT run_id, lifecycle FROM run", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        let tags: Vec<String> = connection
            .prepare("SELECT tag FROM tags ORDER BY position")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        (integrity, run, tags)
    };
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let again = call(&mut engine, 2, "run.start", &start);
    let _ = engine.finish();

    assert_eq!(
        (
            &started["result"]["started_as"],
            integrity.as_str(),
            (run.0.as_str(), run.1.as_str()),
            tags,
            &again["result"]["started_as"],
        ),
        (
            &json!("new"),
            "ok",
            ("cut", "running"),
            vec!["acknowledged".to_owned()],
            &json!("existing"),
        ),
        "{again}"
    );
}

/// In `fast` durability a power cut may take the Run's acknowledged
/// commits, possibly all of them, but never damages its run file, which
/// passes `integrity_check` (ADR 0002).
///
/// Covers: story:3-27, setting:run_state.durability
#[test]
fn a_power_cut_in_fast_durability_leaves_a_run_file_that_passes_its_integrity_check() {
    let env = TestEnv::for_lazyfs();
    env.user_configuration("run_state.durability = \"fast\"\n");
    let home = LazyFs::mount(&env, env.home()).unwrap();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let started = call(
        &mut engine,
        1,
        "run.start",
        &json!({ "run_id": "fast", "tags": ["maybe"] }),
    );

    engine.kill();
    home.clear_cache().unwrap();
    drop(home);
    let _home = LazyFs::mount(&env, env.home()).unwrap();

    let file = HomeLayout::of(env.home()).run_file("fast");
    let integrity: String = Connection::open(&file)
        .unwrap()
        .pragma_query_value(None, "integrity_check", |row| row.get(0))
        .unwrap();
    assert_eq!(
        (&started["result"]["started_as"], integrity.as_str()),
        (&json!("new"), "ok")
    );
}

/// A person's answer to a consent wall, acknowledged before a power cut, is
/// still in the user configuration after it: the next engine reads the file
/// as written and finds the domain listed already (ADR 0002).
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the user configuration back as a reboot would, outside any async worker"
)]
fn a_consent_domain_allowed_before_a_power_cut_is_still_allowed_after_it() {
    let env = TestEnv::for_lazyfs();
    let file = HomeLayout::of(env.home()).user_configuration();
    let home = LazyFs::mount(&env, env.home()).unwrap();
    let mut engine = Attached::start(&env);
    engine.send(&from_the_cli());
    let _ = engine.next();
    let allowed = call(
        &mut engine,
        1,
        "needs_you.allow_consent_domain",
        &json!({ "domain": "walled.test" }),
    );
    let written = std::fs::read_to_string(&file).unwrap();

    engine.kill();
    home.clear_cache().unwrap();
    drop(home);
    let _home = LazyFs::mount(&env, env.home()).unwrap();

    let kept = std::fs::read_to_string(&file).unwrap();
    let mut engine = Attached::start(&env);
    engine.send(&from_the_cli());
    let _ = engine.next();
    let again = call(
        &mut engine,
        2,
        "needs_you.allow_consent_domain",
        &json!({ "domain": "walled.test" }),
    );
    let _ = engine.finish();
    assert_eq!(
        (&allowed["result"]["added"], kept, &again["result"]["added"]),
        (&json!(true), written, &json!(false)),
        "{allowed} {again}"
    );
}

/// A `hello` from the CLI, a person's surface, which may answer a consent
/// wall.
fn from_the_cli() -> Value {
    let mut hello = hello("collie", "0.1");
    hello["params"]["surface"] = json!("cli");
    hello
}
