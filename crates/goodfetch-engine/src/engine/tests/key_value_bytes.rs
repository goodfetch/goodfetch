//! Bytes in a Run's Key-value store, through the engine's public interface
//! over the in-memory network (spec #213's Seam 3; ADRs 0002, 0085 and
//! 0103): `get_bytes` answers the bytes and content type `set_bytes`
//! stored, or null for an absent key; one method never answers two types,
//! so `get` on bytes fails with `value_invalid` naming `getBytes`, and
//! `get_bytes` on JSON naming `get`; bytes over 1 MiB are kept as a
//! content-addressed file in the Run's folder, which the row references, a
//! Checkpoint carries to another machine, and deleting or replacing the
//! key, or discarding the Run, removes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use data_encoding::BASE64;
use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::checkpoint::two_machines;
use super::engine;
use super::layout::{Talk, per_install_on, per_run_on};
use crate::engine::Engine;

const MIB: usize = 1024 * 1024;

fn collie() -> Engine {
    engine(Product::Collie)
}

/// `size` bytes that repeat no short pattern.
fn content(size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    env.home().join("1").join("runs").join(run_id)
}

/// The file the bytes `bytes` are kept in, in the Run folder `folder`.
fn kept_file(folder: &Path, bytes: &[u8]) -> PathBuf {
    folder.join("files").join(sha256(bytes))
}

/// `set_bytes` params for `key` of the Run `run_id`.
fn set_bytes(run_id: &str, key: &str, bytes: &[u8], content_type: &str) -> Value {
    json!({
        "run_id": run_id,
        "key": key,
        "content": BASE64.encode(bytes),
        "content_type": content_type,
    })
}

/// What `get_bytes` answers for `bytes` of `content_type`.
fn bytes_answer(bytes: &[u8], content_type: &str) -> Value {
    json!({ "bytes": { "content": BASE64.encode(bytes), "content_type": content_type } })
}

/// Opens a Connection in the per-run layout and starts the Run `run_id`.
async fn started(talk: &mut Talk, run_id: &str) {
    talk.ok(1, "run.start", json!({ "run_id": run_id })).await;
}

/// What the run file of the Run in `folder` records of `key`: whether its
/// row holds bytes inline, and the SHA-256 of the kept file it references.
fn row_of(folder: &Path, key: &str) -> (bool, Option<String>) {
    let file = Connection::open(folder.join("run.sqlite")).unwrap();
    file.query_row(
        "SELECT k.content IS NOT NULL, f.sha256
         FROM key_values k LEFT JOIN run_files f ON f.position = k.file
         WHERE k.key = ?1",
        [key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

/// Covers: story:6-30, operation:key_value_store.set_bytes, operation:key_value_store.get_bytes
#[test]
fn get_bytes_answers_the_bytes_and_content_type_set_bytes_stored() {
    let stored = b"\x89PNG\r\n\x1a\n\x00\xff".to_vec();
    let given = stored.clone();

    let (set, got) = per_run_on(&collie(), &TestEnv::new(), |mut talk| async move {
        started(&mut talk, "blob").await;
        let set = talk
            .reply(
                2,
                "key_value_store.set_bytes",
                set_bytes("blob", "logo", &given, "image/png"),
            )
            .await;
        let got = talk
            .reply(
                3,
                "key_value_store.get_bytes",
                json!({ "run_id": "blob", "key": "logo" }),
            )
            .await;
        (set, got)
    });

    assert_eq!(
        (set["result"].clone(), got["result"].clone()),
        (json!({}), bytes_answer(&stored, "image/png")),
        "{set} {got}"
    );
}

/// Covers: story:6-30, operation:key_value_store.get_bytes
#[test]
fn get_bytes_on_an_absent_key_answers_null() {
    let got = per_run_on(&collie(), &TestEnv::new(), |mut talk| async move {
        started(&mut talk, "empty").await;
        talk.reply(
            2,
            "key_value_store.get_bytes",
            json!({ "run_id": "empty", "key": "logo" }),
        )
        .await
    });

    assert_eq!(got["result"], json!({ "bytes": null }), "{got}");
}

/// One method never answers two types (S2): `get` on bytes fails naming
/// `getBytes`, and `get_bytes` on JSON naming `get`; the value stays.
#[rstest]
#[case::get_on_bytes(
    "set_bytes",
    json!({ "content": "AAE=", "content_type": "application/octet-stream" }),
    "get",
    "value_bytes",
    "`getBytes`"
)]
#[case::get_bytes_on_json("set", json!({ "value": { "page": 2 } }), "get_bytes", "value_json", "`get`")]
/// Covers: story:6-30, error:value_invalid
fn reading_a_value_with_the_other_method_fails_with_value_invalid_naming_the_right_one(
    #[case] setter: &str,
    #[case] value: Value,
    #[case] reader: &str,
    #[case] reason: &str,
    #[case] named: &str,
) {
    let (setter, reader) = (setter.to_owned(), reader.to_owned());
    let mut params = json!({ "run_id": "typed", "key": "state" });
    params
        .as_object_mut()
        .unwrap()
        .extend(value.as_object().unwrap().clone());

    let read = per_run_on(&collie(), &TestEnv::new(), |mut talk| async move {
        started(&mut talk, "typed").await;
        talk.ok(2, &format!("key_value_store.{setter}"), params)
            .await;
        talk.reply(
            3,
            &format!("key_value_store.{reader}"),
            json!({ "run_id": "typed", "key": "state" }),
        )
        .await
    });

    let data = &read["error"]["data"];
    assert_eq!(
        (
            data["code"].clone(),
            data["key"].clone(),
            data["refused"].clone(),
            data["reason"].clone(),
            data["fix"].as_str().is_some_and(|fix| fix.contains(named)),
        ),
        (
            json!("value_invalid"),
            json!("state"),
            json!("value"),
            json!(reason),
            true,
        ),
        "{read}"
    );
}

/// Bytes of 1 MiB stay in the row; bytes over it are kept as a file in
/// the Run's folder, named by their SHA-256 and referenced from the row,
/// and come back byte for byte.
#[rstest]
#[case::one_mib(MIB, false, (true, None))]
#[case::one_byte_over(
    MIB + 1,
    true,
    (false, Some("5769f52bc3eef28afa39c6fc68cadb7d0bd69812ae3a3d71452f519ec3c7aa56"))
)]
#[case::five_mib(
    5 * MIB,
    true,
    (false, Some("16b632f11cf950dda67dc4c184a3f9e0aa1ffa4c18927bb8977e7da97ca25bca"))
)]
/// Covers: story:6-30, operation:key_value_store.set_bytes, operation:key_value_store.get_bytes
fn bytes_over_1_mib_are_kept_as_a_content_addressed_file_referenced_from_the_row(
    #[case] size: usize,
    #[case] in_a_file: bool,
    #[case] row: (bool, Option<&str>),
) {
    let env = TestEnv::new();
    let stored = content(size);
    let given = stored.clone();

    let got = per_run_on(&collie(), &env, |mut talk| async move {
        started(&mut talk, "large").await;
        talk.ok(
            2,
            "key_value_store.set_bytes",
            set_bytes("large", "archive", &given, "application/zip"),
        )
        .await;
        talk.ok(
            3,
            "key_value_store.get_bytes",
            json!({ "run_id": "large", "key": "archive" }),
        )
        .await
    });

    let folder = run_folder(&env, "large");
    let file = kept_file(&folder, &stored);
    let (in_the_row, named) = row_of(&folder, "archive");
    assert_eq!(
        (
            got == bytes_answer(&stored, "application/zip"),
            std::fs::read(&file).ok() == Some(stored.clone()),
            (in_the_row, named.as_deref()),
        ),
        (true, in_a_file, row)
    );
}

/// A Run paused on one machine and resumed from its Checkpoint on another,
/// which never saw its folder, reads its 5 MiB value back byte for byte.
/// Covers: story:6-30, operation:key_value_store.get_bytes, operation:run.resume
#[test]
fn a_5_mib_value_reads_back_byte_for_byte_after_a_resume_on_another_machine() {
    let (first, second) = (TestEnv::new(), TestEnv::new());
    let stored = content(5 * MIB);
    let given = stored.clone();

    let got = two_machines(&first, &second, |mut here, mut there| async move {
        here.ok(2, "run.start", json!({ "run_id": "moved" })).await;
        here.ok(
            3,
            "key_value_store.set_bytes",
            set_bytes("moved", "archive", &given, "application/zip"),
        )
        .await;
        let paused = here.ok(4, "run.pause", json!({ "run_id": "moved" })).await;
        there
            .ok(
                2,
                "run.resume",
                json!({ "run_id": "moved", "checkpoint": paused["checkpoint"]["location"] }),
            )
            .await;
        let got = there
            .call(
                3,
                "key_value_store.get_bytes",
                json!({ "run_id": "moved", "key": "archive" }),
            )
            .await;
        there
            .ok(4, "run.finish", json!({ "run_id": "moved" }))
            .await;
        got
    });

    assert!(
        got["result"] == bytes_answer(&stored, "application/zip"),
        "{}",
        got["error"]
    );
}

/// What replaces or removes a key's large bytes.
#[rstest]
/// Covers: story:6-30, operation:key_value_store.delete
#[case::delete("delete", json!({}))]
/// Covers: story:6-30, operation:key_value_store.set
#[case::set_json("set", json!({ "value": "done" }))]
/// Covers: story:6-30, operation:key_value_store.set_bytes
#[case::set_other_bytes("set_bytes", json!({ "content": "AAE=", "content_type": "text/plain" }))]
fn deleting_or_replacing_the_key_removes_its_file(#[case] verb: &str, #[case] params: Value) {
    let env = TestEnv::new();
    let stored = content(2 * MIB);
    let given = stored.clone();
    let verb = verb.to_owned();
    let mut replaced = json!({ "run_id": "removed", "key": "archive" });
    replaced
        .as_object_mut()
        .unwrap()
        .extend(params.as_object().unwrap().clone());

    let kept = per_run_on(&collie(), &env, |mut talk| {
        let folder = run_folder(&env, "removed");
        let file = kept_file(&folder, &stored);
        async move {
            started(&mut talk, "removed").await;
            talk.ok(
                2,
                "key_value_store.set_bytes",
                set_bytes("removed", "archive", &given, "application/zip"),
            )
            .await;
            let before = file.exists();
            talk.ok(3, &format!("key_value_store.{verb}"), replaced)
                .await;
            (before, file.exists())
        }
    });

    assert_eq!(kept, (true, false));
}

/// Two keys holding the same large bytes share their file, which stays
/// until the last of them goes.
/// Covers: story:6-30, operation:key_value_store.delete
#[test]
fn a_file_two_keys_share_stays_until_the_last_key_goes() {
    let env = TestEnv::new();
    let stored = content(MIB + 7);
    let given = stored.clone();
    let file = kept_file(&run_folder(&env, "shared"), &stored);
    let watched = file.clone();

    let (after_first, second) = per_run_on(&collie(), &env, |mut talk| async move {
        started(&mut talk, "shared").await;
        for (id, key) in [(2, "a"), (3, "b")] {
            talk.ok(
                id,
                "key_value_store.set_bytes",
                set_bytes("shared", key, &given, "application/zip"),
            )
            .await;
        }
        talk.ok(
            4,
            "key_value_store.delete",
            json!({ "run_id": "shared", "key": "a" }),
        )
        .await;
        let after_first = watched.exists();
        let second = talk
            .ok(
                5,
                "key_value_store.get_bytes",
                json!({ "run_id": "shared", "key": "b" }),
            )
            .await;
        talk.ok(
            6,
            "key_value_store.delete",
            json!({ "run_id": "shared", "key": "b" }),
        )
        .await;
        (after_first, second)
    });

    assert_eq!(
        (
            after_first,
            second == bytes_answer(&stored, "application/zip"),
            file.exists()
        ),
        (true, true, false)
    );
}

/// Content that is no Base64 is refused before anything is stored.
/// Covers: story:6-30, operation:key_value_store.set_bytes
#[test]
fn content_that_is_no_base64_is_refused_as_invalid_params() {
    let (set, got) = per_run_on(&collie(), &TestEnv::new(), |mut talk| async move {
        started(&mut talk, "raw").await;
        let set = talk
            .reply(
                2,
                "key_value_store.set_bytes",
                json!({ "run_id": "raw", "key": "k", "content": "not base64!", "content_type": "text/plain" }),
            )
            .await;
        let got = talk
            .reply(
                3,
                "key_value_store.get_bytes",
                json!({ "run_id": "raw", "key": "k" }),
            )
            .await;
        (set, got)
    });

    assert_eq!(
        (set["error"]["data"]["code"].clone(), got["result"].clone()),
        (json!("invalid_params"), json!({ "bytes": null })),
        "{set} {got}"
    );
}

/// Which layout of run state a discard runs in.
type Layout = fn(&Engine, &TestEnv, Vec<u8>) -> Value;

fn discarded_per_run(engine: &Engine, env: &TestEnv, bytes: Vec<u8>) -> Value {
    per_run_on(engine, env, |talk| discard_after_setting(talk, bytes))
}

fn discarded_per_install(engine: &Engine, env: &TestEnv, bytes: Vec<u8>) -> Value {
    per_install_on(engine, env, |talk| discard_after_setting(talk, bytes))
}

/// Stores `bytes` in the Run `discarded`, ends and discards the Run, and
/// answers the discard's reply.
async fn discard_after_setting(mut talk: Talk, bytes: Vec<u8>) -> Value {
    started(&mut talk, "discarded").await;
    talk.ok(
        2,
        "key_value_store.set_bytes",
        set_bytes("discarded", "archive", &bytes, "text/plain"),
    )
    .await;
    talk.ok(3, "run.finish", json!({ "run_id": "discarded" }))
        .await;
    talk.reply(4, "run.discard", json!({ "run_id": "discarded" }))
        .await
}

/// A discarded Run's large bytes are gone with it in either layout: their
/// text, a Secret canary repeated past 1 MiB, is found nowhere in the
/// Goodfetch home.
#[rstest]
/// Covers: story:6-30, operation:run.discard
#[case::per_run(discarded_per_run)]
/// Covers: story:6-30, operation:run.discard, setting:run_state.layout
#[case::per_install(discarded_per_install)]
fn a_discarded_runs_large_bytes_are_gone_from_the_home(#[case] layout: Layout) {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let token = canaries.plant(SecretKind::EndpointToken).value().to_owned();
    let bytes = format!("{token}\n")
        .repeat(MIB / token.len() + 2)
        .into_bytes();
    assert!(bytes.len() > MIB);

    let discarded = layout(&collie(), &env, bytes);

    let found: BTreeSet<&str> = canaries
        .search(&[Output::Path(env.home())])
        .unwrap()
        .into_iter()
        .map(|hit| hit.kind.name())
        .collect();
    assert_eq!(
        (&discarded["result"], found),
        (&json!({ "run_id": "discarded" }), BTreeSet::new()),
        "{discarded}"
    );
}
