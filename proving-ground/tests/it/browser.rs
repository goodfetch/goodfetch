//! A stock Chromium driven by Playwright (spec #152, Seam 1, for pages
//! whose state only a browser reaches): `browser.mjs` runs one of its
//! programs on a page and prints the result it would submit, which the
//! test then submits to the oracle. No engine crate is involved. The
//! program runs under the test's [`TestEnv`], so the browser writes any
//! crash dump where the test's teardown finds it.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;

use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::proving_ground::ProvingGround;

/// The programs' file.
const PROGRAMS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/it/browser.mjs");

/// Where Playwright keeps its browsers: `PLAYWRIGHT_BROWSERS_PATH` when the
/// test's process has it (as the CI image does), else Playwright's own
/// default under the user's home, or on Windows under its local
/// application data, which sets no `HOME`; the test's cleared environment
/// replaces both.
fn browsers() -> PathBuf {
    if let Some(path) = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH") {
        return PathBuf::from(path);
    }
    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA is set");
        return PathBuf::from(local).join("ms-playwright");
    }
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
    if cfg!(target_os = "macos") {
        home.join("Library/Caches/ms-playwright")
    } else {
        home.join(".cache/ms-playwright")
    }
}

/// Task `id`'s start URL, from the task list.
pub(crate) fn start_url(proving_ground: &ProvingGround, id: &str) -> String {
    proving_ground.task(id)["start_url"]
        .as_str()
        .expect("a start URL")
        .to_owned()
}

/// Runs task `id`'s `program` on its start page, as [`run`] does, and the
/// oracle's answer to the result it would submit.
pub(crate) fn judged(
    proving_ground: &ProvingGround,
    id: &str,
    program: &str,
    options: Value,
) -> Value {
    let result = run(id, program, &start_url(proving_ground, id), options);
    proving_ground.judge(&result)
}

/// Runs `program` on `url` with its `options` (`context` holds the
/// browser context's options, such as its viewport), and returns the
/// result it would submit, with the task added.
pub(crate) fn run(task: &str, program: &str, url: &str, options: Value) -> Value {
    let env = TestEnv::new();
    let mut request = json!({ "program": program, "url": url });
    if let (Value::Object(request), Value::Object(options)) = (&mut request, options) {
        request.extend(options);
    }
    let mut child = env
        .process_command("node")
        .arg(PROGRAMS)
        .env("PATH", std::env::var_os("PATH").expect("PATH is set"))
        .env("PLAYWRIGHT_BROWSERS_PATH", browsers())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("node starts the browser programs");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(request.to_string().as_bytes())
        .expect("the program reads its request");
    let output = child.wait_with_output().expect("the program ends");
    assert!(
        output.status.success(),
        "the browser program {program} failed: {}",
        output.status
    );
    let mut result: Value =
        serde_json::from_slice(&output.stdout).expect("the program prints its result as JSON");
    result["task"] = json!(task);
    result
}
