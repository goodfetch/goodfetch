//! Chromium's launch through Collie's executable (spec #577, ticket #696),
//! over stdio through `goodfetch-protocol`'s types, against the Proving
//! ground's own observations: a headless page shows the site the identity
//! its build shows with a window, with no `HeadlessChrome` anywhere; and a
//! page behind another page of its Session keeps its timers and its focus,
//! the three background flags on every browser's command line. The
//! comparison with a window runs where a virtual display does
//! (`virtual_display`).
#![expect(
    clippy::disallowed_methods,
    reason = "the test polls the Proving ground on its own thread, never an async worker"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use goodfetch_test_support::{CHROME_FOR_TESTING_BUILD, ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, Value, json};

use crate::page::{browser_command_lines, opened, started_with};
use crate::run::call;

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

/// The flags that keep a page behind another unthrottled.
const BACKGROUND_FLAGS: [&str; 3] = [
    "--disable-background-timer-throttling",
    "--disable-renderer-backgrounding",
    "--disable-backgrounding-occluded-windows",
];

/// Whether the build's window gives a high-entropy platform version, which
/// a headless page then shows too. Chromium gives one on macOS and Windows
/// and an empty one on Linux, headless or with a window (measured by
/// `virtual_display`'s comparison on Xvfb), which the identity page
/// reports as `none` and the site receives as `""`.
const WINDOW_GIVES_A_PLATFORM_VERSION: bool = !cfg!(target_os = "linux");

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
    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "identity" }),
    ));
    let went = call(
        &mut engine,
        3,
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
    let _ = call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "identity" }),
    );
    let _ = engine.finish();

    let mut headers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for request in proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
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

/// `browser.headless` is `true` by default, and the headless build shows
/// the site its window's identity: one `Chrome/` user agent on every
/// request and in the page, the build's full version in the high-entropy
/// client hints, the platform version its window gives (none on Linux), and
/// no `HeadlessChrome` anywhere.
/// Covers: story:5-56, setting:browser.headless
#[test]
fn a_headless_page_shows_the_site_its_windows_identity_with_no_headless_token() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let major = CHROME_FOR_TESTING_BUILD.split('.').next().unwrap();

    let seen = identity_seen(&env, &cache, &json!({}), |_| {});

    let user_agent = seen.read.get("user_agent").cloned().unwrap_or_default();
    let anywhere = format!("{seen:?}");
    assert_eq!(
        (
            seen.headless_flag,
            anywhere.contains("HeadlessChrome"),
            user_agent.contains(&format!(" Chrome/{major}.")),
            seen.headers.get("user-agent"),
            seen.read
                .get("full_version_list")
                .is_some_and(|list| list.contains(&format!("Chromium/{CHROME_FOR_TESTING_BUILD}"))),
            seen.headers
                .get("sec-ch-ua-full-version-list")
                .is_some_and(|values| values.iter().all(|value| value
                    .contains(&format!("\"Chromium\";v=\"{CHROME_FOR_TESTING_BUILD}\"")))),
            seen.read
                .get("platform_version")
                .map(|version| version != "none"),
        ),
        (
            true,
            false,
            true,
            Some(&BTreeSet::from([user_agent.clone()])),
            true,
            true,
            Some(WINDOW_GIVES_A_PLATFORM_VERSION),
        ),
        "{seen:#?}"
    );
}

/// The `ticks` reports of the timer page from the `from`th on: how many
/// times its timer fired per second between the first and the last, and
/// how many reports that spans.
fn rate_from(reported: &[(String, String)], from: usize) -> (f64, usize) {
    let ticks: Vec<(f64, f64)> = reported
        .iter()
        .filter(|(check, _)| check == "ticks")
        .skip(from)
        .filter_map(|(_, observed)| {
            let (count, elapsed) = observed.split_once(' ')?;
            Some((count.parse().ok()?, elapsed.parse().ok()?))
        })
        .collect();
    match (ticks.first(), ticks.last()) {
        (Some(first), Some(last)) if last.1 > first.1 => (
            (last.0 - first.0) * 1000.0 / (last.1 - first.1),
            ticks.len(),
        ),
        _ => (0.0, ticks.len()),
    }
}

/// A page behind another page of its Session keeps firing its 100 ms timer
/// and reports that it has focus, and every launched Chromium carries the
/// three background flags (ADR 0085's question). A throttled page gets one
/// wake-up a second; an unthrottled one on a loaded host fires at least
/// twice that.
/// Covers: story:5-11
#[test]
fn a_page_behind_another_of_its_session_keeps_its_timer_and_its_focus() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(&env, &cache, &proving_ground, "timer", &Map::new(), |_| {});
    let behind = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "timer" }),
    ));
    let went = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": behind.page_id, "url": proving_ground.url("access.test", "/timer") }),
    );
    let front = opened(&call(
        &mut engine,
        4,
        "run.new_page",
        &json!({ "run_id": "timer" }),
    ));
    let _ = call(
        &mut engine,
        5,
        "page.goto",
        &json!({ "page_id": front.page_id, "url": proving_ground.url("catalog.test", "/catalog/1") }),
    );
    let ticks =
        |seen: &[(String, String)]| seen.iter().filter(|(check, _)| check == "ticks").count();
    let before = ticks(&reports(&proving_ground, "/timer"));
    let reported = reports_once(&proving_ground, "/timer", |seen| ticks(seen) >= before + 4);
    let browsers = launched(&env);
    let _ = call(&mut engine, 6, "run.finish", &json!({ "run_id": "timer" }));
    let _ = engine.finish();

    let (rate, spanned) = rate_from(&reported, before);
    let focus: BTreeSet<&str> = reported
        .iter()
        .filter(|(check, _)| check == "has_focus")
        .skip(before)
        .map(|(_, observed)| observed.as_str())
        .collect();
    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    assert_eq!(
        (
            spanned >= 4,
            rate >= 2.0,
            focus,
            !browsers.is_empty()
                && browsers
                    .iter()
                    .all(|line| BACKGROUND_FLAGS.iter().all(|flag| line.contains(flag))),
        ),
        (true, true, BTreeSet::from(["true"]), true),
        "{rate} ticks a second over {spanned} reports; {reported:?}; {browsers:?}"
    );
}
