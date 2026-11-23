//! J-012, "Move a Checkpoint to another Goodfetch home and finish the run
//! there", on the `cli` surface (ADRs 0003, 0063 and 0066; ticket #2348):
//! a person's program starts Foxhound's shipped executable as its engine,
//! begins a code-free crawl of the Proving ground's catalog whose program
//! declares the order it places as an Effect, and migrates the crawl
//! partway to a Checkpoint; a second engine on another Goodfetch home
//! resumes the crawl from that Checkpoint, its program declaring the order
//! again, and the crawl finishes there with every product once.
#![expect(
    clippy::disallowed_methods,
    reason = "the person carries the Checkpoint to the second home"
)]

use std::collections::BTreeSet;
use std::path::Path;

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_client::Attached;
use goodfetch_test_support::{JourneyRun, ProvingGround, TestEnv};
use serde_json::{Value, json};

use super::j005::{fields_read, products};
use crate::attached_spawned;
use crate::checkpoint_resume::{ORDER_TASK, orders, place_order};
use crate::fetch_tiers::settings;
use crate::frontier::seed_labels;
use crate::provenance::{records, records_through};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

/// The crawl's Run.
const RUN: &str = "moved";

/// The Effect journal's key of the order the program places.
const ORDER: &str = "order";

/// The task whose Records the crawl returns.
const CATALOG: &str = "catalog/1";

/// The crawl's settings: the `http` tier, a short delay between a domain's
/// requests, so the migration comes partway through the catalog's 110
/// pages, and the Proving ground's hosts mapped, which the second home's
/// engine is given again as this machine's settings.
fn crawl_settings(proving_ground: &ProvingGround) -> Value {
    settings(
        proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "delay": 50 } },
        }),
    )
}

/// `executable`, Foxhound's, as the program's engine in `env`, on its
/// Goodfetch home.
fn engine(env: &TestEnv, executable: &Path) -> Attached {
    let mut command = env.process_command(executable);
    command.arg("attached-engine");
    attached_spawned(env, None, &mut command)
}

/// The reply to the call `id` of `method` with `params`, the engine's
/// other messages meanwhile skipped: a code-free crawl asks its program
/// nothing.
fn answered(engine: &mut Attached, id: &str, method: &str, params: &Value) -> Value {
    engine.call(id, method, params, &mut |handle| {
        panic!("a code-free crawl asked its program {handle}")
    })
}

/// The program's body: declares the order as an Effect and, when the
/// Effect journal says to perform it, places it on the Proving ground and
/// closes the Effect with its output. Answers what `effect.open` answered.
fn order_once(engine: &mut Attached, proving_ground: &ProvingGround) -> Value {
    let opened = answered(
        engine,
        "order",
        "effect.open",
        &json!({ "run_id": RUN, "key": ORDER }),
    );
    if opened["result"]["answer"] == "perform" {
        let order_form = proving_ground.start_url(ORDER_TASK).unwrap();
        let status = place_order(proving_ground, &order_form);
        let closed = answered(
            engine,
            "ordered",
            "effect.close",
            &json!({
                "run_id": RUN,
                "entry": opened["result"]["entry"],
                "outcome": { "ended": "returned", "output": { "status": status } },
            }),
        );
        assert!(closed.get("result").is_some(), "{closed}");
    }
    opened
}

/// Covers: story:3-34, row:RT-10, operation:run.migrate, operation:run.resume, operation:effect.open
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant reads as its file: each step, then each check, in order"
)]
fn cli() {
    let mut run = JourneyRun::start("J-012", "cli", Path::new(FOXHOUND), ResultDocument::journey)
        .expect("J-012's cli variant starts");
    let mut first: Option<Attached> = None;
    let mut placed = Value::Null;
    let mut migrated = Value::Null;
    let mut at_migration: Vec<Value> = Vec::new();
    let checkpoint = run.env().folder().join("moved.sqlite");
    let mut resumed = Value::Null;
    let mut declared_again = Value::Null;
    let mut finished = Value::Null;
    let mut committed: Vec<Value> = Vec::new();

    run.step(
        "Start a run that orders the product of the catalog's order page once, then crawls the catalog",
        |run| {
            let proving_ground = run.proving_ground().expect("J-012 has a Proving ground");
            let mut engine = engine(run.env(), run.engine());
            let seeds = [json!({
                "url": proving_ground.start_url(CATALOG).unwrap(),
                "label": "page",
            })];
            let started = answered(
                &mut engine,
                "start",
                "crawl.start",
                &json!({
                    "run_id": RUN,
                    "seeds": seeds,
                    "labels": seed_labels(&seeds),
                    "datasets": [],
                    "extraction": { "formats": {}, "follow": "scope", "fields": fields_read() },
                    "settings": crawl_settings(proving_ground),
                }),
            );
            assert_eq!(started["result"]["run_id"], RUN, "{started}");
            placed = order_once(&mut engine, proving_ground);
            first = Some(engine);
        },
    );
    run.step(
        "Stop the run once the order is placed and part of the catalog is crawled",
        |_| {
            let engine = first.as_mut().expect("the first engine runs");
            // Answers once the first Record committed.
            records_through(engine, RUN, "pages", 1);
            migrated = answered(
                engine,
                "migrate",
                "run.migrate",
                &json!({ "run_id": RUN, "location": checkpoint.display().to_string() }),
            );
            assert!(migrated.get("result").is_some(), "{migrated}");
            at_migration = records(engine, RUN, "pages");
            assert!(first.take().expect("the first engine runs").finished());
        },
    );
    // The second Goodfetch home, set up as test support sets every home.
    let second = TestEnv::new();
    let carried = second.folder().join("moved.sqlite");
    run.step(
        "Take the run's Checkpoint to a second Goodfetch home",
        |_| {
            std::fs::copy(&checkpoint, &carried).unwrap();
        },
    );
    run.step(
        "Resume the run from its Checkpoint in the second home and let it finish",
        |run| {
            let proving_ground = run.proving_ground().expect("J-012 has a Proving ground");
            let mut engine = engine(&second, run.engine());
            resumed = answered(
                &mut engine,
                "resume",
                "run.resume",
                &json!({
                    "run_id": RUN,
                    "checkpoint": carried.display().to_string(),
                    "settings": crawl_settings(proving_ground),
                }),
            );
            declared_again = order_once(&mut engine, proving_ground);
            finished = answered(
                &mut engine,
                "finish",
                "run.finish",
                &json!({ "run_id": RUN }),
            );
            committed = records(&mut engine, RUN, "pages");
            assert!(engine.finished());
        },
    );

    run.check(
        "The resumed run checked the order its Effect journal holds instead of placing it again",
        |run| {
            let proving_ground = run.proving_ground().expect("J-012 has a Proving ground");
            assert_eq!(
                (
                    orders(proving_ground).len(),
                    &placed["result"]["answer"],
                    &migrated["result"]["stop_reason"]["kind"],
                    &resumed["result"]["started_as"],
                    &declared_again["result"]["answer"],
                    &declared_again["result"]["output"],
                    &finished["result"]["lifecycle"],
                ),
                (
                    1,
                    &json!("perform"),
                    &json!("migrate"),
                    &json!("resumed"),
                    &json!("journal"),
                    &json!({ "status": 200 }),
                    &json!("finished"),
                ),
                "{placed} {migrated} {resumed} {declared_again} {finished}"
            );
        },
    );
    run.expect_task(
        "The oracle verifies the order was placed once",
        &json!({ "status": "success" }),
    );
    run.check("No Record is in the run's Dataset twice", |_| {
        let urls: BTreeSet<&str> = committed
            .iter()
            .filter_map(|record| record["url"].as_str())
            .collect();
        let numbers: Vec<Value> = committed
            .iter()
            .map(|record| record["number"].clone())
            .collect();
        let expected: Vec<Value> = (1..=committed.len()).map(|number| json!(number)).collect();
        assert_eq!(
            (
                !at_migration.is_empty()
                    && at_migration.len() < committed.len()
                    && committed.starts_with(&at_migration),
                urls.len(),
                numbers,
            ),
            (true, committed.len(), expected),
            "{at_migration:?}"
        );
    });
    run.expect_task(
        "The oracle verifies the run's Records",
        &json!({ "status": "success", "records": products(&committed) }),
    );
    run.finish()
        .expect("J-012's document is written")
        .assert_proved();
}
