//! Chromium's launch through Collie's executable (spec #577, ticket #696),
//! over stdio through `goodfetch-protocol`'s types, against the Proving
//! ground's own observations: what identity a Run's headless or headful
//! browser shows the site, which the comparison with a window reads where
//! a virtual display runs (`virtual_display`), and the headless shell's
//! launch, with macOS's refusal of the full build with no GUI session
//! (#3224). The headless identity and a background page's timers are the
//! page suite's `launch` cases.
#![expect(
    clippy::disallowed_methods,
    reason = "the test polls the Proving ground on its own thread, never an async worker"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_test_support::{
    ProvingGround, TestEnv, browser_cache_with_headless_shell, chrome_headless_shell,
};
use serde_json::{Map, Value, json};

use crate::page::{browser_command_lines, opened, started_with};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 696;

/// What the identity page reports, beside that it ran.
const IDENTITY_CHECKS: [&str; 11] = [
    "user_agent",
    "brands",
    "full_version_list",
    "platform",
    "platform_version",
    "architecture",
    "bitness",
    "mobile",
    "model",
    "wow64",
    "form_factors",
];

/// How long a page's script may take to report.
const REPORTS_DEADLINE: Duration = Duration::from_secs(20);

/// The command lines of the browsers the test's engine launched, not of
/// their helper processes, which Chromium starts with a `--type`.
fn launched(env: &TestEnv) -> Vec<String> {
    browser_command_lines(env)
        .into_iter()
        .filter(|line| !line.contains("--type="))
        .collect()
}

/// The reports the Proving ground received from scripts of the page at
/// `path`, in order, as `(check, observed)`.
fn reports(proving_ground: &ProvingGround, path: &str) -> Vec<(String, String)> {
    proving_ground.observations().unwrap()["reports"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|report| report["page"] == path)
        .map(|report| {
            (
                report["check"].as_str().unwrap().to_owned(),
                report["observed"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// The reports of the page at `path` once `done` holds of them, or at the
/// deadline.
fn reports_once(
    proving_ground: &ProvingGround,
    path: &str,
    done: impl Fn(&[(String, String)]) -> bool,
) -> Vec<(String, String)> {
    let started = Instant::now();
    loop {
        let seen = reports(proving_ground, path);
        if done(&seen) || started.elapsed() > REPORTS_DEADLINE {
            return seen;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A Run under `browser.headless_shell: true` launches the pinned
/// release's headless shell, not the full build, and its page loads,
/// showing the site the shell's own identity: the build that starts where
/// the full build's headless mode cannot, as on macOS with no GUI session
/// (#3224).
///
/// Covers: setting:browser.headless_shell
#[test]
fn a_run_under_the_headless_shell_setting_launches_the_headless_shell() {
    let env = TestEnv::new();
    let cache = browser_cache_with_headless_shell(&env).unwrap();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut settings = Map::new();
    settings.insert("browser".to_owned(), json!({ "headless_shell": true }));
    let mut engine = started_with(&env, &cache, &proving_ground, "shell", &settings, |_| {});

    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "shell" })));
    let went = engine.ask(3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("access.localhost", "/identity") }),
    );
    let browsers = launched(&env);
    let _ = engine.ask(4, "run.finish", &json!({ "run_id": "shell" }));
    let _ = engine.finish();

    let shell = chrome_headless_shell(&cache).display().to_string();
    let user_agents: BTreeSet<String> = proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request.host.starts_with("access.localhost"))
        .flat_map(|request| request.headers.clone())
        .filter(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
        .map(|(_, value)| value)
        .collect();
    assert_eq!(
        (
            went["result"]["response"]["status"].clone(),
            browsers.len(),
            browsers.iter().all(|line| line.starts_with(&shell)),
            !user_agents.is_empty()
                && user_agents
                    .iter()
                    .all(|agent| agent.contains("HeadlessChrome/")),
        ),
        (json!(200), 1, true, true),
        "{went}\n{browsers:#?}\n{user_agents:?}"
    );
}

/// On macOS with no GUI session, as in the Release suite's macOS cells,
/// whose runner starts in the Background session, a Run that turns the
/// headless shell off fails its first page at once with
/// `browser_needs_gui_session`, whose fix names the shell, and launches no
/// browser (#3224). Elsewhere the full build starts, so the test exists on
/// macOS alone and skips there in a GUI session.
///
/// Covers: error:browser_needs_gui_session
#[cfg(target_os = "macos")]
#[test]
fn with_no_gui_session_on_macos_the_full_build_headless_needs_a_gui_session() {
    if !goodfetch_test_support::Variables::from_process().no_gui_session() {
        #[expect(
            clippy::print_stderr,
            reason = "a skipped test says why in its own output, which the test harness keeps"
        )]
        {
            eprintln!("skipped: only macOS with no GUI session refuses the full build headless");
        }
        return;
    }
    let env = TestEnv::new();
    let cache = goodfetch_test_support::browser_cache(&env).unwrap();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut settings = Map::new();
    settings.insert("browser".to_owned(), json!({ "headless_shell": false }));
    let mut engine = started_with(&env, &cache, &proving_ground, "full", &settings, |_| {});

    let refused = engine.ask(2, "run.new_page", &json!({ "run_id": "full" }));
    let browsers = launched(&env);
    let _ = engine.ask(3, "run.finish", &json!({ "run_id": "full" }));
    let _ = engine.finish();

    assert_eq!(
        (
            refused["error"]["data"]["code"].clone(),
            refused["error"]["data"]["fix"].clone(),
            browsers.len(),
        ),
        (
            json!("browser_needs_gui_session"),
            json!("set `browser.headless_shell: true`"),
            0
        ),
        "{refused}"
    );
}

/// What a site sees of a browser: the user agents and client hints its
/// requests carried, and what its page's script read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    /// The `User-Agent` and `Sec-CH-UA*` headers of every request to the
    /// identity page's host, by name, each with the values it took.
    pub(crate) headers: BTreeMap<String, BTreeSet<String>>,
    /// What the identity page's script reported, by check.
    pub(crate) read: BTreeMap<String, String>,
    /// Whether the browser's command line holds `--headless`.
    pub(crate) headless_flag: bool,
}

/// The identity a Run of Collie's executable shows the Proving ground's
/// identity page, its `browser` settings `browser`, its engine's command
/// changed by `change`.
pub(crate) fn identity_seen(
    env: &TestEnv,
    cache: &Path,
    browser: &Value,
    change: impl FnOnce(&mut std::process::Command),
) -> Identity {
    let proving_ground = ProvingGround::start(env, SEED).unwrap();
    let mut settings = Map::new();
    settings.insert("browser".to_owned(), browser.clone());
    let mut engine = started_with(env, cache, &proving_ground, "identity", &settings, change);
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "identity" })));
    let went = engine.ask(3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("access.localhost", "/identity") }),
    );
    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    let reported = reports_once(&proving_ground, "/identity", |seen| {
        IDENTITY_CHECKS
            .iter()
            .all(|check| seen.iter().any(|(reported, _)| reported == check))
    });
    let browsers = launched(env);
    let _ = engine.ask(4, "run.finish", &json!({ "run_id": "identity" }));
    let _ = engine.finish();

    let mut headers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for request in proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request.host.starts_with("access.localhost"))
    {
        for (name, value) in &request.headers {
            let name = name.to_ascii_lowercase();
            if name == "user-agent" || name.starts_with("sec-ch-ua") {
                headers.entry(name).or_default().insert(value.clone());
            }
        }
    }
    Identity {
        headers,
        read: reported
            .into_iter()
            .filter(|(check, _)| IDENTITY_CHECKS.contains(&check.as_str()))
            .collect(),
        headless_flag: !browsers.is_empty()
            && browsers.iter().all(|line| line.contains("--headless")),
    }
}
