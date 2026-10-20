//! A power cut of Collie's executable on Linux (ADR 0060): its Goodfetch
//! home, which holds the per-major folder and the run folder in it, is
//! mounted through LazyFS; the engine is killed once `run.start` answered,
//! LazyFS drops every write that was never synced, and the home is mounted
//! again, as after a reboot. The run file then passes `integrity_check` and
//! holds what the start acknowledged, and a new engine takes the same Run ID
//! back as an existing Run (ADR 0002). Only the Release suite runs it, on
//! Linux x64 (`just lazyfs-test`); nextest's profiles leave it out elsewhere.

use goodfetch_test_support::{LazyFs, TestEnv};
use rusqlite::Connection;
use serde_json::json;

use crate::attached::{Attached, hello};
use crate::run::call;

#[test]
fn a_run_acknowledged_before_a_power_cut_is_intact_and_taken_back_after_it() {
    let env = TestEnv::new();
    let home = LazyFs::mount(env.home()).unwrap();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let start = json!({ "run_id": "cut", "tags": ["acknowledged"] });
    let started = call(&mut engine, 1, "run.start", &start);

    engine.kill();
    home.clear_cache().unwrap();
    drop(home);
    let _home = LazyFs::mount(env.home()).unwrap();

    let file = env
        .home()
        .join("0.0")
        .join("runs")
        .join("cut")
        .join("run.sqlite");
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
