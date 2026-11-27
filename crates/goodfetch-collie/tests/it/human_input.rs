//! Human-like input in `stealth` Sessions through Collie's executable
//! against the Proving ground's pointer recorder (`/pointer`, ADR 0035;
//! spec #599, #1018): curved pointer paths, wheel scrolling and key-by-key
//! typing as native trusted CDP events inside the guard's deadlines; direct
//! input when `stealth.human_input` is off; ghost-cursor fallback with
//! `human_input_fallback` when Cursory's recordings are absent offline;
//! runtime-loaded replaceable recordings from the Browser cache with no
//! fallback warning; and zero recording bytes embedded in the executable.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test writes replaceable cache recordings and reads the Run trace and executable on its own thread, never an async worker"
)]

use std::fs;
use std::path::Path;

use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_protocol::page::PageId;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, Value, json};

use crate::COLLIE;
use crate::page::started_with;
use crate::sessions::{page_in, session_of};

const SEED: u64 = 1018;

/// Encodes `payload` (< 65 KiB) as an RFC 1952 gzip stream using a single
/// uncompressed DEFLATE stored block so the test needs no compression crate.
fn gzip_stored(payload: &[u8]) -> Vec<u8> {
    let len = u16::try_from(payload.len()).expect("test fixture fits one DEFLATE stored block");
    let mut crc = 0xffff_ffff_u32;
    for &byte in payload {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    let crc = !crc;
    let mut out = Vec::with_capacity(payload.len() + 23);
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0xff]);
    out.push(0x01);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&u32::from(len).to_le_bytes());
    out
}

/// Builds a synthetic Cursory-format `trajectories.json.gz` with arcs bowing
/// `bow` pixels off the straight line.
fn cursory_fixture(bow: f64) -> Vec<u8> {
    let arc = |length: f64, samples: u32| -> Value {
        let points: Vec<[f64; 2]> = (0..=samples)
            .map(|step| {
                let t = f64::from(step) / f64::from(samples);
                [length * t, bow * (std::f64::consts::PI * t).sin()]
            })
            .collect();
        let timing: Vec<f64> = (0..=samples)
            .map(|step| 1000.0 + 16.0 * f64::from(step))
            .collect();
        json!({
            "start": points[0],
            "end": points[points.len() - 1],
            "points": points,
            "timing": timing,
            "dx": length,
            "dy": 0.0,
            "length": length,
        })
    };
    let json = json!([arc(150.0, 20), arc(350.0, 35), arc(650.0, 50)]).to_string();
    gzip_stored(json.as_bytes())
}

/// Maximum perpendicular distance of `points` from the straight line
/// connecting the first and last point.
fn max_bow(points: &[(f64, f64)]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let from = points[0];
    let to = points[points.len() - 1];
    let length = (to.0 - from.0).hypot(to.1 - from.1);
    if length < 1e-6 {
        return 0.0;
    }
    points
        .iter()
        .map(|&(x, y)| {
            ((to.0 - from.0) * (from.1 - y) - (from.0 - x) * (to.1 - from.1)).abs() / length
        })
        .fold(0.0, f64::max)
}

/// Offline `stealth` Run settings (headless so tests need no display), with
/// `stealth.human_input` set to `human_input`.
fn stealth_settings(human_input: bool) -> Map<String, Value> {
    Map::from_iter([
        (
            "browser".to_owned(),
            json!({ "access_mode": "stealth", "headless": true }),
        ),
        ("stealth".to_owned(), json!({ "human_input": human_input })),
        ("egress".to_owned(), json!({ "offline": true })),
    ])
}

/// Reads the pointer recorder summary JSON from `#summary` via `page.evaluate`.
fn recorded(engine: &mut goodfetch_test_client::Attached, id: i64, page: &PageId) -> Value {
    let evaluated = engine.ask(
        id,
        "page.evaluate",
        &json!({
            "page_id": page,
            "function": "() => ({ summary: JSON.parse(document.querySelector('#summary').textContent), field: document.querySelector('#field').value })",
        }),
    );
    assert!(evaluated.get("result").is_some(), "{evaluated}");
    evaluated["result"]["value"].clone()
}

/// Reads `action: "human_input"` entries from the Run's `provenance.jsonl`.
fn human_input_provenance(home: &Path, run_id: &str) -> Vec<Value> {
    let path = HomeLayout::of(home).run_trace(run_id, RUN_TRACE_PROVENANCE);
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| entry["action"] == json!("human_input"))
        .collect()
}

/// Extracts `(x, y)` coordinates from recorded `moves`.
fn move_points(moves: &[Value]) -> Vec<(f64, f64)> {
    moves
        .iter()
        .map(|step| (step["x"].as_f64().unwrap(), step["y"].as_f64().unwrap()))
        .collect()
}

/// Extracts warning codes from a `run.finish` result.
fn warning_codes(finished: &Value) -> Vec<String> {
    finished["result"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|warning| warning["code"].as_str().map(str::to_owned))
        .collect()
}

/// In a `stealth` Session with `stealth.human_input` on and no recordings
/// in the Browser cache (offline), clicks curve to their target, filling
/// types trusted keys producing the exact value, off-screen targets scroll
/// via trusted wheel notches, Provenance marks `engine_actions_only`, and
/// the Run report carries `human_input_fallback`.
///
/// Covers: story:8-42, story:8-43, story:8-45, setting:stealth.human_input
#[test]
fn stealth_human_input_curves_types_scrolls_and_warns_when_recordings_absent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let settings = stealth_settings(true);
    let mut engine = started_with(&env, &cache, &proving_ground, "human", &settings, |_| {});

    let session = session_of(&engine.ask(2, "run.new_session", &json!({ "run_id": "human" })));
    let page = page_in(&mut engine, 3, &session).page_id;
    let url = proving_ground.url("access.test", "/pointer");
    let loaded = engine.ask(4, "page.goto", &json!({ "page_id": page, "url": url }));
    assert!(loaded.get("result").is_some(), "{loaded}");

    let clicked = engine.ask(
        5,
        "locator.click",
        &json!({ "locator": { "page_id": page, "selector": "#target" } }),
    );
    assert!(clicked.get("result").is_some(), "{clicked}");

    let after_click = recorded(&mut engine, 6, &page);
    let moves = after_click["summary"]["moves"].as_array().unwrap();
    let clicks = after_click["summary"]["clicks"].as_array().unwrap();
    let points = move_points(moves);
    assert!(
        moves.len() > 3,
        "expected curved approach moves, got {} moves: {moves:?}",
        moves.len()
    );
    assert!(
        moves.iter().all(|step| step["trusted"] == json!(true)),
        "every move must be a trusted native event"
    );
    assert!(
        max_bow(&points) > 3.0,
        "expected non-straight mouse path, max bow was {:.2} across {points:?}",
        max_bow(&points)
    );
    assert_eq!(clicks.len(), 1);
    assert_eq!(clicks[0]["id"], json!("target"));
    assert_eq!(clicks[0]["trusted"], json!(true));

    let filled = engine.ask(
        7,
        "locator.fill",
        &json!({
            "locator": { "page_id": page, "selector": "#field" },
            "value": "hello world 42",
        }),
    );
    assert!(filled.get("result").is_some(), "{filled}");

    let after_fill = recorded(&mut engine, 8, &page);
    let keys = after_fill["summary"]["keys"].as_array().unwrap();
    assert_eq!(after_fill["field"], json!("hello world 42"));
    assert!(
        keys.len() >= "hello world 42".len(),
        "expected key-by-key typing, got {} keys",
        keys.len()
    );
    assert!(
        keys.iter().all(|step| step["trusted"] == json!(true)),
        "every keydown must be trusted"
    );

    let scrolled_click = engine.ask(
        9,
        "locator.click",
        &json!({ "locator": { "page_id": page, "selector": "#far" } }),
    );
    assert!(scrolled_click.get("result").is_some(), "{scrolled_click}");

    let after_scroll = recorded(&mut engine, 10, &page);
    let wheels = after_scroll["summary"]["wheels"].as_array().unwrap();
    let clicks_all = after_scroll["summary"]["clicks"].as_array().unwrap();
    assert!(
        !wheels.is_empty(),
        "expected native wheel events for off-screen target"
    );
    assert!(wheels.iter().all(|step| step["trusted"] == json!(true)));
    assert_eq!(clicks_all.len(), 2);
    assert_eq!(clicks_all[1]["id"], json!("far"));

    let finished = engine.ask(11, "run.finish", &json!({ "run_id": "human" }));
    let _ = engine.finish();

    let warnings = warning_codes(&finished);
    assert!(
        warnings.iter().any(|code| code == "human_input_fallback"),
        "expected human_input_fallback warning, got {warnings:?}"
    );
    let provenance = human_input_provenance(env.home(), "human");
    assert_eq!(provenance.len(), 1, "{provenance:?}");
    assert_eq!(provenance[0]["session_id"], json!(session));
    assert_eq!(provenance[0]["source"], json!("ghost_cursor"));
    assert_eq!(provenance[0]["fell_back"], json!(true));
    assert_eq!(provenance[0]["scope"], json!("engine_actions_only"));
}

/// With `stealth.human_input` off, a click goes directly to its target with
/// a single move and no fallback warning.
///
/// Covers: story:8-42, setting:stealth.human_input
#[test]
fn stealth_with_human_input_off_clicks_directly_without_fallback_warning() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let settings = stealth_settings(false);
    let mut engine = started_with(&env, &cache, &proving_ground, "direct", &settings, |_| {});

    let session = session_of(&engine.ask(2, "run.new_session", &json!({ "run_id": "direct" })));
    let page = page_in(&mut engine, 3, &session).page_id;
    let url = proving_ground.url("access.test", "/pointer");
    let loaded = engine.ask(4, "page.goto", &json!({ "page_id": page, "url": url }));
    assert!(loaded.get("result").is_some(), "{loaded}");

    let clicked = engine.ask(
        5,
        "locator.click",
        &json!({ "locator": { "page_id": page, "selector": "#target" } }),
    );
    assert!(clicked.get("result").is_some(), "{clicked}");

    let after_click = recorded(&mut engine, 6, &page);
    let moves = after_click["summary"]["moves"].as_array().unwrap();
    let clicks = after_click["summary"]["clicks"].as_array().unwrap();
    assert_eq!(moves.len(), 1, "expected one direct move, got {moves:?}");
    assert_eq!(clicks.len(), 1);
    assert_eq!(clicks[0]["id"], json!("target"));

    let finished = engine.ask(7, "run.finish", &json!({ "run_id": "direct" }));
    let _ = engine.finish();

    let warnings = warning_codes(&finished);
    assert!(
        !warnings.iter().any(|code| code == "human_input_fallback"),
        "expected no human_input_fallback warning when off, got {warnings:?}"
    );
    assert_eq!(
        human_input_provenance(env.home(), "direct"),
        Vec::<Value>::new()
    );
}

/// Placing replaceable Cursory-format recordings in the Browser cache uses
/// them with no `human_input_fallback` warning, and Provenance names
/// `cursory`.
///
/// Covers: story:8-43, setting:stealth.human_input
#[test]
fn replaceable_cursory_recordings_in_cache_avoid_fallback_warning() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let recordings = cache
        .join("mouse_movements")
        .join("cursory")
        .join("trajectories.json.gz");
    fs::create_dir_all(recordings.parent().unwrap()).unwrap();
    fs::write(&recordings, cursory_fixture(55.0)).unwrap();

    let settings = stealth_settings(true);
    let mut engine = started_with(&env, &cache, &proving_ground, "cached", &settings, |_| {});
    let session = session_of(&engine.ask(2, "run.new_session", &json!({ "run_id": "cached" })));
    let page = page_in(&mut engine, 3, &session).page_id;
    let url = proving_ground.url("access.test", "/pointer");
    let loaded = engine.ask(4, "page.goto", &json!({ "page_id": page, "url": url }));
    assert!(loaded.get("result").is_some(), "{loaded}");

    let clicked = engine.ask(
        5,
        "locator.click",
        &json!({ "locator": { "page_id": page, "selector": "#target" } }),
    );
    assert!(clicked.get("result").is_some(), "{clicked}");

    let after_click = recorded(&mut engine, 6, &page);
    let points = move_points(after_click["summary"]["moves"].as_array().unwrap());
    assert!(
        points.len() > 3 && max_bow(&points) > 5.0,
        "expected curved path from cached recordings, got {points:?}"
    );

    let finished = engine.ask(7, "run.finish", &json!({ "run_id": "cached" }));
    let _ = engine.finish();

    let warnings = warning_codes(&finished);
    assert!(
        !warnings.iter().any(|code| code == "human_input_fallback"),
        "expected no fallback warning with cached recordings, got {warnings:?}"
    );
    let provenance = human_input_provenance(env.home(), "cached");
    assert_eq!(provenance.len(), 1, "{provenance:?}");
    assert_eq!(provenance[0]["source"], json!("cursory"));
    assert_eq!(provenance[0]["fell_back"], json!(false));
    assert_eq!(provenance[0]["scope"], json!("engine_actions_only"));
}

/// Cursory's LGPL recordings are never compiled into the Collie executable:
/// the binary carries no Cursory recording archive bytes.
///
/// Covers: story:8-43
#[test]
fn collie_executable_carries_no_cursory_recording_bytes() {
    // First 32 bytes of Cursory's gzip archive (unique gzip header + deflate
    // prefix) and its unique first-trajectory coordinate signature.
    let marker_json = br#""start":[1004,150],"end":[1160,354]"#;
    let binary = fs::read(COLLIE).expect("collie test executable exists");
    assert!(
        !binary.windows(marker_json.len()).any(|w| w == marker_json),
        "collie executable must never embed Cursory recording bytes"
    );
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    assert!(
        !workspace_root
            .join("crates/goodfetch-page/src/human_input/trajectories.json.gz")
            .exists(),
        "Cursory's recordings must be downloaded at runtime, never checked into source"
    );
}
