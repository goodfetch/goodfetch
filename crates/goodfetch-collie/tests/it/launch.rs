//! Chromium's launch through Collie's executable (spec #577, ticket #696),
//! over stdio through `goodfetch-protocol`'s types, against the Proving
//! ground's own observations: what identity a Run's headless or headful
//! browser shows the site, which the comparison with a window reads where
//! a virtual display runs (`virtual_display`). The headless identity and a
//! background page's timers are the page suite's `launch` cases.
#![expect(
    clippy::disallowed_methods,
    reason = "the test polls the Proving ground on its own thread, never an async worker"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_test_support::{ProvingGround, TestEnv};
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
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|host| host.starts_with("access.localhost"))
        })
    {
        for header in request["headers"].as_array().unwrap() {
            let name = header[0].as_str().unwrap().to_ascii_lowercase();
            if name == "user-agent" || name.starts_with("sec-ch-ua") {
                headers
                    .entry(name)
                    .or_default()
                    .insert(header[1].as_str().unwrap().to_owned());
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
