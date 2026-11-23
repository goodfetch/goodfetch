//! Each Run's controls in the menu: which are offered for each Lifecycle
//! and kind of Run, Resume against "Copy resume command" (ADR 0031), the
//! confirmations the settings require, the calls they make, and a failed
//! call shown to the person.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{RunAbortParams, RunParams, RunStopParams};
use rstest::rstest;
use serde_json::{Map, json};

use super::super::{Choice, Confirmation, Control, Effect, Input, MenuChange, Row, State};
use super::runs::{axes, lines, origin, progress, reached, run_id};
use crate::client::{Call, CallFailure};
use crate::settings::Settings;

const PAUSE: &str = "Pause: a Checkpoint and a stop";
const STOP: &str = "Stop: a Checkpoint, and the Run can resume";
const ABORT: &str = "Abort: no attempt in flight commits; not immediate";
const RETRY_FAILED: &str = "Retry failed: its failed requests run again";
const RESUME: &str = "Resume: the Daemon runs it again from where it stopped";
const COPY_RESUME_COMMAND: &str = "Copy resume command";

/// The Run index's row of `run`, of `kind`, tagged with `recipe`'s name
/// when it has one.
fn row(run: &str, kind: &str, recipe: Option<&str>) -> goodfetch_protocol::run::RunIndexRow {
    let tags: Vec<String> = recipe
        .map(|name| format!("goodfetch.recipe:{name}"))
        .into_iter()
        .collect();
    serde_json::from_value(json!({
        "run_id": run,
        "location": format!("runs/{run}"),
        "kind": kind,
        "needs": if kind == "crawl" { "foxhound" } else { "collie" },
        "tags": tags,
        "lifecycle": "running",
        "started_at": "2026-11-03T09:00:00Z",
        "presence": "present",
        "replayed": false,
    }))
    .unwrap()
}

/// A State on a Daemon of `product` with the live Run `run`, of `kind`,
/// run by `recipe` when it names one.
fn running(product: Product, run: &str, kind: &str, recipe: Option<&str>) -> State {
    let mut state = reached(product);
    state.apply(Input::Events(vec![axes(run, "running")]));
    state.apply(Input::LiveRuns(vec![row(run, kind, recipe)]));
    state
}

/// `run` stops, and its progress gives `stop_reason` as why.
fn stopped(state: &mut State, run: &str, stop_reason: &serde_json::Value) {
    state.apply(Input::Events(vec![axes(run, "stopped")]));
    state.apply(Input::Progress(progress(
        run,
        json!({"lifecycle": "stopped", "last": true, "stop_reason": stop_reason}),
    )));
}

fn limit() -> serde_json::Value {
    json!({
        "kind": "limit",
        "scope": {"kind": "install", "id": ""},
        "usage_kind": "requests",
    })
}

/// The labels of the submenu the menu shows for the Run whose row starts
/// with `name`, or `None` when its row has no submenu.
fn controls(state: &State, name: &str) -> Option<Vec<String>> {
    state.view().rows.into_iter().find_map(|row| match row {
        Row::Submenu { label, rows } if label.starts_with(name) => Some(
            rows.into_iter()
                .map(|row| match row {
                    Row::Choice { label, .. } => label,
                    other => panic!("a control's submenu holds only choices: {other:?}"),
                })
                .collect(),
        ),
        _ => None,
    })
}

fn control(run: &str, control: Control) -> Choice {
    Choice::Control {
        run: run_id(run),
        control,
    }
}

fn read_only() -> Input {
    Input::Settings(Ok(Settings {
        controls: false,
        ..Settings::default()
    }))
}

#[rstest]
#[case::a_script_run(Product::Collie, "collie", None, vec![PAUSE, STOP, ABORT])]
#[case::a_recipe_run(Product::Collie, "collie", Some("nightly"), vec![PAUSE, STOP, ABORT])]
#[case::a_crawl(Product::Foxhound, "crawl", None, vec![PAUSE, STOP, ABORT, RETRY_FAILED])]
#[case::a_crawl_on_a_collie_daemon_hides_foxhounds_retry(Product::Collie, "crawl", None, vec![PAUSE, STOP, ABORT])]
fn a_running_run_offers_pause_stop_abort_and_for_a_crawl_retry_failed(
    #[case] product: Product,
    #[case] kind: &str,
    #[case] recipe: Option<&str>,
    #[case] expected: Vec<&str>,
) {
    let state = running(product, "live-run", kind, recipe);

    let name = recipe.unwrap_or("live-run");
    assert_eq!(controls(&state, name).unwrap(), expected);
}

#[rstest]
#[case::paused_recipe_run(Some("nightly"), json!({"kind": "pause"}), "nightly", RESUME)]
#[case::paused_script_run(None, json!({"kind": "pause"}), "a-run", COPY_RESUME_COMMAND)]
#[case::recipe_run_stopped_by_a_limit(Some("nightly"), limit(), "nightly", RESUME)]
#[case::script_run_stopped_by_a_limit(None, limit(), "a-run", COPY_RESUME_COMMAND)]
fn a_stopped_run_offers_resume_where_the_engine_can_run_it_and_its_command_otherwise(
    #[case] recipe: Option<&str>,
    #[case] stop_reason: serde_json::Value,
    #[case] name: &str,
    #[case] offered: &str,
) {
    let mut state = running(Product::Collie, "a-run", "collie", recipe);

    stopped(&mut state, "a-run", &stop_reason);

    assert_eq!(controls(&state, name).unwrap(), [offered]);
}

#[test]
fn a_failed_run_offers_no_control() {
    let mut state = running(Product::Foxhound, "broken-crawl", "crawl", None);

    state.apply(Input::Events(vec![axes("broken-crawl", "failed")]));

    assert_eq!(controls(&state, "broken-crawl"), None);
    assert!(lines(&state).contains(&"broken-crawl — Failed".to_owned()));
}

#[test]
fn a_paused_run_stays_listed_as_paused_until_it_runs_again() {
    let mut state = running(Product::Collie, "a-run", "collie", None);
    stopped(&mut state, "a-run", &json!({"kind": "pause"}));
    let paused = lines(&state);
    state.apply(Input::Menu(MenuChange::Opened(origin())));
    state.apply(Input::Menu(MenuChange::Closed));
    let after_a_menu = lines(&state);

    state.apply(Input::Events(vec![axes("a-run", "running")]));

    assert_eq!(
        paused,
        [
            "Collie Daemon 0.4.1",
            "---",
            "a-run — Paused >",
            "---",
            "Open details",
            "Quit"
        ]
    );
    assert_eq!(after_a_menu, paused);
    assert_eq!(controls(&state, "a-run").unwrap(), [PAUSE, STOP, ABORT]);
}

#[test]
fn a_read_only_kennel_offers_no_control_and_ignores_a_stale_choice() {
    let mut state = running(Product::Foxhound, "live-crawl", "crawl", None);

    state.apply(read_only());

    assert_eq!(controls(&state, "live-crawl"), None);
    assert!(lines(&state).contains(&"live-crawl — Running".to_owned()));
    assert_eq!(state.choose(control("live-crawl", Control::Pause)), []);
}

#[test]
fn pause_asks_nothing_and_pauses_the_run() {
    let mut state = running(Product::Collie, "a-run", "collie", None);

    let effects = state.choose(control("a-run", Control::Pause));

    assert_eq!(
        effects,
        [Effect::Call(Call::Pause(RunParams {
            run_id: run_id("a-run")
        }))]
    );
}

/// The one confirmation among `effects`.
fn confirmation(effects: Vec<Effect>) -> Confirmation {
    match <[Effect; 1]>::try_from(effects) {
        Ok([Effect::Confirm(confirmation)]) => confirmation,
        other => panic!("one confirmation: {other:?}"),
    }
}

#[rstest]
#[case::stop(
    Control::Stop,
    "Stop nightly?",
    "It stops at its next safe point with a Checkpoint, and it can resume.",
    "Stop"
)]
#[case::abort(
    Control::Abort,
    "Abort nightly?",
    "It stops at its next safe point, not at once. No attempt in flight commits, and its work items return to pending.",
    "Abort"
)]
#[case::retry_failed(
    Control::RetryFailed,
    "Retry the failed requests of nightly?",
    "Each request whose latest outcome is failed runs again in this crawl, with its retries counted afresh.",
    "Retry failed"
)]
fn stop_abort_and_retry_failed_ask_first_naming_the_run_and_the_consequence(
    #[case] chosen: Control,
    #[case] title: &str,
    #[case] consequence: &str,
    #[case] confirm: &str,
) {
    let mut state = running(Product::Foxhound, "crawl-7", "crawl", Some("nightly"));

    let asked = confirmation(state.choose(control("crawl-7", chosen)));

    assert_eq!(
        (
            asked.title.as_str(),
            asked.consequence.as_str(),
            asked.confirm.as_str()
        ),
        (title, consequence, confirm)
    );
}

#[rstest]
#[case::stop(Control::Stop, Call::Stop(RunStopParams { run_id: run_id("crawl-7"), timeout: None }))]
#[case::abort(Control::Abort, Call::Abort(RunAbortParams { run_id: run_id("crawl-7"), forwarded: false }))]
#[case::retry_failed(
    Control::RetryFailed,
    Call::RetryFailed(goodfetch_protocol::foxhound::outcomes::RunRetryFailedParams {
        run_id: run_id("crawl-7"),
        blocked: false,
        settings: Map::new(),
        project: None,
    })
)]
fn a_confirmed_control_makes_its_call(#[case] chosen: Control, #[case] call: Call) {
    let mut state = running(Product::Foxhound, "crawl-7", "crawl", None);
    let asked = confirmation(state.choose(control("crawl-7", chosen)));

    let effects = state.confirmed(asked);

    assert_eq!(effects, [Effect::Call(call)]);
}

#[test]
fn with_the_stop_confirmation_off_stop_asks_nothing_and_abort_still_asks() {
    let mut state = running(Product::Collie, "a-run", "collie", None);
    state.apply(Input::Settings(Ok(Settings {
        confirm_stop: false,
        ..Settings::default()
    })));

    let stop = state.choose(control("a-run", Control::Stop));
    let abort = state.choose(control("a-run", Control::Abort));

    assert_eq!(
        stop,
        [Effect::Call(Call::Stop(RunStopParams {
            run_id: run_id("a-run"),
            timeout: None
        }))]
    );
    assert_eq!(confirmation(abort).confirm, "Abort");
}

#[test]
fn a_confirmation_answered_once_the_control_is_no_longer_offered_does_nothing() {
    let mut state = running(Product::Collie, "a-run", "collie", None);
    let asked = confirmation(state.choose(control("a-run", Control::Abort)));
    state.apply(read_only());

    assert_eq!(state.confirmed(asked), []);
}

#[test]
fn a_control_the_run_does_not_offer_does_nothing() {
    let mut state = running(Product::Collie, "a-run", "collie", None);

    assert_eq!(state.choose(control("a-run", Control::Resume)), []);
    assert_eq!(state.choose(control("a-run", Control::RetryFailed)), []);
    assert_eq!(state.choose(control("no-such-run", Control::Pause)), []);
}

#[test]
fn resume_resumes_a_recipes_run_on_the_daemon() {
    let mut state = running(Product::Collie, "a-run", "collie", Some("nightly"));
    stopped(&mut state, "a-run", &json!({"kind": "pause"}));

    let effects = state.choose(control("a-run", Control::Resume));

    assert_eq!(
        effects,
        [Effect::Call(Call::Resume(
            goodfetch_protocol::checkpoint::RunResumeParams {
                run_id: run_id("a-run"),
                checkpoint: None,
                rollback: false,
                settings: Map::new(),
                project: None,
            }
        ))]
    );
}

#[rstest]
#[case::collie(Product::Collie, "goodfetch-collie run resume a-run")]
#[case::foxhound(Product::Foxhound, "goodfetch-foxhound run resume a-run")]
fn copy_resume_command_copies_the_daemons_products_run_resume(
    #[case] product: Product,
    #[case] command: &str,
) {
    let mut state = running(product, "a-run", "collie", None);
    stopped(&mut state, "a-run", &json!({"kind": "pause"}));

    let effects = state.choose(control("a-run", Control::CopyResumeCommand));

    assert_eq!(effects, [Effect::Copy(command.to_owned())]);
}

#[test]
fn a_failed_call_shows_in_the_menu_until_the_menu_that_showed_it_closes() {
    let mut state = running(Product::Collie, "a-run", "collie", Some("nightly"));
    let call = Call::Pause(RunParams {
        run_id: run_id("a-run"),
    });

    state.apply(Input::Answered(
        call,
        Err(CallFailure {
            code: "run_busy".to_owned(),
            message: "Another engine holds the Run".to_owned(),
        }),
    ));
    let shown = lines(&state);
    state.apply(Input::Menu(MenuChange::Opened(origin())));
    state.apply(Input::Menu(MenuChange::Closed));

    assert!(shown.contains(&"Pausing nightly failed: Another engine holds the Run".to_owned()));
    assert_eq!(
        lines(&state),
        [
            "Collie Daemon 0.4.1",
            "---",
            "nightly — Running >",
            "---",
            "Open details",
            "Quit"
        ]
    );
}
