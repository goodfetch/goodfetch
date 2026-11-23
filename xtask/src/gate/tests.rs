//! The incremental gate's choice, as named cases: an earlier run's record,
//! what the change since each recorded head reaches, and the steps and
//! members this run reruns. The local gate is repository tooling, so the
//! module declares no Feature.

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::host::{REMOVED_BY_THE_GATE, step_variables};
use super::plan::{
    Action, Earlier, EarnedPasses, GateRun, Head, Layer, LintReads, MAX_AGE, Outcome, Pin, Planned,
    Touched, UNOWNED, gate_input, hosted_fingerprint, narrowed, plan, unit,
};
use super::steps::{self, Form, Kind, Owner, Step, entries};
use super::{plan_value, render, runs};
use crate::affected::{Job, RepoPath};

const BASE: &str = "base";
const EARLIER: &str = "earlier";
const HOST: &str = "host";
/// The tree of the head every case plans for, but those on `EARLIER`'s.
const HEAD_TREE: &str = "head-tree";
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
        by: Layer::Merge,
        earned_on: None,
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
        earned: BTreeMap::new(),
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
    let lint_holds = || Ok(());
    let head = Head {
        base: BASE,
        members,
        shuffled,
        tree: HEAD_TREE,
        host: HOST,
        now: NOW,
        lint: LintReads::new(&lint_holds),
        form: Form::All,
        layer: Layer::Merge,
        pin: PIN,
    };
    plan(&head, &earlier(record, &[]), touched)
}

/// The CI image and repository every case's head pins.
const PIN: Pin<'static> = Pin {
    image: IMAGE,
    repository: REPOSITORY,
};
const IMAGE: &str = "image";
const REPOSITORY: &str = "goodfetch/goodfetch";

/// The earlier runs of a case: the local `record`, and the `hosted` runs.
fn earlier<'a>(record: Option<&'a GateRun>, hosted: &'a [GateRun]) -> Earlier<'a> {
    Earlier {
        previous: record,
        no_previous: "no earlier run",
        hosted,
    }
}

/// The plan for a head on `EARLIER`'s tree, where what lint reads besides
/// the tree and the host reads as `lint_holds` now.
fn planned_on_earliers_tree(record: &GateRun, lint_holds: Result<(), &str>) -> Vec<Planned> {
    let members = members();
    let tree = green(EARLIER).tree;
    let lint_holds = || lint_holds.map_err(str::to_owned);
    let head = Head {
        base: BASE,
        members: &members,
        shuffled: &BTreeSet::new(),
        tree: &tree,
        host: HOST,
        now: NOW,
        lint: LintReads::new(&lint_holds),
        form: Form::All,
        layer: Layer::Merge,
        pin: PIN,
    };
    plan(
        &head,
        &earlier(Some(record), &[]),
        &touched_since_earlier(Touched::default()),
    )
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
            Action::Reuse { .. }
            | Action::Covered { .. }
            | Action::Left { .. }
            | Action::Red { .. } => {}
        }
    }
    (full, part)
}

/// The checks every run on another tree reruns, whatever the change.
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
fn a_variable_only_the_launcher_reads_is_not_one_of_the_steps() {
    let launched = "CARGO_INCREMENTAL=0\nGOODFETCH_GATE_MIN_FREE_GB=60\nGOODFETCH_GATE_RECORDS=/records\nGOODFETCH_GATE_LAYER=backstop\nGOODFETCH_LAZYFS=/lazyfs\nGOODFETCH_QUICK_BASE=origin/spec\nGOODFETCH_STEPS_DIR=/steps\nGOODFETCH_STEP=lint";

    assert_eq!(
        step_variables(launched),
        "CARGO_INCREMENTAL=0\nGOODFETCH_LAZYFS=/lazyfs"
    );
}

#[test]
fn a_variable_whose_name_only_starts_as_the_launchers_stays() {
    let launched = "GOODFETCH_GATE_MIN_FREE_GB_PER_STEP=2\nNEXTEST_DOUBLE_SPAWN=0";

    assert_eq!(step_variables(launched), launched);
}

/// The fingerprint may leave a variable out as removed only while
/// `ci/pre-merge.sh` does remove it before its steps: one the script kept
/// would reach them, and a result under another value of it would be reused.
/// Read from the repository's own script, whose loop names those it removes.
#[test]
fn every_variable_the_fingerprint_leaves_out_as_removed_is_one_the_gate_removes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits in the repository");
    let script = std::fs::read_to_string(root.join("ci/pre-merge.sh")).expect("the repository's");
    let removed: Vec<&str> = script
        .lines()
        .filter_map(|line| line.trim().strip_prefix("for variable in "))
        .filter_map(|rest| rest.strip_suffix("; do"))
        .filter(|names| !names.contains("GOODFETCH_"))
        .flat_map(str::split_whitespace)
        .collect();

    for variable in REMOVED_BY_THE_GATE {
        assert!(removed.contains(&variable), "{variable}: {removed:?}");
    }
}

/// The Pre-merge suite's selection of a member holds every Quick test of
/// it (ADR 0135's amendment of 2026-11-22), so no plan runs, reruns or
/// reuses the Quick suite, nor carries or earns a unit of it: not a first
/// run, not one after its own units went red, not one a gate input reruns
/// in full.
#[test]
fn no_plan_runs_the_quick_suite_whose_tests_the_pre_merge_suite_holds() {
    let mut red_quick = full_green_run();
    for member in members() {
        let outcome = Outcome {
            green: false,
            ..green(EARLIER)
        };
        red_quick
            .units
            .insert(unit(Step::Quick, Some(&member)), outcome);
    }
    let every_input = Touched {
        gate_input: Some("justfile".to_owned()),
        ..Touched::default()
    };

    for plan in [
        planned(None, &BTreeMap::new()),
        planned(Some(&red_quick), &touched_since_earlier(Touched::default())),
        planned(Some(&red_quick), &touched_since_earlier(every_input)),
    ] {
        let quick = plan
            .iter()
            .find(|planned| planned.step == Step::Quick)
            .expect("every step is planned");
        assert_eq!(quick.action, Action::Covered { by: Step::PreMerge });
        assert!(quick.carried.is_empty() && quick.earned.is_empty());
        assert!(!matches!(
            action(&plan, Step::PreMerge),
            Action::Covered { .. }
        ));
    }
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
        jobs: BTreeSet::from([Job::PreMerge, Job::Visual]),
        members: BTreeSet::from(["test-support/typescript".to_owned()]),
        ..Touched::default()
    };

    let plan = planned(Some(&record), &touched_since_earlier(touched));

    let (full, part) = reruns(&plan);
    let mut expected = BTreeSet::from(ALWAYS);
    expected.extend(["linux", "visual"]);
    assert_eq!(full, expected);
    let only = vec!["test-support/typescript".to_owned()];
    assert_eq!(
        part,
        BTreeMap::from([("pre-merge", only.clone()), ("stress", only)])
    );
    // Perf, the counted rows, TLC, the MSRV build, Miri, loom and the
    // Windows build read nothing the change reaches.
    for step in [
        Step::Perf,
        Step::Counts,
        Step::Tlc,
        Step::Msrv,
        Step::Miri,
        Step::Loom,
        Step::Windows,
    ] {
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

    for step in ["perf", "counts", "linux", "msrv", "miri", "loom", "windows"] {
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
        "loom",
        "windows",
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
#[case::nextest_settings(".config/nextest.toml")]
#[case::tool_versions("ci/tool-versions.sh")]
#[case::cargo_lockfile("Cargo.lock")]
#[case::build_profiles("Cargo.toml")]
#[case::pnpm_lockfile("pnpm-lock.yaml")]
#[case::uv_lockfile("uv.lock")]
#[case::toolchain("rust-toolchain.toml")]
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
    let every_step_but_the_covered: BTreeSet<&str> = Step::ALL
        .into_iter()
        .filter(|step| step.covered_by().is_none())
        .map(Step::name)
        .collect();
    assert_eq!(full, every_step_but_the_covered);
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

/// A `justfile`'s recipes: each name, with what it depends on and its lines
/// without its comments.
fn recipes(justfile: &str) -> BTreeMap<&str, String> {
    let mut recipes: BTreeMap<&str, String> = BTreeMap::new();
    let mut current = None;
    for line in justfile.lines() {
        if line.trim_start().starts_with('#') || line.starts_with('[') || line.is_empty() {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            if let Some(name) = current {
                let lines = recipes.entry(name).or_default();
                lines.push_str(line);
                lines.push('\n');
            }
            continue;
        }
        current = None;
        if line.contains(":=") {
            continue;
        }
        if let Some((head, dependencies)) = line.rsplit_once(':') {
            let name = head.split_whitespace().next().unwrap_or(head);
            let name = name.trim_start_matches('@');
            current = Some(name);
            let mut calls = String::new();
            for dependency in dependencies.split_whitespace() {
                calls.push_str("just ");
                calls.push_str(dependency);
                calls.push('\n');
            }
            recipes.insert(name, calls);
        }
    }
    recipes
}

/// The `ci/` paths `text` names.
fn ci_paths(text: &str) -> BTreeSet<String> {
    text.match_indices("ci/")
        .map(|(start, _)| {
            text[start..]
                .chars()
                .take_while(|character| {
                    character.is_alphanumeric() || ['_', '.', '/', '-'].contains(character)
                })
                .collect::<String>()
        })
        .collect()
}

/// The `ci/` paths running the `recipes` named `from` reads: those named in
/// their lines, in those of the recipes they call, and in the scripts these
/// name, comments aside.
fn ci_paths_run(
    root: &std::path::Path,
    recipes: &BTreeMap<&str, String>,
    from: &[&str],
) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut todo: Vec<&str> = from.to_vec();
    let mut paths = BTreeSet::new();
    while let Some(name) = todo.pop() {
        if !seen.insert(name) {
            continue;
        }
        let Some(body) = recipes.get(name) else {
            continue;
        };
        paths.extend(ci_paths(body));
        let words: Vec<&str> = body.split_whitespace().collect();
        for pair in words.windows(2) {
            if pair[0].ends_with("}}") || pair[0].contains("just") {
                todo.push(pair[1]);
            }
        }
    }
    let mut unread: Vec<String> = paths.iter().cloned().collect();
    while let Some(script) = unread.pop() {
        let Ok(text) = std::fs::read_to_string(root.join(&script)) else {
            continue;
        };
        let code: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .collect();
        for path in ci_paths(&code.join("\n")) {
            if paths.insert(path.clone()) {
                unread.push(path);
            }
        }
    }
    paths
}

/// The map says which steps read each path it names, so a step that starts
/// to run one must say so here: its next change would otherwise reuse that
/// step's result. Read from the repository's own justfile and scripts.
#[test]
fn every_step_a_path_the_map_names_reaches_is_in_the_map() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits in the repository");
    let justfile = std::fs::read_to_string(root.join("justfile")).expect("the repository's");
    let recipes = recipes(&justfile);
    let mut wrong = Vec::new();
    for path in ci_paths_run(
        root,
        &recipes,
        &["pre-merge", "_gate-plan", "_gate-record", "_os-services"],
    ) {
        if narrowed(&RepoPath::new(path.as_str())).is_some() {
            wrong.push(format!("every step reads {path}"));
        }
    }
    for listed in entries(Form::All) {
        let Some(step) = Step::from_name(listed.name) else {
            continue;
        };
        if ALWAYS.contains(&step.name()) {
            continue;
        }
        for path in ci_paths_run(root, &recipes, &[listed.just[0]]) {
            if narrowed(&RepoPath::new(path.as_str()))
                .is_some_and(|entry| !entry.steps.contains(&step))
            {
                wrong.push(format!("{} reads {path}", step.name()));
            }
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

const SHORT: &str = "goodfetch-engine";

/// A full green run on `EARLIER`, but its stress run of `SHORT`: red, with 7
/// clean passes of one test.
fn run_short_of_its_passes() -> GateRun {
    let mut record = full_green_run();
    record.units.insert(
        unit(Step::Stress, Some(SHORT)),
        Outcome {
            green: false,
            ..green(EARLIER)
        },
    );
    record.earned.insert(
        SHORT.to_owned(),
        EarnedPasses {
            origin: EARLIER.to_owned(),
            at: NOW - 60,
            passes: BTreeMap::from([("goodfetch-engine a::test".to_owned(), 7)]),
        },
    );
    record
}

fn stress_earned(plan: &[Planned]) -> &BTreeMap<String, EarnedPasses> {
    &plan
        .iter()
        .find(|planned| planned.step == Step::Stress)
        .expect("every step is planned")
        .earned
}

/// A member short of its passes reruns, keeping them while nothing that
/// bears on it changed, and it is never reused on them alone.
#[test]
fn a_member_short_of_its_passes_reruns_with_them() {
    let record = run_short_of_its_passes();

    let plan = planned(Some(&record), &touched_since_earlier(Touched::default()));

    assert!(matches!(
        action(&plan, Step::Stress),
        Action::Part { members, why }
            if members == &BTreeSet::from([SHORT.to_owned()]) && why.contains("red on earlier")
    ));
    assert_eq!(stress_earned(&plan).get(SHORT), record.earned.get(SHORT));
    // Only the stress run counts passes.
    for planned in plan.iter().filter(|planned| planned.step != Step::Stress) {
        assert!(planned.earned.is_empty(), "{:?}", planned.step);
    }
}

/// Every member short of its passes is still a part, whose selection takes
/// the earned passes: a run in full takes none.
#[test]
fn every_member_rerun_with_earned_passes_for_one_is_a_part() {
    let record = run_short_of_its_passes();
    let touched = Touched {
        members: members()
            .into_iter()
            .filter(|member| member != SHORT)
            .collect(),
        ..Touched::default()
    };

    let plan = planned(Some(&record), &touched_since_earlier(touched));

    assert!(matches!(action(&plan, Step::Stress), Action::Part { .. }));
    assert_eq!(stress_earned(&plan).len(), 1);
}

/// The passes count only with the member's inputs identical, on the same
/// host, base and order, within the age limit: each case takes them away.
#[rstest]
#[case::the_change_reaches_it(|_: &mut GateRun, touched: &mut Touched| {
    touched.members.insert(SHORT.to_owned());
})]
#[case::the_evidence_reaches_it(|_: &mut GateRun, touched: &mut Touched| {
    touched.evidence.insert(SHORT.to_owned(), "its build read a file".to_owned());
})]
#[case::a_gate_input_changed(|_: &mut GateRun, touched: &mut Touched| {
    touched.gate_input = Some("justfile".to_owned());
})]
#[case::the_host_changed(|record: &mut GateRun, _: &mut Touched| {
    record.host = "another host".to_owned();
})]
#[case::the_base_moved(|record: &mut GateRun, _: &mut Touched| {
    record.base = "older-base".to_owned();
})]
#[case::they_are_older_than_the_limit(|record: &mut GateRun, _: &mut Touched| {
    record.earned.get_mut(SHORT).unwrap().at = NOW - MAX_AGE - 1;
})]
fn passes_count_only_while_nothing_that_bears_on_them_changed(
    #[case] change: fn(&mut GateRun, &mut Touched),
) {
    let mut record = run_short_of_its_passes();
    let mut touched = Touched::default();
    change(&mut record, &mut touched);

    let plan = planned(Some(&record), &touched_since_earlier(touched));

    assert_eq!(stress_earned(&plan), &BTreeMap::new());
    assert!(!matches!(action(&plan, Step::Stress), Action::Reuse { .. }));
}

/// Passes carried from an earlier head are judged by the change since that
/// head, not since the run that carried them.
#[test]
fn carried_passes_are_judged_since_the_head_they_began_on() {
    let mut record = run_short_of_its_passes();
    record.earned.get_mut(SHORT).unwrap().origin = "first".to_owned();
    let touched = BTreeMap::from([
        (EARLIER.to_owned(), Touched::default()),
        (
            "first".to_owned(),
            Touched {
                members: BTreeSet::from([SHORT.to_owned()]),
                ..Touched::default()
            },
        ),
    ]);

    let plan = planned(Some(&record), &touched);

    assert_eq!(stress_earned(&plan), &BTreeMap::new());
}

/// Vitest, Playwright and pytest order their tests by the head's seed, so
/// passes in another head's order are no passes in this one's (the owner's F8,
/// option B): such a member keeps none, whatever a record says.
#[test]
fn a_member_a_shuffling_runner_tests_keeps_no_passes() {
    let record = run_short_of_its_passes();
    let shuffled = BTreeSet::from([SHORT.to_owned()]);

    let plan = planned_shuffling(
        &members(),
        &shuffled,
        Some(&record),
        &touched_since_earlier(Touched::default()),
    );

    assert_eq!(stress_earned(&plan), &BTreeMap::new());
}

/// A green stress result that counted earlier passes reads back with the head
/// they began on; a record without that field is one that counted none.
#[test]
fn a_result_that_counted_earlier_passes_reads_back_with_their_head() {
    let text = "base\tb\nhost\th\nstress:a\tpass\tfeed\tt\t10\tmerge\tc0ffee\nstress:b\tpass\tfeed\tt\t10\tmerge\n";

    let record = runs::parse("feed", text).unwrap();

    assert_eq!(
        record.units["stress:a"].earned_on.as_deref(),
        Some("c0ffee")
    );
    assert_eq!(record.units["stress:b"].earned_on, None);
    assert_eq!(runs::parse("feed", &runs::render(&record)).unwrap(), record);
}

#[test]
fn earned_passes_read_back_as_written() {
    let earned = BTreeMap::from([(
        "goodfetch-engine".to_owned(),
        EarnedPasses {
            origin: "c0ffee".to_owned(),
            at: 10,
            passes: BTreeMap::from([
                ("goodfetch-engine a::one".to_owned(), 7),
                ("goodfetch-engine a::two".to_owned(), 6),
            ]),
        },
    )]);

    let read = runs::parse_earned(&runs::render_earned(&earned)).unwrap();

    assert_eq!(read, earned);
}

/// A file that could mix two members' lines or a count it cannot read says
/// nothing that can be counted on.
#[rstest]
#[case::too_few_fields("a\tc0ffee\t10\n")]
#[case::not_a_count("a\tc0ffee\t10\tmany\ta test\n")]
#[case::not_a_time("a\tc0ffee\tsoon\t7\ta test\n")]
#[case::two_origins_for_a_member("a\tc0ffee\t10\t7\ta one\na\tfeed\t10\t7\ta two\n")]
#[case::two_times_for_a_member("a\tc0ffee\t10\t7\ta one\na\tc0ffee\t11\t7\ta two\n")]
fn an_earned_file_that_cannot_be_read_is_an_error(#[case] text: &str) {
    assert!(runs::parse_earned(text).is_err());
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
        .remove(&unit(Step::PreMerge, Some("goodfetch-engine")));

    let plan = planned(Some(&record), &touched_since_earlier(Touched::default()));

    assert!(matches!(action(&plan, Step::Tlc), Action::Run { .. }));
    assert!(matches!(
        action(&plan, Step::PreMerge),
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
    let text = "base\tb4se\nhost\th0st\nperf\tpass\tc0ffee\tt1\t10\tmerge\nquick:xtask\tpass\tc0ffee\tt1\t10\tmerge\nperf\tfail\tfeed\tt2\t20\tpreflight\n";

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
            by: Layer::Preflight,
            earned_on: None,
        })
    );
    assert_eq!(runs::parse("feed", &runs::render(&record)).unwrap(), record);
}

/// Each result names the layer that ran it (`by`), so one recorded before
/// the field, or by no layer the gate knows, is no record: such records are
/// dropped, not read (the pre-release policy).
#[rstest]
#[case::without_the_layer("base\tb\nhost\th\nperf\tpass\tfeed\tt\t10\n")]
#[case::an_unknown_layer("base\tb\nhost\th\nperf\tpass\tfeed\tt\t10\thosted-someday\n")]
fn a_result_that_names_no_known_layer_is_an_error(#[case] text: &str) {
    assert!(runs::parse("feed", text).is_err());
}

/// The checks a run on the tree of their green result reuses (the codegen
/// tests too, which the full form's Pre-merge suite covers).
const TREE_ONLY: [Step; 5] = [
    Step::Lint,
    Step::FeatureCombinations,
    Step::FuzzCheck,
    Step::Proofs,
    Step::JourneysTasks,
];

#[test]
fn a_run_on_the_same_tree_reuses_the_checks_that_follow_from_the_tree_and_the_host() {
    let plan = planned_on_earliers_tree(&full_green_run(), Ok(()));

    for step in TREE_ONLY {
        assert_eq!(
            action(&plan, step),
            &Action::Reuse {
                origins: BTreeSet::from([EARLIER.to_owned()])
            },
            "{}",
            step.name()
        );
    }
    let (full, part) = reruns(&plan);
    assert_eq!(
        full,
        BTreeSet::from([
            "dco",
            "commit-messages",
            "floor-guard",
            "commit-leaks",
            "audit",
            "notices",
            "features-tests",
        ])
    );
    assert!(part.is_empty());
}

#[test]
fn another_tree_reruns_the_checks_that_follow_from_the_tree_and_the_host() {
    let plan = planned(
        Some(&full_green_run()),
        &touched_since_earlier(Touched::default()),
    );

    for step in TREE_ONLY {
        assert_eq!(
            action(&plan, step),
            &Action::Run {
                why: "the tree changed since earlier".to_owned()
            },
            "{}",
            step.name()
        );
    }
}

/// Lint also reads the day and GitHub (its register and quarantine checks):
/// a result on the same tree holds only while they read as a green lint
/// found them, and the other three checks read neither.
#[test]
fn lint_reruns_on_the_same_tree_when_what_it_reads_besides_the_tree_no_longer_holds() {
    let plan = planned_on_earliers_tree(&full_green_run(), Err("issue #1601 is closed"));

    assert_eq!(
        action(&plan, Step::Lint),
        &Action::Run {
            why: "issue #1601 is closed".to_owned()
        }
    );
    for step in [Step::FeatureCombinations, Step::FuzzCheck, Step::Proofs] {
        assert_eq!(
            action(&plan, step),
            &Action::Reuse {
                origins: BTreeSet::from([EARLIER.to_owned()])
            },
            "{}",
            step.name()
        );
    }
}

#[rstest]
#[case::on_another_host(
    |record: &mut GateRun| record.host = "another host".to_owned(),
    "the host or its tools changed since the run on earlier"
)]
#[case::older_than_a_day(
    |record: &mut GateRun| {
        for step in TREE_ONLY {
            record.units.insert(unit(step, None), Outcome { at: NOW - MAX_AGE - 1, ..green(EARLIER) });
        }
    },
    "its result on earlier is older than a day"
)]
#[case::red(
    |record: &mut GateRun| {
        for step in TREE_ONLY {
            record.units.insert(unit(step, None), Outcome { green: false, ..green(EARLIER) });
        }
    },
    "red on earlier"
)]
fn a_result_on_the_same_tree_reruns(#[case] change: fn(&mut GateRun), #[case] why: &str) {
    let mut record = full_green_run();
    change(&mut record);

    let plan = planned_on_earliers_tree(&record, Ok(()));

    for step in TREE_ONLY {
        assert_eq!(
            action(&plan, step),
            &Action::Run {
                why: why.to_owned()
            },
            "{}",
            step.name()
        );
    }
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
    for step in [Step::PreMerge, Step::Stress] {
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

    for step in [Step::PreMerge, Step::Stress] {
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

/// The steps that judge the whole change in seconds.
const WHOLE_CHANGE_STEPS: [&str; 6] = [
    "dco",
    "commit-messages",
    "floor-guard",
    "commit-leaks",
    "audit",
    "notices",
];

/// The steps that run beside the host's: each builds and writes in folders
/// no host step reads, and its verdict is the one it gives after them.
const BESIDE_STEPS: [&str; 8] = [
    "perf", "counts", "linux", "visual", "tlc", "miri", "loom", "windows",
];

/// Each entry's owning layer, as the spec's table of owning layers sets it
/// (spec #3373): `just merge` judges the change in seconds, the gate host
/// keeps what a noisy runner would move, and the hosted tier owns the rest.
/// A command of a form's list is part of a hosted step: the documents' lint
/// is lint's.
#[rstest]
#[case::all(Form::All)]
#[case::quick(Form::Quick)]
#[case::batch(Form::Batch)]
#[case::docs(Form::Docs)]
#[case::preflight(Form::Preflight)]
fn each_entry_has_the_owner_the_table_of_owning_layers_names(#[case] form: Form) {
    const LOCAL: [&str; 6] = WHOLE_CHANGE_STEPS;
    const GATE_HOST: [&str; 3] = ["perf", "counts", "stress"];
    let owners: Vec<(&str, Owner)> = entries(form)
        .iter()
        .map(|entry| (entry.name, entry.owner))
        .collect();
    let expected: Vec<(&str, Owner)> = owners
        .iter()
        .map(|(name, _)| {
            let owner = if LOCAL.contains(name) {
                Owner::Local
            } else if GATE_HOST.contains(name) {
                Owner::GateHost
            } else {
                Owner::Hosted
            };
            (*name, owner)
        })
        .collect();

    assert_eq!(owners, expected);
    assert!(
        steps::render(form)
            .lines()
            .any(|line| line == format!("{}.owner={}", owners[0].0, owners[0].1.name())),
        "{}",
        steps::render(form)
    );
}

/// `just preflight` runs the gate host's units of the full gate alone, the
/// counted rows and the stress run, through the same plan and record
/// (ADR 0135's amendment of 2026-11-22): the floor guard and the Pre-merge
/// suite are no longer among them.
#[test]
fn the_preflight_form_runs_the_counted_rows_and_the_stress_run_alone() {
    let list = entries(Form::Preflight);
    let names: Vec<(&str, Kind, String)> = list
        .iter()
        .map(|entry| (entry.name, entry.kind, entry.just.join(" ")))
        .collect();

    assert_eq!(
        names,
        [
            ("counts", Kind::Beside, "counted-rows".to_owned()),
            ("stress", Kind::Step, "stress".to_owned()),
        ]
    );
}

#[rstest]
#[case::all(Form::All)]
#[case::changed(Form::Changed)]
fn the_whole_changes_checks_end_the_full_gate_before_any_step_beside(#[case] form: Form) {
    let last_check = WHOLE_CHANGE_STEPS
        .map(|step| position(form, step))
        .into_iter()
        .max();
    let first_beside = BESIDE_STEPS
        .map(|step| position(form, step))
        .into_iter()
        .min();

    assert!(last_check < first_beside, "{last_check:?} {first_beside:?}");
}

/// A step beside the host's that started after lint, the first of the
/// host's steps that takes minutes, would wait behind it: each starts
/// before it, after the checks of the whole change.
#[rstest]
#[case::all(Form::All)]
#[case::changed(Form::Changed)]
fn the_steps_beside_are_these_and_start_before_lint(#[case] form: Form) {
    let list = entries(form);
    let beside: Vec<&str> = list
        .iter()
        .filter(|entry| entry.kind == Kind::Beside)
        .map(|entry| entry.name)
        .collect();
    let last_beside = BESIDE_STEPS
        .map(|step| position(form, step))
        .into_iter()
        .max();

    assert_eq!(beside, BESIDE_STEPS);
    assert!(
        last_beside < Some(position(form, "lint")),
        "{last_beside:?}"
    );
}

#[test]
fn a_plan_reads_back_each_steps_action_members_and_reason() {
    let record = full_green_run();
    let touched = Touched {
        members: BTreeSet::from(["xtask".to_owned()]),
        ..Touched::default()
    };
    let plan = planned(Some(&record), &touched_since_earlier(touched));

    let text = render(Some(EARLIER), None, "head", &members(), &[], &plan);

    assert_eq!(plan_value(&text, "previous").unwrap(), EARLIER);
    assert_eq!(plan_value(&text, "head").unwrap(), "head");
    assert_eq!(plan_value(&text, "unowned").unwrap(), "-");
    assert_eq!(plan_value(&text, "pre-merge.action").unwrap(), "part");
    assert_eq!(plan_value(&text, "pre-merge.members").unwrap(), "xtask");
    assert_eq!(plan_value(&text, "quick.action").unwrap(), "covered");
    assert_eq!(plan_value(&text, "quick.members").unwrap(), "-");
    assert!(
        plan_value(&text, "quick.why")
            .unwrap()
            .starts_with("covered by pre-merge"),
        "{text}"
    );
    assert_eq!(plan_value(&text, "audit.action").unwrap(), "run");
    assert_eq!(
        plan_value(&text, "audit.why").unwrap(),
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
        earned: BTreeMap::new(),
    }];

    let text = render(None, None, "head", &BTreeSet::new(), &[], &plan);

    assert_eq!(
        plan_value(&text, "lint.why").unwrap(),
        "git failed: fatal: bad object"
    );
}

/// The steps the hosted tier owns that the quick form runs.
const HOSTED_QUICK: [Step; 4] = [
    Step::Lint,
    Step::Quick,
    Step::JourneysTasks,
    Step::CodegenTests,
];

/// A hosted job's record of a run on `EARLIER` whose every unit of a step
/// the hosted tier owns passed on `tree`, but the Pre-merge suite's, which
/// would cover the Quick suite's ([`hosted_batch_run`] holds some), its
/// fingerprint naming `image` and `repository`.
fn hosted_run(image: &str, repository: &str, tree: &str) -> GateRun {
    let outcome = Outcome {
        tree: tree.to_owned(),
        by: Layer::Hosted,
        ..green(EARLIER)
    };
    let mut units = BTreeMap::new();
    for step in Step::ALL
        .into_iter()
        .filter(|step| Owner::of(*step) == Owner::Hosted && *step != Step::PreMerge)
    {
        if step.per_member() {
            for member in members() {
                units.insert(unit(step, Some(&member)), outcome.clone());
            }
        } else {
            units.insert(unit(step, None), outcome.clone());
        }
    }
    GateRun {
        head: EARLIER.to_owned(),
        base: BASE.to_owned(),
        host: hosted_fingerprint(image, repository, "ubuntu24/20261116.1", "variables"),
        units,
        earned: BTreeMap::new(),
    }
}

/// The plan of `layer` in `form` for the head, which has no local record,
/// from `hosted` runs, with the shuffling runners' members `shuffled`.
fn planned_from_hosted(
    (form, layer): (Form, Layer),
    shuffled: &BTreeSet<String>,
    hosted: &[GateRun],
    touched: &BTreeMap<String, Touched>,
) -> Vec<Planned> {
    let members = members();
    let lint_holds = || Ok(());
    let head = Head {
        base: BASE,
        members: &members,
        shuffled,
        tree: HEAD_TREE,
        host: HOST,
        now: NOW,
        lint: LintReads::new(&lint_holds),
        form,
        layer,
        pin: PIN,
    };
    plan(&head, &earlier(None, hosted), touched)
}

/// `just merge` on a batch whose hosted run passed on the head's tree runs
/// none of the hosted-owned units of the quick form, and says the hosted
/// layer ran each; the shuffling runners' members too, whose order the tree
/// gives. The checks of the whole change still run here.
#[test]
fn the_merge_reuses_every_hosted_unit_green_on_the_heads_tree() {
    let hosted = [hosted_run(IMAGE, REPOSITORY, HEAD_TREE)];
    let shuffled = BTreeSet::from(["test-support/typescript".to_owned()]);

    let plan = planned_from_hosted(
        (Form::Quick, Layer::Merge),
        &shuffled,
        &hosted,
        &touched_since_earlier(Touched::default()),
    );

    for step in HOSTED_QUICK {
        let planned = plan.iter().find(|planned| planned.step == step).unwrap();
        assert!(
            matches!(&planned.action, Action::Reuse { origins } if origins == &BTreeSet::from([EARLIER.to_owned()])),
            "{step:?}: {:?}",
            planned.action
        );
        assert!(
            !planned.carried.is_empty()
                && planned
                    .carried
                    .values()
                    .all(|outcome| outcome.by == Layer::Hosted),
            "{step:?}"
        );
    }
    for step in WHOLE_CHANGE_STEPS {
        let step = Step::from_name(step).unwrap();
        assert!(
            matches!(action(&plan, step), Action::Run { .. }),
            "{step:?}"
        );
    }
}

/// A hosted record counts only under the head's pin: one from another CI
/// image, or a fork's, whose fingerprint names the fork, is never reused,
/// and neither is a fingerprint that names no image.
#[rstest]
#[case::another_image("other-image", REPOSITORY, "ran in another CI image")]
#[case::a_fork(IMAGE, "someone/goodfetch", "another repository's pull request")]
fn a_hosted_record_off_the_heads_pin_is_not_reused(
    #[case] image: &str,
    #[case] repository: &str,
    #[case] reason: &str,
) {
    let hosted = [hosted_run(image, repository, HEAD_TREE)];

    let plan = planned_from_hosted(
        (Form::Quick, Layer::Merge),
        &BTreeSet::new(),
        &hosted,
        &touched_since_earlier(Touched::default()),
    );

    for step in HOSTED_QUICK {
        assert!(
            matches!(action(&plan, step), Action::Run { why } if why.contains(reason)),
            "{step:?}: {:?}",
            action(&plan, step)
        );
    }
}

#[test]
fn a_local_hosts_record_never_counts_as_a_hosted_one() {
    let mut local = hosted_run(IMAGE, REPOSITORY, HEAD_TREE);
    local.host = HOST.to_owned();

    let plan = planned_from_hosted(
        (Form::Quick, Layer::Merge),
        &BTreeSet::new(),
        &[local],
        &touched_since_earlier(Touched::default()),
    );

    assert!(
        matches!(action(&plan, Step::Lint), Action::Run { why } if why.contains("names no CI image")),
        "{:?}",
        action(&plan, Step::Lint)
    );
}

#[test]
fn a_hosted_unit_older_than_a_day_reruns() {
    let mut hosted = hosted_run(IMAGE, REPOSITORY, HEAD_TREE);
    for outcome in hosted.units.values_mut() {
        outcome.at = NOW - MAX_AGE - 1;
    }

    let plan = planned_from_hosted(
        (Form::Quick, Layer::Merge),
        &BTreeSet::new(),
        &[hosted],
        &touched_since_earlier(Touched::default()),
    );

    for step in HOSTED_QUICK {
        assert!(
            matches!(action(&plan, step), Action::Run { why } if why.contains("older than a day")),
            "{step:?}: {:?}",
            action(&plan, step)
        );
    }
}

/// On a head past the hosted run's, the merge of `main`'s tip the gate
/// pushes for one, a hosted unit holds where the change since its head does
/// not reach it: the members it touches rerun, and so do the checks of the
/// tree and the shuffling runners' members, whose tree changed.
#[test]
fn a_change_since_the_hosted_run_reruns_what_it_reaches() {
    let hosted = [hosted_run(IMAGE, REPOSITORY, &green(EARLIER).tree)];
    let shuffled = BTreeSet::from(["test-support/typescript".to_owned()]);
    let touched = Touched {
        members: BTreeSet::from(["goodfetch-engine".to_owned()]),
        ..Touched::default()
    };

    let plan = planned_from_hosted(
        (Form::Quick, Layer::Merge),
        &shuffled,
        &hosted,
        &touched_since_earlier(touched),
    );

    assert!(
        matches!(
            action(&plan, Step::Quick),
            Action::Part { members, .. }
                if members == &BTreeSet::from(["goodfetch-engine".to_owned(), "test-support/typescript".to_owned()])
        ),
        "{:?}",
        action(&plan, Step::Quick)
    );
    for step in [Step::Lint, Step::JourneysTasks, Step::CodegenTests] {
        assert!(
            matches!(action(&plan, step), Action::Run { why } if why.contains("the tree changed")),
            "{step:?}: {:?}",
            action(&plan, step)
        );
    }
}

/// A hosted unit red on the head's tree is the gate's verdict (ADR 0064): the
/// step is red and runs nowhere beside it. Red on another tree, it reruns.
#[rstest]
#[case::on_the_heads_tree(HEAD_TREE, true)]
#[case::on_another_tree("earlier-tree", false)]
fn a_red_hosted_unit(#[case] tree: &str, #[case] red: bool) {
    let mut hosted = hosted_run(IMAGE, REPOSITORY, tree);
    hosted
        .units
        .get_mut(&unit(Step::Quick, Some("goodfetch-engine")))
        .unwrap()
        .green = false;

    let plan = planned_from_hosted(
        (Form::Quick, Layer::Merge),
        &BTreeSet::new(),
        &[hosted],
        &touched_since_earlier(Touched::default()),
    );

    if red {
        assert!(
            matches!(action(&plan, Step::Quick), Action::Red { why } if why.contains("goodfetch-engine: red in the hosted run on earlier")),
            "{:?}",
            action(&plan, Step::Quick)
        );
    } else {
        assert!(
            matches!(
                action(&plan, Step::Quick),
                Action::Part { members, why }
                    if members.contains("goodfetch-engine") && why.contains("red on earlier")
            ),
            "{:?}",
            action(&plan, Step::Quick)
        );
    }
}

/// A hosted record stands in only for the steps the hosted tier owns: a
/// result it holds of a step `just merge` or the gate host owns is never
/// reused.
#[test]
fn a_hosted_record_never_stands_in_for_a_local_or_gate_host_step() {
    let mut hosted = hosted_run(IMAGE, REPOSITORY, HEAD_TREE);
    let forged = Outcome {
        tree: HEAD_TREE.to_owned(),
        by: Layer::Hosted,
        ..green(EARLIER)
    };
    for step in [Step::Perf, Step::Counts] {
        hosted.units.insert(unit(step, None), forged.clone());
    }

    let plan = planned_from_hosted(
        (Form::All, Layer::Merge),
        &BTreeSet::new(),
        &[hosted],
        &touched_since_earlier(Touched::default()),
    );

    for step in [Step::Perf, Step::Counts] {
        assert!(
            matches!(action(&plan, step), Action::Run { .. }),
            "{step:?}: {:?}",
            action(&plan, step)
        );
    }
}

/// A hosted job plans with the one planner, and leaves each step it does
/// not own to the layer that does (spec #3373's table of owning layers).
#[test]
fn the_hosted_layer_leaves_each_step_to_its_owner() {
    let plan = planned_from_hosted(
        (Form::Quick, Layer::Hosted),
        &BTreeSet::new(),
        &[],
        &BTreeMap::new(),
    );

    assert_eq!(
        action(&plan, Step::Dco),
        &Action::Left {
            owner: Owner::Local
        }
    );
    assert_eq!(
        action(&plan, Step::Perf),
        &Action::Left {
            owner: Owner::GateHost
        }
    );
    for step in HOSTED_QUICK {
        assert!(
            matches!(action(&plan, step), Action::Run { .. }),
            "{step:?}"
        );
    }
}

/// xtask's codegen tests are a step of the quick form, and the Pre-merge
/// suite, which runs every test, covers them in the full forms.
#[rstest]
#[case::all(Form::All, true)]
#[case::preflight(Form::Preflight, true)]
#[case::quick(Form::Quick, false)]
#[case::batch(Form::Batch, true)]
fn the_codegen_tests_are_covered_where_the_pre_merge_suite_runs(
    #[case] form: Form,
    #[case] covered: bool,
) {
    let plan = planned_from_hosted(
        (form, Layer::Merge),
        &BTreeSet::new(),
        &[],
        &BTreeMap::new(),
    );

    assert_eq!(
        matches!(
            action(&plan, Step::CodegenTests),
            Action::Covered { by: Step::PreMerge }
        ),
        covered
    );
    assert_eq!(
        entries(form)
            .iter()
            .any(|entry| entry.name == "codegen-tests"),
        matches!(form, Form::Quick | Form::Batch)
    );
}

/// A batch's form is the quick form's steps and the Pre-merge suite over
/// the whole workspace, in the gate's order (ADR 0135's note of 2026-11-21).
#[test]
fn the_batch_form_runs_the_quick_forms_steps_and_the_pre_merge_suite() {
    let quick: Vec<&str> = entries(Form::Quick)
        .iter()
        .map(|entry| entry.name)
        .collect();
    let batch = entries(Form::Batch);
    let names: Vec<&str> = batch.iter().map(|entry| entry.name).collect();

    let mut expected = quick.clone();
    let after_quick = quick.iter().position(|name| *name == "quick").unwrap() + 1;
    expected.insert(after_quick, "pre-merge");
    assert_eq!(names, expected);
    let pre_merge = batch
        .iter()
        .find(|entry| entry.name == "pre-merge")
        .unwrap();
    assert_eq!(pre_merge.just, ["_suite", "pre-merge", "all"]);
    assert_eq!(pre_merge.owner, Owner::Hosted);
}

/// A crate whose tests launch no browser and use no OS service.
const BROWSERLESS: &str = "goodfetch-protocol";
/// Another, so a plan of the two names both.
const BROWSERLESS_TOO: &str = "goodfetch-crawl";

/// The case's members with two crates whose tests launch no browser, and
/// the shuffling runners' members among them.
fn batch_members() -> (BTreeSet<String>, BTreeSet<String>) {
    let mut all = members();
    all.extend([BROWSERLESS.to_owned(), BROWSERLESS_TOO.to_owned()]);
    let shuffled = BTreeSet::from([
        "test-support/typescript".to_owned(),
        "sdks/python/collie-client".to_owned(),
    ]);
    (all, shuffled)
}

/// The plan of `layer` in `form` for a head of [`batch_members`], from
/// `hosted` runs, the change since them reaching nothing.
fn planned_for_a_batch((form, layer): (Form, Layer), hosted: &[GateRun]) -> Vec<Planned> {
    let (members, shuffled) = batch_members();
    let lint_holds = || Ok(());
    let head = Head {
        base: BASE,
        members: &members,
        shuffled: &shuffled,
        tree: HEAD_TREE,
        host: HOST,
        now: NOW,
        lint: LintReads::new(&lint_holds),
        form,
        layer,
        pin: PIN,
    };
    plan(
        &head,
        &earlier(None, hosted),
        &touched_since_earlier(Touched::default()),
    )
}

/// A hosted record on the head's tree whose Pre-merge units of `held`
/// passed, with the quick form's hosted units of the other members.
fn hosted_batch_run(held: &[&str]) -> GateRun {
    let (members, _) = batch_members();
    let mut run = hosted_run(IMAGE, REPOSITORY, HEAD_TREE);
    let outcome = Outcome {
        tree: HEAD_TREE.to_owned(),
        by: Layer::Hosted,
        ..green(EARLIER)
    };
    for member in &members {
        if held.contains(&member.as_str()) {
            run.units
                .insert(unit(Step::PreMerge, Some(member)), outcome.clone());
        } else {
            run.units
                .insert(unit(Step::Quick, Some(member)), outcome.clone());
        }
    }
    run
}

fn planned_step(plan: &[Planned], step: Step) -> &Planned {
    plan.iter()
        .find(|planned| planned.step == step)
        .expect("every step is planned")
}

/// A batch's hosted run runs the Pre-merge suite of the members whose tests
/// launch no browser and use no OS service, and those members' Quick units
/// are covered by it: the hosted quick job runs only the others' (spec
/// #3373). xtask's tests do launch one, so its codegen tests still run.
#[test]
fn a_batchs_hosted_run_runs_the_pre_merge_suite_of_the_members_that_launch_no_browser() {
    let plan = planned_for_a_batch((Form::Batch, Layer::Hosted), &[]);

    let browserless = BTreeSet::from([BROWSERLESS.to_owned(), BROWSERLESS_TOO.to_owned()]);
    assert!(
        matches!(action(&plan, Step::PreMerge), Action::Part { members, .. } if members == &browserless),
        "{:?}",
        action(&plan, Step::PreMerge)
    );
    let (all, _) = batch_members();
    let others: BTreeSet<String> = all.difference(&browserless).cloned().collect();
    assert!(
        matches!(
            action(&plan, Step::Quick),
            Action::Part { members, why } if members == &others && why.contains("covered by pre-merge")
        ),
        "{:?}",
        action(&plan, Step::Quick)
    );
    assert!(
        matches!(action(&plan, Step::CodegenTests), Action::Run { .. }),
        "{:?}",
        action(&plan, Step::CodegenTests)
    );
}

/// `just merge` on the quick form takes a member's hosted Pre-merge unit
/// green on the tree for its Quick unit, and carries it into its record;
/// the other members reuse their hosted Quick units.
#[test]
fn the_merge_covers_each_quick_unit_a_hosted_pre_merge_unit_holds() {
    let hosted = [hosted_batch_run(&[BROWSERLESS, BROWSERLESS_TOO])];

    let plan = planned_for_a_batch((Form::Quick, Layer::Merge), &hosted);

    let quick = planned_step(&plan, Step::Quick);
    assert!(
        matches!(&quick.action, Action::Reuse { .. }),
        "{:?}",
        quick.action
    );
    assert!(
        !quick
            .carried
            .contains_key(&unit(Step::Quick, Some(BROWSERLESS))),
        "{:?}",
        quick.carried
    );
    let pre_merge = planned_step(&plan, Step::PreMerge);
    for member in [BROWSERLESS, BROWSERLESS_TOO] {
        assert_eq!(
            pre_merge
                .carried
                .get(&unit(Step::PreMerge, Some(member)))
                .map(|outcome| outcome.by),
            Some(Layer::Hosted),
            "{member}"
        );
    }
}

/// Once every member's hosted Pre-merge unit holds, the Quick suite and
/// xtask's codegen tests are covered: xtask's Pre-merge unit runs them.
#[test]
fn the_quick_suite_and_the_codegen_tests_are_covered_once_every_pre_merge_unit_holds() {
    let (members, _) = batch_members();
    let every: Vec<&str> = members.iter().map(String::as_str).collect();
    let hosted = [hosted_batch_run(&every)];

    let plan = planned_for_a_batch((Form::Quick, Layer::Merge), &hosted);

    for step in [Step::Quick, Step::CodegenTests] {
        assert_eq!(
            action(&plan, step),
            &Action::Covered { by: Step::PreMerge },
            "{step:?}"
        );
    }
}

/// A hosted Pre-merge unit red on the head's tree is the gate's verdict
/// (ADR 0064): the quick form, which runs no Pre-merge suite, is red on its
/// Quick suite, and runs neither again.
#[test]
fn a_red_hosted_pre_merge_unit_makes_the_quick_gate_red() {
    let mut hosted = hosted_batch_run(&[BROWSERLESS, BROWSERLESS_TOO]);
    hosted
        .units
        .get_mut(&unit(Step::PreMerge, Some(BROWSERLESS)))
        .unwrap()
        .green = false;

    let plan = planned_for_a_batch((Form::Quick, Layer::Merge), &[hosted]);

    for step in [Step::PreMerge, Step::Quick] {
        assert!(
            matches!(action(&plan, step), Action::Red { why } if why.contains("goodfetch-protocol: red in the hosted run")),
            "{step:?}: {:?}",
            action(&plan, step)
        );
    }
}

/// Every crate whose tests nextest's profile names as launching the pinned
/// browser is one the hosted Pre-merge shards leave out.
#[test]
fn each_crate_nextest_names_as_launching_the_browser_is_left_out_of_the_hosted_shards() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let profile = std::fs::read_to_string(manifest.join("../.config/nextest.toml")).unwrap();

    // The override of the cells without the pinned build names each test
    // that launches it or reads which build launches.
    let (_, browser) = profile
        .split_once("target_env = \"musl\")))'")
        .expect("the override of the cells without the pinned build");
    let (browser, _) = browser.split_once("\"\"\"\n\n").unwrap();
    let named: BTreeSet<&str> = browser
        .split("binary_id(")
        .skip(1)
        .filter_map(|rest| rest.split([':', ')']).next())
        .collect();

    assert!(named.contains("goodfetch-collie"), "{named:?}");
    for name in named {
        assert!(
            super::plan::BROWSER_OR_SERVICE_CRATES.contains(&name),
            "{name} is named by .config/nextest.toml, so its tests may launch a browser"
        );
    }
}
