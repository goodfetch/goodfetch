use std::collections::BTreeMap;

use serde_json::json;

use super::*;

#[test]
fn each_cell_plans_its_pages_and_counts_its_actions() {
    let tasks = |exercise| {
        cell(exercise)
            .pages
            .iter()
            .map(|page| page.task.clone())
            .collect::<Vec<_>>()
    };
    let numbered = |family: &str, numbers: &[u32]| {
        numbers
            .iter()
            .map(|number| format!("{family}/{number}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        tasks(Exercise::Click),
        numbered(
            "actionability",
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 18, 19, 26, 27]
        )
    );
    // Twelve clicks on `#target`, and fifty on page 26's counter.
    assert_eq!(cell(Exercise::Click).actions(), 62);
    assert_eq!(
        tasks(Exercise::Form),
        numbered("actionability", &[20, 21, 22, 23, 24, 28])
    );
    assert_eq!(cell(Exercise::Form).actions(), 22);
    assert_eq!(
        tasks(Exercise::Navigation),
        numbered("portal", &[6, 7, 8, 9, 10, 11, 12, 13])
    );
    assert_eq!(cell(Exercise::Navigation).actions(), 8);
    assert!(
        cell(Exercise::Navigation)
            .pages
            .iter()
            .all(|page| page.steps == [Step::Read])
    );
    let sessions = cell(Exercise::Sessions);
    assert!(sessions.pages.is_empty());
    assert_eq!(
        (sessions.sessions, sessions.actions()),
        (SESSIONS, SESSIONS)
    );
    let at_once = cell(Exercise::AtOnce);
    assert!(at_once.at_once);
    assert_eq!(at_once.pages.len(), 16);
    assert_eq!(at_once.actions(), 81);
    for exercise in [Exercise::Click, Exercise::Form, Exercise::Navigation] {
        assert!(!cell(exercise).at_once);
    }
}

#[test]
fn a_plan_gives_each_page_its_tasks_start_url_and_the_action_timeout() {
    let urls: BTreeMap<String, String> = (6..=13)
        .map(|number| {
            (
                format!("portal/{number}"),
                format!("http://{number}.portal.test:4000/list?page=1"),
            )
        })
        .collect();
    let plan = cell(Exercise::Navigation).plan(&urls).unwrap();
    assert_eq!(plan["at_once"], false);
    assert_eq!(plan["sessions"], 0);
    assert_eq!(plan["action_timeout"], ACTION_TIMEOUT_MILLISECONDS);
    assert_eq!(
        plan["pages"][0],
        json!({
            "task": "portal/6",
            "url": "http://6.portal.test:4000/list?page=1",
            "steps": [{"action": "read"}],
        })
    );
    let form = cell(Exercise::Form)
        .plan(&BTreeMap::from([(
            "actionability/20".to_owned(),
            "http://20.actionability.test:4000/".to_owned(),
        )]))
        .unwrap_err();
    assert!(
        format!("{form:#}").contains("actionability/21"),
        "a task the Proving ground does not list is named: {form:#}"
    );
    let plan = cell(Exercise::Form)
        .plan(
            &(20..=28)
                .map(|number| {
                    (
                        format!("actionability/{number}"),
                        format!("http://{number}.actionability.test:4000/"),
                    )
                })
                .collect(),
        )
        .unwrap();
    assert_eq!(
        plan["pages"][0]["steps"],
        json!([
            {"action": "fill", "selector": "#name", "value": "Goodfetch"},
            {"action": "press", "selector": "#region", "key": "e"},
            {"action": "click", "selector": "#consent"},
            {"action": "click", "selector": "#submit"},
        ])
    );
}

#[test]
fn collies_configuration_maps_each_host_to_loopback_and_lets_it_through() {
    let text = configuration(&[
        "http://1.actionability.test:4000/".to_owned(),
        "http://6.portal.test:4000/list?page=1".to_owned(),
    ])
    .unwrap();
    let configuration: toml::Table = text.parse().unwrap();
    assert_eq!(configuration["version"].as_str(), Some("0.1"));
    let settings = &configuration["settings"];
    assert_eq!(
        settings["network"]["resolve"]["6.portal.test"],
        toml::Value::Array(vec!["127.0.0.1".into()])
    );
    assert_eq!(
        settings["egress"]["private_ranges"],
        toml::Value::Array(vec!["1.actionability.test".into(), "6.portal.test".into()])
    );
    assert_eq!(
        settings["timeout"]["action"].as_integer(),
        Some(i64::try_from(ACTION_TIMEOUT_MILLISECONDS).unwrap())
    );
    assert_eq!(
        settings["run_state"]["key_reference"].as_str(),
        Some(format!("env:{RUN_STATE_KEY_VARIABLE}").as_str())
    );
    // The user configuration is read from the folder of the engine's
    // version line: xtask's version is the workspace's.
    assert_eq!(version_line().unwrap(), "0.0");
}

/// A page as the tool left it.
fn page(task: &str, ended: Ended, verdict: Option<&str>) -> PageDone {
    PageDone {
        task: task.to_owned(),
        ended,
        status: Some(200),
        read: None,
        error: (ended == Ended::Failure).then(|| "timed out".to_owned()),
        verdict: verdict.map(str::to_owned),
    }
}

/// A trial of `exercise` that did its cell's work, every page verified.
fn done(exercise: Exercise, tool: Driver, number: u32, commands: u64) -> CollieTrial {
    let cell = cell(exercise);
    CollieTrial {
        exercise,
        tool,
        number,
        load: None,
        actions: cell.actions(),
        sessions: cell.sessions,
        commands,
        cpu_seconds: 0.5,
        wall_seconds: 1.0,
        peak_bytes: 100 << 20,
        pages: cell
            .pages
            .iter()
            .map(|planned| {
                let mut done = page(
                    &planned.task,
                    Ended::Success,
                    planned
                        .task
                        .starts_with("actionability/")
                        .then_some("verified"),
                );
                if planned.steps == [Step::Read] {
                    done.read = Some(format!("digest of {}", planned.task));
                }
                done
            })
            .collect(),
    }
}

/// Every trial of a run of `trials` of each tool in each cell, Collie's
/// sending `collie` commands, Playwright's `playwright`.
fn every(trials: u32, collie: u64, playwright: u64) -> Vec<CollieTrial> {
    EXERCISES
        .iter()
        .flat_map(|&exercise| {
            (1..=trials).flat_map(move |number| {
                [
                    done(exercise, Driver::Collie, number, collie),
                    done(exercise, Driver::Playwright, number, playwright),
                ]
            })
        })
        .collect()
}

fn measured(trials_run: Vec<CollieTrial>) -> CollieCell {
    CollieCell::Measured {
        trials: 2,
        trials_run,
    }
}

fn kinds(findings: &[Finding]) -> Vec<(&str, &str)> {
    findings
        .iter()
        .map(|finding| (finding.kind, finding.subject.as_str()))
        .collect()
}

#[test]
fn trials_of_the_same_work_pass_whatever_their_outcomes() {
    let mut trials = every(2, 120, 100);
    // A failed click is an outcome, not other work.
    let click = trials
        .iter_mut()
        .find(|trial| trial.exercise == Exercise::Click && trial.tool == Driver::Playwright)
        .unwrap();
    click.pages[6] = page("actionability/7", Ended::Failure, Some("failed"));
    assert!(check(&measured(trials)).is_empty());
    assert!(
        check(&CollieCell::NotRun {
            reason: "not asked".to_owned()
        })
        .is_empty()
    );
}

#[test]
fn a_trial_of_other_work_is_refused_and_named() {
    let mut trials = every(2, 120, 100);
    let index = |trials: &[CollieTrial], exercise, tool, number| {
        trials
            .iter()
            .position(|trial| {
                trial.exercise == exercise && trial.tool == tool && trial.number == number
            })
            .unwrap()
    };
    // Other pages.
    let at = index(&trials, Exercise::Click, Driver::Collie, 1);
    trials[at].pages.pop();
    // Another page's text read.
    let at = index(&trials, Exercise::Navigation, Driver::Playwright, 2);
    trials[at].pages[3].read = Some("another digest".to_owned());
    // Other actions.
    let at = index(&trials, Exercise::Sessions, Driver::Collie, 2);
    trials[at].sessions = 9;
    // Nothing counted.
    let at = index(&trials, Exercise::Form, Driver::Playwright, 1);
    trials[at].commands = 0;
    // A cell short of its trials.
    let at = index(&trials, Exercise::AtOnce, Driver::Playwright, 2);
    trials.remove(at);
    assert_eq!(
        kinds(&check(&measured(trials))),
        [
            ("different_pages", "collie click collie trial 1"),
            ("nothing_counted", "collie form playwright trial 1"),
            ("different_work", "collie navigation playwright trial 2"),
            ("different_actions", "collie sessions collie trial 2"),
            ("trial_count", "collie at_once playwright"),
        ]
    );
}

#[test]
fn a_page_both_tools_split_on_evenly_names_every_trial() {
    let mut trials: Vec<CollieTrial> = every(1, 120, 100)
        .into_iter()
        .filter(|trial| trial.exercise == Exercise::Navigation)
        .collect();
    trials[1].pages[0].status = Some(404);
    let findings = check(&CollieCell::Measured {
        trials: 1,
        trials_run: trials,
    });
    let mut subjects: Vec<_> = kinds(&findings);
    subjects.retain(|(kind, _)| *kind != "trial_count");
    assert_eq!(
        subjects,
        [
            ("different_work", "collie navigation collie trial 1"),
            ("different_work", "collie navigation playwright trial 1"),
        ]
    );
}

const REGISTER: &str = "\
## Quality register

### Enforced

| Dimension | Rule | Checked by |
|---|---|---|
| Lint | clean | `just lint` |

### Measured, not yet enforced

| Row | Metric, per unit | Contract | Today | Benchmark | Ratchet | Checked by | Run by |
|---|---|---|---|---|---|---|---|
| `collie.playwright_command_ratio` | Collie's CDP commands per action over Playwright's | ≤ 1 | TODAY | the Collie scenario | must not grow | `just compare` | owner's report |

### Exceptions

| ID | Rule | Expires |
|---|---|---|
| E1 | a rule | 2027-01-01 |
";

fn measured_section(today: &str) -> Section {
    let mut trials = every(2, 0, 0);
    for trial in &mut trials {
        // Per action: Collie 1.5 commands, Playwright 1.25, on every cell.
        let actions = trial.actions;
        trial.commands = match trial.tool {
            Driver::Collie => actions * 3 / 2,
            Driver::Playwright => actions * 5 / 4,
        };
        trial.wall_seconds =
            0.001 * f64::from(u32::try_from(actions).unwrap()) * f64::from(trial.number);
    }
    let click = trials
        .iter_mut()
        .find(|trial| {
            trial.exercise == Exercise::Click && trial.tool == Driver::Collie && trial.number == 2
        })
        .unwrap();
    click.pages[6] = page("actionability/7", Ended::Failure, Some("failed"));
    section(&measured(trials), Some(&REGISTER.replace("TODAY", today)))
}

#[test]
fn the_section_gives_each_cells_figures_per_action_and_the_pages_each_tool_got_right() {
    let report = serde_json::to_value(measured_section("1.100 (issue 2300)")).unwrap();
    assert_eq!(report["status"], "measured");
    let click = &report["cells"][0];
    assert_eq!(click["exercise"], "click");
    assert_eq!(click["actions"], 62);
    let collie = &click["tools"]["collie"];
    assert_eq!(collie["commands_per_action"]["median"], 93.0 / 62.0);
    assert_eq!(collie["commands_per_action"]["unit"], "count");
    assert_eq!(collie["cpu_seconds_per_action"]["median"], 0.5 / 62.0);
    // Trials 1 and 2: 1 ms and 2 ms per action.
    let wall = &collie["wall_seconds_per_action"];
    for (bound, expected) in [("low", 0.001), ("high", 0.002)] {
        let value = wall[bound].as_f64().unwrap();
        assert!((value - expected).abs() < 1e-12, "{bound}: {value}");
    }
    assert_eq!(collie["peak_bytes"]["median"], f64::from(100 << 20));
    // Page 7 failed in one of Collie's two trials: not right.
    assert_eq!(collie["pages_right"]["right"], 12);
    assert_eq!(collie["pages_right"]["ran"], 13);
    assert_eq!(
        collie["pages_right"]["not_right"][0]["task"],
        "actionability/7"
    );
    assert_eq!(click["tools"]["playwright"]["pages_right"]["right"], 13);
    // The portal's pages and the Sessions are no actionability page.
    assert!(report["cells"][2]["tools"]["collie"]["pages_right"].is_null());
    // Over every cell, each actionability page once: 1 to 9, 15, 18 to 24,
    // 26 to 28.
    assert_eq!(report["pages_right"]["collie"]["ran"], 20);
    assert_eq!(report["pages_right"]["collie"]["right"], 19);
    assert_eq!(report["pages_right"]["playwright"]["right"], 20);
}

#[test]
fn the_section_gives_the_command_ratio_of_the_click_and_form_cells_against_the_rows_today() {
    let over = serde_json::to_value(measured_section("1.100 (issue 2300)")).unwrap();
    let ratios = &over["ratios"];
    assert_eq!(ratios.as_array().unwrap().len(), 2);
    assert_eq!(ratios[0]["exercise"], "click");
    // 93/62 over 77/62, to three decimals.
    assert_eq!(ratios[0]["ratio"], 1.208);
    assert_eq!(ratios[1]["exercise"], "form");
    assert_eq!(over["today"], json!({"status": "figure", "today": 1.1}));
    assert_eq!(ratios[0]["over_today"], true);
    let text = markdown(&measured_section("1.100 (issue 2300)"));
    assert!(text.contains("over the row's Today, 1.100"), "{text}");
    let under = serde_json::to_value(measured_section("1.500")).unwrap();
    assert_eq!(under["ratios"][0]["over_today"], false);
    let none = measured_section("not run yet: first read by issue 2300");
    let json = serde_json::to_value(&none).unwrap();
    assert_eq!(json["today"]["status"], "no_figure");
    assert!(json["ratios"][0]["over_today"].is_null());
    let text = markdown(&none);
    assert!(
        text.contains("has no Today figure yet (not run yet: first read by issue 2300)"),
        "{text}"
    );
}

#[test]
fn a_collie_scenario_not_run_says_why() {
    let section = section(
        &CollieCell::NotRun {
            reason: "the run asked for no Collie cell".to_owned(),
        },
        None,
    );
    assert!(markdown(&section).contains("not run: the run asked for no Collie cell"));
}
