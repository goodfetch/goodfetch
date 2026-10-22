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

use goodfetch_protocol::page::{PageOpened, TargetNotUniqueDetails};
use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache, cli};
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::attached::Attached;
use crate::run::call;

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
struct OnTask<'env> {
    engine: Attached<'env>,
    page: Value,
    next_id: i64,
}

impl<'env> OnTask<'env> {
    fn open(
        env: &'env TestEnv,
        cache: &Path,
        proving_ground: &ProvingGround,
        task: &str,
        surface: &str,
        action_timeout: u64,
    ) -> Self {
        let mut engine = Attached::start_waiting(env, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, cache);
        });
        engine.send(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "hello",
            "params": {
                "protocol_version": "0.1",
                "surface": surface,
                "needs": "collie",
                "product_version": "0.0.0",
            },
        }));
        let _ = engine.next();
        let resolve: Map<String, Value> = proving_ground
            .description()
            .resolve
            .iter()
            .map(|(host, address)| (host.clone(), json!([address])))
            .collect();
        let started = call(
            &mut engine,
            1,
            "run.start",
            &json!({
                "run_id": "locators",
                "settings": {
                    "network": { "resolve": resolve },
                    "timeout": { "action": action_timeout },
                },
            }),
        );
        assert!(started.get("result").is_some(), "{started}");
        let opened = call(
            &mut engine,
            2,
            "run.new_page",
            &json!({ "run_id": "locators" }),
        );
        let opened: PageOpened = serde_json::from_value(opened["result"].clone())
            .unwrap_or_else(|error| panic!("{error}: {opened}"));
        let page = json!(opened.page_id);
        let url = proving_ground.start_url(task).unwrap();
        let went = call(
            &mut engine,
            3,
            "page.goto",
            &json!({ "page_id": page, "url": url }),
        );
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
        call(&mut self.engine, id, method, &params)
    }

    /// Finishes the Run and stops the engine.
    fn finish(mut self) {
        let id = self.next_id;
        let _ = call(
            &mut self.engine,
            id,
            "run.finish",
            &json!({ "run_id": "locators" }),
        );
        let _ = self.engine.finish();
    }
}

/// The oracle's outcome for `task` reported as `status`, asked again until
/// it is `verified` or [`EFFECT_WAIT`] passes, since a page posts its
/// Effect after its event.
fn outcome(proving_ground: &ProvingGround, task: &str, status: &str) -> Value {
    let started = std::time::Instant::now();
    loop {
        let verdict = proving_ground.judge(task, status).unwrap();
        if verdict["outcome"] == "verified" || started.elapsed() > EFFECT_WAIT {
            return verdict;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Each task done through locator actions is verified by the oracle: one
/// trusted event of the right kind on the right control, none on a decoy
/// and none twice. A click waits out an overlay, a disabled or hidden
/// target, a duplicate a re-render leaves for a moment, and a target that
/// slips away from the pointer, whose click is cancelled on the decoy and
/// replayed; a locator survives a re-render that replaces its element;
/// typed text reaches a controlled field and a contenteditable box, Enter
/// a field, and the pointer opens a hover menu. Every action answers an
/// empty result: no handle on an element.
/// Covers: story:5-2, story:5-3, story:5-10, story:5-18, operation:locator.click, operation:locator.fill, operation:locator.press
#[rstest]
#[case::overlay("actionability/2", &[Step::Click("#target")])]
#[case::delayed("actionability/3", &[Step::Click("#target")])]
#[case::hidden("actionability/4", &[Step::Click("#target")])]
#[case::rerender("actionability/6", &[Step::Click("#target")])]
#[case::moved("actionability/18", &[Step::Click("#target")])]
#[case::ambiguous_until_rerendered("actionability/27", &[Step::Click("#target")])]
#[case::hover_menu("actionability/29", &[Step::Click("#menu"), Step::Click("#export")])]
#[case::controlled_field("actionability/28", &[Step::Fill("#name", "Goodfetch"), Step::Click("#save")])]
#[case::enter("actionability/15", &[Step::Press("#field", "Enter")])]
#[case::contenteditable("actionability/16", &[Step::Fill("#editor", "Goodfetch")])]
#[case::unicode("actionability/17", &[Step::Fill("#field", "Ünïcødé — 漢字 😀")])]
fn a_task_done_through_locator_actions_is_verified_by_the_oracle(
    #[case] task: &str,
    #[case] steps: &[Step],
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut page = OnTask::open(&env, &cache, &proving_ground, task, "sdk_python", 10_000);

    let answers: Vec<Value> = steps.iter().map(|step| page.act(*step)).collect();
    let verdict = outcome(&proving_ground, task, "success");
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
    "sdk_python",
    "page.locator(\"#target\")",
    ["page.locator(\"#target\").nth(0)", "page.locator(\"#target\").nth(1)"]
)]
#[case::typescript(
    "sdk_typescript",
    "page.locator('#target')",
    ["page.locator('#target').nth(0)", "page.locator('#target').nth(1)"]
)]
fn a_target_still_ambiguous_at_the_deadline_raises_target_not_unique_listing_every_match(
    #[case] surface: &str,
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
    let verdict = outcome(&proving_ground, "actionability/9", "failure");
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
    let mut page = OnTask::open(&env, &cache, &proving_ground, task, "sdk_python", 1_000);

    let clicked = page.act(Step::Click(selector));
    let verdict = outcome(&proving_ground, task, "failure");
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
    let file = env
        .home()
        .join("0.0")
        .join("runs")
        .join("locators")
        .join("trace")
        .join("cdp-commands.jsonl");
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
/// speed target).
/// Covers: story:5-2, operation:locator.click
#[test]
fn a_click_with_no_animation_running_checks_and_arms_in_one_round_trip() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut page = OnTask::open(
        &env,
        &cache,
        &proving_ground,
        "actionability/1",
        "sdk_python",
        10_000,
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
        (&json!({}), Some(&1), Some(&2), Some(&3), None),
        "{counts:?}"
    );
}

/// The locator actions through the commands generated from the catalog:
/// no engine holds a page the command names, so each fails with `closed`.
/// Covers: command:locator.click, command:locator.fill, command:locator.press
#[rstest]
#[case::click(&["locator", "click"])]
#[case::fill(&["locator", "fill", "--value", "Goodfetch"])]
#[case::press(&["locator", "press", "--key", "Enter"])]
fn a_locator_command_on_a_page_no_engine_holds_fails_with_closed(#[case] command: &[&str]) {
    let env = TestEnv::new();
    let mut arguments = command.to_vec();
    arguments.extend([
        "--locator",
        r##"{"page_id":"page-1","selector":"#target"}"##,
        "--json",
    ]);

    let checked = cli(&env, COLLIE)
        .args(arguments)
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
