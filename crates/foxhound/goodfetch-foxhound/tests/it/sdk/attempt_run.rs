//! `ctx.run()` in Rust on the linked engine, crawling the Proving ground's
//! `catalog` family: it reaches the crawl's Key-value store bound to the
//! attempt, so a write through it commits with the attempt's Records and
//! enqueues or is discarded with them, while a write through the program's
//! own Run value, captured by the handler, commits at once, on every
//! attempt that runs it (ADRs 0093 and 0103; issue 1486). Its reads of
//! Records and rejections end at what had committed (issue 1487). TypeScript's
//! `attempt-run.test.ts` and Python's `test_attempt_run_attached.py` are
//! its twins.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the handlers note what they saw on the test's own side, which loom does not model (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use goodfetch_foxhound::settings::crawl::{handler, retry};
use goodfetch_foxhound::settings::run_state;
use goodfetch_foxhound::{
    ConnectOptions, Crawl, CrawlContext, DatasetDeclaration, Error, Foxhound, KeyValueStore, RunId,
    Seed, blocking, error_code,
};
use goodfetch_protocol::result_document::OracleOutcome;
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::{ProvingGround, TestEnv, ToolStatus, real_time_runtime};
use serde_json::{Value, json};

use super::catalog::{Product, TASK, crawl_listing, judged, products_json};
use super::crawl::site_settings;

/// The Proving ground's seed for these tests, TypeScript's.
const SEED: u64 = 1486;

/// What every connection here opens with: one handler call at a time, and
/// a failed handler retried.
fn retrying(env: &TestEnv, site: &ProvingGround) -> ConnectOptions {
    let settings = site_settings(site)
        .set(handler::CONCURRENCY, 1_u64)
        .set(retry::HANDLER_FAILED, true);
    super::options(env).settings(settings)
}

/// Each listing page's handler fails its first attempt after its outputs:
/// its Records, an enqueue of a decoy and a Key-value write. Its second
/// attempt commits each output once, the first's discarded.
///
/// Covers: story:11-1058, story:11-1064
#[test]
fn a_retried_handlers_outputs_commit_once_and_its_failed_attempts_are_discarded() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    // Every page's attempts, by the first product it lists.
    let attempts = Arc::new(Mutex::new(BTreeMap::<String, Vec<u32>>::new()));
    let decoys = Arc::new(Mutex::new(0_u32));
    let mut shop = Crawl::new();
    let listing = shop.label::<()>("listing").unwrap();
    let decoy = shop.label::<()>("decoy").unwrap();
    let products = shop
        .dataset(DatasetDeclaration::<Product>::new("products").key(&["id"]))
        .unwrap();
    let given = (
        listing.clone(),
        decoy.clone(),
        products.clone(),
        start.clone(),
        Arc::clone(&attempts),
    );
    shop.handle(&listing, move |ctx: CrawlContext<()>| {
        let (listing, decoy, products, start, attempts) = given.clone();
        async move {
            let name = crawl_listing(&ctx, &listing, &products).await?;
            let attempt = ctx.attempt();
            attempts
                .lock()
                .unwrap()
                .entry(name.clone())
                .or_default()
                .push(attempt);
            let store = ctx.run().key_value_store();
            if attempt == 1 {
                ctx.enqueue(start, &decoy, ())
                    .key(format!("decoy of {name}"))
                    .await?;
                store.set(format!("failed:{name}"), &attempt).await?;
                return Err("the first attempt fails after its outputs".into());
            }
            store.set(format!("done:{name}"), &attempt).await?;
            Ok(())
        }
    })
    .unwrap();
    let served = Arc::clone(&decoys);
    shop.handle(&decoy, move |_: CrawlContext<()>| {
        *served.lock().unwrap() += 1;
        async { Ok(()) }
    })
    .unwrap();

    let (records, stored) = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(retrying(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(&shop, [Seed::new(&start, &listing, ())])
            .await
            .unwrap();
        shop.serve(&run).await.unwrap();
        let mut records = Vec::new();
        let mut read = run.records(&products_json());
        while let Some(record) = read.next().await {
            records.push(record.unwrap().value);
        }
        let store = run.run().key_value_store();
        let names: Vec<String> = attempts.lock().unwrap().keys().cloned().collect();
        let mut stored = Vec::new();
        for name in names {
            stored.push((
                store.get::<u32>(format!("failed:{name}")).await.unwrap(),
                store.get::<u32>(format!("done:{name}")).await.unwrap(),
            ));
        }
        foxhound.close().await.unwrap();
        (records, stored)
    });

    let answer = judged(&site, ToolStatus::Success, &records);
    assert_eq!(answer.outcome, OracleOutcome::Verified, "{answer:?}");
    let attempts = attempts.lock().unwrap();
    assert!(attempts.len() > 1, "{attempts:?}");
    assert!(
        attempts.values().all(|numbers| numbers == &[1, 2]),
        "{attempts:?}"
    );
    assert_eq!(*decoys.lock().unwrap(), 0);
    assert_eq!(stored, vec![(None, Some(2)); attempts.len()]);
}

/// What one attempt of a handler sees of its own writes and of its
/// captured Run's.
#[derive(Debug, PartialEq)]
struct Seen {
    run_id: RunId,
    /// The staged and the captured keys, before it writes either.
    found: [Option<u32>; 2],
    through_attempt: (Option<u32>, Option<(Vec<u8>, String)>),
    through_program: (Option<u32>, bool, Option<u32>),
    /// Whether the deletion says the bytes existed, and whether the
    /// attempt still reads them.
    deleted: (bool, bool),
}

/// What the attempt `number` of the Run `run_id` sees, having first found
/// `found`.
fn sees(run_id: &RunId, number: u8, found: [Option<u32>; 2]) -> Seen {
    let written = Some(u32::from(number));
    Seen {
        run_id: run_id.clone(),
        found,
        through_attempt: (written, Some((vec![number], "image/png".to_owned()))),
        through_program: (None, false, written),
        deleted: (true, false),
    }
}

/// Writes the attempt `number`'s values through its own store, `staged`,
/// and through the program's, `captured`, and answers what each then
/// reads.
async fn write_both(
    run_id: RunId,
    staged: KeyValueStore,
    captured: KeyValueStore,
    number: u8,
) -> Result<Seen, Error> {
    let value = u32::from(number);
    let found = [staged.get("staged").await?, staged.get("captured").await?];
    staged.set("staged", &value).await?;
    staged.set_bytes("bytes", [number], "image/png").await?;
    captured.set("captured", &value).await?;
    let bytes = staged.get_bytes("bytes").await?;
    Ok(Seen {
        run_id,
        found,
        through_attempt: (
            staged.get("staged").await?,
            bytes.map(|bytes| (bytes.content, bytes.content_type)),
        ),
        through_program: (
            captured.get("staged").await?,
            captured.get_bytes("bytes").await?.is_some(),
            captured.get("captured").await?,
        ),
        deleted: (
            staged.delete("bytes").await?,
            staged.get_bytes("bytes").await?.is_some(),
        ),
    })
}

/// The trap: the program's own Run value, captured by its handler, commits
/// at once and stays though its attempt fails; the attempt's own writes
/// are staged with it.
///
/// Covers: story:11-1064, operation:key_value_store.get,
/// operation:key_value_store.set, operation:key_value_store.get_bytes,
/// operation:key_value_store.set_bytes, operation:key_value_store.delete
#[test]
fn a_write_through_the_attempts_run_is_staged_where_a_captured_runs_commits_at_once() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let program = Arc::new(OnceLock::<KeyValueStore>::new());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut crawl = Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    let given = (Arc::clone(&program), Arc::clone(&seen));
    crawl
        .handle(&page, move |ctx: CrawlContext<()>| {
            let (program, seen) = given.clone();
            async move {
                let number = u8::try_from(ctx.attempt())?;
                let captured = program.get().ok_or("the Run has not started")?.clone();
                let run = ctx.run();
                let written =
                    write_both(run.id().clone(), run.key_value_store(), captured, number).await?;
                seen.lock().unwrap().push(written);
                if number == 1 {
                    return Err("the first attempt fails after its writes".into());
                }
                Ok(())
            }
        })
        .unwrap();

    let (run_id, committed) = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(retrying(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&start, &page, ())])
            .await
            .unwrap();
        let store = run.run().key_value_store();
        program.set(store.clone()).unwrap();
        crawl.serve(&run).await.unwrap();
        let committed = (
            store.get::<u32>("staged").await.unwrap(),
            store.get_bytes("bytes").await.unwrap(),
            store.get::<u32>("captured").await.unwrap(),
        );
        foxhound.close().await.unwrap();
        (run.id().clone(), committed)
    });

    // The failed attempt's staged write is gone, and its captured one stays.
    assert_eq!(
        *seen.lock().unwrap(),
        [
            sees(&run_id, 1, [None, None]),
            sees(&run_id, 2, [None, Some(1)]),
        ]
    );
    assert_eq!(committed, (Some(2), None, Some(2)));
}

/// A blocking handler's `ctx.run()` stages its writes the same way.
///
/// Covers: story:11-1064
#[test]
fn a_blocking_handlers_run_stages_its_writes_with_its_attempt() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let foxhound = blocking::Foxhound::connect(retrying(&env, &site)).unwrap();
    let found = Arc::new(Mutex::new(Vec::new()));
    let mut crawl = blocking::Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    let given = Arc::clone(&found);
    crawl
        .handle(&page, move |ctx: blocking::CrawlContext<()>| {
            let run: blocking::AttemptRun = ctx.run();
            let store = run.key_value_store();
            let before = store.get::<u32>("staged").send()?;
            store.set("staged", &ctx.attempt()).send()?;
            let after = store.get::<u32>("staged").send()?;
            let found = (run.id().clone(), before, after);
            given.lock().unwrap().push(found);
            if ctx.attempt() == 1 {
                return Err("the first attempt fails after its write".into());
            }
            Ok(())
        })
        .unwrap();

    let run = foxhound
        .start_crawl(&crawl, [Seed::new(&start, &page, ())])
        .send()
        .unwrap();
    crawl.serve(&run).unwrap();
    let committed = run
        .run()
        .key_value_store()
        .get::<u32>("staged")
        .send()
        .unwrap();
    foxhound.close().unwrap();

    let run_id = run.id();
    assert_eq!(
        *found.lock().unwrap(),
        [
            (run_id.clone(), None, Some(1)),
            (run_id.clone(), None, Some(2)),
        ]
    );
    assert_eq!(committed, Some(2));
}

/// What one handler read through its `ctx.run()`: each committed Record's
/// value, then each rejection's number and Record.
type Committed = (Vec<Value>, Vec<(u64, Value)>);

/// What each request pushes before it reads: the first a Record the Run
/// commits and one the engine rejects for its missing key, the second one
/// more Record.
fn pushed(first: bool) -> Vec<Value> {
    if first {
        vec![json!({ "id": "kept" }), json!({ "name": "keyless" })]
    } else {
        vec![json!({ "id": "own" })]
    }
}

/// What the two requests read: the first nothing, its own pushes staged,
/// the second the first's committed Record and rejection, not its own.
fn committed_before_each() -> Vec<Committed> {
    vec![
        (vec![], vec![]),
        (
            vec![json!({ "id": "kept" })],
            vec![(1, json!({ "name": "keyless" }))],
        ),
    ]
}

/// The settings of the reads' crawls: one handler call at a time, each
/// attempt's outcome written as it ends, so the first request has committed
/// before the second runs. At `batch` a reader sees it only once its batch
/// is written (ADR 0142).
fn one_at_a_time(env: &TestEnv, site: &ProvingGround) -> ConnectOptions {
    let settings = site_settings(site)
        .set(handler::CONCURRENCY, 1_u64)
        .set(run_state::DURABILITY, run_state::Durability::Full);
    super::options(env).settings(settings)
}

/// A handler's `ctx.run()` reads the Records and rejections its Run had
/// committed, its own attempt's staged ones not among them, and each read
/// ends though the Run is live, which the handler's own request keeps from
/// ending (ADR 0103).
///
/// Covers: story:11-1060, story:11-1061
#[test]
fn a_handlers_run_reads_what_had_committed_and_ends_while_the_run_is_live() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let read = Arc::new(Mutex::new(Vec::<Committed>::new()));
    let mut crawl = Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    let notes = crawl
        .dataset(DatasetDeclaration::<Value>::new("notes").key(&["id"]))
        .unwrap();
    let given = (page.clone(), notes, start.clone(), Arc::clone(&read));
    crawl
        .handle(&page, move |ctx: CrawlContext<()>| {
            let (page, notes, start, read) = given.clone();
            async move {
                let first = read.lock().unwrap().is_empty();
                for record in pushed(first) {
                    match ctx.push(&notes, &record).await {
                        Err(error) if error.code() != error_code::RECORD_INVALID => {
                            return Err(error.into());
                        }
                        _ => {}
                    }
                }
                let run = ctx.run();
                let mut values = Vec::new();
                let mut records = run.records(&notes);
                while let Some(record) = records.next().await {
                    values.push(record?.value);
                }
                let mut rejections = Vec::new();
                let mut entries = run.rejected_records(&notes);
                while let Some(entry) = entries.next().await {
                    let entry = entry?;
                    rejections.push((entry.number, entry.data));
                }
                read.lock().unwrap().push((values, rejections));
                if first {
                    ctx.enqueue(start, &page, ()).key("second").await?;
                }
                Ok(())
            }
        })
        .unwrap();

    let report = real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(one_at_a_time(&env, &site)).await.unwrap();
        let run = foxhound
            .start_crawl(&crawl, [Seed::new(&start, &page, ())])
            .await
            .unwrap();
        let report = crawl.serve(&run).await.unwrap();
        foxhound.close().await.unwrap();
        report
    });

    assert_eq!(report.report.lifecycle, Lifecycle::Finished, "{report:?}");
    assert_eq!(*read.lock().unwrap(), committed_before_each());
}

/// A blocking handler's `ctx.run()` reads what had committed the same way,
/// each read an iterator that ends.
///
/// Covers: story:11-1060, story:11-1061
#[test]
fn a_blocking_handlers_run_reads_what_had_committed_and_ends() {
    let env = TestEnv::new();
    let site = ProvingGround::start(&env, SEED).unwrap();
    let start = site.start_url(TASK).unwrap();
    let foxhound = blocking::Foxhound::connect(one_at_a_time(&env, &site)).unwrap();
    let read = Arc::new(Mutex::new(Vec::<Committed>::new()));
    let mut crawl = blocking::Crawl::new();
    let page = crawl.label::<()>("page").unwrap();
    let notes = crawl
        .dataset(DatasetDeclaration::<Value>::new("notes").key(&["id"]))
        .unwrap();
    let given = (page.clone(), start.clone(), Arc::clone(&read));
    crawl
        .handle(&page, move |ctx: blocking::CrawlContext<()>| {
            let (page, start, read) = &given;
            let first = read.lock().unwrap().is_empty();
            for record in pushed(first) {
                match ctx.push(&notes, &record) {
                    Err(error) if error.code() != error_code::RECORD_INVALID => {
                        return Err(error.into());
                    }
                    _ => {}
                }
            }
            let run = ctx.run();
            let values = run
                .records(&notes)
                .map(|record| record.map(|record| record.value))
                .collect::<Result<Vec<_>, _>>()?;
            let rejections = run
                .rejected_records(&notes)
                .map(|entry| entry.map(|entry| (entry.number, entry.data)))
                .collect::<Result<Vec<_>, _>>()?;
            read.lock().unwrap().push((values, rejections));
            if first {
                ctx.enqueue(start.clone(), page, ()).key("second").send()?;
            }
            Ok(())
        })
        .unwrap();

    let run = foxhound
        .start_crawl(&crawl, [Seed::new(&start, &page, ())])
        .send()
        .unwrap();
    let report = crawl.serve(&run).unwrap();
    foxhound.close().unwrap();

    assert_eq!(report.report.lifecycle, Lifecycle::Finished, "{report:?}");
    assert_eq!(*read.lock().unwrap(), committed_before_each());
}
