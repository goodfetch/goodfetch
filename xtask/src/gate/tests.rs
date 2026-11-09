//! The incremental gate's choice, as named cases: an earlier run's record,
//! what the change since each recorded head reaches, and the steps and
//! members this run reruns. The local gate is repository tooling, so the
//! module declares no Feature.

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::plan::{
    Action, GateRun, Head, MAX_AGE, Outcome, Planned, Step, Touched, UNOWNED, gate_input, plan,
    unit,
};
use super::steps::{Entry, Form, Kind, entries};
use super::{plan_value, render, runs};
use crate::affected::{Job, RepoPath};

const BASE: &str = "base";
const EARLIER: &str = "earlier";
const HOST: &str = "host";
/// Now, in seconds since the Unix epoch, for every case.
const NOW: u64 = 1_800_000_000;

fn members() -> BTreeSet<String> {
    [
        "goodfetch-engine",
        "xtask",
        "test-support/typescript",
        "sdks/python/collie-client",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn green(origin: &str) -> Outcome {
    Outcome {
        green: true,
        origin: origin.to_owned(),
        tree: format!("{origin}-tree"),
        at: NOW - 60,
    }
}

/// A full green run on `EARLIER`: every unit of every step.
fn full_green_run() -> GateRun {
    let mut units = BTreeMap::new();
    for step in Step::ALL {
        if step.per_member() {
            for member in members() {
                units.insert(unit(step, Some(&member)), green(EARLIER));
            }
        } else {
            units.insert(unit(step, None), green(EARLIER));
        }
    }
    GateRun {
        head: EARLIER.to_owned(),
        base: BASE.to_owned(),
        host: HOST.to_owned(),
        units,
    }
}

fn touched_since_earlier(touched: Touched) -> BTreeMap<String, Touched> {
    BTreeMap::from([(EARLIER.to_owned(), touched)])
}

fn planned(record: Option<&GateRun>, touched: &BTreeMap<String, Touched>) -> Vec<Planned> {
    planned_with(&members(), record, touched)
}

fn planned_with(
    members: &BTreeSet<String>,
    record: Option<&GateRun>,
    touched: &BTreeMap<String, Touched>,
) -> Vec<Planned> {
    planned_shuffling(members, &BTreeSet::new(), record, touched)
}

/// The plan for a head whose `shuffled` members' tests run in Vitest,
/// Playwright or pytest, which order them by the head's seed.
fn planned_shuffling(
    members: &BTreeSet<String>,
    shuffled: &BTreeSet<String>,
    record: Option<&GateRun>,
    touched: &BTreeMap<String, Touched>,
) -> Vec<Planned> {
    let head = Head {
        base: BASE,
        members,
        shuffled,
        host: HOST,
        now: NOW,
    };
    plan(&head, record, "no earlier run", touched)
}

fn action(plan: &[Planned], step: Step) -> &Action {
    &plan
        .iter()
        .find(|planned| planned.step == step)
        .expect("every step is planned")
        .action
}

/// The steps the plan reruns in full, and those it reruns over some members.
fn reruns(plan: &[Planned]) -> (BTreeSet<&'static str>, BTreeMap<&'static str, Vec<String>>) {
    let mut full = BTreeSet::new();
    let mut part = BTreeMap::new();
    for planned in plan {
        match &planned.action {
            Action::Run { .. } => {
                full.insert(planned.step.name());
            }
            Action::Part { members, .. } => {
                part.insert(planned.step.name(), members.iter().cloned().collect());
            }
            Action::Reuse { .. } => {}
        }
    }
    (full, part)
}

/// The checks every run reruns, whatever the change.
const ALWAYS: [&str; 12] = [
    "dco",
    "commit-messages",
    "floor-guard",
    "commit-leaks",
    "audit",
    "notices",
    "lint",
    "feature-combinations",
    "fuzz-check",
    "proofs",
    "journeys-tasks",
    "features-tests",
];

#[test]
fn without_an_earlier_run_every_step_runs_in_full() {
    let plan = planned(None, &BTreeMap::new());

    let (full, part) = reruns(&plan);
    assert_eq!(full.len(), Step::ALL.len());
    assert!(part.is_empty());
    assert!(plan.iter().all(|planned| planned.carried.is_empty()));
}

#[test]
fn a_change_that_reaches_nothing_reruns_only_the_global_checks() {
    let record = full_green_run();
    let plan = planned(Some(&record), &touched_since_earlier(Touched::default()));

    let (full, part) = reruns(&plan);
    assert_eq!(full, BTreeSet::from(ALWAYS));
    assert!(part.is_empty());
    assert_eq!(
        action(&plan, Step::Perf),
        &Action::Reuse {
            origins: BTreeSet::from([EARLIER.to_owned()])
        }
    );
}

#[test]
fn a_change_to_one_package_reruns_its_tests_and_the_jobs_it_selects() {
    let record = full_green_run();
    let touched = Touched {
        gate_input: None,
        jobs: BTreeSet::from([Job::PreMerge, Job::Visual]),
        crates: false,
        members: BTreeSet::from(["test-support/typescript".to_owned()]),
        evidence: BTreeMap::new(),
    };

    let plan = planned(Some(&record), &touched_since_earlier(touched));

    let (full, part) = reruns(&plan);
    let mut expected = BTreeSet::from(ALWAYS);
    expected.extend(["linux", "visual"]);
    assert_eq!(full, expected);
    let only = vec!["test-support/typescript".to_owned()];
    assert_eq!(
        part,
        BTreeMap::from([
            ("pre-merge", only.clone()),
            ("quick", only.clone()),
            ("stress", only)
        ])
    );
    // Perf, TLC, the MSRV build and Miri read nothing the change reaches.
    for step in [Step::Perf, Step::Tlc, Step::Msrv, Step::Miri] {
        assert!(
            matches!(action(&plan, step), Action::Reuse { .. }),
            "{step:?}"
        );
    }
    // The members not rerun carry their green results into this record.
    let carried = &plan
        .iter()
        .find(|planned| planned.step == Step::PreMerge)
        .unwrap()
        .carried;
    assert_eq!(carried.len(), members().len() - 1);
    assert_eq!(carried.get("pre-merge:xtask"), Some(&green(EARLIER)));
}

#[test]
fn a_change_reaching_a_crate_reruns_the_crate_builds() {
    let record = full_green_run();
    let touched = Touched {
        crates: true,
        jobs: BTreeSet::from([Job::PreMerge, Job::Perf]),
        members: BTreeSet::from(["goodfetch-engine".to_owned()]),
        ..Touched::default()
    };

    let (full, _) = reruns(&planned(Some(&record), &touched_since_earlier(touched)));

    for step in ["perf", "linux", "msrv", "miri"] {
        assert!(full.contains(&step), "{step} in {full:?}");
    }
    assert!(!full.contains(&"tlc") && !full.contains(&"visual"));
}

// The Kennel's checks rerun only for a change its selection reaches, and
// such a change reruns none of the Products' test steps (ADR 0141).
#[test]
fn a_change_reaching_the_kennel_alone_reruns_its_checks_and_no_products_tests() {
    let record = full_green_run();
    let touched = Touched {
        jobs: BTreeSet::from([Job::Kennel]),
        ..Touched::default()
    };

    let (full, part) = reruns(&planned(Some(&record), &touched_since_earlier(touched)));

    assert!(full.contains(&"kennel"), "{full:?}");
    for step in [
        "perf",
        "linux",
        "visual",
        "tlc",
        "msrv",
        "miri",
        "quick",
        "pre-merge",
        "stress",
    ] {
        assert!(!full.contains(&step), "{step} in {full:?}");
    }
    assert_eq!(part, BTreeMap::new());
}

#[test]
fn a_change_the_kennel_does_not_read_reuses_its_checks() {
    let record = full_green_run();
    let touched = Touched {
        crates: true,
        jobs: BTreeSet::from([Job::PreMerge]),
        members: BTreeSet::from(["goodfetch-engine".to_owned()]),
        ..Touched::default()
    };

    let plan = planned(Some(&record), &touched_since_earlier(touched));

    assert!(matches!(action(&plan, Step::Kennel), Action::Reuse { .. }));
}

#[rstest]
#[case::justfile("justfile")]
#[case::nextest_settings(".config/nextest.toml")]
#[case::ci_script("ci/pre-merge.sh")]
#[case::tool_versions("ci/tool-versions.sh")]
#[case::cargo_lockfile("Cargo.lock")]
#[case::build_profiles("Cargo.toml")]
#[case::pnpm_lockfile("pnpm-lock.yaml")]
#[case::uv_lockfile("uv.lock")]
#[case::toolchain("rust-toolchain.toml")]
#[case::xtask("xtask/src/stress.rs")]
// What tests read at run time across members, which no evidence shows
// (ADR 0135, N2): the Proving ground's data, shared fixtures, browser pins.
#[case::proving_ground_pages("proving-ground/pages/login.html")]
#[case::proving_ground_fixtures("proving-ground/fixtures/feed.xml")]
#[case::shared_fixtures("test-support/fixtures/page.html")]
#[case::chromium_pins("chromium/README.md")]
#[case::browser_pins("crates/goodfetch-collie/src/pins.json")]
fn a_change_to_a_gate_input_reruns_every_step_in_full(#[case] path: &str) {
    let paths = [RepoPath::new("docs/a.md"), RepoPath::new(path)];
    let touched = Touched {
        gate_input: gate_input(&paths).map(|input| input.as_str().to_owned()),
        ..Touched::default()
    };
    let record = full_green_run();

    let plan = planned(Some(&record), &touched_since_earlier(touched));

    let (full, part) = reruns(&plan);
    assert_eq!(full.len(), Step::ALL.len());
    assert!(part.is_empty());
    let Action::Run { why } = action(&plan, Step::Perf) else {
        panic!("perf is not rerun")
    };
    assert!(why.contains(path), "{why}");
}

#[rstest]
#[case::docs("docs/adr/0135.md")]
#[case::package_source("test-support/typescript/src/errors.ts")]
#[case::crate_source("crates/goodfetch-engine/src/lib.rs")]
fn a_path_outside_the_gate_inputs_is_none(#[case] path: &str) {
    assert_eq!(gate_input(&[RepoPath::new(path)]), None);
}

#[test]
fn a_red_result_is_never_reused() {
    let mut record = full_green_run();
    record.units.insert(
        unit(Step::Visual, None),
        Outcome {
            green: false,
            ..green(EARLIER)
        },
    );
    record.units.insert(
        unit(Step::PreMerge, Some("xtask")),
        Outcome {
            green: false,
            ..green(EARLIER)
        },
    );

    let plan = planned(Some(&record), &touched_since_earlier(Touched::default()));

    assert!(matches!(action(&plan, Step::Visual), Action::Run { .. }));
    assert_eq!(
        action(&plan, Step::PreMerge),
        &Action::Part {
            members: BTreeSet::from(["xtask".to_owned()]),
            why: "red on earlier".to_owned()
        }
    );
}

#[test]
fn a_unit_with_no_result_runs() {
    let mut record = full_green_run();
    record.units.remove(&unit(Step::Tlc, None));
    record
        .units
        .remove(&unit(Step::Quick, Some("goodfetch-engine")));

    let plan = planned(Some(&record), &touched_since_earlier(Touched::default()));

    assert!(matches!(action(&plan, Step::Tlc), Action::Run { .. }));
    assert!(matches!(
        action(&plan, Step::Quick),
        Action::Part { members, .. } if members == &BTreeSet::from(["goodfetch-engine".to_owned()])
    ));
}

#[test]
fn a_moved_base_reruns_the_steps_that_compare_with_it() {
    let mut record = full_green_run();
    record.base = "older-base".to_owned();

    let (full, _) = reruns(&planned(
        Some(&record),
        &touched_since_earlier(Touched::default()),
    ));

    let mut expected = BTreeSet::from(ALWAYS);
    expected.extend(["perf", "stress"]);
    assert_eq!(full, expected);
}

/// A result carried by the earlier run is judged against the change since
/// the head it ran on, not since the earlier run's head.
#[test]
fn a_carried_result_is_judged_since_the_head_it_ran_on() {
    let mut record = full_green_run();
    record.units.insert(unit(Step::Perf, None), green("first"));
    let touched = BTreeMap::from([
        (EARLIER.to_owned(), Touched::default()),
        (
            "first".to_owned(),
            Touched {
                jobs: BTreeSet::from([Job::Perf]),
                ..Touched::default()
            },
        ),
    ]);

    let plan = planned(Some(&record), &touched);

    assert!(matches!(action(&plan, Step::Perf), Action::Run { why } if why.contains("first")));
}

#[test]
fn every_member_rerun_runs_the_step_in_full() {
    let record = full_green_run();
    let touched = Touched {
        members: members(),
        ..Touched::default()
    };

    let plan = planned(Some(&record), &touched_since_earlier(touched));

    assert!(matches!(action(&plan, Step::PreMerge), Action::Run { .. }));
}

#[test]
fn a_record_reads_back_as_written_and_its_last_line_for_a_unit_wins() {
    let text = "base\tb4se\nhost\th0st\nperf\tpass\tc0ffee\tt1\t10\nquick:xtask\tpass\tc0ffee\tt1\t10\nperf\tfail\tfeed\tt2\t20\n";

    let record = runs::parse("feed", text).unwrap();

    assert_eq!(
        (record.base.as_str(), record.host.as_str()),
        ("b4se", "h0st")
    );
    assert_eq!(
        record.units.get("perf"),
        Some(&Outcome {
            green: false,
            origin: "feed".to_owned(),
            tree: "t2".to_owned(),
            at: 20,
        })
    );
    assert_eq!(runs::parse("feed", &runs::render(&record)).unwrap(), record);
}

#[test]
fn a_changed_host_reruns_every_step() {
    let mut record = full_green_run();
    record.host = "another host".to_owned();

    let (full, part) = reruns(&planned(
        Some(&record),
        &touched_since_earlier(Touched::default()),
    ));

    assert_eq!(full.len(), Step::ALL.len());
    assert!(part.is_empty());
}

#[test]
fn a_result_older_than_a_day_reruns() {
    let mut record = full_green_run();
    record.units.insert(
        unit(Step::Perf, None),
        Outcome {
            at: NOW - MAX_AGE - 1,
            ..green(EARLIER)
        },
    );

    let plan = planned(Some(&record), &touched_since_earlier(Touched::default()));

    assert!(
        matches!(action(&plan, Step::Perf), Action::Run { why } if why.contains("older than a day")),
        "{:?}",
        action(&plan, Step::Perf)
    );
    assert!(matches!(action(&plan, Step::Tlc), Action::Reuse { .. }));
}

/// The tests no member owns rerun with any member, and reuse with all.
#[test]
fn the_tests_no_member_owns_rerun_whenever_a_member_does() {
    let mut members = members();
    members.insert(UNOWNED.to_owned());
    let mut record = full_green_run();
    for step in [Step::Quick, Step::PreMerge, Step::Stress] {
        record
            .units
            .insert(unit(step, Some(UNOWNED)), green(EARLIER));
    }
    let reaching_one = Touched {
        members: BTreeSet::from(["test-support/typescript".to_owned()]),
        ..Touched::default()
    };

    let reached = planned_with(
        &members,
        Some(&record),
        &touched_since_earlier(reaching_one),
    );
    let untouched = planned_with(
        &members,
        Some(&record),
        &touched_since_earlier(Touched::default()),
    );

    assert!(matches!(
        action(&reached, Step::PreMerge),
        Action::Part { members, .. }
            if members == &BTreeSet::from([UNOWNED.to_owned(), "test-support/typescript".to_owned()])
    ));
    assert!(matches!(
        action(&untouched, Step::PreMerge),
        Action::Reuse { .. }
    ));
}

/// The evidence of what a member read reaches it where the selection does
/// not, and the plan names that evidence.
#[test]
fn evidence_reaches_a_member_the_selection_misses() {
    let record = full_green_run();
    let touched = Touched {
        evidence: BTreeMap::from([("xtask".to_owned(), "its build read TESTING.md".to_owned())]),
        ..Touched::default()
    };

    let plan = planned(Some(&record), &touched_since_earlier(touched));

    assert!(
        matches!(
            action(&plan, Step::PreMerge),
            Action::Part { members, why }
                if members == &BTreeSet::from(["xtask".to_owned()]) && why.contains("its build read TESTING.md")
        ),
        "{:?}",
        action(&plan, Step::PreMerge)
    );
}

/// Vitest, Playwright and pytest order their tests by the head's seed, so
/// every member they test reruns in the head's order, reused or not, and a
/// verdict equals a full run's (the owner's F8, option B); nextest runs each
/// test in its own process, so a crate's result is still reused.
#[test]
fn the_shuffling_runners_rerun_every_member_in_the_heads_order() {
    let record = full_green_run();
    let shuffled = BTreeSet::from([
        "sdks/python/collie-client".to_owned(),
        "test-support/typescript".to_owned(),
    ]);

    let plan = planned_shuffling(
        &members(),
        &shuffled,
        Some(&record),
        &touched_since_earlier(Touched::default()),
    );

    for step in [Step::Quick, Step::PreMerge, Step::Stress] {
        assert!(
            matches!(
                action(&plan, step),
                Action::Part { members, why } if members == &shuffled && why.contains("the head's test order")
            ),
            "{step:?}: {:?}",
            action(&plan, step)
        );
    }
    assert!(matches!(action(&plan, Step::Perf), Action::Reuse { .. }));
}

/// Where `name` stands in `form`'s list.
fn position(form: Form, name: &str) -> usize {
    entries(form)
        .iter()
        .position(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("{name} is not in the list"))
}

/// The full gate's steps, in order. Each form's list is spelled here, never
/// read from the predicates that choose it: a step dropped from a form takes
/// a check off a gate that merges on its verdict, so it has to fail a test.
const FULL_STEPS: [&str; 22] = [
    "dco",
    "commit-messages",
    "floor-guard",
    "commit-leaks",
    "audit",
    "notices",
    "perf",
    "linux",
    "visual",
    "lint",
    "feature-combinations",
    "fuzz-check",
    "proofs",
    "quick",
    "pre-merge",
    "stress",
    "journeys-tasks",
    "features-tests",
    "tlc",
    "kennel",
    "msrv",
    "miri",
];

/// The steps that judge the whole change in seconds.
const WHOLE_CHANGE_STEPS: [&str; 6] = [
    "dco",
    "commit-messages",
    "floor-guard",
    "commit-leaks",
    "audit",
    "notices",
];

/// The steps that run beside the host's, in a container of their own.
const BESIDE_STEPS: [&str; 3] = ["perf", "linux", "visual"];

/// The quick form's steps, in order: those that judge every change, never a
/// test of the Pre-merge suite or a step beside the host's.
const QUICK_STEPS: [&str; 9] = [
    "dco",
    "commit-messages",
    "floor-guard",
    "commit-leaks",
    "audit",
    "notices",
    "lint",
    "quick",
    "journeys-tasks",
];

/// The docs form's entries, in order: the checks that read the change's
/// commits and the bar, then the checks of `just lint` that read documents.
const DOCS_ENTRIES: [&str; 5] = [
    "dco",
    "commit-messages",
    "floor-guard",
    "commit-leaks",
    "lint-documents",
];

#[test]
fn the_full_gate_runs_every_step_once_in_the_order_the_plan_lists_them() {
    let names: Vec<&str> = entries(Form::All)
        .into_iter()
        .filter(|entry| entry.kind != Kind::Command)
        .map(|entry| entry.name)
        .collect();

    assert_eq!(names, FULL_STEPS);
}

#[rstest]
#[case::all(Form::All)]
#[case::changed(Form::Changed)]
fn the_whole_changes_checks_end_the_full_gate_before_the_console_build_and_any_step_beside(
    #[case] form: Form,
) {
    let console = position(form, "console-build");

    for step in WHOLE_CHANGE_STEPS {
        assert!(position(form, step) < console, "{step}");
    }
    for step in BESIDE_STEPS {
        assert!(position(form, step) > console, "{step}");
    }
}

#[test]
fn the_quick_gate_runs_no_step_beside_and_no_suite_but_the_quick_one() {
    let list = entries(Form::Quick);

    assert!(list.iter().all(|entry| entry.kind == Kind::Step));
    assert_eq!(
        list.iter().map(|entry| entry.name).collect::<Vec<_>>(),
        QUICK_STEPS
    );
}

#[test]
fn the_docs_gate_runs_the_checks_of_the_commits_then_the_document_checks() {
    let list = entries(Form::Docs);

    let (documents, _) = list.split_last().expect("the docs gate runs something");
    assert_eq!(documents.just, ["lint-documents"]);
    assert_eq!(
        list.iter().map(|entry| entry.name).collect::<Vec<_>>(),
        DOCS_ENTRIES
    );
}

/// A step over the whole workspace takes `all`, one over what changed
/// nothing; a step that is one recipe runs it by its name.
#[rstest]
#[case::whole(Form::All, "quick", &["_suite", "quick", "all"])]
#[case::what_changed(Form::Changed, "quick", &["_suite", "quick"])]
#[case::the_quick_gate(Form::Quick, "quick", &["_suite", "quick", "all"])]
#[case::two_recipes(Form::All, "features-tests", &["features-tests", "clients-tests"])]
#[case::its_name(Form::All, "miri", &["miri"])]
fn each_step_runs_its_just_command(#[case] form: Form, #[case] name: &str, #[case] just: &[&str]) {
    let list = entries(form);
    let entry: &Entry = list
        .iter()
        .find(|entry| entry.name == name)
        .expect("the form runs the step");

    assert_eq!(entry.just, just);
}

#[test]
fn a_plan_reads_back_each_steps_action_members_and_reason() {
    let record = full_green_run();
    let touched = Touched {
        members: BTreeSet::from(["xtask".to_owned()]),
        ..Touched::default()
    };
    let plan = planned(Some(&record), &touched_since_earlier(touched));

    let text = render(Some(EARLIER), "head", &members(), &[], &plan);

    assert_eq!(plan_value(&text, "previous").unwrap(), EARLIER);
    assert_eq!(plan_value(&text, "head").unwrap(), "head");
    assert_eq!(plan_value(&text, "unowned").unwrap(), "-");
    assert_eq!(plan_value(&text, "quick.action").unwrap(), "part");
    assert_eq!(plan_value(&text, "quick.members").unwrap(), "xtask");
    assert_eq!(plan_value(&text, "lint.action").unwrap(), "run");
    assert_eq!(
        plan_value(&text, "lint.why").unwrap(),
        "every run reruns it"
    );
    assert_eq!(plan_value(&text, "perf.action").unwrap(), "reuse");
}

#[rstest]
#[case::missing("previous=none\n", "head")]
#[case::twice("head=a\nhead=b\n", "head")]
fn a_plan_without_its_key_or_with_it_twice_is_an_error(#[case] text: &str, #[case] key: &str) {
    assert!(plan_value(text, key).is_err());
}

#[test]
fn a_reason_over_several_lines_stays_on_its_key_line() {
    let plan = [Planned {
        step: Step::Lint,
        action: Action::Run {
            why: "git failed:\nfatal: bad object".to_owned(),
        },
        carried: BTreeMap::new(),
    }];

    let text = render(None, "head", &BTreeSet::new(), &[], &plan);

    assert_eq!(
        plan_value(&text, "lint.why").unwrap(),
        "git failed: fatal: bad object"
    );
}
