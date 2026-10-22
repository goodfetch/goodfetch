//! Collie's Console as its executable serves it (ADRs 0031, 0043, 0057 and
//! 0067): the executable's composition carries Collie's Console build and
//! an SDK crate's linked engine none; every answer of every route carries
//! the production headers, the build's files served as `xtask console
//! build` stored them, with `Content-Encoding: br`; the Console's port is
//! the Daemon's own, reported in its runtime file and `daemon status`, and
//! a port another program holds fails only the Console's start.
//!
//! The headers test is a table each later route adds its rows to (ADR
//! 0067).
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the Console build's files and the Daemon's runtime file themselves"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a test holds a port the Console then finds taken, from outside the engine (ADR 0060)"
)]

#[path = "../../src/console_build.rs"]
mod console_build;

use std::path::{Path, PathBuf};

use goodfetch_test_support::{ConsoleState, DAEMON_DEADLINE, Daemon, RuntimeFile, TestEnv, cli};
use rstest::rstest;
use serde_json::Value;

use crate::daemon::{StopsTheDaemon, validate};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The production Content Security Policy every answer carries (ADR 0031).
const POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; require-trusted-types-for 'script'; trusted-types 'none'; report-uri /csp-report";

/// The Console build Collie's executable embeds.
fn console_build() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("console-build")
}

/// The path the Console serves the build's first file at whose name ends
/// in `suffix`, such as `.js`.
fn served_path_ending_in(suffix: &str) -> String {
    let assets = console_build().join("assets");
    let mut names: Vec<String> = std::fs::read_dir(&assets)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(&format!("{suffix}.br")))
        .collect();
    names.sort();
    let name = names.first().unwrap();
    format!("/assets/{}", name.strip_suffix(".br").unwrap())
}

fn console_url(env: &TestEnv) -> String {
    let runtime_file = RuntimeFile::read(&RuntimeFile::path(env.home(), VERSION)).unwrap();
    match runtime_file.console {
        ConsoleState::Serving(url) => url,
        other => panic!("the Daemon serves no Console: {other:?}"),
    }
}

/// Covers: story:12-128
#[test]
fn the_executables_composition_carries_collies_console() {
    let composition = console_build::composition();

    assert!(composition.console().is_some());
}

/// Covers: story:12-128
#[test]
fn an_sdk_crates_linked_engine_serves_no_console() {
    let composition = goodfetch_collie::composition();

    assert!(composition.console().is_none());
}

/// What a row asks for: the page, a file of the build by the ending of its
/// name, or a path as it is.
#[derive(Clone, Copy, Debug)]
enum Route {
    Path(&'static str),
    Asset(&'static str),
}

/// Covers: story:12-128
#[rstest]
#[case::the_page(
    "GET",
    Route::Path("/"),
    200,
    Some("text/html; charset=utf-8"),
    "no-cache"
)]
#[case::the_page_by_its_name(
    "GET",
    Route::Path("/index.html"),
    200,
    Some("text/html; charset=utf-8"),
    "no-cache"
)]
#[case::the_pages_head(
    "HEAD",
    Route::Path("/"),
    200,
    Some("text/html; charset=utf-8"),
    "no-cache"
)]
#[case::its_icon(
    "GET",
    Route::Path("/favicon.svg"),
    200,
    Some("image/svg+xml"),
    "no-cache"
)]
#[case::a_script(
    "GET",
    Route::Asset(".js"),
    200,
    Some("text/javascript; charset=utf-8"),
    "public, max-age=31536000, immutable"
)]
#[case::a_style_sheet(
    "GET",
    Route::Asset(".css"),
    200,
    Some("text/css; charset=utf-8"),
    "public, max-age=31536000, immutable"
)]
#[case::a_font(
    "GET",
    Route::Asset(".woff2"),
    200,
    Some("font/woff2"),
    "public, max-age=31536000, immutable"
)]
#[case::a_policy_report("POST", Route::Path("/csp-report"), 204, None, "no-store")]
#[case::a_path_with_no_file("GET", Route::Path("/runs"), 404, None, "no-store")]
#[case::a_write_to_the_page("POST", Route::Path("/"), 405, None, "no-store")]
#[case::a_sign_in_from_no_page("POST", Route::Path("/sign-in"), 403, None, "no-store")]
#[case::a_read_of_the_sign_in("GET", Route::Path("/sign-in"), 405, None, "no-store")]
#[case::a_session_signed_out("GET", Route::Path("/session"), 401, None, "no-store")]
#[case::a_websocket_from_no_page("GET", Route::Path("/protocol"), 403, None, "no-store")]
fn every_console_answer_carries_the_production_headers(
    #[case] method: &str,
    #[case] route: Route,
    #[case] status: u16,
    #[case] content_type: Option<&str>,
    #[case] cache: &str,
) {
    let env = TestEnv::new();
    let daemon = Daemon::start_program(&env, Path::new(COLLIE), DAEMON_DEADLINE).unwrap();
    let path = match route {
        Route::Path(path) => path.to_owned(),
        Route::Asset(suffix) => served_path_ending_in(suffix),
    };
    let agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .new_agent();
    let url = format!("{}{}", console_url(&env).trim_end_matches('/'), path);

    let response = match method {
        "GET" => agent.get(&url).call(),
        "HEAD" => agent.head(&url).call(),
        _ => agent.post(&url).send_empty(),
    }
    .unwrap();

    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .map(|value| value.to_str().unwrap().to_owned())
    };
    assert_eq!(response.status().as_u16(), status);
    assert_eq!(header("content-security-policy").as_deref(), Some(POLICY));
    assert_eq!(header("referrer-policy").as_deref(), Some("no-referrer"));
    assert_eq!(
        header("cross-origin-opener-policy").as_deref(),
        Some("same-origin")
    );
    assert_eq!(
        header("cross-origin-resource-policy").as_deref(),
        Some("same-origin")
    );
    assert_eq!(header("x-content-type-options").as_deref(), Some("nosniff"));
    assert_eq!(header("cache-control").as_deref(), Some(cache));
    assert_eq!(header("content-type").as_deref(), content_type);
    let encoding = content_type.map(|_| "br");
    assert_eq!(header("content-encoding").as_deref(), encoding);
    daemon.stop(&env).unwrap();
}

/// Covers: story:12-128
#[test]
fn the_console_serves_its_builds_files_as_they_are_stored() {
    let env = TestEnv::new();
    let daemon = Daemon::start_program(&env, Path::new(COLLIE), DAEMON_DEADLINE).unwrap();
    let path = served_path_ending_in(".js");
    let url = format!("{}{}", console_url(&env).trim_end_matches('/'), path);

    let body = ureq::get(&url)
        .call()
        .unwrap()
        .into_body()
        .read_to_vec()
        .unwrap();

    let stored = std::fs::read(console_build().join(format!("{}.br", &path[1..]))).unwrap();
    assert_eq!(body, stored);
    daemon.stop(&env).unwrap();
}

/// Covers: story:12-3, setting:console.address, command:daemon.start, command:daemon.status
#[test]
fn a_daemon_on_port_zero_reports_its_consoles_address() {
    let env = TestEnv::new();
    env.user_configuration("console.address = \"127.0.0.1:0\"\n");
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let started = cli(&env, COLLIE)
        .args(["daemon", "start", "--json"])
        .run()
        .unwrap();

    assert_eq!(started.exit, Some(0), "{}", started.stderr);
    let started: Value = serde_json::from_str(&started.stdout).unwrap();
    validate(&started, "DaemonStartOutput");
    let url = console_url(&env);
    let port: u16 = url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches('/')
        .parse()
        .unwrap();
    assert_ne!(port, 0);
    assert_eq!(started["daemon"]["console"]["serving"], url.as_str());
    let status = cli(&env, COLLIE)
        .args(["daemon", "status", "--json"])
        .run()
        .unwrap();
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["daemon"]["console"]["serving"], url.as_str());
}

/// Covers: story:12-3, setting:console.address, command:daemon.start, command:daemon.status
#[test]
fn a_console_port_another_program_holds_fails_only_the_consoles_start() {
    let env = TestEnv::new();
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = held.local_addr().unwrap();
    env.user_configuration(&format!("console.address = \"{address}\"\n"));
    let _stops = StopsTheDaemon::of(&env, COLLIE);

    let started = cli(&env, COLLIE)
        .args(["daemon", "start", "--json"])
        .run()
        .unwrap();

    // `console_port_in_use` has no row of its own: its group's, `8`.
    assert_eq!(started.exit, Some(8), "{}", started.stderr);
    let started: Value = serde_json::from_str(&started.stdout).unwrap();
    assert_eq!(
        started["daemon"]["console"]["failed"]["code"],
        "console_port_in_use"
    );
    assert!(
        started["daemon"]["console"]["failed"]["fix"]
            .as_str()
            .unwrap()
            .contains("`console.address`")
    );
    let status = cli(&env, COLLIE)
        .args(["daemon", "status", "--json"])
        .run()
        .unwrap();
    assert_eq!(status.exit, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(
        status["daemon"]["console"]["failed"]["code"],
        "console_port_in_use"
    );
    drop(held);
}
