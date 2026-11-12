//! `dataset.list` and `dataset.page` on Foxhound's executable over stdio
//! (spec #593's seam: the protocol, with a test program as the Client; ADRs
//! 0031 and 0103): a crawl's Datasets listed with their schema, their key
//! and their committed, dropped and rejected counts, and a Dataset's
//! Records paged by key, filtered, sorted and narrowed to columns in the
//! engine, a page never repeating or skipping a Record while the crawl
//! commits more.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;

use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::frontier::{Inside, crawl_from};
use crate::site::Site;

/// The Datasets the crawls declare: `books`, with a schema requiring
/// `title` and `isbn` and keyed by `isbn`, and `notes`, with neither.
fn declared() -> Value {
    json!([
        {
            "name": "books",
            "schema": {
                "type": "object",
                "required": ["title", "isbn"],
                "properties": {
                    "title": { "type": "string" },
                    "isbn": { "type": "string" },
                    "price": { "type": "number" },
                },
            },
            "key": ["isbn"],
        },
        { "name": "notes" },
    ])
}

/// `ctx.push(dataset, record)` from the attempt `handle` gave: its reply.
fn push(inside: &mut Inside<'_>, handle: &Value, dataset: &str, data: &Value) -> Value {
    inside.call(
        "crawl_request.push",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "dataset": dataset,
            "data": data,
        }),
    )
}

/// The seeds `/1` to `/n` of `site`.
fn seeds(site: &Site, n: u64) -> Value {
    (1..=n)
        .map(|page| json!({ "url": site.page(&format!("/{page}")), "label": "page" }))
        .collect()
}

/// The request number of the crawl request `handle` gave, from its URL.
fn page_of(handle: &Value) -> u64 {
    handle["crawl_request"]["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// Each Record of a `dataset.page` reply as `(number, value)`.
fn read(reply: &Value) -> Vec<(u64, Value)> {
    reply["result"]["records"]
        .as_array()
        .unwrap_or_else(|| panic!("{reply}"))
        .iter()
        .map(|record| (record["number"].as_u64().unwrap(), record["value"].clone()))
        .collect()
}

/// A crawl's Datasets are listed in the order declared, each with its
/// JSON Schema, its key and how many Records were committed, dropped and
/// rejected, while the crawl runs and once it ended.
///
/// Covers: story:12-204, operation:dataset.list
#[test]
fn a_crawls_datasets_list_their_schema_key_and_committed_dropped_and_rejected_counts() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let mut live = Value::Null;
    let report = crawl_from(
        &mut engine,
        &json!({ "run_id": "listed", "seeds": seeds(&site, 3), "datasets": declared() }),
        &mut |inside, handle| {
            let n = page_of(handle);
            if n == 3 {
                live = inside.call("dataset.list", &json!({ "run_id": "listed" }));
            }
            let isbn = format!("isbn-{n}");
            push(
                inside,
                handle,
                "books",
                &json!({ "title": "Dune", "isbn": isbn }),
            );
            push(inside, handle, "books", &json!({ "title": "No isbn" }));
            if n == 1 {
                push(inside, handle, "notes", &json!({ "text": "one" }));
            }
            json!({ "result": { "records": [] } })
        },
    );
    let ended = engine.call(
        "list",
        "dataset.list",
        &json!({ "run_id": "listed" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let books = |committed: u64, rejected: u64| {
        json!({
            "name": "books",
            "schema": declared()[0]["schema"],
            "key": ["isbn"],
            "committed": committed,
            "dropped": 0,
            "rejected": rejected,
        })
    };
    let notes = json!({ "name": "notes", "key": [], "committed": 1, "dropped": 0, "rejected": 0 });
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &live["result"],
            &ended["result"]
        ),
        (
            &json!("finished"),
            &json!({ "datasets": [books(2, 2), notes] }),
            &json!({ "datasets": [books(3, 3), notes] }),
        )
    );
}

/// Pages read while the crawl commits more Records, sorted by a field the
/// commits do not follow, read each Record committed when the first page
/// was read once, none twice and none after `through`, in the sort's
/// order; Records committed meanwhile wait for a new first page.
///
/// Covers: story:12-205, operation:dataset.page
#[test]
fn paging_by_key_under_concurrent_commits_never_repeats_or_skips_a_record() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    // Each request's Record takes a rank no order of numbers follows, two of
    // them sharing one, and request 5's has none.
    let rank = |n: u64| (n * 7) % 11;
    let mut pages: Vec<Value> = Vec::new();
    let report = crawl_from(
        &mut engine,
        &json!({ "run_id": "paged", "seeds": seeds(&site, 20), "datasets": declared() }),
        &mut |inside, handle| {
            let n = page_of(handle);
            if n >= 9
                && pages
                    .last()
                    .is_none_or(|page| page["result"].get("next").is_some())
            {
                let mut params = json!({
                    "run_id": "paged",
                    "dataset": "books",
                    "sort": { "field": "rank", "direction": "ascending" },
                    "limit": 3,
                });
                if let Some(last) = pages.last() {
                    params["after"] = last["result"]["next"].clone();
                    params["through"] = last["result"]["through"].clone();
                }
                pages.push(inside.call("dataset.page", &params));
            }
            let mut book = json!({ "title": format!("Book {n}"), "isbn": format!("{n}") });
            if n != 5 {
                book["rank"] = json!(rank(n) / 2);
            }
            push(inside, handle, "books", &book);
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let through: BTreeSet<String> = pages
        .iter()
        .map(|page| page["result"]["through"].to_string())
        .collect();
    let numbers: Vec<u64> = pages
        .iter()
        .flat_map(|page| read(page).into_iter().map(|(number, _)| number))
        .collect();
    // Every page reads through the number the first page answered, the
    // Records committed before request 9's handler read it, fewer than the
    // crawl's 20: ranks
    // (n*7 % 11) / 2, request 5 having none, sorted by rank, then by number.
    let first = pages[0]["result"]["through"].as_u64().unwrap();
    assert!((1..20).contains(&first), "{}", pages[0]);
    let mut expected: Vec<(Option<u64>, u64)> = (1..=first)
        .map(|n| ((n != 5).then(|| rank(n) / 2), n))
        .collect();
    expected.sort_by_key(|&(rank, number)| (rank.is_none(), rank, number));
    assert_eq!(
        (&report["result"]["lifecycle"], through, numbers),
        (
            &json!("finished"),
            BTreeSet::from([first.to_string()]),
            expected
                .into_iter()
                .map(|(_, number)| number)
                .collect::<Vec<_>>(),
        )
    );
}

/// Filters, a descending sort and columns run in the engine: a page holds
/// only the Records that meet every field condition and the `wall`
/// filter, in the sort's order with those lacking the field last, each
/// value holding only the columns asked for. A Dataset the crawl does not
/// declare is refused with `dataset_unknown`, and a `limit` past 1000 or a
/// field holding `"` with `invalid_params`.
///
/// Covers: story:12-205, operation:dataset.page, error:dataset_unknown
#[test]
fn filters_sorts_and_columns_run_in_the_engine() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let books = [
        json!({ "title": "Dune", "isbn": "1", "price": 9.5, "lang": "en" }),
        json!({ "title": "Dune Messiah", "isbn": "2", "price": 12, "lang": "en" }),
        json!({ "title": "الخيميائي", "isbn": "3", "price": 7, "lang": "ar" }),
        json!({ "title": "Children of Dune", "isbn": "4", "lang": "en" }),
    ];
    let report = crawl_from(
        &mut engine,
        &json!({ "run_id": "filtered", "seeds": seeds(&site, 1), "datasets": declared() }),
        &mut |inside, handle| {
            for book in &books {
                push(inside, handle, "books", book);
            }
            json!({ "result": { "records": [] } })
        },
    );
    let mut page = |params: Value| {
        let mut params = params;
        params["run_id"] = json!("filtered");
        params["dataset"] = params.get("dataset").cloned().unwrap_or(json!("books"));
        engine.call("page", "dataset.page", &params, &mut |_| None)
    };
    let contains_dune = page(json!({
        "filter": { "fields": [{ "field": "title", "op": "contains", "text": "Dune" }] },
        "sort": { "field": "price", "direction": "descending" },
        "columns": ["title", "price"],
    }));
    let english_without_wall = page(json!({
        "filter": {
            "fields": [{ "field": "lang", "op": "equals", "value": "en" }],
            "wall": "without",
        },
        "columns": ["isbn"],
    }));
    let with_wall = page(json!({ "filter": { "wall": "with" } }));
    let unknown = page(json!({ "dataset": "bokks" }));
    let too_many = page(json!({ "limit": 1001 }));
    let quoted = page(json!({ "sort": { "field": "a\"b", "direction": "ascending" } }));
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            read(&contains_dune),
            &contains_dune["result"]["next"],
            read(&english_without_wall),
            read(&with_wall),
            &unknown["error"]["data"]["code"],
            &too_many["error"]["code"],
            &quoted["error"]["code"],
        ),
        (
            &json!("finished"),
            vec![
                (2, json!({ "title": "Dune Messiah", "price": 12 })),
                (1, json!({ "title": "Dune", "price": 9.5 })),
                (4, json!({ "title": "Children of Dune" })),
            ],
            &Value::Null,
            vec![
                (1, json!({ "isbn": "1" })),
                (2, json!({ "isbn": "2" })),
                (4, json!({ "isbn": "4" }))
            ],
            vec![],
            &json!("dataset_unknown"),
            &json!(-32602),
            &json!(-32602),
        )
    );
}
