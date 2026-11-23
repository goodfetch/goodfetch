//! The Export cases of the Response decoding, Dataset and Record, and File
//! saving conformance cases on Foxhound's executable over stdio (spec
//! #582's seam: the protocol; ADRs 0073, 0103, 0116, 0119 and 0122):
//! `run.export` writes a Dataset's Records committed when the call is made,
//! in number order, as JSON or JSONL in UTF-8 with no BOM, characters as
//! themselves and each value as committed, so the Unicode record
//! round-trips byte for byte; it answers an Export only once its file is
//! whole, which `export.save_as` writes at the program's path and
//! `export.stream` sends, and which outlives the engine killed during the
//! next call. On a tmpfs of its own, a write the full filesystem refuses
//! fails with `file_write_failed`, side `engine`, leaving no unfinished
//! file, and one at the `critical` storage level fails before writing.
//! Exports and Records read under `TZ=Pacific/Chatham` and
//! `LANG=tr_TR.UTF-8` are byte-identical to those read under the host's.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "a test reads the files the engine wrote, outside any async worker"
)]

use std::path::{Path, PathBuf};

use goodfetch_test_client::Attached;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{DISPATCH_TARGET, TestEnv, unicode_record};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use goodfetch_test_support::{fill_filesystem, free_filesystem, inside_mount, tmpfs_mounted_home};

use crate::attached;
use crate::attached_spawned;
use crate::site::Site;

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The shared Unicode record as one Record's value: each field's text
/// under its name.
pub(crate) fn unicode_value() -> Value {
    Value::Object(
        unicode_record()
            .fields
            .into_iter()
            .map(|field| (field.name, Value::String(field.text)))
            .collect::<Map<_, _>>(),
    )
}

/// The Record of `books` that the crawl request of `page` commits.
pub(crate) fn book(page: u64) -> Value {
    json!({ "page": page, "title": format!("Café {page}") })
}

/// Starts a crawl `run_id` of `pages` seeds, one request at a time,
/// declaring `books`.
pub(crate) fn crawl((engine, site): (&mut Attached, &Site), run_id: &str, pages: u64) {
    crawl_with((engine, site), run_id, pages, json!({}));
}

/// [`crawl`] with `settings`, its handler's concurrency set to 1.
fn crawl_with(
    (engine, site): (&mut Attached, &Site),
    run_id: &str,
    pages: u64,
    mut settings: Value,
) {
    settings["crawl"]["handler"] = json!({ "concurrency": 1 });
    let seeds: Vec<Value> = (1..=pages)
        .map(|page| json!({ "url": site.page(&format!("/{page}")), "label": "page" }))
        .collect();
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "datasets": [{ "name": "books" }],
            "settings": settings,
            "labels": { "page": {} },
        }),
        &mut |_| None,
    );
    assert_eq!(started["result"]["run_id"], json!(run_id), "{started}");
}

/// The page number of the crawl request `params` hands the handler.
fn page_of(params: &Value) -> u64 {
    let url = params["crawl_request"]["url"].as_str().unwrap();
    url.rsplit('/').next().unwrap().parse().unwrap()
}

/// The answer of a handler that pushes `value` into `books`.
fn pushing(value: &Value) -> Value {
    json!({ "result": { "records": [{ "dataset": "books", "data": value }] } })
}

/// Finishes `run_id`, each request answered by pushing `record(page)`.
pub(crate) fn finish(engine: &mut Attached, run_id: &str, record: impl Fn(u64) -> Value) {
    let finished = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut |params| Some(pushing(&record(page_of(params)))),
    );
    assert_eq!(
        finished["result"]["lifecycle"],
        json!("finished"),
        "{finished}"
    );
}

/// `run.export`'s reply for `books` of `run_id` in `format`.
fn export(engine: &mut Attached, run_id: &str, format: &str) -> Value {
    engine.call(
        "export",
        "run.export",
        &json!({ "run_id": run_id, "dataset": "books", "format": format }),
        &mut |_| None,
    )
}

/// Saves the Export `export_id` at `path` and answers the reply.
pub(crate) fn save_as(engine: &mut Attached, export_id: &Value, path: &Path) -> Value {
    engine.call(
        "save",
        "export.save_as",
        &json!({ "export_id": export_id, "path": path }),
        &mut |_| None,
    )
}

/// The bytes of the Export `export_id` as `export.stream` sends them, and
/// its reply.
fn streamed(engine: &mut Attached, export_id: &Value) -> (Vec<u8>, Value) {
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "stream", "method": "export.stream",
        "params": { "export_id": export_id },
    }));
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "stream", "credits": 1000 },
    }));
    let mut bytes = Vec::new();
    loop {
        let message = engine.next().expect("the engine closed");
        if message["method"] == "$/chunk" {
            assert_eq!(
                message["params"]["offset"].as_u64().unwrap(),
                u64::try_from(bytes.len()).unwrap()
            );
            bytes.extend(
                data_encoding::BASE64
                    .decode(message["params"]["data"].as_str().unwrap().as_bytes())
                    .unwrap(),
            );
        } else if message["id"] == "stream" {
            return (bytes, message);
        }
    }
}

/// The Records of `books` of `run_id` as `run.records` reads them, each
/// committed time checked as UTC and left out.
fn records(engine: &mut Attached, run_id: &str) -> Vec<Value> {
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "records", "method": "run.records",
        "params": { "run_id": run_id, "dataset": "books" },
    }));
    engine.send(&json!({
        "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "records", "credits": 1000 },
    }));
    let mut read = Vec::new();
    loop {
        let message = engine.next().expect("the engine closed");
        if message["method"] == "$/record" {
            let mut record = message["params"]["record"].clone();
            let committed_at = record
                .as_object_mut()
                .unwrap()
                .remove("committed_at")
                .unwrap();
            assert!(
                committed_at.as_str().is_some_and(|at| at.ends_with('Z')),
                "{committed_at}"
            );
            read.push(record);
        } else if message["id"] == "records" {
            return read;
        }
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The Run folder of `run_id` in the test's home.
fn run_folder(env: &TestEnv, run_id: &str) -> PathBuf {
    HomeLayout::of(env.home()).run_folder(run_id)
}

/// The names in `folder`, sorted, none when it does not exist.
fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// A JSON and a JSONL Export of a Dataset holding the Unicode record are
/// UTF-8 with no BOM, write `é` as itself rather than `é`, and hold
/// each Record's value as committed, in number order: the Unicode record's
/// every field round-trips byte for byte. Each Export answers its Record
/// count, its last number, its size and its SHA-256, which the file
/// `export.save_as` writes and the bytes `export.stream` sends match.
///
/// Covers: story:6-277, operation:run.export, operation:export.save_as, operation:export.stream, format:export
#[test]
fn json_and_jsonl_exports_are_utf_8_with_no_bom_and_hold_each_value_as_committed() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    crawl((&mut engine, &site), "unicode", 3);
    finish(&mut engine, "unicode", |page| {
        if page == 2 {
            unicode_value()
        } else {
            book(page)
        }
    });
    let json = export(&mut engine, "unicode", "json");
    let jsonl = export(&mut engine, "unicode", "jsonl");
    let saved_to = env.home().join("saved").join("books.json");
    let saved = save_as(&mut engine, &json["result"]["export_id"], &saved_to);
    let (sent, stream_reply) = streamed(&mut engine, &jsonl["result"]["export_id"]);
    assert!(engine.finished());

    let file = std::fs::read(&saved_to).unwrap();
    let text = String::from_utf8(file.clone()).unwrap();
    let lines = String::from_utf8(sent.clone()).unwrap();
    let parsed: Vec<Value> = serde_json::from_str(&text).unwrap();
    let parsed_lines: Vec<Value> = lines
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let described = |reply: &Value, bytes: &[u8]| {
        (
            reply["result"]["records"].clone(),
            reply["result"]["last_number"].clone(),
            reply["result"]["size"].as_u64() == Some(u64::try_from(bytes.len()).unwrap()),
            reply["result"]["sha256"].as_str() == Some(sha256(bytes).as_str()),
        )
    };
    let expected = vec![book(1), unicode_value(), book(3)];
    assert_eq!(
        (
            (
                file.starts_with(b"\xEF\xBB\xBF"),
                sent.starts_with(b"\xEF\xBB\xBF")
            ),
            (text.contains("Café 1"), text.contains("\\u00e9")),
            text.starts_with(r#"[{"page":1,"title":"Café 1"},"#),
            lines.ends_with("{\"page\":3,\"title\":\"Café 3\"}\n"),
            (&parsed, &parsed_lines),
            (described(&json, &file), described(&jsonl, &sent)),
            (&saved["result"]["written"], &saved["result"]["sha256"]),
            &stream_reply["result"]["sha256"],
        ),
        (
            (false, false),
            (true, false),
            true,
            true,
            (&expected, &expected),
            (
                (json!(3), json!(3), true, true),
                (json!(3), json!(3), true, true)
            ),
            (&json!(true), &json["result"]["sha256"]),
            &jsonl["result"]["sha256"],
        )
    );
}

/// The next crawl request the engine hands the handler, other messages
/// dropped.
fn next_request(engine: &mut Attached) -> Value {
    loop {
        let message = engine.next().expect("the engine closed");
        if message["method"] == "crawl_request.handle" {
            return message;
        }
    }
}

/// Answers the crawl request `request` by pushing its page's book.
fn answer(engine: &mut Attached, request: &Value) {
    let mut reply = pushing(&book(page_of(&request["params"])));
    reply["jsonrpc"] = json!("2.0");
    reply["id"] = request["id"].clone();
    engine.send(&reply);
}

/// The values of a JSONL Export's `bytes`, in order.
fn lines_of(bytes: &[u8]) -> Vec<Value> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// An Export holds the Records committed when the call is made, in number
/// order: one made while the Run is live, two Records committed and the
/// third request held by its handler, holds those two; the third,
/// committed after it, is in the Export made once the Run finished.
///
/// Covers: story:6-277, operation:run.export
#[test]
fn an_export_holds_the_records_committed_when_the_call_is_made_in_number_order() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    // At `full` each answered request's Record is committed as it ends: at
    // `batch` a reader sees it only once its batch is written (ADR 0142).
    crawl_with(
        (&mut engine, &site),
        "live",
        3,
        json!({ "run_state": { "durability": "full" } }),
    );
    for _ in 0..2 {
        let request = next_request(&mut engine);
        answer(&mut engine, &request);
    }
    let held = next_request(&mut engine);
    let while_live = export(&mut engine, "live", "jsonl");
    answer(&mut engine, &held);
    finish(&mut engine, "live", book);
    let after = export(&mut engine, "live", "jsonl");
    let live_bytes = streamed(&mut engine, &while_live["result"]["export_id"]).0;
    let after_bytes = streamed(&mut engine, &after["result"]["export_id"]).0;
    assert!(engine.finished());

    assert_eq!(
        (
            (
                &while_live["result"]["records"],
                &while_live["result"]["last_number"]
            ),
            lines_of(&live_bytes),
            (&after["result"]["records"], &after["result"]["last_number"]),
            lines_of(&after_bytes),
        ),
        (
            (&json!(2), &json!(2)),
            vec![book(1), book(2)],
            (&json!(3), &json!(3)),
            vec![book(1), book(2), book(3)],
        )
    );
}

/// An Export the engine answered is whole even when the engine is killed
/// during the next call, another Export: a new engine on the same home
/// saves it with the bytes its SHA-256 names, and every file the Run's
/// Exports folder shows is whole, named by its own bytes' SHA-256, a killed
/// write leaving at most a hidden temporary file; the same Export made there
/// has the same SHA-256.
///
/// Covers: story:6-280, operation:run.export, operation:export.save_as
#[test]
fn an_export_answered_is_whole_after_the_engine_is_killed_during_the_next_call() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    crawl((&mut engine, &site), "killed", 200);
    finish(&mut engine, "killed", book);
    let held = export(&mut engine, "killed", "json");
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "next", "method": "run.export",
        "params": { "run_id": "killed", "dataset": "books", "format": "jsonl" },
    }));
    engine.kill();

    let mut later = attached(&env);
    let saved_to = env.home().join("books.json");
    let saved = save_as(&mut later, &held["result"]["export_id"], &saved_to);
    let exports = run_folder(&env, "killed").join("exports").join("1");
    let visible_whole: Vec<bool> = names_in(&exports)
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .map(|name| sha256(&std::fs::read(exports.join(&name)).unwrap()) == name)
        .collect();
    // The same Export again, on the engine that exits as it should.
    let again = export(&mut later, "killed", "json");
    assert!(later.finished());
    let file = std::fs::read(&saved_to).unwrap();
    let values: Vec<Value> = serde_json::from_slice(&file).unwrap();
    assert_eq!(
        (
            &saved["result"]["sha256"],
            sha256(&file).as_str() == held["result"]["sha256"],
            values.len(),
            !visible_whole.is_empty() && visible_whole.iter().all(|whole| *whole),
            &again["result"]["sha256"],
        ),
        (
            &held["result"]["sha256"],
            true,
            200,
            true,
            &held["result"]["sha256"]
        )
    );
}

/// An Export carries the run epoch of the Run's state it was written from:
/// a crawl resumed after its engine was killed writes under run epoch 2,
/// which its answer, its ID and its kept file's folder carry, and which
/// `export.save_as` and `export.stream` answer too, so a destination that
/// compares and sets can refuse a fenced copy's Export (ADR 0003).
///
/// Covers: story:3-43, operation:run.export, operation:export.save_as, operation:export.stream
#[test]
fn an_export_carries_the_run_epoch_of_the_state_it_was_written_from() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut killed = site.engine(&env);
    crawl((&mut killed, &site), "resumed", 2);
    killed.kill();
    let mut engine = site.engine(&env);
    let resumed = engine.call(
        "resume",
        "run.resume",
        &json!({ "run_id": "resumed" }),
        &mut |_| None,
    );
    finish(&mut engine, "resumed", book);
    let exported = export(&mut engine, "resumed", "jsonl");
    let saved = save_as(
        &mut engine,
        &exported["result"]["export_id"],
        &env.home().join("books.jsonl"),
    );
    let (_, sent) = streamed(&mut engine, &exported["result"]["export_id"]);
    assert!(engine.finished());

    let sha256 = exported["result"]["sha256"].as_str().unwrap().to_owned();
    let exports = run_folder(&env, "resumed").join("exports");
    assert_eq!(
        (
            &resumed["result"]["started_as"],
            &exported["result"]["records"],
            &exported["result"]["run_epoch"],
            &exported["result"]["export_id"],
            (&saved["result"]["run_epoch"], &sent["result"]["run_epoch"]),
            (names_in(&exports), names_in(&exports.join("2"))),
        ),
        (
            &json!("resumed"),
            &json!(2),
            &json!(2),
            &json!(format!("resumed.export-2-{sha256}")),
            (&json!(2), &json!(2)),
            (vec!["2".to_owned()], vec![sha256]),
        )
    );
}

/// An Export the engine answered stays in the test's exercise record when
/// the engine is killed next, whatever its log kept: this engine's log
/// keeps nothing (`log.output = "none"`), the worst a kill leaves of the
/// lines its log writer's thread had not written yet (ADR 0122, #1779). The
/// record holds each call the Client read answered, in its order.
///
/// Covers: operation:run.export
#[test]
fn an_export_answered_before_the_engine_is_killed_stays_in_the_tests_exercise_record() {
    let env = TestEnv::new();
    env.user_configuration("log.output = \"none\"");
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    crawl((&mut engine, &site), "killed", 1);
    finish(&mut engine, "killed", book);
    let exported = export(&mut engine, "killed", "jsonl");
    engine.kill();

    let dispatched: Vec<Value> = env
        .exercise_lines()
        .into_iter()
        .filter(|line| line["target"] == DISPATCH_TARGET)
        .map(|line| line["fields"]["method"].clone())
        .collect();
    assert_eq!(
        (&exported["result"]["records"], dispatched),
        (
            &json!(1),
            vec![
                json!("hello"),
                json!("crawl.start"),
                json!("run.finish"),
                json!("run.export")
            ]
        )
    );
}

/// An Export of a Dataset the crawl does not declare fails with
/// `dataset_unknown`, and a save of an Export ID no engine answered with
/// `invalid_params`.
///
/// Covers: operation:run.export, operation:export.save_as, error:dataset_unknown
#[test]
fn an_export_of_a_dataset_the_crawl_does_not_declare_fails_with_dataset_unknown() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    crawl((&mut engine, &site), "declared", 1);
    finish(&mut engine, "declared", book);
    let unknown = engine.call(
        "unknown",
        "run.export",
        &json!({ "run_id": "declared", "dataset": "pages", "format": "json" }),
        &mut |_| None,
    );
    let named = format!("declared.export-1-{}", "0".repeat(64));
    let no_export = save_as(&mut engine, &json!(named), &env.home().join("x.json"));
    assert!(engine.finished());

    assert_eq!(
        (
            &unknown["error"]["data"]["code"],
            &unknown["error"]["data"]["declared"],
            &no_export["error"]["code"],
        ),
        (&json!("dataset_unknown"), &json!(["books"]), &json!(-32602))
    );
}

/// What one host's crawl gives: its JSON and JSONL Exports' bytes and its
/// Records as `run.records` reads them.
fn crawled_on(env: &TestEnv, site: &Site) -> (Vec<u8>, Vec<u8>, Vec<Value>) {
    let mut engine = site.engine(env);
    crawl((&mut engine, site), "hosted", 3);
    finish(&mut engine, "hosted", |page| {
        if page == 2 {
            unicode_value()
        } else {
            book(page)
        }
    });
    let json = export(&mut engine, "hosted", "json");
    let jsonl = export(&mut engine, "hosted", "jsonl");
    assert_eq!(
        (&json["result"]["records"], &jsonl["result"]["records"]),
        (&json!(3), &json!(3)),
        "{json} {jsonl}"
    );
    let json = streamed(&mut engine, &json["result"]["export_id"]).0;
    let jsonl = streamed(&mut engine, &jsonl["result"]["export_id"]).0;
    let read = records(&mut engine, "hosted");
    assert!(engine.finished());
    (json, jsonl, read)
}

/// Exports and Records never depend on the host's time zone or locale
/// (ADR 0073): a crawl run under `TZ=Pacific/Chatham` and
/// `LANG=tr_TR.UTF-8` gives the same Export bytes and the same Records,
/// each committed time in UTC, as the same crawl under the host's own.
///
/// Covers: story:6-277, operation:run.export, operation:run.records
#[test]
fn exports_and_records_under_another_time_zone_and_locale_are_byte_identical() {
    let host = TestEnv::new();
    let elsewhere = TestEnv::new()
        .with_time_zone("Pacific/Chatham")
        .unwrap()
        .with_locale("tr-TR")
        .unwrap();

    let site = Site::start(&host);

    let on_host = crawled_on(&host, &site);
    let on_elsewhere = crawled_on(&elsewhere, &site);

    assert_eq!(on_elsewhere, on_host);
}

const MIB: u64 = 1 << 20;

/// The tmpfs's size over the home.
const SIZE: u64 = 64 * MIB;

/// An Attached engine on a tmpfs of its own over the test's home, whose
/// user configuration holds `settings`, with a finished crawl `run_id` of
/// 40 Records of about 50 kB each in `books`.
fn crawled_on_a_tmpfs((env, site): (&TestEnv, &Site), settings: &str, run_id: &str) -> Attached {
    env.user_configuration(settings);
    let mut command = tmpfs_mounted_home(env, SIZE, FOXHOUND);
    command.arg("attached-engine");
    let mut engine =
        attached_spawned(env, None, &mut command).settings_by_default(&site.settings());
    crawl((&mut engine, site), run_id, 40);
    finish(
        &mut engine,
        run_id,
        |page| json!({ "page": page, "text": "é".repeat(25_000) }),
    );
    engine
}

/// A write the engine's storage refuses mid-file fails `run.export` with
/// `file_write_failed`, side `engine`, reason `storage_full`, naming the
/// Run's folder, and leaves no unfinished file there, hidden or not; once
/// space is freed the same call writes the Export.
///
/// Covers: story:6-280, operation:run.export, error:file_write_failed
#[test]
fn a_write_the_full_filesystem_refuses_fails_with_file_write_failed_and_leaves_no_file() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = crawled_on_a_tmpfs(
        (&env, &site),
        "storage.low_free = 0\nstorage.critical_free = 0\n",
        "full",
    );
    let pid = engine.pid();
    let folder = run_folder(&env, "full");
    let filler = fill_filesystem(pid, env.home(), 512 * 1024);
    let failed = export(&mut engine, "full", "json");
    let exports = folder.join("exports").join("1");
    let left = names_in(&inside_mount(pid, &exports));
    free_filesystem(&filler);
    let written = export(&mut engine, "full", "json");
    let kept = names_in(&inside_mount(pid, &exports));
    assert!(engine.finished());

    let data = &failed["error"]["data"];
    assert_eq!(
        (
            (
                &data["code"],
                &data["side"],
                &data["reason"],
                &data["os_code"]
            ),
            &data["path"],
            left,
            (&written["result"]["records"], kept),
        ),
        (
            (
                &json!("file_write_failed"),
                &json!("engine"),
                &json!("storage_full"),
                &json!("ENOSPC")
            ),
            &json!(folder.display().to_string()),
            Vec::<String>::new(),
            (
                &json!(40),
                vec![written["result"]["sha256"].as_str().unwrap().to_owned()]
            ),
        )
    );
}

/// While the Run folder's filesystem is at the `critical` storage level,
/// `run.export` fails at once with `file_write_failed`, side `engine`,
/// reason `storage_full`, before writing a byte: the Run's folder holds no
/// Exports folder at all (ADR 0122).
///
/// Covers: story:6-280, operation:run.export, error:file_write_failed, setting:storage.critical_free
#[test]
fn an_export_at_the_critical_storage_level_fails_before_writing() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = crawled_on_a_tmpfs(
        (&env, &site),
        "storage.low_free = 41943040\nstorage.critical_free = 20971520\n",
        "critical",
    );
    let pid = engine.pid();
    let folder = run_folder(&env, "critical");
    let filler = fill_filesystem(pid, env.home(), 10 * MIB);
    let failed = export(&mut engine, "critical", "json");
    let exports = inside_mount(pid, &folder.join("exports")).exists();
    free_filesystem(&filler);
    assert!(engine.finished());

    let data = &failed["error"]["data"];
    assert_eq!(
        (
            (&data["code"], &data["side"], &data["reason"]),
            &data["path"],
            exports,
        ),
        (
            (
                &json!("file_write_failed"),
                &json!("engine"),
                &json!("storage_full")
            ),
            &json!(folder.display().to_string()),
            false,
        )
    );
}

/// `run.export`'s reply for `books` of `run_id` in `format`, under the
/// run layer `settings`.
pub(crate) fn export_under(
    engine: &mut Attached,
    run_id: &str,
    format: &str,
    settings: &Value,
) -> Value {
    engine.call(
        "export",
        "run.export",
        &json!({ "run_id": run_id, "dataset": "books", "format": format, "settings": settings }),
        &mut |_| None,
    )
}

/// The bytes of the Export `reply` answered, saved by `export.save_as` in
/// the test's home as `name`.
pub(crate) fn saved(engine: &mut Attached, env: &TestEnv, reply: &Value, name: &str) -> Vec<u8> {
    let path = env.home().join("saved").join(name);
    let saved = save_as(engine, &reply["result"]["export_id"], &path);
    assert_eq!(saved["result"]["written"], json!(true), "{reply} {saved}");
    std::fs::read(path).unwrap()
}

/// The rows a CSV reader reads from `bytes`, its header row first.
pub(crate) fn csv_rows(bytes: &[u8], delimiter: u8) -> Vec<Vec<String>> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .delimiter(delimiter)
        .from_reader(bytes)
        .records()
        .map(|record| record.unwrap().iter().map(str::to_owned).collect())
        .collect()
}

/// The text of the ZIP entry `name` in `archive`.
pub(crate) fn entry(archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str) -> String {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut archive.by_name(name).unwrap(), &mut text).unwrap();
    text
}

/// The value of `element`'s attribute `name`, unescaped.
pub(crate) fn attribute(
    element: &quick_xml::events::BytesStart<'_>,
    name: &[u8],
) -> Option<String> {
    element
        .attributes()
        .map(Result::unwrap)
        .find(|attribute| attribute.key.as_ref() == name)
        .map(|attribute| {
            attribute
                .normalized_value(quick_xml::XmlVersion::Explicit1_0)
                .unwrap()
                .into_owned()
        })
}

/// The column index of a cell reference such as `AB12`, from 0.
pub(crate) fn column_of(reference: &str) -> usize {
    reference
        .bytes()
        .take_while(u8::is_ascii_uppercase)
        .fold(0, |index, letter| {
            index * 26 + usize::from(letter - b'A' + 1)
        })
        - 1
}

/// The sheets' names and the rows of the first in the XLSX `bytes`, read as
/// ECMA-376 lays a workbook out: the ZIP (the `zip` crate), the workbook's
/// `sheet` elements, and the sheet's cells placed by their references
/// (`quick-xml`), each an inline string, a boolean or a number, as a
/// spreadsheet reads them; the header row first. No spreadsheet library
/// fits the build: calamine turns on `flate2`'s zlib-rs backend for the
/// whole workspace, which changes the Proving ground's compressed bodies.
pub(crate) fn xlsx_rows(bytes: Vec<u8>) -> (Vec<String>, Vec<Vec<String>>) {
    use quick_xml::Reader;
    use quick_xml::escape::resolve_predefined_entity;
    use quick_xml::events::Event;

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let workbook = entry(&mut archive, "xl/workbook.xml");
    let mut reader = Reader::from_str(&workbook);
    let mut names = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == b"sheet" =>
            {
                names.push(attribute(&element, b"name").unwrap());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let sheet = entry(&mut archive, "xl/worksheets/sheet1.xml");
    let mut reader = Reader::from_str(&sheet);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let (mut column, mut kind, mut text, mut inside) = (0, String::new(), String::new(), false);
    loop {
        match reader.read_event().unwrap() {
            Event::Start(element) | Event::Empty(element) => match element.local_name().as_ref() {
                b"row" => rows.push(Vec::new()),
                b"c" => {
                    column = column_of(&attribute(&element, b"r").unwrap());
                    kind = attribute(&element, b"t").unwrap_or_default();
                    text.clear();
                }
                b"t" | b"v" => inside = true,
                _ => {}
            },
            Event::Text(part) if inside => text.push_str(&part.decode().unwrap()),
            Event::GeneralRef(reference) if inside => match reference.resolve_char_ref().unwrap() {
                Some(character) => text.push(character),
                None => {
                    text.push_str(resolve_predefined_entity(&reference.decode().unwrap()).unwrap());
                }
            },
            Event::End(element) => match element.local_name().as_ref() {
                b"t" | b"v" => inside = false,
                b"c" => {
                    let value = match kind.as_str() {
                        "b" => (text == "1").to_string(),
                        "inlineStr" | "str" => text.clone(),
                        _ => {
                            let number: f64 = text.parse().unwrap();
                            if number.fract() == 0.0 {
                                format!("{number:.0}")
                            } else {
                                number.to_string()
                            }
                        }
                    };
                    let row = rows.last_mut().unwrap();
                    row.resize(column, String::new());
                    row.push(value);
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(width, String::new());
    }
    (names, rows)
}

/// The title and the rows an HTML parser reads in `text`, its header row
/// first.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a test's own parse, on no Shared path"
)]
pub(crate) fn html_rows(text: &str) -> (String, Vec<Vec<String>>) {
    let document = scraper::Html::parse_document(text);
    let title = scraper::Selector::parse("title").unwrap();
    let rows = scraper::Selector::parse("tr").unwrap();
    let cells = scraper::Selector::parse("th, td").unwrap();
    (
        document
            .select(&title)
            .next()
            .unwrap()
            .text()
            .collect::<String>(),
        document
            .select(&rows)
            .map(|row| {
                row.select(&cells)
                    .map(|cell| cell.text().collect::<String>())
                    .collect()
            })
            .collect(),
    )
}

/// The rows of the table a Markdown parser reads in `text`, its header row
/// first, an inline HTML line break kept as its text.
pub(crate) fn markdown_rows(text: &str) -> Vec<Vec<String>> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    for event in Parser::new_ext(text, Options::ENABLE_TABLES) {
        match event {
            Event::Start(Tag::TableCell) => cell.clear(),
            Event::Text(text) | Event::InlineHtml(text) | Event::Code(text) => {
                cell.push_str(&text);
            }
            Event::End(TagEnd::TableCell) => row.push(std::mem::take(&mut cell)),
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                rows.push(std::mem::take(&mut row));
            }
            _ => {}
        }
    }
    rows
}

/// The text a table Export's cell holds for `value`: nothing for none or
/// `null`, a string's text, and JSON's text of anything else.
pub(crate) fn cell_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

/// The rows a table of `records` under `columns` holds, its header row
/// first.
pub(crate) fn table_of(columns: &[&str], records: &[Value]) -> Vec<Vec<String>> {
    let header = columns.iter().map(|&name| name.to_owned()).collect();
    std::iter::once(header)
        .chain(records.iter().map(|record| {
            columns
                .iter()
                .map(|&name| cell_text(record.get(name)))
                .collect()
        }))
        .collect()
}

/// The third book, which adds fields the first two lack: a nested object,
/// a boolean, a `null`, a number and an array.
pub(crate) fn third_book() -> Value {
    json!({
        "author": { "name": "Zoé" },
        "in_stock": true,
        "note": null,
        "page": 3,
        "price": 12.5,
        "tags": ["roman", "poche"],
        "title": "Café 3",
    })
}

/// Pitfall D07's acceptance test: CSV, XLSX, HTML table and Markdown
/// Exports of Records whose later ones add fields carry every field, in
/// first-seen order, a field a Record lacks as an empty cell; each opens in
/// a reference reader (a CSV reader, a spreadsheet library, an HTML parser,
/// a Markdown parser) and gives the Records back, the Unicode record among
/// them. CSV carries no BOM by default, and the XLSX sheet is named after
/// the Dataset.
///
/// Covers: story:6-278, story:6-277, operation:run.export, operation:export.save_as, format:export
#[test]
fn csv_xlsx_html_and_markdown_exports_carry_every_field_seen_in_first_seen_order() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    crawl((&mut engine, &site), "tables", 3);
    finish(&mut engine, "tables", |page| match page {
        1 => book(1),
        2 => unicode_value(),
        _ => third_book(),
    });
    let none = json!({});
    let csv = export_under(&mut engine, "tables", "csv", &none);
    let xlsx = export_under(&mut engine, "tables", "xlsx", &none);
    let html = export_under(&mut engine, "tables", "html_table", &none);
    let markdown = export_under(&mut engine, "tables", "markdown", &none);
    let csv_bytes = saved(&mut engine, &env, &csv, "books.csv");
    let xlsx_bytes = saved(&mut engine, &env, &xlsx, "books.xlsx");
    let html_bytes = saved(&mut engine, &env, &html, "books.html");
    let markdown_bytes = saved(&mut engine, &env, &markdown, "books.md");
    assert!(engine.finished());

    let columns = [
        "page",
        "title",
        "astral",
        "bidi",
        "bom",
        "nfc",
        "nfd",
        "tab_and_newline",
        "zero_width_joiner",
        "zwj_emoji",
        "author",
        "in_stock",
        "note",
        "price",
        "tags",
    ];
    let expected = table_of(&columns, &[book(1), unicode_value(), third_book()]);
    let in_markdown: Vec<Vec<String>> = expected
        .iter()
        .map(|row| row.iter().map(|cell| cell.replace('\n', "<br>")).collect())
        .collect();
    assert_eq!(
        (
            csv_bytes.starts_with(b"\xEF\xBB\xBF"),
            csv_rows(&csv_bytes, b','),
            xlsx_rows(xlsx_bytes),
            html_rows(&String::from_utf8(html_bytes).unwrap()),
            markdown_rows(&String::from_utf8(markdown_bytes).unwrap()),
            [&csv, &xlsx, &html, &markdown].map(|reply| reply["result"]["records"].clone()),
        ),
        (
            false,
            expected.clone(),
            (vec!["books".to_owned()], expected.clone()),
            ("books".to_owned(), expected),
            in_markdown,
            [json!(3), json!(3), json!(3), json!(3)],
        )
    );
}

/// `export.csv.bom` puts a BOM first, `export.csv.delimiter` sets the
/// character between fields, `export.csv.nested: flatten` writes an
/// object's fields in dotted columns, and `export.xlsx.sheet` names the
/// sheet; a delimiter of two characters and a sheet name holding `/` fail
/// with `setting_invalid` naming the key.
///
/// Covers: story:6-278, story:6-277, operation:run.export, setting:export.csv.bom, setting:export.csv.delimiter, setting:export.csv.nested, setting:export.xlsx.sheet, error:setting_invalid
#[test]
fn the_export_settings_shape_csv_and_name_the_xlsx_sheet() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    crawl((&mut engine, &site), "shaped", 3);
    finish(&mut engine, "shaped", |page| {
        if page == 3 { third_book() } else { book(page) }
    });
    let csv = export_under(
        &mut engine,
        "shaped",
        "csv",
        &json!({ "export": { "csv": { "bom": true, "delimiter": ";", "nested": "flatten" } } }),
    );
    let xlsx = export_under(
        &mut engine,
        "shaped",
        "xlsx",
        &json!({ "export": { "xlsx": { "sheet": "Livres 2026" } } }),
    );
    let two = export_under(
        &mut engine,
        "shaped",
        "csv",
        &json!({ "export": { "csv": { "delimiter": ";;" } } }),
    );
    let slashed = export_under(
        &mut engine,
        "shaped",
        "xlsx",
        &json!({ "export": { "xlsx": { "sheet": "a/b" } } }),
    );
    let csv_bytes = saved(&mut engine, &env, &csv, "books.csv");
    let xlsx_bytes = saved(&mut engine, &env, &xlsx, "books.xlsx");
    assert!(engine.finished());

    let flattened = json!({
        "author.name": "Zoé",
        "in_stock": true,
        "page": 3,
        "price": 12.5,
        "tags": ["roman", "poche"],
        "title": "Café 3",
    });
    let refused = |reply: &Value| {
        (
            reply["error"]["data"]["code"].clone(),
            reply["error"]["data"]["key"].clone(),
        )
    };
    assert_eq!(
        (
            csv_bytes.starts_with(b"\xEF\xBB\xBF"),
            csv_rows(&csv_bytes[3..], b';'),
            xlsx_rows(xlsx_bytes).0,
            refused(&two),
            refused(&slashed),
        ),
        (
            true,
            table_of(
                &[
                    "page",
                    "title",
                    "author.name",
                    "in_stock",
                    "note",
                    "price",
                    "tags"
                ],
                &[book(1), book(2), flattened],
            ),
            vec!["Livres 2026".to_owned()],
            (json!("setting_invalid"), json!("export.csv.delimiter")),
            (json!("setting_invalid"), json!("export.xlsx.sheet")),
        )
    );
}
