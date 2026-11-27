//! A crawl's Sinks (ADRs 0026 and 0103), on Foxhound's executable over
//! stdio against the Proving ground: `sinks.<name>.*` on the crawl's Run
//! names where its Datasets' Records go, here the in-binary SQLite Sink
//! whose `destination` is a Secret reference to the database's path, and
//! the crawl itself stays the same.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;

use goodfetch_test_support::{ProvingGround, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::code_free::crawl_read;

/// The Proving ground's seed.
const SEED: u64 = 994;

/// The Markdown task, whose index links its Markdown pages.
const MARKDOWN_TASK: &str = "extraction/7";

/// The variable the Sink's `destination` reads the database's path from,
/// outside the `GOODFETCH_` prefix that ADR 0034 reserves.
const DESTINATION: &str = "SINK_TEST_DATABASE";

/// The variable a second Sink's `destination` reads a path in a missing
/// folder from.
const MISSING: &str = "SINK_TEST_MISSING";

/// One row the SQLite Sink wrote.
#[derive(Debug, PartialEq)]
struct Row {
    delivery_id: String,
    run_id: String,
    run_epoch: i64,
    dataset: String,
    number: i64,
    record: Value,
}

/// The rows of the table `records` of the database at `path`, by Record
/// number, reconstructing each Record from its non-`_goodfetch_*` columns.
fn rows(path: &Path) -> Vec<Row> {
    let connection = Connection::open(path).unwrap();
    let user_cols: Vec<String> = connection
        .prepare_cached("SELECT name FROM pragma_table_info('records') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .into_iter()
        .filter(|name| !name.starts_with("_goodfetch_"))
        .collect();
    let extra = if user_cols.is_empty() {
        String::new()
    } else {
        format!(
            ", {}",
            user_cols
                .iter()
                .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let mut select = connection
        .prepare(&format!(
            "SELECT _goodfetch_delivery_id, _goodfetch_run_id, _goodfetch_run_epoch,
                    _goodfetch_dataset, _goodfetch_record_number{extra}
             FROM records ORDER BY _goodfetch_record_number"
        ))
        .unwrap();
    select
        .query_map((), |row| {
            let mut map = serde_json::Map::new();
            for (idx, name) in user_cols.iter().enumerate() {
                let val = match row.get_ref(5 + idx)? {
                    rusqlite::types::ValueRef::Null => continue,
                    rusqlite::types::ValueRef::Integer(i) => Value::from(i),
                    rusqlite::types::ValueRef::Real(f) => Value::from(f),
                    rusqlite::types::ValueRef::Text(bytes) => {
                        let s = std::str::from_utf8(bytes)
                            .map_err(|err| rusqlite::Error::Utf8Error(5 + idx, err))?;
                        Value::String(s.to_owned())
                    }
                    rusqlite::types::ValueRef::Blob(bytes) => Value::from(bytes.to_vec()),
                };
                map.insert(name.clone(), val);
            }
            Ok(Row {
                delivery_id: row.get(0)?,
                run_id: row.get(1)?,
                run_epoch: row.get(2)?,
                dataset: row.get(3)?,
                number: row.get(4)?,
                record: Value::Object(map),
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// A code-free crawl whose Run names a SQLite Sink of its Dataset `pages`
/// ends with every Record of `pages` in the Sink's table, each row
/// carrying its delivery ID, its Dataset and number and the run epoch: set
/// on the Run, the Sink moves the data and the crawl's own declaration is
/// unchanged. The Run requires `fenced` of its Sinks, which the SQLite
/// Sink's probe proves.
///
/// Covers: story:6-282, story:6-283, story:6-285, operation:crawl.start, setting:crawl.min_sink_guarantee
#[test]
fn a_crawls_records_reach_the_sqlite_sink_its_run_names() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let database = env.folder().join("sink.sqlite");
    let mut engine = crate::attached_with(&env, None, |command| {
        command.env(DESTINATION, &database);
    });
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        (
            "delivered",
            &[
                json!({ "url": index, "label": "page" }),
                json!({ "url": format!("{index}/headings/levels"), "label": "page" }),
            ],
        ),
        &json!({ "formats": { "markdown": true }, "follow": "none" }),
        &json!({
            "fetch": { "tier": "http" },
            "crawl": { "min_sink_guarantee": "fenced" },
            "sinks": {
                "mine": {
                    "type": "sqlite",
                    "datasets": ["pages"],
                    "destination": format!("env:{DESTINATION}"),
                },
            },
        }),
    );
    assert!(started.get("result").is_some(), "{started}");
    assert!(engine.finished());

    let epoch: i64 = crate::run_file(&env, "delivered")
        .prepare_cached("SELECT epoch FROM run")
        .unwrap()
        .query_row((), |row| row.get(0))
        .unwrap();
    let expected: Vec<Row> = records
        .iter()
        .map(|record| {
            let number = record["number"].as_i64().unwrap();
            Row {
                delivery_id: format!("delivered/mine/pages/{number}"),
                run_id: "delivered".to_owned(),
                run_epoch: epoch,
                dataset: "pages".to_owned(),
                number,
                record: record["value"].clone(),
            }
        })
        .collect();
    assert_eq!(expected.len(), 2, "{records:?}");
    assert_eq!(rows(&database), expected);
}

/// Two Runs crawling the same pages into a SQLite Sink whose write mode is
/// `replace` leave its table holding the second Run's Records alone, each
/// once: the Sink names no `key`, so each Record is keyed on its Dataset's,
/// the `url` of `pages`, and the finished Run's Records are swapped in at
/// its end.
///
/// Covers: story:6-286, operation:crawl.start
#[test]
fn a_replace_sink_holds_the_last_finished_runs_records_alone() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let database = env.folder().join("sink.sqlite");
    let mut engine = crate::attached_with(&env, None, |command| {
        command.env(DESTINATION, &database);
    });
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let seeds = [
        json!({ "url": index, "label": "page" }),
        json!({ "url": format!("{index}/headings/levels"), "label": "page" }),
    ];
    let settings = json!({
        "fetch": { "tier": "http" },
        "sinks": {
            "mine": {
                "type": "sqlite",
                "datasets": ["pages"],
                "mode": "replace",
                "destination": format!("env:{DESTINATION}"),
            },
        },
    });
    let extraction = json!({ "formats": { "markdown": true }, "follow": "none" });

    let mut records = Vec::new();
    for run_id in ["first", "second"] {
        let (started, read) = crawl_read(
            &mut engine,
            &proving_ground,
            (run_id, &seeds),
            &extraction,
            &settings,
        );
        assert!(started.get("result").is_some(), "{started}");
        records = read;
    }

    let held: Vec<(String, i64, Value)> = rows(&database)
        .into_iter()
        .map(|row| (row.run_id, row.number, row.record))
        .collect();
    let expected: Vec<(String, i64, Value)> = records
        .iter()
        .map(|record| {
            (
                "second".to_owned(),
                record["number"].as_i64().unwrap(),
                record["value"].clone(),
            )
        })
        .collect();
    assert_eq!(expected.len(), 2, "{records:?}");
    assert_eq!(held, expected);
}

/// A Sink of a `type` this engine does not deliver to yet refuses the
/// crawl at its start with `capability_unavailable`, naming the entry's
/// `type`, rather than run with Records going nowhere.
///
/// Covers: story:6-282, operation:crawl.start
#[test]
fn a_sink_of_a_type_not_built_refuses_the_crawl_at_its_start() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::attached(&env);
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();

    let (started, _) = crawl_read(
        &mut engine,
        &proving_ground,
        ("refused", &[json!({ "url": index, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "none" }),
        &json!({
            "fetch": { "tier": "http" },
            "sinks": {
                "warehouse": {
                    "type": "postgres",
                    "datasets": ["pages"],
                    "destination": "env:WAREHOUSE_DSN",
                },
            },
        }),
    );

    let refused = &started["error"]["data"];
    assert_eq!(
        (&refused["code"], &refused["key"], &refused["kind"]),
        (
            &json!("capability_unavailable"),
            &json!("sinks.warehouse.type"),
            &json!("postgres"),
        ),
        "{started}"
    );
}

/// A Run requiring `fenced` of a SQLite Sink whose database cannot open,
/// its folder missing, is refused at its start with
/// `sink_guarantee_unmet`, naming the Sink, the class required and the
/// probe's cause, and fetches nothing.
///
/// Covers: story:6-284, operation:crawl.start, error:sink_guarantee_unmet, setting:crawl.min_sink_guarantee
#[test]
fn a_sink_that_cannot_prove_the_runs_minimum_refuses_the_crawl_before_any_fetch() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let database = env.folder().join("missing").join("sink.sqlite");
    let mut engine = crate::attached_with(&env, None, |command| {
        command.env(DESTINATION, &database);
    });
    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let received_before = proving_ground.received_requests().unwrap().every().count();

    let (started, records) = crawl_read(
        &mut engine,
        &proving_ground,
        ("unmet", &[json!({ "url": index, "label": "page" })]),
        &json!({ "formats": { "markdown": true }, "follow": "none" }),
        &json!({
            "fetch": { "tier": "http" },
            "crawl": { "min_sink_guarantee": "fenced" },
            "sinks": {
                "mine": {
                    "type": "sqlite",
                    "datasets": ["pages"],
                    "destination": format!("env:{DESTINATION}"),
                },
            },
        }),
    );

    let refused = &started["error"]["data"];
    assert_eq!(
        (&refused["code"], &refused["sink"], &refused["required"]),
        (
            &json!("sink_guarantee_unmet"),
            &json!("mine"),
            &json!("fenced"),
        ),
        "{started}"
    );
    assert!(refused.get("proved").is_none(), "{started}");
    let cause = refused["cause"].as_str().unwrap_or_default();
    assert!(cause.contains("did not open"), "{started}");
    assert!(
        !cause.contains("sink.sqlite"),
        "the cause shows the path: {started}"
    );
    assert_eq!(records, Vec::<Value>::new());
    assert_eq!(
        proving_ground.received_requests().unwrap().every().count(),
        received_before,
        "the refused Run fetched"
    );
}

/// `sink.probe` probes a Sink by its name among the settings' `sinks`
/// without a Run: a SQLite Sink whose database opens proves `fenced`, one
/// whose folder is missing fails with its cause, and a name the settings
/// do not hold fails naming those they do.
///
/// Covers: story:6-283, operation:sink.probe
#[test]
fn sink_probe_answers_the_class_a_sink_proves_or_why_it_failed() {
    let env = TestEnv::new();
    let database = env.folder().join("probed.sqlite");
    let missing = env.folder().join("missing").join("probed.sqlite");
    let mut engine = crate::attached_with(&env, None, |command| {
        command.env(DESTINATION, &database).env(MISSING, &missing);
    });
    let settings = json!({
        "sinks": {
            "mine": {
                "type": "sqlite",
                "datasets": ["pages"],
                "destination": format!("env:{DESTINATION}"),
            },
            "lost": {
                "type": "sqlite",
                "datasets": ["pages"],
                "destination": format!("env:{MISSING}"),
            },
        },
    });
    let mut probe = |id: &str, sink: &str| {
        engine.ask(
            id,
            "sink.probe",
            &json!({ "sink": sink, "settings": settings }),
        )
    };

    let proved = probe("proved", "mine");
    let failed = probe("failed", "lost");
    let unknown = probe("unknown", "absent");

    assert_eq!(
        proved["result"],
        json!({ "sink": "mine", "outcome": { "status": "proved", "class": "fenced" } }),
        "{proved}"
    );
    assert_eq!(failed["result"]["outcome"]["status"], "failed", "{failed}");
    assert!(
        failed["result"]["outcome"]["cause"]
            .as_str()
            .is_some_and(|cause| cause.contains("did not open")),
        "{failed}"
    );
    assert_eq!(
        unknown["result"]["outcome"]["status"], "failed",
        "{unknown}"
    );
    assert!(
        unknown["result"]["outcome"]["cause"]
            .as_str()
            .is_some_and(|cause| cause.contains("`lost`") && cause.contains("`mine`")),
        "{unknown}"
    );
}

/// A Record a SQLite Sink's constraint rejects does not block the Sink:
/// the batch splits, every good Record lands, the bad one joins the
/// Creates `records` in `database` with a `CHECK` constraint rejecting
/// `/headings/levels`, or rebuilds it without that constraint.
fn set_headings_constraint(database: &Path, reject: bool) {
    let conn = Connection::open(database).unwrap();
    if reject {
        conn.execute_batch(
            "CREATE TABLE records (
                 _goodfetch_delivery_id TEXT NOT NULL PRIMARY KEY,
                 _goodfetch_run_id TEXT NOT NULL,
                 _goodfetch_run_epoch INTEGER NOT NULL,
                 _goodfetch_dataset TEXT NOT NULL,
                 _goodfetch_record_number INTEGER NOT NULL,
                 _goodfetch_committed_at TEXT NOT NULL,
                 _goodfetch_meta TEXT,
                 _goodfetch_key TEXT,
                 url TEXT CHECK (url NOT LIKE '%/headings/levels%')
             ) STRICT;",
        )
        .unwrap();
    } else {
        let cols: Vec<(String, String)> = conn
            .prepare("SELECT name, type FROM pragma_table_info('records') ORDER BY cid")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let defs: Vec<String> = cols
            .iter()
            .map(|(name, ty)| {
                let q = format!("\"{name}\"");
                if name == "_goodfetch_delivery_id" {
                    format!("{q} {ty} NOT NULL PRIMARY KEY")
                } else {
                    format!("{q} {ty}")
                }
            })
            .collect();
        let names: Vec<String> = cols.iter().map(|(name, _)| format!("\"{name}\"")).collect();
        let col_list = names.join(", ");
        conn.execute_batch(&format!(
            "CREATE TABLE records_new ({defs}) STRICT;
             INSERT INTO records_new ({col_list}) SELECT {col_list} FROM records;
             DROP TABLE records;
             ALTER TABLE records_new RENAME TO records;",
            defs = defs.join(", "),
        ))
        .unwrap();
    }
}

/// Sink's dead-letter list with SQLite's reason, and once the constraint
/// is lifted a later Run retries the dead letter and removes it on
/// success (ADR 0026).
///
/// Covers: story:6-289
#[test]
fn a_bad_record_is_dead_lettered_and_retried_on_a_later_run() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let database = env.folder().join("sink.sqlite");
    set_headings_constraint(&database, true);

    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let second = format!("{index}/headings/levels");
    let sink_settings = json!({
        "fetch": { "tier": "http" },
        "sinks": {
            "mine": {
                "type": "sqlite",
                "datasets": ["pages"],
                "destination": format!("env:{DESTINATION}"),
            },
        },
    });
    let extraction = json!({ "formats": { "markdown": true }, "follow": "none" });

    let mut engine = crate::attached_with(&env, None, |command| {
        command.env(DESTINATION, &database);
    });
    let (started1, _) = crawl_read(
        &mut engine,
        &proving_ground,
        (
            "dl-run-1",
            &[
                json!({ "url": index, "label": "page" }),
                json!({ "url": second, "label": "page" }),
            ],
        ),
        &extraction,
        &sink_settings,
    );
    assert!(started1.get("result").is_some(), "{started1}");
    assert!(engine.finished());

    let landed_after_1 = rows(&database);
    assert_eq!(landed_after_1.len(), 1);
    assert_eq!(landed_after_1[0].run_id, "dl-run-1");
    let run1_file = crate::run_file(&env, "dl-run-1");
    let dead_after_1 = goodfetch_sink::dead_letters::list(&run1_file, "mine").unwrap();
    assert_eq!(dead_after_1.len(), 1);
    assert!(
        dead_after_1[0]
            .value
            .to_string()
            .contains("/headings/levels")
    );
    assert_ne!(dead_after_1[0].reason, "");
    drop(run1_file);

    set_headings_constraint(&database, false);

    let mut engine2 = crate::attached_with(&env, None, |command| {
        command.env(DESTINATION, &database);
    });
    let (started2, _) = crawl_read(
        &mut engine2,
        &proving_ground,
        ("dl-run-2", &[json!({ "url": index, "label": "page" })]),
        &extraction,
        &sink_settings,
    );
    assert!(started2.get("result").is_some(), "{started2}");
    assert!(engine2.finished());

    let run1_file = crate::run_file(&env, "dl-run-1");
    assert_eq!(
        goodfetch_sink::dead_letters::list(&run1_file, "mine").unwrap(),
        Vec::new(),
        "Run 1's dead letter was removed once Run 2 delivered it"
    );
    let landed_after_2 = rows(&database);
    assert_eq!(landed_after_2.len(), 3);
    assert!(
        landed_after_2
            .iter()
            .any(|row| row.run_id == "dl-run-1"
                && row.record.to_string().contains("/headings/levels"))
    );
}

/// The crawl's Run report carries each Sink's delivery status: `complete`,
/// `complete_with_rejections` and `failed` (ADR 0026).
///
/// Covers: story:6-290
#[test]
fn the_run_report_reports_each_sinks_delivery_status() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let ok_db = env.folder().join("ok.sqlite");
    let rejected_db = env.folder().join("rejected.sqlite");
    let missing_db = env.folder().join("no-such-dir").join("broken.sqlite");
    set_headings_constraint(&rejected_db, true);

    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let second = format!("{index}/headings/levels");
    let settings = json!({
        "fetch": { "tier": "http" },
        "sinks": {
            "ok": {
                "type": "sqlite",
                "datasets": ["pages"],
                "destination": "env:SINK_TEST_OK",
            },
            "rejected": {
                "type": "sqlite",
                "datasets": ["pages"],
                "destination": "env:SINK_TEST_REJ",
            },
            "broken": {
                "type": "sqlite",
                "datasets": ["pages"],
                "destination": "env:SINK_TEST_BAD",
            },
        },
    });
    let extraction = json!({ "formats": { "markdown": true }, "follow": "none" });
    let mut engine = crate::attached_with(&env, None, |command| {
        command
            .env("SINK_TEST_OK", &ok_db)
            .env("SINK_TEST_REJ", &rejected_db)
            .env("SINK_TEST_BAD", &missing_db);
    });
    let (started, _) = crawl_read(
        &mut engine,
        &proving_ground,
        (
            "report-sinks",
            &[
                json!({ "url": index, "label": "page" }),
                json!({ "url": second, "label": "page" }),
            ],
        ),
        &extraction,
        &settings,
    );
    assert!(started.get("result").is_some(), "{started}");
    let report = engine.ask("report", "run.report", &json!({ "run_id": "report-sinks" }));
    assert!(engine.finished());

    let sinks = report["result"]["sinks"]
        .as_array()
        .unwrap_or_else(|| panic!("Run report must include `sinks`: {report}"));
    let status_of = |name: &str| {
        let entry = sinks
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("missing sink `{name}` in {sinks:?}"));
        (
            entry["status"].as_str().unwrap().to_owned(),
            entry["dead_letters"].as_u64().unwrap(),
            entry["guarantee_class"].as_str().unwrap().to_owned(),
        )
    };
    assert_eq!(
        status_of("ok"),
        ("complete".to_owned(), 0, "fenced".to_owned())
    );
    assert_eq!(
        status_of("rejected"),
        (
            "complete_with_rejections".to_owned(),
            1,
            "fenced".to_owned()
        )
    );
    assert_eq!(
        status_of("broken"),
        ("failed".to_owned(), 0, "at_least_once".to_owned())
    );
}

/// `sink.list` reports each Sink's Guarantee class, dead-letter count and
/// deliveries outside the deduplication window, both scoped to one Run
/// (matching the Run report) and across configured Sinks (ADRs 0026, 0099).
///
/// Covers: story:6-298, operation:sink.list
#[test]
fn sink_list_reports_each_sinks_class_dead_letters_and_late_deliveries() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let database = env.folder().join("listed.sqlite");
    let missing = env.folder().join("absent-dir").join("broken.sqlite");
    set_headings_constraint(&database, true);

    let index = proving_ground.start_url(MARKDOWN_TASK).unwrap();
    let second = format!("{index}/headings/levels");
    let settings = json!({
        "fetch": { "tier": "http" },
        "sinks": {
            "mine": {
                "type": "sqlite",
                "datasets": ["pages"],
                "destination": format!("env:{DESTINATION}"),
            },
            "lost": {
                "type": "sqlite",
                "datasets": ["pages"],
                "destination": format!("env:{MISSING}"),
            },
        },
    });
    let extraction = json!({ "formats": { "markdown": true }, "follow": "none" });
    let mut engine = crate::attached_with(&env, None, |command| {
        command.env(DESTINATION, &database).env(MISSING, &missing);
    });
    let (started, _) = crawl_read(
        &mut engine,
        &proving_ground,
        (
            "list-run",
            &[
                json!({ "url": index, "label": "page" }),
                json!({ "url": second, "label": "page" }),
            ],
        ),
        &extraction,
        &settings,
    );
    assert!(started.get("result").is_some(), "{started}");

    let by_run = engine.ask("list-run", "sink.list", &json!({ "run_id": "list-run" }));
    let configured = engine.ask("list-cfg", "sink.list", &json!({ "settings": settings }));
    let report = engine.ask("report", "run.report", &json!({ "run_id": "list-run" }));
    assert!(engine.finished());

    assert_eq!(
        by_run["result"]["sinks"],
        json!([
            {
                "name": "lost",
                "guarantee_class": "at_least_once",
                "dead_letters": 0,
                "outside_dedup_window": 0,
                "status": "failed",
            },
            {
                "name": "mine",
                "guarantee_class": "fenced",
                "dead_letters": 1,
                "outside_dedup_window": 0,
                "status": "complete_with_rejections",
            },
        ]),
        "{by_run}"
    );
    assert_eq!(
        report["result"]["sinks"],
        json!([
            {
                "name": "lost",
                "guarantee_class": "at_least_once",
                "dead_letters": 0,
                "outside_dedup_window": 0,
                "status": "failed",
            },
            {
                "name": "mine",
                "guarantee_class": "fenced",
                "dead_letters": 1,
                "outside_dedup_window": 0,
                "status": "complete_with_rejections",
            },
        ]),
        "{report}"
    );

    let cfg_sinks = configured["result"]["sinks"].as_array().unwrap();
    assert_eq!(cfg_sinks.len(), 2, "{configured}");
    assert_eq!(cfg_sinks[0]["name"], "lost");
    assert!(cfg_sinks[0].get("guarantee_class").is_none());
    assert!(cfg_sinks[0]["probe_cause"].as_str().is_some());
    assert_eq!(
        cfg_sinks[1],
        json!({
            "name": "mine",
            "guarantee_class": "fenced",
            "dead_letters": 1,
            "outside_dedup_window": 0,
        }),
        "{configured}"
    );
}
