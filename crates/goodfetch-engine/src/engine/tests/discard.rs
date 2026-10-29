//! `run.discard` through the engine's public interface over the in-memory
//! network (spec #213's Seam 3; ADRs 0001, 0002, 0015 and 0036): a
//! discarded Run leaves no state behind in either layout, and none of the
//! Secret canaries planted in it is found anywhere in the Goodfetch home,
//! raw or in any of Goodfetch's encodings (ADR 0067), while a Run kept
//! beside it keeps its own. A discard runs beside the Run registry, which
//! answers meanwhile and refuses a resume of the Run being discarded.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use serde_json::{Value, json};

use super::daemon::{cli_hello, daemon, opened, the_pick};
use super::engine;
use super::layout::{Talk, install_file, per_install, per_run};
use super::run::{call, run_folder};
use crate::run_state::RunStateOptions;
use crate::run_state::install::InstallDatabase;

/// The secret values a test plants, by where it puts them.
struct Planted {
    canaries: Canaries,
    /// Set as a Key-value store value of the discarded Run.
    token: String,
    /// Its status line.
    password: String,
    /// Set in the Run kept beside it.
    kept: String,
}

fn planted(env: &TestEnv) -> Planted {
    let mut canaries = Canaries::new(env.seed());
    let token = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let password = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let kept = canaries.plant(SecretKind::SessionCookie).value().to_owned();
    Planted {
        canaries,
        token,
        password,
        kept,
    }
}

/// Starts the Run `secrets` and the Run `kept`, plants the values, ends
/// `secrets` and discards it, answering the discard's reply.
async fn plant_and_discard(talk: &mut Talk, token: &str, password: &str, kept: &str) -> Value {
    talk.ok(1, "run.start", json!({ "run_id": "secrets" }))
        .await;
    talk.ok(2, "run.start", json!({ "run_id": "kept" })).await;
    talk.ok(
        3,
        "key_value_store.set",
        json!({ "run_id": "secrets", "key": "token", "value": token }),
    )
    .await;
    talk.ok(
        4,
        "run.set_progress_message",
        json!({ "run_id": "secrets", "message": password }),
    )
    .await;
    // A later status line leaves the first only in the pages its rewrite
    // freed.
    talk.ok(
        5,
        "run.set_progress_message",
        json!({ "run_id": "secrets", "message": "Reading page 2" }),
    )
    .await;
    talk.ok(
        6,
        "key_value_store.set",
        json!({ "run_id": "kept", "key": "session", "value": kept }),
    )
    .await;
    talk.ok(7, "run.finish", json!({ "run_id": "secrets" }))
        .await;
    talk.reply(8, "run.discard", json!({ "run_id": "secrets" }))
        .await
}

/// The kinds of the canaries found anywhere in the Goodfetch home.
fn found_in_home(env: &TestEnv, canaries: &Canaries) -> BTreeSet<&'static str> {
    canaries
        .search(&[Output::Path(env.home())])
        .unwrap()
        .into_iter()
        .map(|hit| hit.kind.name())
        .collect()
}

/// Covers: story:3-39, operation:run.discard
#[test]
fn a_run_discarded_in_the_per_run_layout_leaves_none_of_its_secrets_in_the_home() {
    let env = TestEnv::new();
    let Planted {
        canaries,
        token,
        password,
        kept,
    } = planted(&env);

    let discarded = per_run(&env, |mut talk| async move {
        plant_and_discard(&mut talk, &token, &password, &kept).await
    });

    assert_eq!(
        (
            &discarded["result"],
            found_in_home(&env, &canaries),
            env.home().join("1").join("runs").join("secrets").exists(),
        ),
        (
            &json!({ "run_id": "secrets" }),
            BTreeSet::from(["session_cookie"]),
            false
        )
    );
}

/// In the per-install layout the Run's rows are deleted from the install
/// database with `secure_delete` on, then its WAL truncated: neither the
/// database nor its `-wal` and `-shm` files hold any of the Run's secrets,
/// while the Run kept beside it is still listed and found (ADR 0002).
///
/// Covers: story:3-39, operation:run.discard, setting:run_state.layout
#[test]
fn a_run_discarded_in_the_per_install_layout_leaves_none_of_its_secrets_in_the_install_database() {
    let env = TestEnv::new();
    let Planted {
        canaries,
        token,
        password,
        kept,
    } = planted(&env);

    let discarded = per_install(&env, |mut talk| async move {
        plant_and_discard(&mut talk, &token, &password, &kept).await
    });

    let install = install_file(&env);
    let files = ["", "-wal", "-shm"].map(|suffix| {
        let mut name = install.clone().into_os_string();
        name.push(suffix);
        std::path::PathBuf::from(name)
    });
    let outputs: Vec<Output<'_>> = files
        .iter()
        .filter(|file| file.exists())
        .map(|file| Output::Path(file))
        .collect();
    let in_install: BTreeSet<&str> = canaries
        .search(&outputs)
        .unwrap()
        .into_iter()
        .map(|hit| hit.kind.name())
        .collect();
    let listed = InstallDatabase::open(
        &install,
        &env.home().join("1").join("runs"),
        &RunStateOptions::default(),
    )
    .unwrap()
    .runs()
    .unwrap();
    assert_eq!(
        (
            &discarded["result"],
            in_install,
            found_in_home(&env, &canaries),
            listed,
        ),
        (
            &json!({ "run_id": "secrets" }),
            BTreeSet::from(["session_cookie"]),
            BTreeSet::from(["session_cookie"]),
            vec!["kept".to_owned()],
        )
    );
}

/// The row of `run_id` in the Run index at `index`: its presence, tags and
/// status line.
fn index_row(index: &Path, run_id: &str) -> (String, String, Option<String>) {
    rusqlite::Connection::open(index)
        .unwrap()
        .query_row(
            "SELECT presence, tags, progress_message FROM runs WHERE run_id = ?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

/// A discarded Run's Run index row stays, *erased*, without its tags or
/// status line; discarding it again changes nothing; and a start under its
/// Run ID begins a new Run that finds none of its Key-value store (ADRs
/// 0001 and 0036).
/// Covers: story:3-39, operation:run.discard
#[test]
fn a_start_under_a_discarded_runs_id_begins_a_new_run_that_finds_none_of_its_state() {
    let env = TestEnv::new();
    let index = env.home().join("1").join("run-index.sqlite");
    let reading = index.clone();

    let (again, erased, restarted, got) = per_run(&env, |mut talk| async move {
        talk.ok(
            1,
            "run.start",
            json!({ "run_id": "reset", "tags": ["nightly"] }),
        )
        .await;
        talk.ok(
            2,
            "key_value_store.set",
            json!({ "run_id": "reset", "key": "cursor", "value": "page-9" }),
        )
        .await;
        talk.ok(
            3,
            "run.set_progress_message",
            json!({ "run_id": "reset", "message": "Reading page 9" }),
        )
        .await;
        talk.ok(4, "run.finish", json!({ "run_id": "reset" })).await;
        talk.ok(5, "run.discard", json!({ "run_id": "reset" }))
            .await;
        let again = talk
            .reply(6, "run.discard", json!({ "run_id": "reset" }))
            .await;
        let erased = index_row(&reading, "reset");
        let restarted = talk.ok(7, "run.start", json!({ "run_id": "reset" })).await;
        let got = talk
            .ok(
                8,
                "key_value_store.get",
                json!({ "run_id": "reset", "key": "cursor" }),
            )
            .await;
        (again, erased, restarted, got)
    });

    assert_eq!(
        (
            again,
            erased,
            &restarted["started_as"],
            got,
            index_row(&index, "reset").0
        ),
        (
            json!({ "jsonrpc": "2.0", "id": 6, "result": { "run_id": "reset" } }),
            ("erased".to_owned(), "[]".to_owned(), None),
            &json!("new"),
            json!({ "value": null }),
            "present".to_owned(),
        )
    );
}

/// A discard runs beside the Run registry, never in it (ADR 0050): while
/// one waits for a reader's test of its Run's writer lock to let go, the
/// Daemon still answers `daemon.health` live and ready, and a resume, a
/// progress read and a start of the Run being discarded are refused with
/// `run_in_use`; once the reader lets go, the discard ends and the Run is
/// gone.
/// Covers: story:3-39, story:3-102, operation:run.discard, operation:daemon.health, error:run_in_use
#[test]
fn a_discard_leaves_the_registry_answering_and_refuses_a_resume_of_its_run() {
    let env = TestEnv::new();
    let folder = run_folder(&env, "discarded");
    let watched = folder.clone();

    let (_, (order, replies)) = daemon(
        &engine(Product::Collie),
        &env,
        the_pick(),
        |dialer| async move {
            let mut client = opened(&dialer, cli_hello()).await;
            call(
                &mut client,
                1,
                "run.start",
                json!({ "run_id": "discarded" }),
            )
            .await;
            call(
                &mut client,
                2,
                "run.finish",
                json!({ "run_id": "discarded" }),
            )
            .await;
            let reader = std::fs::File::open(watched.join("run.lock")).unwrap();
            reader.try_lock_shared().unwrap();
            client
                .send(&[
                    request(3, "run.discard", json!({ "run_id": "discarded" })),
                    request(4, "daemon.health", json!({})),
                    request(5, "run.resume", json!({ "run_id": "discarded" })),
                    request(6, "run.progress", json!({ "run_id": "discarded" })),
                    request(7, "run.start", json!({ "run_id": "discarded" })),
                ])
                .await;
            let (mut order, mut replies) = (Vec::new(), BTreeMap::new());
            let mut reader = Some(reader);
            while replies.len() < 5 {
                let reply = client.next().await.unwrap();
                let Some(id) = reply["id"].as_i64() else {
                    continue;
                };
                order.push(id);
                replies.insert(id, reply);
                if [4, 5, 6, 7].iter().all(|id| replies.contains_key(id)) {
                    // The reader's test lets go.
                    drop(reader.take());
                }
            }
            client.close().await;
            (order, replies)
        },
    );

    assert_eq!(
        (
            &replies[&3]["result"],
            &replies[&4]["result"],
            [5, 6, 7].map(|id| replies[&id]["error"]["data"]["code"].clone()),
            order.last(),
            folder.exists(),
        ),
        (
            &json!({ "run_id": "discarded" }),
            &json!({ "live": true, "ready": true, "unready": [] }),
            [
                json!("run_in_use"),
                json!("run_in_use"),
                json!("run_in_use")
            ],
            Some(&3),
            false,
        ),
        "{replies:?}"
    );
}

/// The JSON-RPC request `id` of `method` with `params`.
fn request(id: i64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// A Run this engine runs is not discarded, failing with `run_live`; one
/// another engine holds the writer lock of with `run_in_use`; a Run ID no
/// Run has with `run_not_found`. Each leaves the Run as it was.
/// Covers: story:3-39, operation:run.discard, error:run_live, error:run_in_use, error:run_not_found
#[test]
fn a_live_run_one_another_engine_writes_and_an_unknown_one_are_not_discarded() {
    let env = TestEnv::new();
    let held = env.home().join("1").join("runs").join("held");
    std::fs::create_dir_all(&held).unwrap();
    std::fs::write(held.join("run.sqlite"), b"").unwrap();
    let other = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(held.join("run.lock"))
        .unwrap();
    other.try_lock().unwrap();

    let codes = per_run(&env, |mut talk| async move {
        talk.ok(1, "run.start", json!({ "run_id": "live" })).await;
        let mut codes = Vec::new();
        for (id, run_id) in [(2, "live"), (3, "held"), (4, "unknown")] {
            let reply = talk
                .reply(id, "run.discard", json!({ "run_id": run_id }))
                .await;
            codes.push(reply["error"]["data"]["code"].clone());
        }
        let used = talk
            .ok(
                5,
                "key_value_store.set",
                json!({ "run_id": "live", "key": "still", "value": "here" }),
            )
            .await;
        (codes, used)
    });

    assert_eq!(
        (codes, held.join("run.sqlite").exists()),
        (
            (
                vec![
                    json!("run_live"),
                    json!("run_in_use"),
                    json!("run_not_found")
                ],
                json!({})
            ),
            true
        )
    );
}
