//! The Resource governor's shedding ladder on Collie's executable over
//! stdio (spec #213's Seam 1; ADR 0040), with a small
//! `governor.memory_allowance` in the user configuration, which the
//! governor reads again at every sample, and the Proving ground's heavy
//! page: at critical an idle browser closes; a page past
//! `governor.browser.page_memory_cap` ends with `page_memory_exceeded`,
//! which is retryable; and at the last resort the Run with the largest
//! footprint checkpoints and stops, `stopped` and `incomplete` with
//! `resource: "memory"`, while a Run without a browser goes on once memory
//! frees up. Each test has its own Goodfetch home, Proving ground and
//! Browser cache (ADRs 0059 and 0094).
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

use crate::page::{browser_command_lines, opened, started_with};
use goodfetch_test_client::Attached;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 236;

const MIB: u64 = 1 << 20;

/// How long a browser has to start or to exit, or the governor to act,
/// before the test judges what it sees.
const SETTLE: Duration = Duration::from_secs(30);

/// A Memory allowance no engine fits in: memory is critical at every
/// sample.
const CRITICAL: &str = "governor.memory_allowance = 1\n";

/// The user configuration's samples every 100 ms, with `more`.
fn configure(env: &TestEnv, more: &str) {
    env.user_configuration(&format!("governor.sample_interval = 100\n{more}"));
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
    let mut engine = started_with(&env, &cache, &proving_ground, "idle", &Map::new(), |_| {});
    let session = engine.ask(2, "run.new_session", &json!({ "run_id": "idle" }));

    let launched = browsers_once(&env, 1);
    configure(&env, CRITICAL);
    let critical = browsers_once(&env, 0);
    configure(&env, "");
    let _ = engine.ask(3, "run.finish", &json!({ "run_id": "idle" }));
    let _ = engine.finish();

    assert_eq!(
        (session.get("result").is_some(), launched, critical),
        (true, 1, 0),
        "{session}"
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
    let mut engine = started_with(&env, &cache, &proving_ground, "heavy", &settings, |_| {});
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "heavy" })));
    let loaded = heavy(&mut engine, 3, &proving_ground, &page, 64);

    configure(&env, CRITICAL);
    let deadline = Instant::now() + SETTLE;
    let mut id = 4;
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
    configure(&env, "");
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
            &json!({ "key": "governor.browser.page_memory_cap", "value": 32 * MIB, "layer": "run" }),
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
    let mut engine = started_with(&env, &cache, &proving_ground, "heavy", &Map::new(), |_| {});
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "heavy" })));
    let loaded = heavy(&mut engine, 3, &proving_ground, &page, 128);
    let light = engine.ask(4, "run.start", &json!({ "run_id": "fetching" }));
    engine.send(
        &json!({ "jsonrpc": "2.0", "id": "events", "method": "events.subscribe", "params": {} }),
    );
    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "events", "credits": 64 } }));

    // Samples every 500 ms, so the test takes memory back in time: after
    // a stop, the governor measures again from the second sample on.
    env.user_configuration(&format!(
        "governor.sample_interval = 500\ngovernor.last_resort_after = 1000\n{CRITICAL}"
    ));
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
    configure(&env, "");
    let heavy_report = answer(&mut engine, 5, "run.finish", &json!({ "run_id": "heavy" }));
    let light_report = answer(
        &mut engine,
        6,
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
