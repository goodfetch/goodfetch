//! A Run's Key-value store through the Rust SDK on the linked engine (ADR
//! 0103; issues 1284 and 1486): `get` on an absent key answers `None`, a set
//! value comes back, `delete` answers whether the key existed, and a null
//! value fails in the engine with `value_invalid`, naming `delete`;
//! `get_bytes` answers the bytes and content type `set_bytes` stored, and
//! neither `get` nor `get_bytes` answers the other's type (S2). A program's
//! type reads its value back (ADR 0082), and what is not JSON, or what the
//! type does not read, is refused in the program. The client conformance
//! cases of the `key-value-store` capability, in `linked` mode; TypeScript's
//! `key-value-store.test.ts` and Python's `test_key_value_store_attached.py`
//! are its twins.

use std::collections::BTreeMap;

use goodfetch_collie::{Collie, Error, ErrorCode, KeyValueStore, Run, StoredBytes, blocking};
use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::key_value_store::{Refused, ValueInvalidDetails, ValueInvalidReason};
use goodfetch_test_support::{TestEnv, case_logs, real_time_runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::options;

/// What `body` answers of the Key-value store of a Run on an engine of its
/// own, which is finished and closed after.
fn with_store<T>(body: impl AsyncFnOnce(&Run, KeyValueStore) -> T) -> T {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let answered = body(&run, run.key_value_store()).await;
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        answered
    })
}

/// What the `value_invalid` of `result` says of the value it refused.
#[track_caller]
fn refusal<T: std::fmt::Debug>(result: Result<T, Error>) -> ValueInvalidDetails {
    result
        .unwrap_err()
        .details()
        .expect("a value_invalid's details")
}

/// The key `details` names, what of it was refused, why and where.
fn refused(details: &ValueInvalidDetails) -> (&str, Refused, ValueInvalidReason, Option<&str>) {
    (
        &details.key,
        details.refused,
        details.reason,
        details.path.as_deref(),
    )
}

/// Covers: story:6-28, story:6-31, operation:key_value_store.get,
/// operation:key_value_store.delete
#[test]
fn get_answers_none_for_an_absent_key_a_set_value_and_delete_whether_the_key_existed() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "key-value-store-get-absent-answers-null",
        RustMode::Linked,
    )
    .unwrap();
    let cursor = json!({ "page": 3, "seen": [1, null] });

    let (absent, stored, deleted) = with_store(async |_, store| {
        let absent: Option<Value> = store.get("cursor").await.unwrap();
        store.set("cursor", &cursor).await.unwrap();
        let stored: Option<Value> = store.get("cursor").await.unwrap();
        let deleted = [
            store.delete("cursor").await.unwrap(),
            store.delete("cursor").await.unwrap(),
        ];
        (absent, stored, deleted)
    });

    case.check("get on an absent key answers null", || {
        assert_eq!(absent, None);
    });
    case.check("a set value comes back with its nested null", || {
        assert_eq!(stored, Some(cursor));
    });
    case.check("delete answers whether the key existed", || {
        assert_eq!(deleted, [true, false]);
    });
}

/// Covers: story:6-29, operation:key_value_store.set, error:value_invalid
#[test]
fn a_null_value_is_refused_naming_delete() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "key-value-store-set-null-refused",
        RustMode::Linked,
    )
    .unwrap();

    let (null, after) = with_store(async |_, store| {
        let null = store.set("cursor", &None::<u32>).await;
        (null, store.get::<Value>("cursor").await.unwrap())
    });
    let null = refusal(null);

    case.check("the engine refused the null with value_invalid", || {
        assert_eq!(
            refused(&null),
            (
                "cursor",
                Refused::Value,
                ValueInvalidReason::ValueNull,
                None
            )
        );
    });
    case.check("its fix names delete", || {
        assert!(null.fix.contains("`delete`"), "{null:?}");
    });
    case.check("nothing was stored", || assert_eq!(after, None));
}

/// Covers: story:6-30, operation:key_value_store.get_bytes,
/// operation:key_value_store.set_bytes
#[test]
fn get_bytes_answers_the_bytes_and_content_type_set_bytes_stored() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "key-value-store-bytes-round-trip",
        RustMode::Linked,
    )
    .unwrap();
    let logo = b"\x89PNG\r\n\x1a\n\x00\xff";

    let (absent, stored) = with_store(async |_, store| {
        let absent = store.get_bytes("logo").await.unwrap();
        store.set_bytes("logo", *logo, "image/png").await.unwrap();
        (absent, store.get_bytes("logo").await.unwrap())
    });

    case.check("getBytes on an absent key answers null", || {
        assert_eq!(absent, None);
    });
    case.check(
        "getBytes answers the bytes and content type setBytes stored",
        || {
            let stored = stored.unwrap();
            assert_eq!(
                (stored.content.as_slice(), stored.content_type.as_str()),
                (logo.as_slice(), "image/png")
            );
        },
    );
}

/// Covers: story:6-30, error:value_invalid
#[test]
fn neither_reader_answers_the_other_type() {
    let mut case = ClientCaseRun::start(
        case_logs().as_deref(),
        "key-value-store-read-of-the-other-type-refused",
        RustMode::Linked,
    )
    .unwrap();

    let (bytes_with_get, json_with_get_bytes) = with_store(async |_, store| {
        store
            .set_bytes("logo", [1, 2, 3], "image/png")
            .await
            .unwrap();
        store.set("cursor", &json!({ "page": 2 })).await.unwrap();
        (
            store.get::<Value>("logo").await,
            store.get_bytes("cursor").await,
        )
    });
    let bytes = refusal(bytes_with_get);
    case.check(
        "get on bytes fails with value_invalid naming getBytes",
        || {
            let reason = ValueInvalidReason::ValueBytes;
            assert_eq!(refused(&bytes), ("logo", Refused::Value, reason, None));
            assert!(bytes.fix.contains("`getBytes`"), "{bytes:?}");
        },
    );
    let json = refusal(json_with_get_bytes);
    case.check(
        "getBytes on JSON fails with value_invalid naming get",
        || {
            let reason = ValueInvalidReason::ValueJson;
            assert_eq!(refused(&json), ("cursor", Refused::Value, reason, None));
            assert!(json.fix.contains("`get`"), "{json:?}");
        },
    );
}

/// A program's own type.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Book {
    title: String,
    pages: u32,
}

/// A type whose `Serialize` writes a field its `Deserialize` skips.
#[derive(Debug, Serialize, Deserialize)]
struct Draft {
    title: String,
    #[serde(skip_deserializing)]
    note: String,
}

/// A value comes back in the program's type, and one that is not JSON, that
/// its type does not read back as given, or that the type it is read with
/// does not read, is refused, the stored value unchanged.
/// Covers: story:11-66, story:11-68, error:value_invalid
#[test]
fn a_type_reads_its_value_back_and_what_it_does_not_read_is_refused() {
    let dune = || Book {
        title: "Dune".to_owned(),
        pages: 412,
    };
    let (read, refusals, unchanged) = with_store(async |_, store| {
        store.set("book", &dune()).await.unwrap();
        store.set("books", &vec![dune()]).await.unwrap();
        let read = (
            store.get::<Book>("book").await.unwrap(),
            store.get::<Vec<Book>>("books").await.unwrap(),
            store.get::<Book>("absent").await.unwrap(),
        );
        let draft = Draft {
            title: "Dune".to_owned(),
            note: "reread".to_owned(),
        };
        let price = BTreeMap::from([("amount".to_owned(), f64::NAN)]);
        let refusals = [
            refusal(store.set("book", &price).await),
            refusal(store.set("book", &draft).await),
            refusal(store.get::<Book>("books").await),
        ];
        (read, refusals, store.get::<Value>("book").await.unwrap())
    });

    assert_eq!(read, (Some(dune()), Some(vec![dune()]), None));
    let (not_json, schema) = (
        ValueInvalidReason::ValueNotJson,
        ValueInvalidReason::ValueSchema,
    );
    assert_eq!(
        refusals.each_ref().map(refused),
        [
            ("book", Refused::Value, not_json, Some("/amount")),
            ("book", Refused::Value, schema, Some("/note")),
            ("books", Refused::Value, schema, Some("")),
        ]
    );
    assert_eq!(unchanged, Some(json!({ "title": "Dune", "pages": 412 })));
}

/// A store's calls are its Run's, so a closed Run's fail with `closed`.
/// Covers: story:11-17
#[test]
fn the_store_of_a_closed_run_is_closed() {
    let env = TestEnv::new();
    let after = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let store = run.key_value_store();
        run.close().await.unwrap();
        let after = store.set("cursor", &1).await;
        collie.close().await.unwrap();
        after
    });

    assert!(
        matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
        "{after:?}"
    );
}

/// A program without async uses the same store.
/// Covers: story:6-28, story:6-30
#[test]
fn a_program_without_async_reads_and_writes_the_store() {
    let env = TestEnv::new();
    let collie = blocking::Collie::connect(options(&env)).unwrap();
    let run = collie.start().send().unwrap();
    let store: blocking::KeyValueStore = run.key_value_store();

    store.set("page", &3).send().unwrap();
    let page = store.get::<u32>("page").send().unwrap();
    store.set_bytes("logo", [9], "image/png").send().unwrap();
    let logo: Option<StoredBytes> = store.get_bytes("logo").send().unwrap();
    let deleted = [
        store.delete("logo").send().unwrap(),
        store.delete("logo").send().unwrap(),
    ];
    run.finish().send().unwrap();
    collie.close().unwrap();

    assert_eq!(page, Some(3));
    assert_eq!(
        logo.map(|logo| (logo.content, logo.content_type)),
        Some((vec![9], "image/png".to_owned()))
    );
    assert_eq!(deleted, [true, false]);
}
