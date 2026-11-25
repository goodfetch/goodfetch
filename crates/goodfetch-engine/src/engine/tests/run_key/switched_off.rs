//! Field encryption and secure deletion switched off, and what a discard
//! leaves of a Run's fields, through the engine's public interface over the
//! in-memory network (spec #564, issue 618; ADRs 0015 and 0016): with
//! `security.field_encryption` off a Run stores its fields as it was given
//! them and makes no data key; with `security.secure_delete` off what its
//! run file deletes stays in its free pages; and thirty Runs discarded one
//! after another, in either layout, leave none of their fields in the home.

use std::path::{Path, PathBuf};

use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

use super::super::layout::{per_install_on, per_run_on};
use super::{engine, found_in, run_folder, wrapped_key};
use crate::run_state::install::INSTALL_FILE;

/// The settings of a Run whose field encryption is off.
fn unsealed() -> Value {
    json!({ "security": { "field_encryption": false } })
}

/// With `security.field_encryption` off, a Run stores what it seals as it
/// was given: the value opens back, lies in clear in its run file, and
/// the Run makes neither a data key nor the engine's default
/// key-encryption key; its report lists the control as weakened. A value
/// stored so opens only in a Run whose field encryption was off: a Run
/// that seals refuses it.
/// Covers: story:4-67, setting:security.field_encryption
#[test]
fn a_run_with_field_encryption_off_stores_its_fields_unsealed_and_makes_no_key() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let cookie = canaries.plant(SecretKind::SessionCookie).value().to_owned();
    let stored = cookie.clone();
    let made_key = env.home().join("secrets").join("run-state-key.age");

    let (sealed, opened, opened_unsealed, finished, made_before, refused) =
        per_run_on(&engine(), &env, |mut talk| async move {
            talk.ok(
                1,
                "run.start",
                json!({ "run_id": "clear", "settings": unsealed() }),
            )
            .await;
            let sealed = talk
                .ok(
                    2,
                    "test.seal",
                    json!({ "run_id": "clear", "row": 1, "value": stored }),
                )
                .await;
            let opened = talk
                .ok(3, "test.open", json!({ "run_id": "clear", "row": 1 }))
                .await;
            let opened_unsealed = talk
                .ok(
                    4,
                    "test.open",
                    json!({ "run_id": "clear", "row": 2, "unsealed": "stored in clear" }),
                )
                .await;
            let finished = talk.ok(5, "run.finish", json!({ "run_id": "clear" })).await;
            let made_before = made_key.exists();
            talk.ok(6, "run.start", json!({ "run_id": "sealing" }))
                .await;
            talk.ok(
                7,
                "test.seal",
                json!({ "run_id": "sealing", "row": 1, "value": "sealed" }),
            )
            .await;
            let refused = talk
                .reply(
                    8,
                    "test.open",
                    json!({ "run_id": "sealing", "row": 2, "unsealed": "stored in clear" }),
                )
                .await;
            talk.ok(9, "run.finish", json!({ "run_id": "sealing" }))
                .await;
            (
                sealed,
                opened,
                opened_unsealed,
                finished,
                made_before,
                refused,
            )
        });

    let folder = run_folder(&env, "clear");
    assert_eq!(
        (
            sealed,
            opened,
            opened_unsealed,
            &finished["weakened"],
            !found_in(
                &[
                    folder.join("run.sqlite").as_path(),
                    folder.join("run.sqlite-wal").as_path(),
                ],
                &[("the value", cookie.as_bytes())],
            )
            .is_empty(),
            wrapped_key(&folder.join("run.sqlite")),
            made_before,
            refused["error"]["message"].as_str(),
        ),
        (
            json!({ "data_key": null }),
            json!({ "value": cookie }),
            json!({ "value": "stored in clear" }),
            &json!([{
                "key": "security.field_encryption",
                "value": false,
                "layer": "run",
                "origin": "settings",
            }]),
            true,
            None,
            false,
            Some("the value does not open under this key at this place"),
        ),
        "{finished}"
    );
}

/// With `security.secure_delete` on, the default, a field overwritten
/// leaves none of its bytes in the run file once its WAL is checkpointed,
/// free pages included; off, its bytes stay there (the positive control).
/// The fields are stored unsealed, so their bytes are the value's own.
/// Covers: story:4-67, setting:security.secure_delete
#[rstest]
#[case::on(true)]
#[case::off(false)]
fn an_overwritten_field_stays_in_the_run_file_only_with_secure_deletion_off(
    #[case] secure_delete: bool,
) {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let cookie = canaries.plant(SecretKind::SessionCookie).value().to_owned();
    // Long enough to spill into overflow pages, which the overwrite frees.
    let first = format!("{cookie}{}", "-".repeat(8192));
    let settings = json!({
        "security": { "field_encryption": false, "secure_delete": secure_delete }
    });

    per_run_on(&engine(), &env, |mut talk| async move {
        talk.ok(
            1,
            "run.start",
            json!({ "run_id": "deleting", "settings": settings }),
        )
        .await;
        talk.ok(
            2,
            "test.seal",
            json!({ "run_id": "deleting", "row": 1, "value": first }),
        )
        .await;
        talk.ok(3, "run.checkpoint", json!({ "run_id": "deleting" }))
            .await;
        talk.ok(
            4,
            "test.seal",
            json!({ "run_id": "deleting", "row": 1, "value": "short" }),
        )
        .await;
        talk.ok(5, "run.finish", json!({ "run_id": "deleting" }))
            .await;
    });

    let run_file = run_folder(&env, "deleting").join("run.sqlite");
    checkpointed(&run_file);
    let hits = canaries.leaks(&[Output::Path(&run_file)]).unwrap();
    assert_eq!(
        !hits.is_empty(),
        !secure_delete,
        "{:?}",
        hits.iter().map(|hit| hit.offset).collect::<Vec<_>>()
    );
}

/// Copies the WAL beside the run file at `path` into it and empties it.
#[expect(
    clippy::disallowed_methods,
    reason = "the test checkpoints the run file the engine wrote and closed; its SQL statement is its own, on no Shared path (ADR 0140)"
)]
fn checkpointed(path: &Path) {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
}

/// The layout a test runs the engine in.
#[derive(Clone, Copy, Debug)]
enum Layout {
    PerRun,
    PerInstall,
}

/// The Runs each part of the thirty-Run test discards in a row: five parts
/// of six make the thirty, each part within the Quick suite's per-test
/// kill on a loaded host, as a suite that outgrows it runs in parts.
const ROUNDS: usize = 6;

/// Thirty Runs in turn, as a reused pool serves one tenant after another
/// (ADR 0015's reset fixture), in five parts of six on one home each,
/// each store a fresh value in its secret column, unsealed so a byte search
/// finds it before the discard (the positive control), and are discarded:
/// none of those values is left in the home, the Run index and the install
/// database included.
/// Covers: story:4-65, operation:run.discard, setting:run_state.layout, setting:security.field_encryption
#[rstest]
fn thirty_discarded_runs_leave_none_of_their_fields_in_the_home(
    #[values(Layout::PerRun, Layout::PerInstall)] layout: Layout,
    #[values(0, 1, 2, 3, 4)] part: usize,
) {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let values: Vec<String> = (0..ROUNDS)
        .map(|_| canaries.plant(SecretKind::SessionCookie).value().to_owned())
        .collect();
    // A Run's state is its own run file in the per-run layout and the
    // install database in the per-install one (ADR 0002).
    let install = env.home().join("1").join(INSTALL_FILE);
    let rounds = part * ROUNDS..(part + 1) * ROUNDS;
    let state: Vec<[PathBuf; 4]> = rounds
        .clone()
        .map(|round| {
            let folder = run_folder(&env, &format!("round-{round}"));
            [
                folder.join("run.sqlite"),
                folder.join("run.sqlite-wal"),
                install.clone(),
                install.with_extension("sqlite-wal"),
            ]
        })
        .collect();
    let discards = |mut talk: super::super::layout::Talk| async move {
        let mut found_before = Vec::new();
        for (round, (value, files)) in rounds.zip(values.iter().zip(&state)) {
            let run_id = format!("round-{round}");
            let id = i64::try_from(round * 4).unwrap();
            talk.ok(
                id + 1,
                "run.start",
                json!({ "run_id": run_id, "settings": unsealed() }),
            )
            .await;
            talk.ok(
                id + 2,
                "test.seal",
                json!({ "run_id": run_id, "row": 1, "value": value }),
            )
            .await;
            talk.ok(id + 3, "run.finish", json!({ "run_id": run_id }))
                .await;
            found_before.push(
                !found_in(
                    &files.each_ref().map(PathBuf::as_path),
                    &[("the value", value.as_bytes())],
                )
                .is_empty(),
            );
            talk.ok(id + 4, "run.discard", json!({ "run_id": run_id }))
                .await;
        }
        found_before
    };

    let found_before = match layout {
        Layout::PerRun => per_run_on(&engine(), &env, discards),
        Layout::PerInstall => per_install_on(&engine(), &env, discards),
    };

    assert_eq!(
        (
            found_before,
            canaries
                .leaks(&[Output::Path(env.home())])
                .unwrap()
                .into_iter()
                .map(|hit| format!("{} in {}", hit.canary, hit.output))
                .collect::<Vec<_>>(),
        ),
        (vec![true; ROUNDS], Vec::<String>::new())
    );
}
