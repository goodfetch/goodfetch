//! Full-text search on Foxhound's executable (ADR 0031; spec #1937,
//! issue 1217), beside the SQL tab crate's cases over real run files:
//! `dataset.search` finds a run's Records by any word, the shared Unicode
//! record's included, and pages by number with neither repeat nor skip;
//! `record.search` finds them in only the runs it selects, at most 100;
//! with `dataset.search.index` off, a crawl's Records are kept and none is
//! indexed.

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv, unicode_record};
use serde_json::{Map, Value, json};

/// The `books` Record of page `page`.
fn book(page: u64) -> Value {
    json!({ "page": page, "title": format!("Café {page}") })
}

/// The shared Unicode record as a Record: each field's name to its text.
fn unicode() -> Value {
    Value::Object(
        unicode_record()
            .fields
            .into_iter()
            .map(|field| (field.name, Value::String(field.text)))
            .collect::<Map<_, _>>(),
    )
}

/// Crawls `run_id` to its end under `settings`: `pages` seeds, pages of
/// `proving_ground`'s `shop.test`, each pushing its `books` Record but the
/// last, which pushes the Unicode record.
fn crawled(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    run_id: &str,
    pages: u64,
    settings: &Value,
) {
    let seeds: Vec<Value> = (1..=pages)
        .map(|page| json!({ "url": proving_ground.url("shop.test", &format!("/{page}")), "label": "page" }))
        .collect();
    let mut all = json!({ "crawl": { "handler": { "concurrency": 1 } } });
    if let (Some(all), Some(more)) = (all.as_object_mut(), settings.as_object()) {
        all.extend(more.clone());
    }
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "datasets": [{ "name": "books" }],
            "settings": all,
            "labels": { "page": {} },
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
            let data = if page == pages { unicode() } else { book(page) };
            Some(json!({ "result": { "records": [{ "dataset": "books", "data": data }] } }))
        },
    );
    assert_eq!(
        finished["result"]["lifecycle"],
        json!("finished"),
        "{finished}"
    );
}

/// `method`'s reply for `params`.
fn call(engine: &mut Attached, method: &str, params: &Value) -> Value {
    engine.call("search", method, params, &mut |_| None)
}

/// The values of the Records a reply's `records` holds.
fn values(reply: &Value) -> Vec<Value> {
    reply["result"]["records"]
        .as_array()
        .unwrap_or_else(|| panic!("{reply}"))
        .iter()
        .map(|record| record["value"].clone())
        .collect()
}

/// `values` in one order, whatever order a search found them in.
fn sorted(mut values: Vec<Value>) -> Vec<Value> {
    values.sort_by_key(ToString::to_string);
    values
}

/// The JSON-RPC code and the Error code of a reply's error.
fn refusal(reply: &Value) -> (Value, Value) {
    (
        reply["error"]["code"].clone(),
        reply["error"]["data"]["code"].clone(),
    )
}

/// A search in a run finds the Unicode record by any of its words, in any
/// script, case or normalization, and only it; pages of two, each after
/// the last number of the one before, read every Record holding a word
/// once, in number order; text with no word and a limit past the most are
/// `invalid_params`, and a run that does not exist is `run_not_found`.
///
/// Covers: story:12-208, operation:dataset.search
#[test]
fn a_search_in_a_run_finds_a_record_by_any_word_and_pages_by_number() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 1217).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    crawled(&mut engine, &proving_ground, "shop", 5, &json!({}));
    let mut search = |text: &str, after: Option<u64>, limit: u32| {
        call(
            &mut engine,
            "dataset.search",
            &json!({ "run_id": "shop", "dataset": "books", "text": text, "after": after, "limit": limit }),
        )
    };
    let words = ["שלום", "مرحبا", "עברית", "CAFE\u{301}", "three"];
    let found: Vec<Vec<Value>> = words
        .iter()
        .map(|word| sorted(values(&search(word, None, 10))))
        .collect();
    let mut paged = Vec::new();
    let mut after = None;
    loop {
        let page = search("café", after, 2);
        paged.extend(
            page["result"]["records"]
                .as_array()
                .unwrap()
                .iter()
                .map(|record| (record["number"].clone(), record["value"].clone())),
        );
        after = page["result"]["next"].as_u64();
        if after.is_none() {
            break;
        }
    }
    let wordless = search(" \t ", None, 10);
    let past_most = search("café", None, 1001);
    let missing = call(
        &mut engine,
        "dataset.search",
        &json!({ "run_id": "never-ran", "dataset": "books", "text": "café" }),
    );

    let every = sorted((1..=4).map(book).chain([unicode()]).collect());
    let numbers: Vec<Value> = paged.iter().map(|(number, _)| number.clone()).collect();
    let paged_values = sorted(paged.into_iter().map(|(_, value)| value).collect());
    assert_eq!(
        (found, numbers, paged_values),
        (
            vec![
                vec![unicode()],
                vec![unicode()],
                vec![unicode()],
                every.clone(),
                vec![unicode()],
            ],
            (1..=5).map(Value::from).collect(),
            every,
        ),
        "each word finds the Records holding it, and the pages read each once"
    );
    assert_eq!(
        (refusal(&wordless), refusal(&past_most), refusal(&missing).1),
        (
            (json!(-32602), json!("invalid_params")),
            (json!(-32602), json!("invalid_params")),
            json!("run_not_found"),
        )
    );
    assert!(engine.finished());
}

/// A search across runs finds the Records holding every word in the runs
/// it selects only, in the Datasets it names; more than 100 runs are
/// `invalid_params`, and a run that does not exist is `run_not_found`.
///
/// Covers: story:12-209, operation:record.search
#[test]
fn a_search_across_runs_finds_records_in_the_selected_runs_only() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 1217).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    for (run, pages) in [("monday", 2), ("tuesday", 3), ("wednesday", 3)] {
        crawled(&mut engine, &proving_ground, run, pages, &json!({}));
    }
    let mut search = |params: Value| call(&mut engine, "record.search", &params);

    let unicode_found = search(json!({ "text": "שלום", "runs": ["monday", "tuesday"] }));
    let book_found = search(json!({ "text": "CAFE 2", "runs": ["tuesday", "wednesday"] }));
    let elsewhere = search(json!({ "text": "café", "runs": ["tuesday"], "datasets": ["pages"] }));
    let too_many = search(json!({
        "text": "café",
        "runs": (0..101).map(|run| format!("run-{run}")).collect::<Vec<_>>(),
    }));
    let missing = search(json!({ "text": "café", "runs": ["never-ran"] }));

    let mut runs: Vec<(Value, Value)> = unicode_found["result"]["records"]
        .as_array()
        .unwrap_or_else(|| panic!("{unicode_found}"))
        .iter()
        .map(|record| (record["run_id"].clone(), record["value"].clone()))
        .collect();
    runs.sort_by_key(|(run, _)| run.to_string());
    let mut books: Vec<Value> = book_found["result"]["records"]
        .as_array()
        .unwrap_or_else(|| panic!("{book_found}"))
        .iter()
        .map(|record| json!([record["run_id"], record["dataset"], record["value"]]))
        .collect();
    books.sort_by_key(ToString::to_string);
    assert_eq!(
        (runs, books, values(&elsewhere)),
        (
            vec![(json!("monday"), unicode()), (json!("tuesday"), unicode())],
            vec![
                json!(["tuesday", "books", book(2)]),
                json!(["wednesday", "books", book(2)])
            ],
            Vec::new(),
        )
    );
    assert_eq!(
        (refusal(&too_many), refusal(&missing).1),
        (
            (json!(-32602), json!("invalid_params")),
            json!("run_not_found")
        )
    );
    assert!(engine.finished());
}

/// With `dataset.search.index` off, a crawl keeps its Records, which
/// `dataset.query` reads, and indexes none: neither search finds them.
///
/// Covers: setting:dataset.search.index, story:12-208
#[test]
fn a_crawl_with_indexing_off_keeps_its_records_and_indexes_none() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 1217).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    crawled(
        &mut engine,
        &proving_ground,
        "quiet",
        3,
        &json!({ "dataset": { "search": { "index": false } } }),
    );

    let kept = call(
        &mut engine,
        "dataset.query",
        &json!({ "sql": "SELECT count(*) FROM records('quiet')", "runs": ["quiet"] }),
    );
    let in_run = call(
        &mut engine,
        "dataset.search",
        &json!({ "run_id": "quiet", "dataset": "books", "text": "café" }),
    );
    let across = call(
        &mut engine,
        "record.search",
        &json!({ "text": "café", "runs": ["quiet"] }),
    );

    assert_eq!(
        (&kept["result"]["rows"], values(&in_run), values(&across)),
        (&json!([[3]]), Vec::new(), Vec::new()),
        "{kept} {in_run} {across}"
    );
    assert!(engine.finished());
}
