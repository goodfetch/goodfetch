//! A Collie program's pages over stdio (spec #577, ticket #695), through
//! `goodfetch-protocol`'s types: `run.new_page` opens a page in the Run's
//! default Session, launching Chromium over a pipe from the Browser cache,
//! and `page.goto` answers a Proving ground page's `Response`. Each test has
//! its own Goodfetch home, a cleared environment, its own Proving ground and
//! a Browser cache in its own folder (ADRs 0059 and 0094).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![cfg_attr(
    unix,
    expect(
        clippy::disallowed_methods,
        reason = "the test reads the Run trace, lists processes and polls the Proving ground on its own thread, never an async worker"
    )
)]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use goodfetch_engine::browser_cache::{BrowserCache, CacheLocation, CleanUp};
use goodfetch_protocol::home::RUN_TRACE_CDP_COMMANDS;
use goodfetch_protocol::page::{Navigated, PageOpened};
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, CHROME_FOR_TESTING_BUILD, Mode, ProvingGround, TestEnv, browser_cache,
    cli, runtime,
};
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::attached::{Attached, hello};
use crate::run::call;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Proving ground's own seed for these tests.
const SEED: u64 = 695;

/// How long a call may take: the first page launches the browser, checking
/// its build's digest, and the Run's end waits for the browser to exit.
pub(crate) const BROWSER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

/// Starts an Attached engine whose Browser cache is `cache`, and a Run whose
/// `network.resolve` maps the Proving ground's hosts to its address.
pub(crate) fn started<'env>(
    env: &'env TestEnv,
    cache: &Path,
    proving_ground: &ProvingGround,
    run_id: &str,
) -> Attached<'env> {
    started_with(env, cache, proving_ground, run_id, &Map::new(), |_| {})
}

/// Starts an Attached engine whose Browser cache is `cache`, its command
/// changed by `change`, and a Run whose `network.resolve` maps the Proving
/// ground's hosts to its address, beside `settings`, more of the Run's
/// settings by area.
pub(crate) fn started_with<'env>(
    env: &'env TestEnv,
    cache: &Path,
    proving_ground: &ProvingGround,
    run_id: &str,
    settings: &Map<String, Value>,
    change: impl FnOnce(&mut Command),
) -> Attached<'env> {
    started_after(env, cache, proving_ground, run_id, settings, change, &[])
}

/// As [`started_with`], once the engine answered `before`, calls such as
/// `usage.set` the Run's start depends on, each with an ID from 100.
pub(crate) fn started_after<'env>(
    env: &'env TestEnv,
    cache: &Path,
    proving_ground: &ProvingGround,
    run_id: &str,
    settings: &Map<String, Value>,
    change: impl FnOnce(&mut Command),
    before: &[(&str, Value)],
) -> Attached<'env> {
    begun(
        env,
        cache,
        proving_ground,
        ("run.start", run_id),
        settings,
        change,
        before,
    )
}

/// As [`started_with`], resuming the Run `run_id` with `run.resume` in a
/// new engine, such as after its engine was killed.
pub(crate) fn resumed_with<'env>(
    env: &'env TestEnv,
    cache: &Path,
    proving_ground: &ProvingGround,
    run_id: &str,
    settings: &Map<String, Value>,
    change: impl FnOnce(&mut Command),
) -> Attached<'env> {
    begun(
        env,
        cache,
        proving_ground,
        ("run.resume", run_id),
        settings,
        change,
        &[],
    )
}

/// An engine started as [`started_after`] says, once `method` began the
/// Run `run_id`.
fn begun<'env>(
    env: &'env TestEnv,
    cache: &Path,
    proving_ground: &ProvingGround,
    (method, run_id): (&str, &str),
    settings: &Map<String, Value>,
    change: impl FnOnce(&mut Command),
    before: &[(&str, Value)],
) -> Attached<'env> {
    let mut engine = Attached::start_waiting(env, BROWSER_DEADLINE, |command| {
        command.env(BROWSERS_PATH_VARIABLE, cache);
        change(command);
    });
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    for ((method, params), id) in before.iter().zip(100..) {
        let answered = call(&mut engine, id, method, params);
        assert!(answered.get("result").is_some(), "{answered}");
    }
    let resolve: Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    let mut settings = settings.clone();
    allow_private(&mut settings, resolve.keys());
    settings
        .entry("network")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("a test's `network` settings are a table")
        .insert("resolve".to_owned(), Value::Object(resolve));
    let started = call(
        &mut engine,
        1,
        method,
        &json!({ "run_id": run_id, "settings": settings }),
    );
    assert!(started.get("result").is_some(), "{started}");
    engine
}

/// Lists `hosts`, names a Run's `network.resolve` maps to the Proving
/// ground's loopback address, in its `egress.private_ranges`, unless the
/// test's own `settings` list ranges: the relay judges a mapping's answers
/// as any other (spec #563's reading of #325), so a test reaches loopback
/// by listing it and keeps the resolved-address check on. Listing the
/// names rather than `127.0.0.0/8` keeps `localhost` and literal loopback
/// addresses refused.
pub(crate) fn allow_private<'host>(
    settings: &mut Map<String, Value>,
    hosts: impl IntoIterator<Item = &'host String>,
) {
    let egress = settings
        .entry("egress")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .expect("a test's `egress` settings are a table");
    egress
        .entry("private_ranges")
        .or_insert_with(|| json!(hosts.into_iter().collect::<Vec<_>>()));
}

pub(crate) fn opened(answer: &Value) -> PageOpened {
    serde_json::from_value(answer["result"].clone())
        .unwrap_or_else(|error| panic!("{error}: {answer}"))
}

/// Covers: story:5-61, operation:run.new_page, operation:page.goto
#[test]
fn a_program_opens_pages_in_the_runs_default_session_and_gets_a_documents_response() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "pages");
    let url = proving_ground.url("catalog.test", "/catalog/1");

    let first = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "pages" }),
    ));
    let went = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": first.page_id, "url": url }),
    );
    let second = opened(&call(
        &mut engine,
        4,
        "run.new_page",
        &json!({ "run_id": "pages" }),
    ));
    let finished = call(&mut engine, 5, "run.finish", &json!({ "run_id": "pages" }));
    let (rest, exited) = engine.finish();

    let response = serde_json::from_value::<Navigated>(went["result"].clone())
        .unwrap_or_else(|error| panic!("{error}: {went}"))
        .response
        .unwrap();
    assert_eq!(
        (
            response.status,
            response.url,
            response
                .headers
                .get("content-type")
                .is_some_and(|value| value.starts_with("text/html")),
            second.session_id == first.session_id,
            second.page_id == first.page_id,
            &finished["result"]["lifecycle"],
            (rest, exited),
        ),
        (
            200,
            url,
            true,
            true,
            false,
            &json!("finished"),
            (vec![], true)
        )
    );
}

/// An engine whose temporary folder is too deep for the OS's socket paths,
/// as a sandbox's or a build tool's `TMPDIR` can be, still starts its
/// browser and goes to a page: Chromium puts its own socket in its
/// temporary folder and stops when that path is too long.
///
/// Covers: operation:run.new_page, operation:page.goto
#[cfg(unix)]
#[test]
fn an_engine_under_a_deep_temporary_folder_starts_its_browser_and_goes_to_a_page() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let temporary = env
        .folder()
        .join("a folder as deep as a sandbox's or a build tool's temporary folder can be");
    std::fs::create_dir_all(&temporary).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "deep",
        &Map::new(),
        |command| {
            command.env("TMPDIR", &temporary);
        },
    );
    let url = proving_ground.url("catalog.test", "/catalog/1");

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "deep" }),
    ));
    let went = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    );
    let _ = call(&mut engine, 4, "run.finish", &json!({ "run_id": "deep" }));
    let _ = engine.finish();

    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
}

/// `page.formats` reads the main content of the page a program is on as
/// Markdown, answers only the formats asked, and answers a page with no
/// main content with empty Markdown and `none`, a value.
/// Covers: story:6-1, story:6-8, story:6-12, operation:page.formats
#[test]
fn a_program_reads_the_main_content_of_its_page_as_markdown() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "formats");
    let product = proving_ground.url("extraction.test", "/markdown/mixed/product");
    let navigation = proving_ground.url("extraction.test", "/markdown/none/navigation-only");
    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "formats" }),
    ));

    call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": product }),
    );
    let markdown = call(
        &mut engine,
        4,
        "page.formats",
        &json!({ "page_id": page.page_id, "formats": { "markdown": true } }),
    );
    let envelope = call(
        &mut engine,
        5,
        "page.formats",
        &json!({ "page_id": page.page_id, "formats": {} }),
    );
    call(
        &mut engine,
        6,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": navigation }),
    );
    let none = call(
        &mut engine,
        7,
        "page.formats",
        &json!({ "page_id": page.page_id, "formats": { "markdown": { "content": "main" } } }),
    );
    call(
        &mut engine,
        8,
        "run.finish",
        &json!({ "run_id": "formats" }),
    );
    let (rest, exited) = engine.finish();

    assert_eq!(
        (
            &markdown["result"],
            &envelope["result"],
            &none["result"],
            (rest, exited)
        ),
        (
            &json!({
                "url": product,
                "base_url": product,
                "main_content": "found",
                "markdown": "## Desk Lamp\n\n**Price:** 39 €\n\n## Specifications\n\n| Power | 8 W |\n| --- | --- |\n| Colour | Black |\n\n## In the box\n\n- Lamp\n- USB cable\n",
            }),
            &json!({ "url": product, "base_url": product }),
            &json!({
                "url": navigation,
                "base_url": navigation,
                "main_content": "none",
                "markdown": "",
            }),
            (vec![], true)
        )
    );
}

/// Covers: error:browser_not_installed
#[cfg(unix)]
#[test]
fn a_launch_with_an_empty_browser_cache_fails_from_the_call_that_launches() {
    use std::os::unix::fs::PermissionsExt as _;

    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = env.folder().join("empty-browsers");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "no-browser");

    let opened = call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "no-browser" }),
    );
    let _ = call(
        &mut engine,
        3,
        "run.finish",
        &json!({ "run_id": "no-browser" }),
    );
    let _ = engine.finish();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        (
            &opened["error"]["data"]["code"],
            &opened["error"]["data"]["missing"]["artifact"]["build"],
        ),
        (
            &json!("browser_not_installed"),
            &json!(CHROME_FOR_TESTING_BUILD)
        )
    );
}

/// The command lines of every browser process the test's engine started:
/// those whose profile is in the test's folder, which `ps` names by the
/// folder name's last, ASCII, part.
#[cfg(unix)]
pub(crate) fn browser_command_lines(env: &TestEnv) -> Vec<String> {
    let name = env.folder().file_name().unwrap().to_string_lossy();
    let marker = name.rsplit(' ').next().unwrap().to_owned();
    let listed = Command::new("ps")
        .args(["-axww", "-o", "args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|line| line.contains(&marker) && line.contains("--user-data-dir="))
        .map(str::to_owned)
        .collect()
}

/// The CDP methods the Run's trace recorded.
#[cfg(unix)]
pub(crate) fn traced_methods(env: &TestEnv, run_id: &str) -> BTreeSet<String> {
    let file = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_CDP_COMMANDS);
    std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("{}: {error}", file.display()))
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["method"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

/// The checks the Proving ground's CDP probe page reported, once it
/// reported that it ran (within 10 s).
#[cfg(unix)]
fn probe_reports(proving_ground: &ProvingGround) -> Vec<(String, String)> {
    let ran = ("ran".to_owned(), "content".to_owned());
    let mut reports = Vec::new();
    for _ in 0..100 {
        reports = proving_ground.observations().unwrap()["reports"]
            .as_array()
            .unwrap()
            .iter()
            .map(|report| {
                (
                    report["check"].as_str().unwrap().to_owned(),
                    report["observed"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        if reports.contains(&ran) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    reports
}

/// What the probe would see of `Runtime` enabled and of a script outside
/// its isolated world. `navigator.webdriver`, which it also reports, is
/// the same with a window or without (#696); hiding it is area 8's.
#[cfg(unix)]
const SIDE_EFFECTS: [&str; 2] = ["runtime_enable", "automation_bindings"];

/// The browser runs over a pipe with no debugging port, `Runtime` is never
/// enabled (a headless browser's identity is read with `Runtime.evaluate`
/// on a page of its own, which needs no `Runtime.enable`), and a page's own
/// script sees neither `Runtime`'s side effect nor the guard's isolated
/// world (ADRs 0017 and 0019).
/// Covers: story:5-61
#[cfg(unix)]
#[test]
fn the_browser_runs_over_a_pipe_never_enables_runtime_and_hides_the_guards_world() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "probe");

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "probe" }),
    ));
    let went = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("access.test", "/cdp-probe") }),
    );
    let browsers = browser_command_lines(&env);
    // The probe reports that it ran, then each side effect it saw, right
    // after; closing the browser gives those reports the time to arrive.
    let _ = probe_reports(&proving_ground);
    let _ = call(&mut engine, 4, "run.finish", &json!({ "run_id": "probe" }));
    let _ = engine.finish();
    let methods = traced_methods(&env, "probe");
    let reports = probe_reports(&proving_ground);

    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    assert!(!browsers.is_empty(), "no browser process names the cache");
    assert_eq!(
        (
            browsers
                .iter()
                .all(|line| !line.contains("--remote-debugging-port")),
            browsers
                .iter()
                .any(|line| line.contains("--remote-debugging-pipe")),
            methods.contains("Page.navigate"),
            methods.contains("Runtime.enable"),
            reports.contains(&("ran".to_owned(), "content".to_owned())),
            reports
                .iter()
                .filter(|(check, observed)| {
                    check == "cdp" && SIDE_EFFECTS.contains(&observed.as_str())
                })
                .map(|(_, observed)| observed.clone())
                .collect::<Vec<_>>(),
        ),
        (true, true, true, false, true, Vec::<String>::new()),
        "{browsers:?}"
    );
}

/// A launched browser holds a shared lock on its build until it exits, so
/// the Browser cache's clean-up refuses to delete it meanwhile (ADR 0098).
/// Covers: story:5-61
#[test]
fn a_running_browser_keeps_its_build_from_the_clean_up_until_it_exits() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "held");
    let clean_up = || {
        let request = CleanUp {
            kept: BTreeSet::new(),
            now: jiff::Timestamp::now() + jiff::SignedDuration::from_hours(1),
            max_unused_age: std::time::Duration::ZERO,
            dry_run: false,
        };
        runtime()
            .block_on(
                BrowserCache::new(&CacheLocation::new(env.home(), Some(cache.clone())))
                    .clean_up(&request),
            )
            .unwrap()
            .deleted
            .into_iter()
            .map(|artifact| artifact.build)
            .collect::<Vec<_>>()
    };

    let _ = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "held" }),
    ));
    let while_running = clean_up();
    let _ = call(&mut engine, 3, "run.finish", &json!({ "run_id": "held" }));
    let _ = engine.finish();
    let after_exit = clean_up();

    assert_eq!(
        (while_running, after_exit),
        (
            Vec::<String>::new(),
            vec![CHROME_FOR_TESTING_BUILD.to_owned()]
        )
    );
}

/// `run new-page` through the command generated from the catalog: a Run
/// the command's own Attached engine stopped as the command ended has no
/// page to open, so the call fails typed (ADR 0099).
/// Covers: command:run.new-page
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_new_page_on_a_run_an_earlier_command_stopped_fails_with_run_not_found(#[case] mode: Mode) {
    let env = TestEnv::new();
    let started = cli(&env, COLLIE)
        .args(["run", "start", "--run-id", "first", "--attached"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{started}");

    let checked = cli(&env, COLLIE)
        .args(["run", "new-page", "first", "--json"])
        .mode(mode)
        .json("CommandFailed")
        .unwrap();

    assert_eq!(
        (
            checked.output.exit,
            &checked.documents[0]["error"]["code"],
            &checked.documents[0]["error"]["operation"],
        ),
        (Some(3), &json!("run_not_found"), &json!("run.new_page"))
    );
}

/// `page goto` through the command generated from the catalog: no engine
/// holds a page the command names, so the call fails with `closed`.
/// Covers: command:page.goto
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn page_goto_of_a_page_no_engine_holds_fails_with_closed(#[case] mode: Mode) {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args([
            "page",
            "goto",
            "page-1",
            "--url",
            "http://catalog.test/",
            "--json",
        ])
        .mode(mode)
        .json("CommandFailed")
        .unwrap();

    assert_eq!(
        (
            checked.output.exit,
            &checked.documents[0]["error"]["code"],
            &checked.documents[0]["error"]["operation"],
        ),
        (Some(3), &json!("closed"), &json!("page.goto"))
    );
}

/// `page formats` through the command generated from the catalog, its
/// request given as JSON: no engine holds a page the command names, so the
/// call fails with `closed`.
/// Covers: command:page.formats
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn page_formats_of_a_page_no_engine_holds_fails_with_closed(#[case] mode: Mode) {
    let env = TestEnv::new();

    let checked = cli(&env, COLLIE)
        .args([
            "page",
            "formats",
            "page-1",
            "--formats",
            r#"{"markdown":true}"#,
            "--json",
        ])
        .mode(mode)
        .json("CommandFailed")
        .unwrap();

    assert_eq!(
        (
            checked.output.exit,
            &checked.documents[0]["error"]["code"],
            &checked.documents[0]["error"]["operation"],
        ),
        (Some(3), &json!("closed"), &json!("page.formats"))
    );
}

/// Runs `page <command> page-1 --json` in `mode`, which names a page no
/// engine holds, and returns its exit status, Error code and operation.
fn closed_page(command: &str, mode: Mode) -> (Option<i32>, Value, Value) {
    let env = TestEnv::new();
    let checked = cli(&env, COLLIE)
        .args(["page", command, "page-1", "--json"])
        .mode(mode)
        .json("CommandFailed")
        .unwrap();
    (
        checked.output.exit,
        checked.documents[0]["error"]["code"].clone(),
        checked.documents[0]["error"]["operation"].clone(),
    )
}

/// `page reload` through the command generated from the catalog (#698): no
/// engine holds the page it names, so the call fails with `closed`.
/// Covers: command:page.reload
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn page_reload_of_a_page_no_engine_holds_fails_with_closed(#[case] mode: Mode) {
    assert_eq!(
        closed_page("reload", mode),
        (Some(3), json!("closed"), json!("page.reload"))
    );
}

/// `page close` through the command generated from the catalog (#719): no
/// engine holds the page it names, so the call fails with `closed`.
/// Covers: command:page.close
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn page_close_of_a_page_no_engine_holds_fails_with_closed(#[case] mode: Mode) {
    assert_eq!(
        closed_page("close", mode),
        (Some(3), json!("closed"), json!("page.close"))
    );
}

/// `page go-back` through the command generated from the catalog (#698):
/// no engine holds the page it names, so the call fails with `closed`.
/// Covers: command:page.go-back
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn page_go_back_of_a_page_no_engine_holds_fails_with_closed(#[case] mode: Mode) {
    assert_eq!(
        closed_page("go-back", mode),
        (Some(3), json!("closed"), json!("page.go_back"))
    );
}

/// `page go-forward` through the command generated from the catalog
/// (#698): no engine holds the page it names, so the call fails with
/// `closed`.
/// Covers: command:page.go-forward
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn page_go_forward_of_a_page_no_engine_holds_fails_with_closed(#[case] mode: Mode) {
    assert_eq!(
        closed_page("go-forward", mode),
        (Some(3), json!("closed"), json!("page.go_forward"))
    );
}
