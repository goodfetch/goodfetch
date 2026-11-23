//! A resume from a Checkpoint behind the run file already here (ADR 0002's
//! note of #2322): refused with `checkpoint_behind`, the run file left as it
//! was, unless the resume asks to roll back, which replaces it and writes
//! an Audit entry; a run file here that cannot be read is refused the same
//! way, with its read's error; a Checkpoint at or past the run file, or with
//! none here, resumes as before. Positions compare by run epoch, then
//! journal position.

use std::path::Path;

use goodfetch_test_support::TestEnv;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::super::attached_on;
use super::super::consent_walls::audit_entries;
use super::{Talk, engine, notes, run_folder, sha256, two_machines};

/// The run epoch and journal position of the run file or Checkpoint at
/// `path`.
fn position(path: &Path) -> Value {
    let file = Connection::open(path).unwrap();
    let row: String = file
        .query_row(
            "SELECT json_object('run_epoch', epoch, 'journal_position', journal_position) FROM run",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&row).unwrap()
}

/// Starts `run_id`, notes 1, checkpoints it to `location`, notes 2 and
/// pauses it, so its run file is past that Checkpoint: the Checkpoint's
/// answer.
fn checkpointed_then_run_further(env: &TestEnv, run_id: &str, location: &Path) -> Value {
    let location = location.display().to_string();
    let run_id = run_id.to_owned();
    attached_on(&engine(), env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(2, "test.keep", json!({ "run_id": run_id })).await;
        talk.ok(3, "test.note", json!({ "run_id": run_id, "value": 1 }))
            .await;
        let taken = talk
            .ok(
                4,
                "run.checkpoint",
                json!({ "run_id": run_id, "location": location }),
            )
            .await;
        talk.ok(5, "test.note", json!({ "run_id": run_id, "value": 2 }))
            .await;
        talk.ok(6, "run.pause", json!({ "run_id": run_id })).await;
        taken
    })
}

/// As [`checkpointed_then_run_further`], then fabricates the run file's
/// run-state version a major newer than this engine reads.
fn checkpointed_over_an_unreadable_run_file(env: &TestEnv, run_id: &str, location: &Path) {
    checkpointed_then_run_further(env, run_id, location);
    Connection::open(run_folder(env, run_id).join("run.sqlite"))
        .unwrap()
        .execute(
            "UPDATE format SET version = '1.0' WHERE name = 'run-state'",
            [],
        )
        .unwrap();
}

/// Covers: story:3-34, operation:run.resume, error:checkpoint_behind
#[test]
fn a_resume_from_a_checkpoint_behind_the_run_file_here_is_refused_and_changes_nothing() {
    let env = TestEnv::new();
    let location = env.folder().join("early.sqlite");
    let taken = checkpointed_then_run_further(&env, "behind", &location);
    let file = run_folder(&env, "behind").join("run.sqlite");
    let before = (notes(&file), position(&file));

    let refused = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.call(
            2,
            "run.resume",
            json!({ "run_id": "behind", "checkpoint": location.display().to_string() }),
        )
        .await
    });

    let data = &refused["error"]["data"];
    assert_eq!(
        (
            data["code"].clone(),
            data["retryable"].clone(),
            data["run_id"].clone(),
            data["checkpoint"].clone(),
            data["run_file"].clone(),
            data["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains("rollback")),
            (notes(&file), position(&file)),
        ),
        (
            json!("checkpoint_behind"),
            json!(false),
            json!("behind"),
            json!({ "run_epoch": taken["run_epoch"], "journal_position": taken["journal_position"] }),
            before.1.clone(),
            true,
            before,
        )
    );
}

/// Covers: story:3-34, operation:run.resume
#[test]
fn a_resume_that_asks_to_roll_back_replaces_the_run_file_resumes_and_is_audited() {
    let env = TestEnv::new();
    let location = env.folder().join("early.sqlite");
    let taken = checkpointed_then_run_further(&env, "rolled", &location);
    let file = run_folder(&env, "rolled").join("run.sqlite");
    let ahead = position(&file);

    let resumed = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        let resumed = talk
            .call(
                2,
                "run.resume",
                json!({
                    "run_id": "rolled",
                    "checkpoint": location.display().to_string(),
                    "rollback": true,
                }),
            )
            .await;
        talk.ok(3, "run.finish", json!({ "run_id": "rolled" }))
            .await;
        resumed
    });

    let rolled_back: Vec<Value> = audit_entries(&env)
        .into_iter()
        .filter(|entry| entry["kind"] == "run_rolled_back")
        .map(|entry| {
            json!({
                "actor": entry["actor"],
                "run_id": entry["run_id"],
                "details": entry["details"],
            })
        })
        .collect();
    assert_eq!(
        (
            resumed["result"]["started_as"].clone(),
            notes(&file),
            rolled_back
        ),
        (
            json!("resumed"),
            vec![1],
            vec![json!({
                "actor": { "kind": "person", "name": "os_user" },
                "run_id": "rolled",
                "details": {
                    "checkpoint": { "run_epoch": taken["run_epoch"], "journal_position": taken["journal_position"] },
                    "run_file": ahead,
                },
            })],
        )
    );
}

/// A run file here that this engine cannot read cannot show it is not
/// ahead: the resume is refused with its read's error and leaves it as it
/// was.
/// Covers: story:3-34, operation:run.resume, error:format_version_unsupported
#[test]
fn a_resume_from_a_checkpoint_over_an_unreadable_run_file_here_is_refused_and_changes_nothing() {
    let env = TestEnv::new();
    let location = env.folder().join("early.sqlite");
    checkpointed_over_an_unreadable_run_file(&env, "unread", &location);
    let file = run_folder(&env, "unread").join("run.sqlite");
    let before = sha256(&file);

    let refused = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.call(
            2,
            "run.resume",
            json!({ "run_id": "unread", "checkpoint": location.display().to_string() }),
        )
        .await
    });

    assert_eq!(
        (refused["error"]["data"]["code"].clone(), sha256(&file)),
        (json!("format_version_unsupported"), before)
    );
}

/// A roll-back replaces a run file here that cannot be read, and its Audit
/// entry's `run_file` is null.
/// Covers: story:3-34, operation:run.resume
#[test]
fn a_resume_that_asks_to_roll_back_replaces_an_unreadable_run_file_and_is_audited() {
    let env = TestEnv::new();
    let location = env.folder().join("early.sqlite");
    checkpointed_over_an_unreadable_run_file(&env, "unread", &location);

    let resumed = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        let resumed = talk
            .call(
                2,
                "run.resume",
                json!({
                    "run_id": "unread",
                    "checkpoint": location.display().to_string(),
                    "rollback": true,
                }),
            )
            .await;
        talk.ok(3, "run.finish", json!({ "run_id": "unread" }))
            .await;
        resumed
    });

    let run_files: Vec<Value> = audit_entries(&env)
        .into_iter()
        .filter(|entry| entry["kind"] == "run_rolled_back")
        .map(|entry| entry["details"]["run_file"].clone())
        .collect();
    assert_eq!(
        (
            resumed["result"]["started_as"].clone(),
            notes(&run_folder(&env, "unread").join("run.sqlite")),
            run_files,
        ),
        (json!("resumed"), vec![1], vec![Value::Null])
    );
}

/// How many `run_rolled_back` entries the Audit log in `env` holds.
fn rolled_back(env: &TestEnv) -> usize {
    audit_entries(env)
        .iter()
        .filter(|entry| entry["kind"] == "run_rolled_back")
        .count()
}

/// A Run migrated to a second machine and back resumes over the older run
/// file of the first, whose run epoch the second's passed: it is not behind,
/// so it needs no roll-back and writes no Audit entry.
/// Covers: story:3-34, operation:run.resume
#[test]
fn a_resume_from_a_checkpoint_past_the_run_file_here_goes_on() {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    let away = first.folder().join("away.sqlite").display().to_string();
    let back = first.folder().join("back.sqlite").display().to_string();

    let back_home = two_machines(&first, &second, |mut home, mut other| async move {
        home.ok(2, "test.keep", json!({ "run_id": "travels" }))
            .await;
        home.ok(3, "test.note", json!({ "run_id": "travels", "value": 1 }))
            .await;
        home.ok(
            4,
            "run.migrate",
            json!({ "run_id": "travels", "location": away }),
        )
        .await;
        other
            .ok(
                2,
                "run.resume",
                json!({ "run_id": "travels", "checkpoint": away }),
            )
            .await;
        other
            .ok(
                3,
                "run.migrate",
                json!({ "run_id": "travels", "location": back }),
            )
            .await;
        let back_home = home
            .call(
                5,
                "run.resume",
                json!({ "run_id": "travels", "checkpoint": back }),
            )
            .await;
        home.ok(6, "run.finish", json!({ "run_id": "travels" }))
            .await;
        back_home
    });

    assert_eq!(
        (
            back_home["result"]["started_as"].clone(),
            notes(&run_folder(&first, "travels").join("run.sqlite")),
            rolled_back(&first),
        ),
        (json!("resumed"), vec![1], 0)
    );
}

/// A resume from the Checkpoint a pause wrote, at its run file's position,
/// goes on with no roll-back and no Audit entry.
/// Covers: story:3-34, operation:run.resume
#[test]
fn a_resume_from_the_checkpoint_at_the_run_file_here_goes_on() {
    let env = TestEnv::new();
    let own = run_folder(&env, "paused")
        .join("checkpoint.sqlite")
        .display()
        .to_string();

    let at_own = attached_on(&engine(), &env, |client| async move {
        let mut talk = Talk::open(client).await;
        talk.ok(2, "test.keep", json!({ "run_id": "paused" })).await;
        talk.ok(3, "test.note", json!({ "run_id": "paused", "value": 1 }))
            .await;
        talk.ok(4, "run.pause", json!({ "run_id": "paused" })).await;
        let at_own = talk
            .call(
                5,
                "run.resume",
                json!({ "run_id": "paused", "checkpoint": own }),
            )
            .await;
        talk.ok(6, "run.finish", json!({ "run_id": "paused" }))
            .await;
        at_own
    });

    assert_eq!(
        (
            at_own["result"]["started_as"].clone(),
            notes(&run_folder(&env, "paused").join("run.sqlite")),
            rolled_back(&env),
        ),
        (json!("resumed"), vec![1], 0)
    );
}
