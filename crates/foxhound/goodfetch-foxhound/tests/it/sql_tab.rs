//! The SQL tab through `dataset.query` on Foxhound's executable (spec
//! #593; ADR 0062), beside the crate's own cases over real run files: a
//! query reads the Records `run.records` reads, only from the runs it
//! selected; every write, attach, install and outside read is refused with
//! `query_invalid` beside an allowed `SELECT` over the same run, and a
//! read of the Proving ground's watched target reaches it on no
//! connection; `timeout.dataset_query` and `dataset.query.memory_limit`
//! end a heavy query with their codes; rows past `dataset.query.max_rows`
//! are counted, not returned.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "a test writes and reads back the file outside the runs, outside any async worker"
)]

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

/// A query that runs for far longer than 200 ms.
const LONG: &str = "SELECT count(*) FROM range(1000000000000) a WHERE a.range % 7 = 3";

/// A query that holds far more memory than 16 MiB at once.
const HEAVY: &str = "SELECT list(range) FROM range(10000000)";

/// The `books` Record of page `page`.
fn book(page: u64) -> Value {
    json!({ "page": page, "title": format!("Café {page}") })
}

/// Crawls `run_id` to its end: `pages` seeds, pages of `proving_ground`'s
/// `shop.test`, each pushing its `books` Record.
fn crawled(engine: &mut Attached, proving_ground: &ProvingGround, run_id: &str, pages: u64) {
    let seeds: Vec<Value> = (1..=pages)
        .map(|page| json!({ "url": proving_ground.url("shop.test", &format!("/{page}")), "label": "page" }))
        .collect();
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "datasets": [{ "name": "books" }],
            "settings": { "crawl": { "handler": { "concurrency": 1 } } },
        }),
        &mut |_| None,
    );
    assert_eq!(started["result"]["run_id"], json!(run_id), "{started}");
    let finished = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut |params| {
            let url = params["crawl_request"]["url"].as_str().unwrap();
            let page: u64 = url.rsplit('/').next().unwrap().parse().unwrap();
            Some(json!({ "result": { "records": [{ "dataset": "books", "data": book(page) }] } }))
        },
    );
    assert_eq!(
        finished["result"]["lifecycle"],
        json!("finished"),
        "{finished}"
    );
}

/// `dataset.query`'s reply for `sql` over `runs`.
fn query(engine: &mut Attached, sql: &str, runs: &[&str]) -> Value {
    engine.call(
        "query",
        "dataset.query",
        &json!({ "sql": sql, "runs": runs }),
        &mut |_| None,
    )
}

/// The Records of `books` of `run_id` as `run.records` reads them: each
/// number and value.
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
            let record = &message["params"]["record"];
            read.push(json!([
                record["number"],
                record["value"],
                record["request_id"],
                record["url"]
            ]));
        } else if message["id"] == "records" {
            return read;
        }
    }
}

/// A query over two crawls joins and aggregates their Records, which are
/// those `run.records` reads; a run it did not select is refused with
/// `query_invalid`, and one that does not exist with `run_not_found`.
///
/// Covers: story:12-210, story:12-212, operation:dataset.query, error:query_invalid
#[test]
fn a_query_reads_the_records_run_records_reads_only_from_the_runs_it_selected() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 1208).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    crawled(&mut engine, &proving_ground, "monday", 2);
    crawled(&mut engine, &proving_ground, "tuesday", 3);
    let expected = records(&mut engine, "tuesday");

    let read = query(
        &mut engine,
        "SELECT number, value::JSON, request_id, url FROM records('tuesday') ORDER BY number",
        &["tuesday"],
    );
    let joined = query(
        &mut engine,
        "WITH seen AS (SELECT value->>'title' AS title FROM records('monday') \
         UNION ALL SELECT value->>'title' FROM records('tuesday')) \
         SELECT title, count(*) AS runs FROM seen GROUP BY title ORDER BY title",
        &["monday", "tuesday"],
    );
    let unselected = query(&mut engine, "SELECT * FROM records('monday')", &["tuesday"]);
    let missing = query(&mut engine, "SELECT 1", &["never-ran"]);

    let rows: Vec<Value> = read["result"]["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("{read}"))
        .iter()
        .map(|row| json!([row[0], row[1].clone(), row[2], row[3]]))
        .collect();
    assert_eq!(rows, expected);
    assert_eq!(
        joined["result"]["rows"],
        json!([["Café 1", 2], ["Café 2", 2], ["Café 3", 1]]),
        "{joined}"
    );
    assert_eq!(
        (
            &unselected["error"]["data"]["code"],
            &unselected["error"]["data"]["reason"]
        ),
        (&json!("query_invalid"), &json!("run_not_selected")),
        "{unselected}"
    );
    assert_eq!(
        missing["error"]["data"]["code"],
        json!("run_not_found"),
        "{missing}"
    );
    assert!(engine.finished());
}

/// The statement `case` names, reading `url` or `outside` where it reads.
fn statement(case: &str, url: &str, outside: &str) -> String {
    match case {
        "insert" => "INSERT INTO kept VALUES (1)".to_owned(),
        "attach" => format!("ATTACH '{outside}' AS other"),
        "install" => "INSTALL httpfs".to_owned(),
        "load" => "LOAD httpfs".to_owned(),
        "copy" => format!("COPY (SELECT * FROM records('shop')) TO '{outside}'"),
        "read_csv_url" => format!("SELECT * FROM read_csv('{url}')"),
        "read_csv_path" => format!("SELECT * FROM read_csv('{outside}')"),
        other => panic!("no case {other}"),
    }
}

/// How many connections reached the Proving ground's watched target.
fn target_connections(proving_ground: &ProvingGround) -> usize {
    proving_ground.observations().unwrap()["connections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|connection| connection["listener"] == "target")
        .count()
}

/// Each write, attach, install and outside read is refused with
/// `query_invalid`, beside a `SELECT` over the same run that is allowed;
/// the URL is the Proving ground's watched target, which no connection
/// reaches, and no extension is fetched.
///
/// Covers: story:12-211, operation:dataset.query, error:query_invalid
#[rstest]
#[case::insert("insert")]
#[case::attach("attach")]
#[case::install("install")]
#[case::load("load")]
#[case::copy("copy")]
#[case::read_csv_url("read_csv_url")]
#[case::read_csv_path("read_csv_path")]
fn a_write_attach_install_or_outside_read_is_refused_beside_an_allowed_select(#[case] case: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 1208).unwrap();
    let url = format!(
        "http://{}/books.csv",
        proving_ground.description().fixtures.target
    );
    let outside = env.home().join("outside.csv");
    std::fs::write(&outside, "a,b\n1,2\n").unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    crawled(&mut engine, &proving_ground, "shop", 2);
    let before = target_connections(&proving_ground);

    let refused = query(
        &mut engine,
        &statement(case, &url, &outside.display().to_string()),
        &["shop"],
    );
    let allowed = query(
        &mut engine,
        "SELECT count(*) FROM records('shop')",
        &["shop"],
    );

    assert_eq!(
        refused["error"]["data"]["code"],
        json!("query_invalid"),
        "{case}: {refused}"
    );
    assert_eq!(allowed["result"]["rows"], json!([[2]]), "{allowed}");
    assert_eq!(target_connections(&proving_ground), before, "{case}");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "a,b\n1,2\n");
    assert!(engine.finished());
}

/// A heavy query ends at `timeout.dataset_query` with `deadline_exceeded`
/// naming it, and at `dataset.query.memory_limit` with
/// `query_memory_exceeded` naming it.
///
/// Covers: story:12-211, setting:timeout.dataset_query, setting:dataset.query.memory_limit
#[rstest]
/// Covers: error:deadline_exceeded
#[case::timeout(
    "timeout.dataset_query = 200\n",
    LONG,
    "deadline_exceeded",
    "timeout.dataset_query"
)]
/// Covers: error:query_memory_exceeded
#[case::memory(
    "dataset.query.memory_limit = 16777216\n",
    HEAVY,
    "query_memory_exceeded",
    "dataset.query.memory_limit"
)]
fn a_heavy_query_ends_at_its_limit_with_its_code(
    #[case] settings: &str,
    #[case] heavy: &str,
    #[case] code: &str,
    #[case] key: &str,
) {
    let env = TestEnv::new();
    env.user_configuration(settings);
    let proving_ground = ProvingGround::start(&env, 1208).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    crawled(&mut engine, &proving_ground, "shop", 1);

    let ended = query(&mut engine, heavy, &["shop"]);
    let allowed = query(
        &mut engine,
        "SELECT count(*) FROM records('shop')",
        &["shop"],
    );

    assert_eq!(
        (
            &ended["error"]["data"]["code"],
            &ended["error"]["data"]["limit"]["key"]
        ),
        (&json!(code), &json!(key)),
        "{ended}"
    );
    assert_eq!(allowed["result"]["rows"], json!([[1]]), "{allowed}");
    assert!(engine.finished());
}

/// A failure DuckDB's translation point does not recognise, such as a
/// value that cannot be cast, answers `query_failed`, and the engine goes on
/// answering queries.
///
/// Covers: story:12-211, error:query_failed
#[test]
fn an_unrecognised_failure_answers_query_failed() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 1208).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    crawled(&mut engine, &proving_ground, "shop", 1);

    let failed = query(
        &mut engine,
        "SELECT CAST('not a number' AS INTEGER)",
        &["shop"],
    );
    let allowed = query(
        &mut engine,
        "SELECT count(*) FROM records('shop')",
        &["shop"],
    );

    assert_eq!(
        failed["error"]["data"]["code"],
        json!("query_failed"),
        "{failed}"
    );
    assert_eq!(allowed["result"]["rows"], json!([[1]]), "{allowed}");
    assert!(engine.finished());
}

/// Rows past `dataset.query.max_rows` are counted in `total`, not
/// returned.
///
/// Covers: story:12-211, setting:dataset.query.max_rows
#[test]
fn rows_past_max_rows_are_counted_not_returned() {
    let env = TestEnv::new();
    env.user_configuration("dataset.query.max_rows = 2\n");
    let proving_ground = ProvingGround::start(&env, 1208).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    crawled(&mut engine, &proving_ground, "shop", 3);

    let limited = query(
        &mut engine,
        "SELECT number FROM records('shop') ORDER BY number",
        &["shop"],
    );

    assert_eq!(
        (&limited["result"]["rows"], &limited["result"]["total"]),
        (&json!([[1], [2]]), &json!(3)),
        "{limited}"
    );
    assert!(engine.finished());
}
