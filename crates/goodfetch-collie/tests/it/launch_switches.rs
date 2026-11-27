//! Real-browser proof of the Chromium launch switches reconciled against
//! Playwright 1.64.1 on the pinned Chrome for Testing build (issue 2330):
//! exercises `launch.test/ipc-flood` (`--disable-ipc-flooding-protection`),
//! `launch.test/popup` (`--disable-popup-blocking`), and `launch.test/repost`
//! (`--disable-prompt-on-repost`), both with Collie's launched browser and
//! against a raw pinned Chrome launched without those switches.
#![expect(
    clippy::disallowed_methods,
    reason = "the test launches a comparison Chrome process and polls its debugging port on its own thread, never an async worker"
)]

use std::process::Stdio;

use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_protocol::page::PageId;
use goodfetch_test_client::Attached;
use goodfetch_test_support::{
    ProvingGround, TestEnv, browser_cache, chrome_headless, reported_debugging_port,
};
use serde_json::{Value, json};

use crate::page::{opened, started};

const SEED: u64 = 2330;

/// Observed results of the three `launch.test` behaviour cases on one page.
#[derive(Debug, PartialEq, Eq)]
struct Outcomes {
    ipc_hash: String,
    popup_result: String,
    popup_target_requests: usize,
    repost_reload_status: Value,
    repost_post_count: usize,
}

fn text_of(engine: &mut Attached, id: i64, page_id: &PageId, selector: &str) -> String {
    let read = engine.ask(
        id,
        "page.evaluate",
        &json!({
            "page_id": page_id,
            "function": format!("() => document.querySelector({selector:?})?.textContent ?? ''"),
        }),
    );
    read["result"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("expected string value: {read}"))
        .to_owned()
}

fn run_cases(
    engine: &mut Attached,
    page_id: &PageId,
    proving_ground: &ProvingGround,
    base_id: i64,
) -> Outcomes {
    let before_popup = count_requests(proving_ground, "GET", "/popup-target");
    let before_repost = count_requests(proving_ground, "POST", "/repost");

    // 1. IPC flood (`--disable-ipc-flooding-protection`)
    let flood = engine.ask(
        base_id,
        "page.goto",
        &json!({ "page_id": page_id, "url": proving_ground.url("launch.test", "/ipc-flood") }),
    );
    assert_eq!(flood["result"]["response"]["status"], json!(200), "{flood}");
    let ipc_hash = text_of(engine, base_id + 1, page_id, "#result");

    // 2. Unprompted popup (`--disable-popup-blocking`)
    let popup = engine.ask(
        base_id + 2,
        "page.goto",
        &json!({ "page_id": page_id, "url": proving_ground.url("launch.test", "/popup") }),
    );
    assert_eq!(popup["result"]["response"]["status"], json!(200), "{popup}");
    let popup_result = text_of(engine, base_id + 3, page_id, "#result");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while popup_result == "opened"
        && count_requests(proving_ground, "GET", "/popup-target") == before_popup
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let popup_target_requests =
        count_requests(proving_ground, "GET", "/popup-target") - before_popup;

    // 3. Form resubmission on reload (`--disable-prompt-on-repost`)
    let form = engine.ask(
        base_id + 4,
        "page.goto",
        &json!({ "page_id": page_id, "url": proving_ground.url("launch.test", "/repost") }),
    );
    assert_eq!(form["result"]["response"]["status"], json!(200), "{form}");
    let clicked = engine.ask(
        base_id + 5,
        "locator.click",
        &json!({ "locator": { "page_id": page_id, "selector": "#send" } }),
    );
    assert!(clicked.get("result").is_some(), "{clicked}");
    let posted = engine.ask(
        base_id + 6,
        "locator.wait_for",
        &json!({ "locator": { "page_id": page_id, "selector": "#result" } }),
    );
    assert!(posted.get("result").is_some(), "{posted}");
    let reloaded = engine.ask(base_id + 7, "page.reload", &json!({ "page_id": page_id }));
    let repost_reload_status = reloaded["result"]["response"]["status"].clone();
    let repost_post_count = count_requests(proving_ground, "POST", "/repost") - before_repost;

    Outcomes {
        ipc_hash,
        popup_result,
        popup_target_requests,
        repost_reload_status,
        repost_post_count,
    }
}

fn count_requests(proving_ground: &ProvingGround, method: &str, target: &str) -> usize {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| {
            request.host == "launch.test" && request.method == method && request.target == target
        })
        .count()
}

/// On the pinned Chrome for Testing build, Collie's reconciled launch passes
/// all three `launch.test` behaviour cases: 500 `history.replaceState` calls
/// complete without IPC throttling, an unprompted `window.open` popup opens
/// and loads `/popup-target`, and reloading a `POST` form resubmits cleanly.
/// Running the same cases against the same pinned binary without those three
/// switches shows which switches change behaviour on this build (issue 2330).
#[test]
fn reconciled_launch_switches_pass_ipc_flood_popup_and_repost_on_pinned_chromium() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();

    // Part 1: Collie's own launched browser (with reconciled switches).
    let mut engine = started(&env, &cache, &proving_ground, "with-switches");
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "with-switches" })));
    let with_switches = run_cases(&mut engine, &page.page_id, &proving_ground, 10);
    let _ = engine.ask(40, "run.finish", &json!({ "run_id": "with-switches" }));
    let _ = engine.finish();

    // Part 2: Raw pinned Chrome without `--disable-ipc-flooding-protection`,
    // `--disable-popup-blocking`, or `--disable-prompt-on-repost`, driven
    // through Collie's Borrowed `cdp_url` Session.
    let raw_profile = env.folder().join("raw-chrome-without-switches");
    let address = proving_ground.description().resolve["launch.test"].clone();
    let mut command = env.process_command(chrome_headless(&cache));
    command
        .args(chromium_background_traffic_off())
        .args([
            "--headless",
            "--no-first-run",
            "--no-default-browser-check",
            "--use-mock-keychain",
            "--password-store=basic",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--host-resolver-rules=MAP launch.test {address}"))
        .arg(format!("--user-data-dir={}", raw_profile.display()))
        .arg("about:blank")
        .stderr(Stdio::null());
    let raw_chrome = env.spawn(&mut command).unwrap();
    let port = reported_debugging_port(&raw_profile);
    let active = std::fs::read_to_string(raw_profile.join("DevToolsActivePort")).unwrap();
    let ws_path = active.lines().nth(1).unwrap().to_owned();
    let cdp_url = format!("ws://127.0.0.1:{port}{ws_path}");

    let mut borrowed = started(&env, &cache, &proving_ground, "without-switches");
    let session = borrowed.ask(
        2,
        "run.new_session",
        &json!({
            "run_id": "without-switches",
            "source": { "kind": "cdp_url", "url": cdp_url },
        }),
    );
    let session_id = session["result"]["session_id"]
        .as_str()
        .unwrap_or_else(|| panic!("expected borrowed session: {session}"))
        .to_owned();
    let raw_page =
        opened(&borrowed.ask(3, "session.new_page", &json!({ "session_id": session_id })));
    let without_switches = run_cases(&mut borrowed, &raw_page.page_id, &proving_ground, 10);
    let _ = borrowed.ask(40, "run.finish", &json!({ "run_id": "without-switches" }));
    let _ = borrowed.finish();
    raw_chrome.interrupt().unwrap();
    let _ = raw_chrome.child().wait();

    // With Collie's reconciled switches, all three cases pass cleanly.
    assert_eq!(
        with_switches,
        Outcomes {
            ipc_hash: "#i=499".to_owned(),
            popup_result: "opened".to_owned(),
            popup_target_requests: 1,
            repost_reload_status: json!(200),
            repost_post_count: 2,
        },
        "without_switches={without_switches:?}"
    );
    assert_eq!(
        (
            without_switches.ipc_hash.as_str() == "#i=499",
            without_switches.popup_result.as_str(),
            without_switches.popup_target_requests,
            &without_switches.repost_reload_status,
            without_switches.repost_post_count,
        ),
        (false, "blocked", 0, &json!(200), 2),
        "observed raw Chrome 156 without switches: {without_switches:?}"
    );
}
