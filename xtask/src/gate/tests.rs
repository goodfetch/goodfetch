//! The incremental gate's choice, as named cases: an earlier run's record,
//! what the change since each recorded head reaches, and the steps and
//! members this run reruns. The local gate is repository tooling, so the
//! module declares no Feature.

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::plan::{Action, GateRun, Head, Outcome, Planned, Step, Touched, gate_input, plan, unit};
use super::runs;
use crate::affected::{Job, RepoPath};

const BASE: &str = "base";
const EARLIER: &str = "earlier";

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
        units,
    }
}

fn touched_since_earlier(touched: Touched) -> BTreeMap<String, Touched> {
    BTreeMap::from([(EARLIER.to_owned(), touched)])
}

fn planned(record: Option<&GateRun>, touched: &BTreeMap<String, Touched>) -> Vec<Planned> {
    let members = members();
    let head = Head {
        base: BASE,
        members: &members,
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
fn reruns(plan: &[Planned]) -> (Vec<&'static str>, BTreeMap<&'static str, Vec<String>>) {
    let mut full = Vec::new();
    let mut part = BTreeMap::new();
    for planned in plan {
        match &planned.action {
            Action::Run { .. } => full.push(planned.step.name()),
            Action::Part { members, .. } => {
                part.insert(planned.step.name(), members.iter().cloned().collect());
            }
            Action::Reuse { .. } => {}
        }
    }
    (full, part)
}

/// The checks every run reruns, whatever the change.
const ALWAYS: [&str; 11] = [
    "dco",
    "commit-messages",
    "lint",
    "feature-combinations",
    "audit",
    "commit-leaks",
    "fuzz-check",
    "proofs",
    "journeys-tasks",
    "features-tests",
    "notices",
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
    assert_eq!(full, ALWAYS);
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
    };

    let plan = planned(Some(&record), &touched_since_earlier(touched));

    let (full, part) = reruns(&plan);
    let mut expected: Vec<&str> = ALWAYS.to_vec();
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

    let mut expected: Vec<&str> = ALWAYS.to_vec();
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
    let text =
        "base\tb4se\nperf\tpass\tc0ffee\tt1\nquick:xtask\tpass\tc0ffee\tt1\nperf\tfail\tfeed\tt2\n";

    let record = runs::parse("feed", text).unwrap();

    assert_eq!(record.base, "b4se");
    assert_eq!(
        record.units.get("perf"),
        Some(&Outcome {
            green: false,
            origin: "feed".to_owned(),
            tree: "t2".to_owned()
        })
    );
    assert_eq!(runs::parse("feed", &runs::render(&record)).unwrap(), record);
}
