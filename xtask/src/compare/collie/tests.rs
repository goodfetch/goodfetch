use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use serde_json::json;

use super::super::trials::{CollieStopped, DriverApart};
use super::*;

#[test]
fn each_cell_plans_its_pages_and_counts_its_actions() {
    let tasks = |exercise| {
        cell(exercise)
            .pages()
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
            .pages()
            .iter()
            .all(|page| page.steps == [Step::Read])
    );
    let sessions = cell(Exercise::Sessions);
    assert!(sessions.pages().is_empty());
    assert_eq!(
        (sessions.sessions(), sessions.actions()),
        (SESSIONS, SESSIONS)
    );
    let at_once = cell(Exercise::AtOnce);
    assert!(matches!(at_once, Cell::Pages { at_once: true, .. }));
    assert_eq!(at_once.pages().len(), 16);
    assert_eq!(at_once.actions(), 81);
    for exercise in [Exercise::Click, Exercise::Form, Exercise::Navigation] {
        assert!(matches!(cell(exercise), Cell::Pages { at_once: false, .. }));
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
}

#[test]
fn the_opening_cell_opens_its_pages_one_after_another_in_one_session() {
    let opening = cell(Exercise::Opening);
    assert!(opening.pages().is_empty());
    assert_eq!(
        (opening.sessions(), opening.openings(), opening.actions()),
        (0, OPENINGS, OPENINGS)
    );
    let plan = opening.plan(&BTreeMap::new()).unwrap();
    assert_eq!(plan["openings"], OPENINGS);
    assert_eq!(plan["sessions"], 0);
    assert_eq!(plan["pages"], json!([]));
    assert_eq!(EXERCISES.last(), Some(&Exercise::Opening));
    assert_eq!(Exercise::Opening.name(), "opening");
}

/// A page as the tool left it, after `steps` steps.
fn page(task: &str, ended: Ended, steps: u64, outcome: Option<OracleOutcome>) -> PageDone {
    PageDone {
        task: task.to_owned(),
        ended,
        status: Some(200),
        steps,
        read: None,
        error: (ended == Ended::Failure).then(|| "timed out".to_owned()),
        outcome,
    }
}

/// A trial of `exercise` that did its cell's work, every page verified.
fn done(exercise: Exercise, tool: Contender, number: u32, commands: u64) -> CollieTrial {
    let cell = cell(exercise);
    CollieTrial {
        exercise,
        tool,
        number,
        load: None,
        actions: cell.actions(),
        sessions: cell.sessions(),
        commands,
        rounds: commands / 2,
        cpu_seconds: 0.5,
        wall_seconds: 1.0,
        peak_bytes: 100 << 20,
        driver: None,
        pages: cell
            .pages()
            .iter()
            .map(|planned| {
                let mut done = page(
                    &planned.task,
                    Ended::Success,
                    planned.steps.len() as u64,
                    planned
                        .task
                        .starts_with("actionability/")
                        .then_some(OracleOutcome::Verified),
                );
                if planned.steps == [Step::Read] {
                    done.read = Some(format!("digest of {}", planned.task));
                }
                done
            })
            .collect(),
        retried: None,
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
                    done(exercise, Contender::Collie, number, collie),
                    done(exercise, Contender::Playwright, number, playwright),
                ]
            })
        })
        .collect()
}

fn measured(trials_run: Vec<CollieTrial>) -> CollieScenario {
    CollieScenario::Measured {
        trials: 2,
        tools: vec![Contender::Collie, Contender::Playwright],
        trials_run,
        stopped: Vec::new(),
        loads: page_load::tests::every(2, |_, _| 1.0),
        loads_stopped: Vec::new(),
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
        .find(|trial| trial.exercise == Exercise::Click && trial.tool == Contender::Playwright)
        .unwrap();
    click.pages[6] = page(
        "actionability/7",
        Ended::Failure,
        1,
        Some(OracleOutcome::Failed),
    );
    assert!(check(&measured(trials)).is_empty());
    assert!(
        check(&CollieScenario::NotRun {
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
    let at = index(&trials, Exercise::Click, Contender::Collie, 1);
    trials[at].pages.pop();
    // Another page's text read.
    let at = index(&trials, Exercise::Navigation, Contender::Playwright, 2);
    trials[at].pages[3].read = Some("another digest".to_owned());
    // Other actions.
    let at = index(&trials, Exercise::Sessions, Contender::Collie, 2);
    trials[at].sessions = 9;
    // Nothing counted.
    let at = index(&trials, Exercise::Form, Contender::Playwright, 1);
    trials[at].commands = 0;
    // A cell short of its trials.
    let at = index(&trials, Exercise::AtOnce, Contender::Playwright, 2);
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
fn a_page_that_took_other_steps_than_its_plan_is_other_work() {
    let mut trials = every(2, 120, 100);
    let at = |trials: &[CollieTrial], tool, number| {
        trials
            .iter()
            .position(|trial| {
                trial.exercise == Exercise::Form && trial.tool == tool && trial.number == number
            })
            .unwrap()
    };
    // Succeeded, though it took one step of its plan's four.
    let first = at(&trials, Contender::Collie, 1);
    trials[first].pages[0].steps = 1;
    // Failed on its second step of four: an outcome, not other work.
    let second = at(&trials, Contender::Playwright, 2);
    trials[second].pages[0] = page(
        "actionability/20",
        Ended::Failure,
        2,
        Some(OracleOutcome::Failed),
    );
    let findings = check(&measured(trials));
    assert_eq!(
        kinds(&findings),
        [("different_actions", "collie form collie trial 1")]
    );
    assert!(
        findings[0]
            .message
            .contains("actionability/20 took 1 steps of 4"),
        "{}",
        findings[0].message
    );
}

#[test]
fn an_opening_trial_that_opened_other_pages_is_other_work() {
    let mut trials = every(2, 120, 100);
    let at = trials
        .iter()
        .position(|trial| {
            trial.exercise == Exercise::Opening
                && trial.tool == Contender::Playwright
                && trial.number == 1
        })
        .unwrap();
    trials[at].actions = 9;
    let findings = check(&measured(trials));
    assert_eq!(
        kinds(&findings),
        [("different_actions", "collie opening playwright trial 1")]
    );
    assert!(
        findings[0]
            .message
            .contains("9 pages opened, where it plans 10"),
        "{}",
        findings[0].message
    );
}

#[test]
fn a_tool_stopped_twice_in_a_cell_is_not_measured_there() {
    let mut trials = every(2, 120, 100);
    // Collie's second click trial was stopped twice.
    trials.retain(|trial| {
        !(trial.exercise == Exercise::Click && trial.tool == Contender::Collie && trial.number == 2)
    });
    let scenario = CollieScenario::Measured {
        trials: 2,
        tools: vec![Contender::Collie, Contender::Playwright],
        trials_run: trials,
        stopped: vec![CollieStopped {
            exercise: Exercise::Click,
            tool: Contender::Collie,
            number: 2,
            reason: "the tool ran past 300 s".to_owned(),
        }],
        loads: page_load::tests::every(2, |_, _| 1.0),
        loads_stopped: Vec::new(),
    };
    assert!(check(&scenario).is_empty());
    let section = section(&scenario, "");
    let report = serde_json::to_value(&section).unwrap();
    let click = &report["cells"][0]["tools"]["collie"]["commands_per_action"];
    assert_eq!(click["status"], "not_measured");
    assert_eq!(report["ratios"].as_array().unwrap().len(), 1);
    assert_eq!(report["ratios"][0]["exercise"], "form");
    let text = markdown(&section);
    assert!(
        text.contains("trial 2 was stopped twice: the tool ran past 300 s"),
        "{text}"
    );
    assert!(text.contains("- click: not measured"), "{text}");
}

#[test]
fn a_trial_that_stopped_short_is_measured_per_action_it_took() {
    let mut trials = every(2, 0, 0);
    for trial in &mut trials {
        trial.commands = trial.actions * 2;
    }
    // Collie's first click trial took 31 actions of its 62 for 93 commands.
    let short = trials
        .iter_mut()
        .find(|trial| trial.exercise == Exercise::Click && trial.tool == Contender::Collie)
        .unwrap();
    short.actions = 31;
    short.commands = 93;
    let report = serde_json::to_value(section(&measured(trials), "")).unwrap();
    let commands = &report["cells"][0]["tools"]["collie"]["commands_per_action"];
    // 3 and 2 commands per action.
    assert_eq!(commands["high"], 3.0);
    assert_eq!(commands["low"], 2.0);
}

#[test]
fn a_page_both_tools_split_on_evenly_names_every_trial() {
    let mut trials: Vec<CollieTrial> = every(1, 120, 100)
        .into_iter()
        .filter(|trial| trial.exercise == Exercise::Navigation)
        .collect();
    trials[1].pages[0].status = Some(404);
    let findings = check(&CollieScenario::Measured {
        trials: 1,
        tools: vec![Contender::Collie, Contender::Playwright],
        trials_run: trials,
        stopped: Vec::new(),
        loads: Vec::new(),
        loads_stopped: Vec::new(),
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
            Contender::Collie => actions * 3 / 2,
            Contender::Playwright => actions * 5 / 4,
            arm @ (Contender::CollieProgram
            | Contender::PuppeteerLocator
            | Contender::PuppeteerPage
            | Contender::Stagehand
            | Contender::AgentBrowser
            | Contender::Chromiumoxide) => {
                unreachable!("`every` runs no {}", arm.name())
            }
        };
        trial.wall_seconds =
            0.001 * f64::from(u32::try_from(actions).unwrap()) * f64::from(trial.number);
    }
    let click = trials
        .iter_mut()
        .find(|trial| {
            trial.exercise == Exercise::Click
                && trial.tool == Contender::Collie
                && trial.number == 2
        })
        .unwrap();
    click.pages[6] = page(
        "actionability/7",
        Ended::Failure,
        1,
        Some(OracleOutcome::Failed),
    );
    section(&measured(trials), &REGISTER.replace("TODAY", today))
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
fn the_section_gives_the_opening_cells_wall_time_beside_the_commands_awaited_one_after_another() {
    // 30 commands over 10 pages opened, 15 of them awaited one after
    // another, in 1 s.
    let section = section(&measured(every(2, 30, 20)), "");
    let report = serde_json::to_value(&section).unwrap();
    let opening = &report["cells"][5];
    assert_eq!(opening["exercise"], "opening");
    assert_eq!(opening["actions"], OPENINGS);
    let collie = &opening["tools"]["collie"];
    assert_eq!(collie["rounds_per_action"]["median"], 1.5);
    assert_eq!(collie["commands_per_action"]["median"], 3.0);
    let text = markdown(&section);
    assert!(
        text.contains(
            "- collie: 100.0 ms per page opened, 1.50 CDP commands awaited one after another, of 3.00 sent."
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "- playwright: 100.0 ms per page opened, 1.00 CDP commands awaited one after another, of 2.00 sent."
        ),
        "{text}"
    );
    assert!(text.contains("### The page-load cell"), "{text}");
}

#[test]
fn a_collie_scenario_not_run_says_why() {
    let section = section(
        &CollieScenario::NotRun {
            reason: "the run asked for no Collie cell".to_owned(),
        },
        "",
    );
    assert!(markdown(&section).contains("not run: the run asked for no Collie cell"));
}

/// Every trial of a run of `trials` of every tool, the arms with Collie and
/// Playwright, in each cell it runs, each sending 100 commands.
fn every_tool(trials: u32) -> Vec<CollieTrial> {
    EXERCISES
        .iter()
        .flat_map(|&exercise| {
            Contender::ALL
                .into_iter()
                .filter(move |&tool| skipped(tool, exercise).is_none())
                .flat_map(move |tool| {
                    (1..=trials).map(move |number| done(exercise, tool, number, 100))
                })
        })
        .collect()
}

fn measured_every_tool(
    trials_run: Vec<CollieTrial>,
    stopped: Vec<CollieStopped>,
) -> CollieScenario {
    CollieScenario::Measured {
        trials: 2,
        tools: Contender::ALL.to_vec(),
        trials_run,
        stopped,
        loads: page_load::tests::every(2, |_, _| 1.0),
        loads_stopped: Vec::new(),
    }
}

/// `tool`'s first trial that ran `task` did not get it right.
fn miss(trials: &mut [CollieTrial], tool: Contender, task: &str) {
    let page = trials
        .iter_mut()
        .filter(|trial| trial.tool == tool)
        .flat_map(|trial| trial.pages.iter_mut())
        .find(|page| page.task == task)
        .unwrap();
    *page = page_of(task, page.steps);
}

fn page_of(task: &str, steps: u64) -> PageDone {
    page(task, Ended::Failure, steps, Some(OracleOutcome::Failed))
}

#[test]
fn the_scenario_runs_each_arm_in_every_cell_but_stagehand_and_agent_browser_in_the_sessions_cell() {
    assert_eq!(
        Contender::ALL.map(Contender::name),
        [
            "collie",
            "collie_program",
            "playwright",
            "puppeteer_locator",
            "puppeteer_page",
            "stagehand",
            "agent_browser",
            "chromiumoxide"
        ]
    );
    for tool in Contender::ALL {
        assert_eq!(serde_json::to_value(tool).unwrap(), json!(tool.name()));
    }
    let mut state = 1;
    let order = order(&Contender::ALL, 1, &mut state, |state| {
        *state += 1;
        *state
    });
    assert_eq!(order.len(), EXERCISES.len() * Contender::ALL.len() - 2);
    assert!(!order.contains(&(Exercise::Sessions, Contender::Stagehand, 1)));
    assert!(!order.contains(&(Exercise::Sessions, Contender::AgentBrowser, 1)));
    assert!(order.contains(&(Exercise::AtOnce, Contender::AgentBrowser, 1)));
    assert!(order.contains(&(Exercise::Sessions, Contender::PuppeteerPage, 1)));
    assert!(order.contains(&(Exercise::Sessions, Contender::Chromiumoxide, 1)));
    assert!(order.contains(&(Exercise::Sessions, Contender::CollieProgram, 1)));
    assert!(order.contains(&(Exercise::Click, Contender::Stagehand, 1)));
    // A run of every tool, Stagehand's and agent-browser's Sessions not
    // run, did its cells' work.
    assert_eq!(
        kinds(&check(&measured_every_tool(every_tool(2), Vec::new()))),
        []
    );
}

#[test]
fn each_tool_runs_its_own_driver_against_the_browser_xtask_launched() {
    let tools = Tools {
        proving_ground: Path::new("proving-ground"),
        engine: Path::new("engine"),
        driver: PathBuf::from("compare-collie/act.ts"),
        arms_driver: PathBuf::from("compare-collie-arms/act.ts"),
        chromiumoxide: PathBuf::from("builds/goodfetch-compare-chromiumoxide"),
        browser: Path::new("chrome"),
        goodfetch_variables: &[OsString::from("GOODFETCH_TEST_SUITE")],
    };
    let run = |tool| {
        let command = tools.command(tool);
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_owned())
            .collect();
        assert_eq!(removed, [OsString::from("GOODFETCH_TEST_SUITE")]);
        let mut words = vec![command.get_program().to_owned()];
        words.extend(command.get_args().map(ToOwned::to_owned));
        words
    };
    // A Node.js driver carries out the plan for whichever tool `--tool`
    // names.
    for tool in [
        Contender::Collie,
        Contender::CollieProgram,
        Contender::Playwright,
    ] {
        assert_eq!(
            run(tool),
            ["node", "compare-collie/act.ts", "--tool", tool.name()]
        );
    }
    for arm in Contender::ARMS
        .into_iter()
        .filter(|&arm| arm != Contender::Chromiumoxide)
    {
        assert_eq!(
            run(arm),
            ["node", "compare-collie-arms/act.ts", "--tool", arm.name()]
        );
    }
    // chromiumoxide is a Rust library: its side is a program of its own,
    // built beside the other Rust sides, which runs that one tool and takes
    // no `--tool`.
    assert_eq!(
        run(Contender::Chromiumoxide),
        ["builds/goodfetch-compare-chromiumoxide"]
    );
    // Stagehand's extension speaks CDP over a WebSocket of its own, which
    // Chrome opens to it only with flags of its own; every other tool's
    // browser is launched as it was.
    assert!(browser_flags(Contender::Stagehand).contains(&"--enable-unsafe-extension-debugging"));
    for tool in [
        Contender::Collie,
        Contender::CollieProgram,
        Contender::Playwright,
        Contender::PuppeteerLocator,
        Contender::PuppeteerPage,
        Contender::AgentBrowser,
        Contender::Chromiumoxide,
    ] {
        assert!(browser_flags(tool).is_empty(), "{}", tool.name());
    }
}

#[test]
fn the_report_lists_each_arm_and_the_calls_its_steps_map_to() {
    let text = markdown(&section(
        &measured_every_tool(every_tool(2), Vec::new()),
        "",
    ));
    for (tool, click) in [
        ("puppeteer_locator", "`Locator.click()`"),
        ("puppeteer_page", "`page.click()`"),
        ("stagehand", "`page.locator(…).click()`"),
        ("agent_browser", "`agent-browser click`"),
        ("chromiumoxide", "`Element::click()`"),
    ] {
        assert!(
            text.lines()
                .any(|line| line.starts_with(&format!("- {tool}: ")) && line.contains(click)),
            "{tool}: {text}"
        );
        assert!(
            text.contains(&format!("| click | 62 | {tool} | 1.6 (1.6–1.6) |")),
            "{tool}: {text}"
        );
    }
}

#[test]
fn each_arm_is_read_against_the_collie_row_the_pages_it_got_right_place_it_at() {
    let mut trials = every_tool(2);
    // Playwright missed page 5, and so did Puppeteer's locators. Its page
    // methods missed page 7 besides; Stagehand, page 7 alone.
    miss(&mut trials, Contender::Playwright, "actionability/5");
    miss(&mut trials, Contender::PuppeteerLocator, "actionability/5");
    miss(&mut trials, Contender::PuppeteerPage, "actionability/5");
    miss(&mut trials, Contender::PuppeteerPage, "actionability/7");
    miss(&mut trials, Contender::Stagehand, "actionability/7");
    let section = section(&measured_every_tool(trials, Vec::new()), "");
    let report = serde_json::to_value(&section).unwrap();
    let placements = &report["placements"];
    assert_eq!(placements["puppeteer_locator"], json!({"row": "guard_on"}));
    for arm in ["puppeteer_page", "stagehand"] {
        assert_eq!(
            placements[arm],
            json!({"row": "guard_off", "missed": ["actionability/7"]}),
            "{arm}"
        );
    }
    assert!(placements.get("collie").is_none());
    assert!(placements.get("playwright").is_none());
    let text = markdown(&section);
    assert!(
        text.contains("- puppeteer_locator: Collie's Guard-on row."),
        "{text}"
    );
    assert!(
        text.contains(
            "- stagehand: Collie's Guard-off row (#2440), still to come: it did not get \
             actionability/7 right, which Playwright got right."
        ),
        "{text}"
    );
}

#[test]
fn stagehand_is_not_measured_in_the_sessions_cell_and_an_arm_stopped_twice_is_not_placed() {
    let mut trials = every_tool(2);
    trials.retain(|trial| {
        !(trial.exercise == Exercise::Click
            && trial.tool == Contender::PuppeteerPage
            && trial.number == 2)
    });
    let scenario = measured_every_tool(
        trials,
        vec![CollieStopped {
            exercise: Exercise::Click,
            tool: Contender::PuppeteerPage,
            number: 2,
            reason: "the tool ran past 300 s".to_owned(),
        }],
    );
    assert_eq!(kinds(&check(&scenario)), []);
    let section = section(&scenario, "");
    let report = serde_json::to_value(&section).unwrap();
    let sessions = &report["cells"][3];
    assert_eq!(sessions["exercise"], "sessions");
    let commands = &sessions["tools"]["stagehand"]["commands_per_action"];
    assert_eq!(commands["status"], "not_measured");
    assert_eq!(
        commands["reason"],
        "Stagehand drives the browser's one context, so it opens no Session"
    );
    assert_eq!(
        report["placements"]["puppeteer_page"],
        json!({"row": "not_placed", "reason": "its click cell was not measured"})
    );
    let text = markdown(&section);
    assert!(
        text.contains("- puppeteer_page: not placed, its click cell was not measured."),
        "{text}"
    );
}

/// Collie's program arm runs the plan as Collie's raw row does: a page it
/// did otherwise fails the same-work check, named (issue 2406).
#[test]
fn a_program_arm_trial_of_other_work_is_refused_and_named() {
    let mut trials = every_tool(2);
    let at = trials
        .iter()
        .position(|trial| {
            trial.exercise == Exercise::Navigation
                && trial.tool == Contender::CollieProgram
                && trial.number == 1
        })
        .unwrap();
    trials[at].pages[0].status = Some(404);
    assert_eq!(
        kinds(&check(&measured_every_tool(trials, Vec::new()))),
        [("different_work", "collie navigation collie_program trial 1")]
    );
}

/// Each of Collie's rows gives, per action, its program's CPU and its
/// engine's apart and the JSON-RPC messages it sent (issue 2406).
#[test]
fn collies_rows_give_the_programs_and_the_engines_cpu_apart_and_the_messages_sent() {
    let mut trials = every_tool(2);
    for trial in &mut trials {
        let messages = match trial.tool {
            Contender::Collie => 1,
            Contender::CollieProgram => 2,
            _ => continue,
        };
        // Of the tree's 0.5 s, the program's own process spent 0.125 s.
        trial.driver = Some(DriverApart {
            cpu_seconds: 0.125,
            messages: trial.actions * messages,
        });
    }
    let section = section(&measured_every_tool(trials, Vec::new()), "");
    let report = serde_json::to_value(&section).unwrap();
    let click = &report["cells"][0]["tools"];
    let program = &click["collie_program"]["engine_apart"];
    assert_eq!(program["program_cpu_seconds"]["median"], 0.125 / 62.0);
    assert_eq!(program["engine_cpu_seconds"]["median"], 0.375 / 62.0);
    assert_eq!(program["messages"]["median"], 2.0);
    assert_eq!(click["collie"]["engine_apart"]["messages"]["median"], 1.0);
    assert!(click["playwright"]["engine_apart"].is_null());
    let text = markdown(&section);
    assert!(
        text.contains(
            "| click | collie_program | 16.1 ms (16.1 ms–16.1 ms) | 2.0 ms (2.0 ms–2.0 ms) \
             | 6.0 ms (6.0 ms–6.0 ms) | 2 (2–2) |"
        ),
        "{text}"
    );
}

/// The program arm is placed against no Collie row, and the report says it
/// carries no bar: ADR 0019's target is read on the raw row (issue 2406).
#[test]
fn the_program_arm_is_placed_against_no_row_and_carries_no_bar() {
    let section = section(&measured_every_tool(every_tool(2), Vec::new()), "");
    let report = serde_json::to_value(&section).unwrap();
    assert!(report["placements"].get("collie_program").is_none());
    let text = markdown(&section);
    assert!(text.contains("collie_program carries no bar"), "{text}");
}

/// The report's mapping says how each of Collie's rows drives the engine:
/// the raw row over JSON-RPC, the program arm through the SDK (issue 2406).
#[test]
fn each_collie_rows_mapping_says_how_it_drives_the_engine() {
    let text = markdown(&section(
        &measured_every_tool(every_tool(2), Vec::new()),
        "",
    ));
    assert!(
        text.lines().any(
            |line| line.starts_with("- collie_program: ") && line.contains("@goodfetch/collie")
        ),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("- collie: ") && line.contains("raw JSON-RPC")),
        "{text}"
    );
}

/// The driver adopts what its descendants detach: agent-browser's command
/// starts its daemon in a session of its own and exits, and the daemon
/// stays in the driver's tree, so its CPU and memory count, and the tree's
/// kill reaches it.
#[cfg(target_os = "linux")]
#[test]
fn a_process_the_driver_detached_stays_in_its_tree() {
    let mut command = Command::new("sh");
    command.args(["-c", "(setsid sleep 37 &); exec sleep 30"]);
    adopt_orphans(&mut command);
    let mut driver = command.spawn().unwrap();
    let pid = driver.id();
    let detached = || {
        sampler::tree(pid).into_iter().skip(1).find(|&child| {
            std::fs::read_to_string(format!("/proc/{child}/cmdline"))
                .is_ok_and(|line| line == "sleep\u{0}37\u{0}")
        })
    };
    let mut found = None;
    for _ in 0..400 {
        found = detached();
        if found.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    trial::kill_tree(pid);
    driver.wait().unwrap();

    let found = found.expect("the detached sleep is in the driver's tree");
    let gone = |pid: u32| {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_or(true, |stat| stat.contains(") Z "))
    };
    for _ in 0..400 {
        if gone(found) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(gone(found), "the tree's kill reached {found}");
}
