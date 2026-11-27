//! Headless and headful Chromium through Collie's executable on a virtual
//! display (spec #577, ticket #696; ADRs 0021 and 0092): the site sees one
//! browser either way, so a login a person made in a window still works
//! when the Run is unattended (golden scenario 04, failure point 6). Linux
//! only, on an Xvfb display the test starts, as CI's Linux job runs it; on
//! macOS a headful browser would open its window on the host's own screen.
//! With no display in its environment, the engine starts a virtual display
//! of its own for each headful browser, as a `stealth` Session's is by
//! default (spec #599, ticket #1012; ADR 0035).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads a process's environment and lists processes on its own thread, never an async worker"
)]

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::Command;

use goodfetch_test_support::{Process, ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, Value, json};

use crate::launch::identity_seen;
use crate::page::{browser_processes, started_with};
use crate::sessions::{page_in, session_of};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 1012;

/// An Xvfb server on a display number it picks itself, stopped with the
/// value.
struct VirtualDisplay {
    process: Process,
    /// Its `DISPLAY`, such as `:99`.
    name: String,
}

impl VirtualDisplay {
    fn start(env: &TestEnv) -> Self {
        let mut command = env.process_command("Xvfb");
        command.args([
            "-displayfd",
            "1",
            "-screen",
            "0",
            "1280x800x24",
            "-nolisten",
            "tcp",
        ]);
        let process = env.spawn(&mut command).unwrap();
        let mut line = String::new();
        BufReader::new(process.child().stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        Self {
            process,
            name: format!(":{}", line.trim()),
        }
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        let _ = self.process.stop();
    }
}

/// With `browser.headless: false` on a virtual display, Chromium shows the
/// site exactly what it shows headless: every user agent and client hint
/// its requests carry and every value its page's script reads.
/// Covers: story:5-56, setting:browser.headless
#[test]
fn headless_and_headful_chromium_show_the_site_one_identity() {
    let headless_env = TestEnv::new();
    let headless_cache = browser_cache(&headless_env).unwrap();
    let headful_env = TestEnv::new();
    let headful_cache = browser_cache(&headful_env).unwrap();
    let display = VirtualDisplay::start(&headful_env);

    let headless = identity_seen(&headless_env, &headless_cache, &json!({}), |_| {});
    let headful = identity_seen(
        &headful_env,
        &headful_cache,
        &json!({ "headless": false }),
        |command| {
            command.env("DISPLAY", &display.name);
        },
    );

    assert_eq!(
        (
            headless.headless_flag,
            headful.headless_flag,
            &headful.headers,
            &headful.read
        ),
        (true, false, &headless.headers, &headless.read)
    );
}

/// The settings of a `stealth` Run whose Sessions each get a browser
/// process of their own (`governor.browser.contexts_per_process: 1`),
/// without human-like input, which this engine does not build yet, its
/// other `browser` settings `browser`.
fn stealth(browser: &Value) -> Map<String, Value> {
    let mut chosen = browser.clone();
    chosen["access_mode"] = json!("stealth");
    let mut settings = Map::new();
    settings.insert("browser".to_owned(), chosen);
    settings.insert("stealth".to_owned(), json!({ "human_input": false }));
    settings.insert(
        "governor".to_owned(),
        json!({ "browser": { "contexts_per_process": 1 } }),
    );
    settings
}

/// The `DISPLAY` in the environment of the process `pid`.
fn display_of(pid: u32) -> String {
    let environment = std::fs::read(format!("/proc/{pid}/environ")).unwrap();
    environment
        .split(|byte| *byte == 0)
        .find_map(|variable| variable.strip_prefix(b"DISPLAY="))
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .unwrap_or_default()
}

/// The process ids of the Xvfb servers the process `parent` started.
fn servers_of(parent: u32) -> Vec<u32> {
    let listed = Command::new("ps")
        .args(["-ax", "-o", "pid=,ppid=,comm="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let (id, started_by, name) = (fields.next()?, fields.next()?, fields.next()?);
            (name == "Xvfb" && started_by.parse() == Ok(parent)).then(|| id.parse().unwrap())
        })
        .collect()
}

/// On Linux with no display in the engine's environment, each `stealth`
/// Session's browser is headful, on a virtual display of its own: its page
/// sees a headed window, taller than its viewport, two browser processes
/// have two displays, and each display's server is gone once the Run
/// ends.
///
/// Covers: story:8-40, setting:browser.headless, setting:browser.access_mode, operation:run.new_session
#[test]
fn each_stealth_browser_is_headful_on_a_virtual_display_of_its_own() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "headful",
        &stealth(&json!({})),
        |_| {},
    );
    let mut headed = |id: i64| {
        let session =
            session_of(&engine.ask(id, "run.new_session", &json!({ "run_id": "headful" })));
        let page = page_in(&mut engine, id + 1, &session);
        let went = engine.ask(
            id + 2,
            "page.goto",
            &json!({ "page_id": page.page_id, "url": proving_ground.url("access.localhost", "/identity") }),
        );
        assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
        engine.ask(
            id + 3,
            "page.evaluate",
            &json!({ "page_id": page.page_id, "function": "() => window.outerHeight > window.innerHeight" }),
        )["result"]["value"]
            .clone()
    };
    let windows = [headed(2), headed(6)];
    let browsers: Vec<(u32, String)> = browser_processes(&env)
        .into_iter()
        .filter(|(_, line)| !line.contains("--type="))
        .collect();
    let displays: BTreeSet<String> = browsers.iter().map(|(pid, _)| display_of(*pid)).collect();
    let servers = servers_of(engine.pid());
    let _ = engine.ask(10, "run.finish", &json!({ "run_id": "headful" }));
    let _ = engine.finish();
    let remaining: Vec<u32> = servers
        .iter()
        .copied()
        .filter(|pid| Path::new(&format!("/proc/{pid}")).exists())
        .collect();

    assert_eq!(
        (
            windows,
            browsers.len(),
            browsers.iter().any(|(_, line)| line.contains("--headless")),
            displays.len(),
            servers.len(),
            remaining,
        ),
        ([json!(true), json!(true)], 2, false, 2, 2, Vec::new())
    );
}

/// The answer to `run.new_session` in a `stealth` Run whose engine finds
/// no Xvfb on its `PATH`, its other `browser` settings `browser`.
fn opened_without_xvfb(browser: &Value) -> Value {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let empty = env.folder().join("no-xvfb");
    std::fs::create_dir(&empty).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "bare",
        &stealth(browser),
        |command| {
            command.env("PATH", &empty);
        },
    );
    let answer = engine.ask(2, "run.new_session", &json!({ "run_id": "bare" }));
    let _ = engine.ask(3, "run.finish", &json!({ "run_id": "bare" }));
    let _ = engine.finish();
    answer
}

/// On a Linux host without Xvfb, a `stealth` Session fails when it opens
/// with `capability_unavailable` naming `browser.headless` and a headless
/// launch as its fix; with `browser.headless: true` it opens, headless.
///
/// Covers: story:8-41, error:capability_unavailable, setting:browser.headless, operation:run.new_session
#[test]
fn without_xvfb_a_headful_session_names_browser_headless_and_a_headless_one_opens() {
    let headful = opened_without_xvfb(&json!({}));
    let headless = opened_without_xvfb(&json!({ "headless": true }));

    let data = &headful["error"]["data"];
    assert_eq!(
        (
            [
                &data["code"],
                &data["key"],
                &data["layer"],
                &data["kind"],
                &data["fix"]
            ],
            headless["result"]["session_id"].is_string(),
        ),
        (
            [
                &json!("capability_unavailable"),
                &json!("browser.headless"),
                &json!("default"),
                &json!("virtual_display"),
                &json!("`browser.headless: true`"),
            ],
            true,
        ),
        "{headful} {headless}"
    );
}
