//! The Resource governor's shedding ladder on Collie's executable over
//! stdio (spec #213's Seam 1; ADR 0040), with a small
//! `governor.memory_allowance` a person writes through the engine
//! (`config.user_write`), which the governor follows from its next sample,
//! and the Proving ground's heavy page: at critical an idle browser closes;
//! a page past
//! `governor.browser.page_memory_cap` ends with `page_memory_exceeded`,
//! which is retryable; and at the last resort the Run with the largest
//! footprint checkpoints and stops, `stopped` and `incomplete` with
//! `resource: "memory"`, while a Run without a browser goes on once memory
//! frees up. The same allowance edited in the file by hand while the engine
//! runs is in force from the next Run's start. Each test has its own
//! Goodfetch home, Proving ground and Browser cache (ADRs 0059 and 0094).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test lists the engine's browser processes and waits for the governor's samples on its own thread, never an async worker"
)]

use std::time::{Duration, Instant};

use goodfetch_protocol::page::PageOpened;
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, Value, json};

use crate::page::{browser_command_lines, next_sample, opened, started_by_a_person};
use goodfetch_test_client::Attached;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 236;

const MIB: u64 = 1 << 20;

/// How long a browser has to start or to exit, or the governor to act,
/// before the test judges what it sees.
const SETTLE: Duration = Duration::from_secs(30);

/// The Memory allowance's key.
const ALLOWANCE: &str = "governor.memory_allowance";

/// A Memory allowance no engine fits in: memory is critical at every
/// sample.
const CRITICAL: u64 = 1;

/// Writes the user configuration by hand, its samples every 100 ms, with
/// `more`: before the engine starts, or as a hand edit the engine is not
/// told of.
fn configure(env: &TestEnv, more: &str) {
    env.user_configuration(&format!("governor.sample_interval = 100\n{more}"));
}

/// Writes `changes`, user-level values by key, `null` removing one, as the
/// call `id` of `config.user_write`, a person's write through the engine:
/// the governor follows it from its next sample (ADR 0040).
fn write(engine: &mut Attached, id: i64, changes: &[(&str, Value)]) {
    let changes: Vec<Value> = changes
        .iter()
        .map(|(key, value)| json!({ "key": key, "value": value }))
        .collect();
    let written = answer(
        engine,
        id,
        "config.user_write",
        &json!({ "changes": changes }),
    );
    assert!(written.get("result").is_some(), "{written}");
}

/// The browser processes the test's engine runs, helpers left out.
fn browsers(env: &TestEnv) -> usize {
    browser_command_lines(env)
        .iter()
        .filter(|line| !line.contains("--type="))
        .count()
}

/// The number of browser processes once it is `expected`, or after
/// [`SETTLE`] whatever it is then.
fn browsers_once(env: &TestEnv, expected: usize) -> usize {
    let deadline = Instant::now() + SETTLE;
    loop {
        let found = browsers(env);
        if found == expected || Instant::now() > deadline {
            return found;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Loads the heavy page, which keeps `mib` MiB, on `page`.
fn heavy(
    engine: &mut Attached,
    id: i64,
    proving_ground: &ProvingGround,
    page: &PageOpened,
    mib: u64,
) -> Value {
    let url = proving_ground.url("scope.test", &format!("/heavy?mib={mib}"));
    engine.ask(
        id,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    )["result"]["response"]["status"]
        .clone()
}

/// The answer to request `id`, the events in between skipped.
fn answer(engine: &mut Attached, id: i64, method: &str, params: &Value) -> Value {
    engine.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    loop {
        let message = engine.next().unwrap();
        if message["id"] == json!(id) {
            return message;
        }
    }
}

/// At critical, a browser with no open page closes at the next sample:
/// the shedding ladder's first rung.
///
/// Covers: story:3-59, setting:governor.memory_allowance, operation:run.new_session
#[test]
fn at_critical_an_idle_browser_closes() {
    let env = TestEnv::new();
    configure(&env, "");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_by_a_person(&env, &cache, &proving_ground, "idle", &Map::new());
    let session = engine.ask(2, "run.new_session", &json!({ "run_id": "idle" }));

    let launched = browsers_once(&env, 1);
    write(&mut engine, 3, &[(ALLOWANCE, json!(CRITICAL))]);
    let critical = browsers_once(&env, 0);
    write(&mut engine, 4, &[(ALLOWANCE, Value::Null)]);
    let _ = engine.ask(5, "run.finish", &json!({ "run_id": "idle" }));
    let _ = engine.finish();

    assert_eq!(
        (session.get("result").is_some(), launched, critical),
        (true, 1, 0),
        "{session}"
    );
}

/// A hand edit of the user configuration while the engine runs is no write
/// of the engine's: the governor follows it from the engine's next read of
/// the file, the next Run's start at the latest, which resolves the
/// machine's settings again (ADR 0040). Nothing reads the file while one
/// Run idles, so an idle browser stays through three samples after the
/// Memory allowance was edited to critical by hand, none of them under
/// that allowance, and closes once another Run starts.
///
/// Covers: story:3-59, setting:governor.memory_allowance, operation:run.start
#[test]
fn a_hand_edit_of_the_memory_allowance_is_in_force_from_the_next_runs_start() {
    let env = TestEnv::new();
    configure(&env, "");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_by_a_person(&env, &cache, &proving_ground, "idle", &Map::new());
    let session = engine.ask(2, "run.new_session", &json!({ "run_id": "idle" }));
    let launched = browsers_once(&env, 1);
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "samples", "method": "events.subscribe",
        "params": { "system_info": true },
    }));
    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "samples", "credits": 1 } }));

    configure(&env, &format!("{ALLOWANCE} = {CRITICAL}\n"));
    let sampled: Vec<Value> = (0..3)
        .map(|_| next_sample(&mut engine)["memory"]["allowance"].clone())
        .collect();
    let before_the_next_run = browsers(&env);
    let next = answer(&mut engine, 3, "run.start", &json!({ "run_id": "next" }));
    let from_the_next_run = browsers_once(&env, 0);
    write(&mut engine, 4, &[(ALLOWANCE, Value::Null)]);
    let _ = answer(&mut engine, 5, "run.finish", &json!({ "run_id": "idle" }));
    let _ = answer(&mut engine, 6, "run.finish", &json!({ "run_id": "next" }));
    let _ = engine.finish();

    assert_eq!(
        (
            session.get("result").is_some(),
            launched,
            sampled.contains(&json!(CRITICAL)),
            before_the_next_run,
            next.get("result").is_some(),
            from_the_next_run,
        ),
        (true, 1, false, 1, true, 0),
        "{session}\n{sampled:?}\n{next}"
    );
}

/// At critical, a page holding more than `governor.browser.page_memory_cap`
/// ends once no call on it is in flight, and its next call raises
/// `page_memory_exceeded`, retryable, naming the setting; a new page opens.
///
/// Covers: story:3-59, error:page_memory_exceeded, setting:governor.browser.page_memory_cap, operation:page.formats
#[test]
fn at_critical_a_page_past_its_memory_cap_ends_with_page_memory_exceeded() {
    let env = TestEnv::new();
    configure(&env, "");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut settings = Map::new();
    settings.insert(
        "governor".to_owned(),
        json!({ "browser": { "page_memory_cap": 32 * MIB } }),
    );
    let mut engine = started_by_a_person(&env, &cache, &proving_ground, "heavy", &settings);
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "heavy" })));
    let loaded = heavy(&mut engine, 3, &proving_ground, &page, 64);

    write(&mut engine, 4, &[(ALLOWANCE, json!(CRITICAL))]);
    let deadline = Instant::now() + SETTLE;
    let mut id = 5;
    let ended = loop {
        let read = engine.ask(
            id,
            "page.formats",
            &json!({ "page_id": page.page_id, "formats": {} }),
        );
        if read.get("error").is_some() || Instant::now() > deadline {
            break read;
        }
        id += 1;
        std::thread::sleep(Duration::from_millis(100));
    };
    write(&mut engine, 999, &[(ALLOWANCE, Value::Null)]);
    let again = engine.ask(1000, "run.new_page", &json!({ "run_id": "heavy" }));
    let _ = engine.ask(1001, "run.finish", &json!({ "run_id": "heavy" }));
    let _ = engine.finish();

    let data = &ended["error"]["data"];
    assert_eq!(
        (
            loaded,
            &data["code"],
            &data["retryable"],
            &data["page_id"],
            &data["limit"],
            data["memory"]
                .as_u64()
                .is_some_and(|memory| memory > 32 * MIB),
            again.get("result").is_some(),
        ),
        (
            json!(200),
            &json!("page_memory_exceeded"),
            &json!(true),
            &json!(page.page_id),
            &json!({ "key": "governor.browser.page_memory_cap", "value": 32 * MIB, "layer": "run", "origin": "settings" }),
            true,
            true,
        ),
        "{ended}"
    );
}

/// After `governor.last_resort_after` critical at the floor, the Run with
/// the largest footprint, the one holding the heavy page, checkpoints and
/// stops, `stopped` and `incomplete` with `resource: "memory"`; the Run
/// with no browser, first by its Run ID, goes on, and finishes once memory
/// frees up.
///
/// Covers: story:3-59, setting:governor.last_resort_after, event:run.axes_changed
#[test]
fn at_the_last_resort_the_run_with_the_largest_footprint_stops_and_another_goes_on() {
    let env = TestEnv::new();
    configure(&env, "");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_by_a_person(&env, &cache, &proving_ground, "heavy", &Map::new());
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "heavy" })));
    let loaded = heavy(&mut engine, 3, &proving_ground, &page, 128);
    let light = engine.ask(4, "run.start", &json!({ "run_id": "fetching" }));
    engine.send(
        &json!({ "jsonrpc": "2.0", "id": "events", "method": "events.subscribe", "params": {} }),
    );
    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "events", "credits": 64 } }));

    // Samples every 500 ms, so the test takes memory back in time: after
    // a stop, the governor measures again from the second sample on.
    write(
        &mut engine,
        5,
        &[
            ("governor.sample_interval", json!(500)),
            ("governor.last_resort_after", json!(1000)),
            (ALLOWANCE, json!(CRITICAL)),
        ],
    );
    let stopped = loop {
        let message = engine.next().unwrap();
        let events = message["params"]["events"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(event) = events
            .into_iter()
            .find(|event| event["data"]["lifecycle"] == "stopped")
        {
            break event;
        }
    };
    write(
        &mut engine,
        6,
        &[
            ("governor.sample_interval", json!(100)),
            ("governor.last_resort_after", Value::Null),
            (ALLOWANCE, Value::Null),
        ],
    );
    let heavy_report = answer(&mut engine, 7, "run.finish", &json!({ "run_id": "heavy" }));
    let light_report = answer(
        &mut engine,
        8,
        "run.finish",
        &json!({ "run_id": "fetching" }),
    );
    let _ = engine.finish();

    assert_eq!(
        (
            loaded,
            light.get("result").is_some(),
            &stopped["data"]["run_id"],
            &heavy_report["result"]["lifecycle"],
            &heavy_report["result"]["completeness"],
            &heavy_report["result"]["stop_reason"],
            heavy_report["result"]["checkpoint"].is_object(),
            &light_report["result"]["lifecycle"],
        ),
        (
            json!(200),
            true,
            &json!("heavy"),
            &json!("stopped"),
            &json!("incomplete"),
            &json!({ "kind": "resources", "resource": "memory" }),
            true,
            &json!("finished"),
        ),
        "{heavy_report}\n{light_report}"
    );
}
