//! A Collie program's locator actions over stdio (spec #577, ticket #700),
//! through `goodfetch-protocol`'s types: `locator.click`, `locator.fill`
//! and `locator.press` act through the guard on the Proving ground's
//! actionability pages, whose oracle sees which control received which
//! event, and whether the event was trusted. Each test has its own
//! Goodfetch home, a cleared environment, its own Proving ground and a
//! Browser cache in its own folder (ADRs 0059 and 0094); the guard's
//! window, `timeout.action`, is shortened through the Run's settings.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test polls the Proving ground's oracle and reads the Run trace on its own thread, never an async worker"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::home::RUN_TRACE_CDP_COMMANDS;
use goodfetch_protocol::page::{PageOpened, TargetNotUniqueDetails};
use goodfetch_test_client::Hello;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::ToolStatus;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, Mode, ProvingGround, TestEnv, browser_cache, cli,
};
use rstest::rstest;
use serde_json::{Map, Value, json};

use goodfetch_test_client::Attached;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Proving ground's own seed for these tests.
const SEED: u64 = 700;

/// How long a call may take: the first page launches the browser, checking
/// its build's digest, and the Run's end waits for the browser to exit.
const BROWSER_DEADLINE: Duration = Duration::from_secs(60);

/// How long the oracle is asked again for an Effect a page posts after its
/// event.
const EFFECT_WAIT: Duration = Duration::from_secs(10);

/// One locator action of a task.
#[derive(Clone, Copy, Debug)]
enum Step {
    Click(&'static str),
    Fill(&'static str, &'static str),
    Press(&'static str, &'static str),
}

impl Step {
    fn call(self, page: &Value) -> (&'static str, Value) {
        let locator = |selector: &str| json!({ "page_id": page, "selector": selector });
        match self {
            Self::Click(selector) => ("locator.click", json!({ "locator": locator(selector) })),
            Self::Fill(selector, value) => (
                "locator.fill",
                json!({ "locator": locator(selector), "value": value }),
            ),
            Self::Press(selector, key) => (
                "locator.press",
                json!({ "locator": locator(selector), "key": key }),
            ),
        }
    }
}

/// A Run's page on the Proving ground's task `task`, its engine's
/// `hello` naming `surface`, its Run's `timeout.action` `action_timeout`.
struct OnTask {
    engine: Attached,
    page: Value,
    next_id: i64,
}

impl OnTask {
    fn open(
        env: &TestEnv,
        cache: &Path,
        proving_ground: &ProvingGround,
        task: &str,
        surface: Surface,
        action_timeout: u64,
    ) -> Self {
        Self::open_with(
            env,
            cache,
            proving_ground,
            task,
            surface,
            action_timeout,
            &Map::new(),
        )
    }

    /// [`Self::open`], the Run's settings holding `settings` too.
    fn open_with(
        env: &TestEnv,
        cache: &Path,
        proving_ground: &ProvingGround,
        task: &str,
        surface: Surface,
        action_timeout: u64,
        settings: &Map<String, Value>,
    ) -> Self {
        let mut engine = Attached::start_with(env, COLLIE, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, cache);
        });
        engine.send(&Hello::needing(Product::Collie).surface(surface).message());
        let _ = engine.next();
        let resolve: Map<String, Value> = proving_ground
            .description()
            .resolve
            .iter()
            .map(|(host, address)| (host.clone(), json!([address])))
            .collect();
        let mut run_settings = settings.clone();
        crate::page::allow_private(&mut run_settings, resolve.keys());
        run_settings.insert("network".to_owned(), json!({ "resolve": resolve }));
        run_settings.insert("timeout".to_owned(), json!({ "action": action_timeout }));
        let started = engine.ask(
            1,
            "run.start",
            &json!({ "run_id": "locators", "settings": run_settings }),
        );
        assert!(started.get("result").is_some(), "{started}");
        let opened = engine.ask(2, "run.new_page", &json!({ "run_id": "locators" }));
        let opened: PageOpened = serde_json::from_value(opened["result"].clone())
            .unwrap_or_else(|error| panic!("{error}: {opened}"));
        let page = json!(opened.page_id);
        let url = proving_ground.start_url(task).unwrap();
        let went = engine.ask(3, "page.goto", &json!({ "page_id": page, "url": url }));
        assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
        Self {
            engine,
            page,
            next_id: 4,
        }
    }

    fn act(&mut self, step: Step) -> Value {
        let (method, params) = step.call(&self.page);
        let id = self.next_id;
        self.next_id += 1;
        self.engine.ask(id, method, &params)
    }

    /// Finishes the Run and stops the engine.
    fn finish(mut self) {
        let id = self.next_id;
        let _ = self
            .engine
            .ask(id, "run.finish", &json!({ "run_id": "locators" }));
        let _ = self.engine.finish();
    }
}

/// The oracle's outcome for `task` reported as `status`, asked again until
/// it is `verified` or [`EFFECT_WAIT`] passes, since a page posts its
/// Effect after its event.
fn outcome(proving_ground: &ProvingGround, task: &str, status: ToolStatus) -> Value {
    let started = std::time::Instant::now();
    loop {
        let verdict = proving_ground.judge(task, status).unwrap();
        if verdict["outcome"] == "verified" || started.elapsed() > EFFECT_WAIT {
            return verdict;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Each task done through clicks is verified by the oracle. A click waits
/// out an overlay, a disabled or hidden target, a duplicate a re-render
/// leaves for a moment, and a target that slips away from the pointer,
/// whose click is cancelled on the decoy and replayed; a locator survives
/// a re-render that replaces its element; and the pointer opens a hover
/// menu.
/// Covers: story:5-2, story:5-3, story:5-10, story:5-18, operation:locator.click
#[rstest]
#[case::overlay("actionability/2", &[Step::Click("#target")])]
#[case::delayed("actionability/3", &[Step::Click("#target")])]
#[case::hidden("actionability/4", &[Step::Click("#target")])]
#[case::rerender("actionability/6", &[Step::Click("#target")])]
#[case::moved("actionability/18", &[Step::Click("#target")])]
#[case::ambiguous_until_rerendered("actionability/27", &[Step::Click("#target")])]
#[case::hover_menu("actionability/29", &[Step::Click("#menu"), Step::Click("#export")])]
fn a_task_done_through_clicks_is_verified_by_the_oracle(
    #[case] task: &str,
    #[case] steps: &[Step],
) {
    verified_by_the_oracle(task, steps);
}

/// Each task done through typed text is verified by the oracle: the text
/// reaches a controlled field, which then saves, a contenteditable box, and
/// a field taking text beyond ASCII.
/// Covers: story:5-10, operation:locator.fill
#[rstest]
#[case::controlled_field("actionability/28", &[Step::Fill("#name", "Goodfetch"), Step::Click("#save")])]
#[case::contenteditable("actionability/16", &[Step::Fill("#editor", "Goodfetch")])]
#[case::unicode("actionability/17", &[Step::Fill("#field", "Ünïcødé — 漢字 😀")])]
fn a_task_done_through_typed_text_is_verified_by_the_oracle(
    #[case] task: &str,
    #[case] steps: &[Step],
) {
    verified_by_the_oracle(task, steps);
}

/// Enter pressed in a field is verified by the oracle.
/// Covers: story:5-10, operation:locator.press
#[test]
fn enter_pressed_in_a_field_is_verified_by_the_oracle() {
    verified_by_the_oracle("actionability/15", &[Step::Press("#field", "Enter")]);
}

/// Does `task` through the locator actions `steps`, and asserts that the
/// oracle verifies it: one trusted event of the right kind on the right
/// control, none on a decoy and none twice. Every action answers an empty
/// result: no handle on an element.
fn verified_by_the_oracle(task: &str, steps: &[Step]) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut page = OnTask::open(
        &env,
        &cache,
        &proving_ground,
        task,
        Surface::SdkPython,
        10_000,
    );

    let answers: Vec<Value> = steps.iter().map(|step| page.act(*step)).collect();
    let verdict = outcome(&proving_ground, task, ToolStatus::Success);
    page.finish();

    assert_eq!(
        (
            answers
                .iter()
                .map(|answer| answer
                    .get("result")
                    .cloned()
                    .unwrap_or_else(|| answer.clone()))
                .collect::<Vec<_>>(),
            &verdict["outcome"],
        ),
        (vec![json!({}); steps.len()], &json!("verified")),
        "{verdict}"
    );
}

/// A locator still matching two elements when `timeout.action` passes
/// raises `target_not_unique`, listing every match with the locator that
/// reaches it alone in the caller's language, its `limit` naming the
/// setting and its layer; nothing was clicked.
/// Covers: story:5-3, error:target_not_unique, setting:timeout.action, operation:locator.click
#[rstest]
#[case::python(
    Surface::SdkPython,
    "page.locator(\"#target\")",
    ["page.locator(\"#target\").nth(0)", "page.locator(\"#target\").nth(1)"]
)]
#[case::typescript(
    Surface::SdkTypescript,
    "page.locator('#target')",
    ["page.locator('#target').nth(0)", "page.locator('#target').nth(1)"]
)]
fn a_target_still_ambiguous_at_the_deadline_raises_target_not_unique_listing_every_match(
    #[case] surface: Surface,
    #[case] target: &str,
    #[case] matches: [&str; 2],
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut page = OnTask::open(
        &env,
        &cache,
        &proving_ground,
        "actionability/9",
        surface,
        1_000,
    );

    let clicked = page.act(Step::Click("#target"));
    let verdict = outcome(&proving_ground, "actionability/9", ToolStatus::Failure);
    page.finish();

    let details: TargetNotUniqueDetails = serde_json::from_value(clicked["error"]["data"].clone())
        .unwrap_or_else(|error| panic!("{error}: {clicked}"));
    assert_eq!(
        (
            &clicked["error"]["data"]["code"],
            details.target.as_str(),
            details
                .matches
                .iter()
                .map(|found| (found.element.as_str(), found.locator.as_str()))
                .collect::<Vec<_>>(),
            (
                details.limit.key.as_str(),
                &details.limit.value,
                details.limit.layer.as_str()
            ),
            details.elapsed >= 1_000,
            &verdict["outcome"],
        ),
        (
            &json!("target_not_unique"),
            target,
            vec![
                ("<button id=\"target\">Target</button>", matches[0]),
                ("<button id=\"target\">Other target</button>", matches[1]),
            ],
            ("timeout.action", &json!(1_000), "run"),
            true,
            &json!("verified"),
        ),
        "{clicked}"
    );
}

/// A target never found, never enabled, never uncovered, or swapped for
/// another control under the pointer raises `deadline_exceeded` when
/// `timeout.action` passes, naming the step it waited for and the
/// setting; nothing was carried out on the page.
/// Covers: story:5-2, error:deadline_exceeded, setting:timeout.action, operation:locator.click
#[rstest]
#[case::never_found("actionability/7", "#nothing", "target_attached")]
#[case::never_enabled("actionability/7", "#target", "target_enabled")]
#[case::never_uncovered("actionability/8", "#target", "target_receives_events")]
#[case::swapped_under_the_pointer("actionability/19", "#target", "target_attached")]
fn a_target_never_actionable_raises_deadline_exceeded_naming_the_step(
    #[case] task: &str,
    #[case] selector: &'static str,
    #[case] waited_for: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut page = OnTask::open(
        &env,
        &cache,
        &proving_ground,
        task,
        Surface::SdkPython,
        1_000,
    );

    let clicked = page.act(Step::Click(selector));
    let verdict = outcome(&proving_ground, task, ToolStatus::Failure);
    page.finish();

    assert_eq!(
        (
            &clicked["error"]["data"]["code"],
            &clicked["error"]["data"]["waited_for"],
            &clicked["error"]["data"]["limit"],
            &verdict["outcome"],
        ),
        (
            &json!("deadline_exceeded"),
            &json!(waited_for),
            &json!({ "key": "timeout.action", "value": 1_000, "layer": "run" }),
            &json!("verified"),
        ),
        "{clicked} {verdict}"
    );
}

/// How many times the Run's trace records each CDP method.
fn traced_counts(env: &TestEnv) -> BTreeMap<String, usize> {
    let file = HomeLayout::of(env.home()).run_trace("locators", RUN_TRACE_CDP_COMMANDS);
    let mut counts = BTreeMap::new();
    for line in std::fs::read_to_string(&file)
        .unwrap_or_else(|error| panic!("{}: {error}", file.display()))
        .lines()
    {
        let method = serde_json::from_str::<Value>(line).unwrap()["method"]
            .as_str()
            .unwrap()
            .to_owned();
        *counts.entry(method).or_default() += 1;
    }
    counts
}

/// A click on a target with no animation or transition running checks and
/// arms in one round trip into the guard's world, sends its three pointer
/// events and disarms: no stability wait, no second check (ADR 0019's
/// speed target). Live hiding and the Consent prompt runner's answering
/// are off, and the Page check matches by URL alone, so that the Run trace
/// holds the guard's commands beside block detection's one read of the
/// document the `goto` reached (a world and a call, since a rule reads a
/// 200 page's captcha widget), the wall test's start in it, which runs
/// with the answering off, and the `goto`'s question to the runner whether
/// its first pass there found a prompt to wait on (a world and a call
/// each, ADR 0048): the reads of each document are no action's.
/// Covers: story:5-2, operation:locator.click
#[test]
fn a_click_with_no_animation_running_checks_and_arms_in_one_round_trip() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut settings = Map::new();
    settings.insert("live_view".to_owned(), json!({ "hiding": "off" }));
    settings.insert("consent".to_owned(), json!({ "handling": false }));
    settings.insert(
        "session".to_owned(),
        json!({ "page_check": { "login_page": { "kind": "glob", "pattern": "**/login" } } }),
    );
    let mut page = OnTask::open_with(
        &env,
        &cache,
        &proving_ground,
        "actionability/1",
        Surface::SdkPython,
        10_000,
        &settings,
    );

    let clicked = page.act(Step::Click("#target"));
    page.finish();
    let counts = traced_counts(&env);

    assert_eq!(
        (
            &clicked["result"],
            counts.get("Page.createIsolatedWorld"),
            counts.get("Runtime.callFunctionOn"),
            counts.get("Input.dispatchMouseEvent"),
            counts.get("Runtime.enable"),
        ),
        (&json!({}), Some(&4), Some(&5), Some(&3), None),
        "{counts:?}"
    );
}

/// `locator click`, generated from the catalog: no engine holds the page
/// it names, so it fails with `closed`.
/// Covers: command:locator.click
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn locator_click_on_a_page_no_engine_holds_fails_with_closed(#[case] mode: Mode) {
    fails_with_closed(&["locator", "click"], mode);
}

/// `locator fill`, generated from the catalog: no engine holds the page
/// it names, so it fails with `closed`.
/// Covers: command:locator.fill
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn locator_fill_on_a_page_no_engine_holds_fails_with_closed(#[case] mode: Mode) {
    fails_with_closed(&["locator", "fill", "--value", "Goodfetch"], mode);
}

/// `locator press`, generated from the catalog: no engine holds the page
/// it names, so it fails with `closed`.
/// Covers: command:locator.press
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn locator_press_on_a_page_no_engine_holds_fails_with_closed(#[case] mode: Mode) {
    fails_with_closed(&["locator", "press", "--key", "Enter"], mode);
}

/// Runs the locator command `command` in `mode` on a page no engine
/// holds, and asserts that it fails with `closed`, naming its operation.
fn fails_with_closed(command: &[&str], mode: Mode) {
    let env = TestEnv::new();
    let mut arguments = command.to_vec();
    arguments.extend([
        "--locator",
        r##"{"page_id":"page-1","selector":"#target"}"##,
        "--json",
    ]);

    let checked = cli(&env, COLLIE)
        .args(arguments)
        .mode(mode)
        .json("CommandFailed")
        .unwrap();

    assert_eq!(
        (
            checked.output.exit,
            &checked.documents[0]["error"]["code"],
            &checked.documents[0]["error"]["operation"],
        ),
        (
            Some(3),
            &json!("closed"),
            &json!(format!("locator.{}", command[1]))
        )
    );
}
