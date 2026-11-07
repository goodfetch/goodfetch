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

use goodfetch_test_client::DaemonClient;
use goodfetch_test_support::{ConsoleState, DAEMON_DEADLINE, Daemon, RuntimeFile, TestEnv, cli};
use rstest::rstest;
use serde_json::{Value, json};

use crate::content_origin::Signed;
use crate::daemon::{StopsTheDaemon, validate};
use crate::daemon_hello;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// This commit's Product version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The production Content Security Policy every answer carries (ADR 0031):
/// with a content origin `content`, it lets the Console frame that origin,
/// and nothing else's, and loosens no other directive: the content origin's
/// files, images included, show in its `sandbox` frames. With none, it has
/// no `frame-src`.
pub(crate) fn policy(content: Option<&str>) -> String {
    let frames = content.map_or_else(String::new, |content| format!("frame-src {content}; "));
    format!(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; connect-src 'self'; manifest-src 'self'; {frames}base-uri 'none'; form-action 'self'; frame-ancestors 'none'; require-trusted-types-for 'script'; trusted-types 'none'; report-uri /csp-report"
    )
}

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
#[case::a_route_of_the_build(
    "GET",
    Route::Path("/settings"),
    200,
    Some("text/html; charset=utf-8"),
    "no-cache"
)]
#[case::a_route_with_a_parameter(
    "GET",
    Route::Path("/runs/nightly-run"),
    200,
    Some("text/html; charset=utf-8"),
    "no-cache"
)]
#[case::a_path_longer_than_a_route(
    "GET",
    Route::Path("/runs/nightly-run/more"),
    404,
    None,
    "no-store"
)]
#[case::a_route_with_an_empty_parameter("GET", Route::Path("/runs/"), 404, None, "no-store")]
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
#[case::a_path_with_no_file("GET", Route::Path("/no-such-screen"), 404, None, "no-store")]
#[case::a_write_to_the_page("POST", Route::Path("/"), 405, None, "no-store")]
#[case::a_sign_in_from_no_page("POST", Route::Path("/sign-in"), 403, None, "no-store")]
#[case::a_read_of_the_sign_in("GET", Route::Path("/sign-in"), 405, None, "no-store")]
#[case::a_session_signed_out("GET", Route::Path("/session"), 401, None, "no-store")]
#[case::an_offline_key_from_no_page("POST", Route::Path("/offline-key"), 403, None, "no-store")]
#[case::a_read_of_the_offline_key("GET", Route::Path("/offline-key"), 405, None, "no-store")]
#[case::a_websocket_from_no_page("GET", Route::Path("/protocol"), 403, None, "no-store")]
#[case::an_event_stream_signed_out("GET", Route::Path("/protocol/events"), 401, None, "no-store")]
#[case::a_write_to_the_event_stream("POST", Route::Path("/protocol/events"), 405, None, "no-store")]
#[case::a_post_to_an_event_stream_from_no_page(
    "POST",
    Route::Path("/protocol/messages/0000000000000000"),
    403,
    None,
    "no-store"
)]
#[case::a_read_of_an_event_streams_messages(
    "GET",
    Route::Path("/protocol/messages/0000000000000000"),
    405,
    None,
    "no-store"
)]
#[case::a_file_by_handle_signed_out(
    "GET",
    Route::Path("/bodies/first/0000000000000000000000000000000000000000000000000000000000000000"),
    401,
    None,
    "no-store"
)]
#[case::a_content_address_signed_out(
    "GET",
    Route::Path(
        "/content-address/first/0000000000000000000000000000000000000000000000000000000000000000"
    ),
    401,
    None,
    "no-store"
)]
#[case::a_write_to_a_file_by_handle(
    "POST",
    Route::Path("/bodies/first/0000000000000000000000000000000000000000000000000000000000000000"),
    405,
    None,
    "no-store"
)]
fn every_console_answer_carries_the_production_headers(
    #[case] method: &str,
    #[case] route: Route,
    #[case] status: u16,
    #[case] content_type: Option<&str>,
    #[case] cache: &str,
) {
    let env = TestEnv::new();
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let content = Signed::started(&env).content();
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
    assert_eq!(
        header("content-security-policy"),
        Some(policy(Some(&content))),
        "the Console's policy names its content origin"
    );
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

/// A Run on a Daemon that serves the Console names its page there in its
/// report (ADRs 0031 and 0102).
/// Covers: story:3-154, operation:run.start, operation:run.finish
#[test]
fn a_runs_report_names_its_page_in_the_console_its_daemon_serves() {
    let env = TestEnv::new();
    env.user_configuration("console.address = \"127.0.0.1:0\"\n");
    let _stops = StopsTheDaemon::of(&env, COLLIE);
    let started = cli(&env, COLLIE)
        .args(["daemon", "start", "--json"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{}", started.stderr);

    let found = RuntimeFile::read(&RuntimeFile::path(env.home(), VERSION)).unwrap();
    let mut client = DaemonClient::opened(&found.endpoint, &daemon_hello(&found.token));

    let started = client.call(1, "run.start", &serde_json::json!({ "run_id": "paged" }));
    let finished = client.call(2, "run.finish", &serde_json::json!({ "run_id": "paged" }));

    assert_eq!(started["result"]["run_id"], "paged", "{started}");
    assert_eq!(
        finished["result"]["console_url"],
        Value::from(format!("{}runs/paged", console_url(&env)))
    );
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
    validate(&started, "DaemonStatus");
    let url = console_url(&env);
    let port: u16 = url
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches('/')
        .parse()
        .unwrap();
    assert_ne!(port, 0);
    assert_eq!(started["console_url"], url.as_str());
    let status = cli(&env, COLLIE)
        .args(["daemon", "status", "--json"])
        .run()
        .unwrap();
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["console_url"], url.as_str());
}

/// Covers: story:12-3, setting:console.address, command:daemon.start, command:daemon.status, command:daemon.restart
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

    // `console_port_in_use` has no row of its own: its group's, `8`. Under
    // `--json` the status printed carries the failure as `console_error`
    // (608-O4), and a person reads it as every failure prints.
    assert_eq!(started.exit, Some(8), "{}", started.stderr);
    let printed: Value = serde_json::from_str(&started.stdout).unwrap();
    let error = &printed["console_error"];
    assert_eq!(
        (
            printed["pid"].is_u64(),
            printed["console_url"].clone(),
            error["code"].clone(),
            error["fix"]
                .as_str()
                .is_some_and(|fix| fix.contains("`console.address`")),
        ),
        (true, Value::Null, json!("console_port_in_use"), true),
        "{started}"
    );
    let status = cli(&env, COLLIE)
        .args(["daemon", "status", "--json"])
        .run()
        .unwrap();
    assert_eq!(status.exit, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(
        (
            status["console_url"].clone(),
            status["console_error"]["code"].clone()
        ),
        (Value::Null, json!("console_port_in_use"))
    );
    let restarted = cli(&env, COLLIE).args(["daemon", "restart"]).run().unwrap();
    assert_eq!(
        (
            restarted.exit,
            restarted
                .stderr
                .lines()
                .any(|line| line == "code: console_port_in_use"),
        ),
        (Some(8), true),
        "{restarted}"
    );
    drop(held);
}
