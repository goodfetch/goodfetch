//! Headless and headful Chromium through Collie's executable on a virtual
//! display (spec #577, ticket #696; ADRs 0021 and 0092): the site sees one
//! browser either way, so a login a person made in a window still works
//! when the Run is unattended (golden scenario 04, failure point 6). Linux
//! only, on an Xvfb display the test starts, as CI's Linux job runs it; on
//! macOS a headful browser would open its window on the host's own screen.

use std::io::{BufRead as _, BufReader};

use goodfetch_test_support::{Process, TestEnv, browser_cache};
use serde_json::json;

use crate::launch::identity_seen;

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
