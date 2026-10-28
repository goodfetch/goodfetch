//! The Pre-merge suite on a local host (`just pre-merge`, `ci/pre-merge.sh`),
//! which the local gate runs before every merge (ADR 0135): every check of
//! TESTING.md's Pre-merge tier runs, the ones that need Linux beside the host
//! steps, and the suite fails when any of them fails. The steps run through a
//! stand-in `just` that records each call. The local gate is repository
//! tooling, so the module declares no Feature.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::workspace::Workspace;

/// Every step of `just pre-merge all`: the host's in the order it runs them,
/// Collie's Console build first, which the steps beside find in the
/// workspace they mount, then those beside them.
const STEPS: [&str; 22] = [
    "console-build",
    "dco",
    "commit-messages",
    "lint",
    "feature-combinations",
    "audit",
    "commit-leaks",
    "fuzz-check",
    "proofs",
    "_suite quick all",
    "_suite pre-merge all",
    "features-audit",
    "stress",
    "journeys-tasks --check",
    "features-tests clients-tests",
    "_tlc all",
    "msrv",
    "miri",
    "notices",
    "perf",
    "linux",
    "visual",
];

/// The steps that need Linux, run beside the host's.
const BESIDE: [&str; 3] = ["perf", "linux", "visual"];

/// The OS-service setup the suite runs first on a Linux host, which runs the
/// tests only Linux decides itself (`ci/linux.sh`); elsewhere `just linux`'s
/// container sets the services up for them.
const HOST_SETUP: &[&str] = if cfg!(target_os = "linux") {
    &["_os-services"]
} else {
    &[]
};

/// A stand-in `just` that appends its arguments to `calls`, and them with
/// the OS services' variables it was given to `variables`, and fails with
/// status 3 when they are `failing`'s content, or with the status a
/// `statuses` line `<arguments>=<status>` gives them. As `just _gate-plan` it
/// prints `plan`'s content, the incremental gate's plan, when there is one.
/// When they are `lingering`'s content, it starts a child, as `just perf`'s
/// recipe starts `xtask perf build` and its `cargo bench`, its output
/// thrown away, writes the child's process ID to `descendant` and waits for
/// it; the failing step then fails only once that child has started. When
/// they are `late`'s content, it fails with status 3 once the run's output
/// holds `awaiting`'s text.
fn stand_in_just(workspace: &Workspace) -> PathBuf {
    let folder = workspace.path().join("stand-in");
    std::fs::create_dir_all(&folder).unwrap();
    let script = format!(
        r#"#!/bin/sh
echo "$*" >>"{calls}"
echo "$*|${{DBUS_SESSION_BUS_ADDRESS:-}}|${{GOODFETCH_OS_SERVICES:-}}|${{GOODFETCH_TEST_DNS_NAMESPACE:-}}" >>"{variables}"
if [ "$1" = _gate-plan ] && [ -f "{plan}" ]; then
  cat "{plan}"
fi
if [ -f "{statuses}" ]; then
  status=$(awk -v call="$*=" 'index($0, call) == 1 {{ print substr($0, length(call) + 1) }}' "{statuses}")
  [ -z "$status" ] || exit "$status"
fi
if [ -f "{failing}" ] && [ "$*" = "$(cat "{failing}")" ]; then
  waited=0
  while [ -f "{lingering}" ] && [ ! -s "{descendant}" ] && [ $waited -lt 300 ]; do
    sleep 0.1
    waited=$((waited + 1))
  done
  exit 3
fi
if [ -f "{late}" ] && [ "$*" = "$(cat "{late}")" ]; then
  waited=0
  while ! grep -qF -- "$(cat "{awaiting}")" "{stdout}" && [ $waited -lt 300 ]; do
    sleep 0.1
    waited=$((waited + 1))
  done
  exit 3
fi
if [ -f "{lingering}" ] && [ "$*" = "$(cat "{lingering}")" ]; then
  sleep 300 >/dev/null 2>&1 &
  echo $! >"{descendant}"
  wait
fi
if [ -f "{killing}" ] && [ "$*" = "$(cat "{killing}")" ]; then
  kill -KILL "$PPID"
  exit 0
fi
"#,
        calls = folder.join("calls").display(),
        variables = folder.join("variables").display(),
        plan = folder.join("plan").display(),
        failing = folder.join("failing").display(),
        statuses = folder.join("statuses").display(),
        killing = folder.join("killing").display(),
        lingering = folder.join("lingering").display(),
        descendant = folder.join("descendant").display(),
        late = folder.join("late").display(),
        awaiting = folder.join("awaiting").display(),
        stdout = folder.join("stdout").display(),
    );
    let just = folder.join("just");
    std::fs::write(&just, script).unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&just, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    folder
}

struct Run {
    code: Option<i32>,
    /// The steps' calls, the incremental gate's own aside.
    calls: Vec<String>,
    /// The incremental gate's calls: `_gate-plan`, `_gate-record <step> <status>`.
    gate: Vec<String>,
    /// Each call as `<arguments>|<session bus>|<GOODFETCH_OS_SERVICES>|<GOODFETCH_TEST_DNS_NAMESPACE>`.
    variables: Vec<String>,
    stdout: String,
    stderr: String,
}

/// The session bus that the stand-in setup's environment file names.
const BUS: &str = "unix:path=/stand-in/bus";

/// Runs the repository's `ci/pre-merge.sh` in the workspace, with the
/// stand-in first on `PATH`.
fn pre_merge(workspace: &Workspace, stand_in: &Path, args: &[&str]) -> Run {
    pre_merge_with_env(workspace, stand_in, args, &[])
}

/// Runs `ci/pre-merge.sh` with these variables added, and the environment
/// file `ci/os-services.sh setup` writes in the workspace's temporary folder.
fn pre_merge_with_env(
    workspace: &Workspace,
    stand_in: &Path,
    args: &[&str],
    variables: &[(&str, &str)],
) -> Run {
    workspace.write(
        "runner-temp/goodfetch-os-services/env",
        format!("DBUS_SESSION_BUS_ADDRESS={BUS}\n").as_bytes(),
    );
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/pre-merge.sh");
    let path = format!("{}:/usr/bin:/bin", stand_in.display());
    // Its output goes to files, not pipes, so a process it leaves behind
    // holding them cannot keep the run from ending.
    let stdout = stand_in.join("stdout");
    let stderr = stand_in.join("stderr");
    let status = workspace
        .command(script)
        .args(args)
        .env("PATH", path)
        .env("RUNNER_TEMP", workspace.path().join("runner-temp"))
        .envs(variables.iter().copied())
        .stdout(std::fs::File::create(&stdout).unwrap())
        .stderr(std::fs::File::create(&stderr).unwrap())
        .status()
        .unwrap();
    let read = |file: &str| -> Vec<String> {
        std::fs::read_to_string(stand_in.join(file))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    };
    let is_gate = |call: &String| call.starts_with("_gate-");
    let (gate, calls) = read("calls").into_iter().partition(is_gate);
    Run {
        code: status.code(),
        calls,
        gate,
        variables: read("variables")
            .into_iter()
            .filter(|call| !is_gate(call))
            .collect(),
        stdout: String::from_utf8_lossy(&std::fs::read(stdout).unwrap()).into_owned(),
        stderr: String::from_utf8_lossy(&std::fs::read(stderr).unwrap()).into_owned(),
    }
}

#[test]
fn every_pre_merge_check_runs_and_the_skipped_parts_are_listed_last() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut ran = run.calls.clone();
    assert_eq!(ran.pop().as_deref(), Some("_skips-report"));
    ran.sort();
    let mut expected: Vec<String> = HOST_SETUP
        .iter()
        .chain(&STEPS)
        .map(|step| (*step).to_owned())
        .collect();
    expected.sort();
    assert_eq!(ran, expected);
    let host: Vec<&String> = run
        .calls
        .iter()
        .filter(|call| !BESIDE.contains(&call.as_str()) && *call != "_skips-report")
        .collect();
    assert_eq!(
        host,
        HOST_SETUP
            .iter()
            .chain(&STEPS[..STEPS.len() - BESIDE.len()])
            .collect::<Vec<_>>()
    );
}

#[test]
fn without_all_the_suites_run_over_what_changed() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut ran = run.calls;
    ran.sort();
    // The same steps, `all` left empty: `just _suite quick ""`.
    let mut expected: Vec<String> = HOST_SETUP
        .iter()
        .chain(&STEPS)
        .map(|step| step.replace(" all", " "))
        .chain(["_skips-report".to_owned()])
        .collect();
    expected.sort();
    assert_eq!(ran, expected);
}

/// The quick form's steps, in order: those that judge every change, never a
/// test of the Pre-merge suite or a step beside the host's.
const QUICK_STEPS: [&str; 8] = [
    "dco",
    "commit-messages",
    "lint",
    "audit",
    "commit-leaks",
    "_suite quick all",
    "journeys-tasks --check",
    "notices",
];

#[test]
fn the_quick_form_runs_the_checks_every_change_needs_and_nothing_beside() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &["quick"]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut expected: Vec<String> = HOST_SETUP
        .iter()
        .chain(&QUICK_STEPS)
        .map(|step| (*step).to_owned())
        .collect();
    expected.push("_skips-report".to_owned());
    assert_eq!(run.calls, expected);
}

/// When one step of a gate run started and ended, and its exit status, as
/// its log says.
#[derive(Debug)]
struct StepTimes {
    started: jiff::Timestamp,
    ended: jiff::Timestamp,
    seconds: i64,
    status: i32,
}

/// Each step's times in a gate run's output, by step: the `Gate step
/// <step>: started at <time>` and `… ended at <time> after <n> s, exit
/// status <code>` lines, a step beside the host's behind its `[<step>] `
/// mark, and the steps that started and never ended. A step logged twice,
/// or ended before it started, fails.
fn step_times(stdout: &str) -> (std::collections::BTreeMap<String, StepTimes>, Vec<String>) {
    let line = regex::Regex::new(
        r"^(?:\[[a-z]+\] )?Gate step ([a-z-]+): (?:started at (\S+)|ended at (\S+) after (\d+) s, exit status (\d+))$",
    )
    .unwrap();
    let mut started = std::collections::BTreeMap::new();
    let mut times = std::collections::BTreeMap::new();
    for captures in stdout.lines().filter_map(|text| line.captures(text)) {
        let step = captures[1].to_owned();
        if let Some(time) = captures.get(2) {
            let time: jiff::Timestamp = time.as_str().parse().unwrap();
            assert!(
                started.insert(step.clone(), time).is_none(),
                "{step} started twice"
            );
        } else {
            let start = started
                .remove(&step)
                .unwrap_or_else(|| panic!("{step} ended before it started"));
            let ended: jiff::Timestamp = captures[3].parse().unwrap();
            assert!(start <= ended, "{step} ended at {ended}, before {start}");
            let step_times = StepTimes {
                started: start,
                ended,
                seconds: captures[4].parse().unwrap(),
                status: captures[5].parse().unwrap(),
            };
            assert!(
                times.insert(step.clone(), step_times).is_none(),
                "{step} ended twice"
            );
        }
    }
    (times, started.into_keys().collect())
}

/// Each step of a gate run logs when it started and ended, so a gate log
/// says where its time went (the test-gate audit's option 6).
#[rstest]
#[case::all(
    "all",
    &[
        "console-build", "perf", "linux", "visual", "dco", "commit-messages", "lint",
        "feature-combinations", "audit", "commit-leaks", "fuzz-check", "proofs", "quick",
        "pre-merge", "stress", "journeys-tasks", "features-tests", "tlc", "msrv", "miri",
        "notices",
    ],
)]
#[case::quick(
    "quick",
    &[
        "dco", "commit-messages", "lint", "audit", "commit-leaks", "quick", "journeys-tasks",
        "notices",
    ],
)]
fn each_step_logs_when_it_started_and_ended(#[case] form: &str, #[case] steps: &[&str]) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    let before = jiff::Timestamp::now();

    let run = pre_merge(&workspace, &stand_in, &[form]);

    let after = jiff::Timestamp::now();
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let (times, unended) = step_times(&run.stdout);
    assert!(unended.is_empty(), "never ended: {unended:?}");
    let mut expected: Vec<&str> = if cfg!(target_os = "linux") {
        vec!["os-services"]
    } else {
        vec![]
    };
    expected.extend(steps);
    expected.sort_unstable();
    assert_eq!(
        times.keys().map(String::as_str).collect::<Vec<_>>(),
        expected
    );
    // The log's times are whole seconds.
    let floor = before
        .checked_sub(jiff::SignedDuration::from_secs(1))
        .unwrap();
    for (step, step_times) in &times {
        assert!(
            floor <= step_times.started && step_times.ended <= after,
            "{step}: {step_times:?} outside {before}..{after}"
        );
        assert_eq!(step_times.status, 0, "{step}");
        assert!(
            step_times.seconds <= after.duration_since(before).as_secs() + 1,
            "{step}"
        );
    }
}

/// A failed step logs its end with its exit status, whether it ran on the
/// host or beside it.
#[rstest]
#[case::host_step("lint", "lint")]
#[case::beside("visual", "visual")]
#[case::suite("_suite pre-merge all", "pre-merge")]
fn a_failed_step_logs_its_end_with_its_exit_status(#[case] call: &str, #[case] step: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing"), call).unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    let (times, _) = step_times(&run.stdout);
    assert_eq!(
        times.get(step).map(|times| times.status),
        Some(3),
        "{}",
        run.stdout
    );
}

#[test]
fn an_unknown_argument_is_refused_before_any_step() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &["most"]);

    assert_eq!(run.code, Some(2), "{}", run.stderr);
    assert!(run.calls.is_empty(), "{:?}", run.calls);
}

#[rstest]
#[case::perf("perf")]
#[case::linux("linux")]
#[case::visual("visual")]
fn a_failed_check_beside_the_host_steps_fails_the_suite(#[case] step: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing"), step).unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        run.stderr.contains(&format!("just {step} failed")),
        "{}",
        run.stderr
    );
    assert!(
        run.calls.iter().any(|call| call == "notices"),
        "{:?}",
        run.calls
    );
}

#[test]
fn a_failed_stress_run_stops_the_suite_before_the_later_steps() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing"), "stress").unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        !run.calls.iter().any(|call| call == "notices"),
        "{:?}",
        run.calls
    );
    assert_eq!(run.calls.last().map(String::as_str), Some("_skips-report"));
}

/// On a Linux host every step after the setup meets the services it
/// started, and runs where a missing one fails instead of skipping its
/// tests (`ci/os-services.sh skipped`), with the DNS namespace's way
/// declared, so a silent fallback fails too.
#[cfg(target_os = "linux")]
#[rstest]
#[case::all("all")]
#[case::quick("quick")]
fn on_linux_every_step_runs_with_the_os_services_set_up(#[case] form: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge(&workspace, &stand_in, &[form]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.calls.first().map(String::as_str), Some("_os-services"));
    let steps: Vec<(&str, &str)> = run
        .variables
        .iter()
        .filter_map(|line| line.split_once('|'))
        .filter(|(call, _)| !["_os-services", "_skips-report"].contains(call))
        .collect();
    assert!(steps.len() > 1, "{:?}", run.variables);
    for (call, variables) in steps {
        assert_eq!(variables, format!("{BUS}|required|sudo"), "{call}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn on_linux_a_declared_dns_namespace_way_is_kept() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge_with_env(
        &workspace,
        &stand_in,
        &["quick"],
        &[("GOODFETCH_TEST_DNS_NAMESPACE", "user_namespace")],
    );

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(
        run.variables
            .contains(&format!("lint|{BUS}|required|user_namespace")),
        "{:?}",
        run.variables
    );
}

#[cfg(target_os = "linux")]
#[test]
fn on_linux_a_failed_os_service_setup_stops_the_suite_before_any_step() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("failing"), "_os-services").unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert_eq!(run.calls, ["_os-services", "_skips-report"]);
}

/// A plan of the incremental gate (ADR 0135), as `xtask gate plan` prints
/// it: perf, the Pre-merge suite and TLC reused from an earlier run on
/// `earlier`, the Quick suite rerun for two members and the stress run for
/// one; every other step runs.
const PLAN: &str = "previous\tearlier
head\tplanned
members\ta b c :unowned
unowned\tconsole/screens.spec.ts
perf\treuse\t-\tgreen on earlier
pre-merge\treuse\t-\tgreen on earlier
tlc\treuse\t-\tgreen on earlier
quick\tpart\ta b :unowned\tthe change since earlier reaches it
stress\tpart\ta :unowned\tthe change since earlier reaches it
lint\trun\t-\tevery run reruns it
";

#[test]
fn the_incremental_gate_reruns_only_what_its_plan_reruns_and_records_each_result() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("plan"), PLAN).unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    for reused in ["perf", "_suite pre-merge all", "features-audit", "_tlc all"] {
        assert!(
            !run.calls.iter().any(|call| call == reused),
            "{reused} ran: {:?}",
            run.calls
        );
    }
    for rerun in [
        "test quick --shuffled-all a b console/screens.spec.ts",
        "stress  earlier a :unowned",
        "lint",
        "linux",
        "visual",
        "msrv",
    ] {
        assert!(
            run.calls.iter().any(|call| call == rerun),
            "{rerun} did not run: {:?}",
            run.calls
        );
    }
    assert!(
        run.stdout.contains("perf: reused (green on earlier)"),
        "{}",
        run.stdout
    );
    assert_eq!(run.gate.first().map(String::as_str), Some("_gate-plan"));
    let mut recorded = run.gate[1..].to_vec();
    recorded.sort();
    let mut expected: Vec<String> = [
        "dco",
        "commit-messages",
        "lint",
        "feature-combinations",
        "audit",
        "commit-leaks",
        "fuzz-check",
        "proofs",
        "quick",
        "stress",
        "journeys-tasks",
        "features-tests",
        "msrv",
        "miri",
        "notices",
        "linux",
        "visual",
    ]
    .iter()
    .map(|step| format!("_gate-record planned {step} pass"))
    .collect();
    expected.sort();
    assert_eq!(recorded, expected);
}

#[rstest]
#[case::host_step("lint", "lint")]
#[case::beside("perf", "perf")]
#[case::suite("_suite pre-merge all", "pre-merge")]
fn a_failed_step_is_recorded_as_failed(#[case] call: &str, #[case] step: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    // A plan with no earlier run: every step runs.
    std::fs::write(stand_in.join("plan"), "previous\tnone\nhead\tplanned\n").unwrap();
    std::fs::write(stand_in.join("failing"), call).unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        run.gate
            .contains(&format!("_gate-record planned {step} fail")),
        "{:?}",
        run.gate
    );
}

/// A failed Quick suite or stress run hands `xtask gate record` the files
/// its crates' results are read from, and runs with none of them left from
/// an earlier run (the test-gate audit's option 4).
#[rstest]
#[case::quick(
    "_suite quick all",
    "quick fail --junit target/nextest/quick/junit.xml",
    "target/nextest/quick/junit.xml"
)]
#[case::stress(
    "stress",
    "stress fail --selection target/stress-selection.tsv",
    "target/junit/repeat-20/nextest-ci.xml"
)]
fn a_failed_step_run_per_member_is_recorded_with_its_own_evidence(
    #[case] call: &str,
    #[case] recorded: &str,
    #[case] earlier: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("plan"), "previous\tnone\nhead\tplanned\n").unwrap();
    std::fs::write(stand_in.join("failing"), call).unwrap();
    workspace.write(earlier, b"an earlier run's");

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        run.gate
            .contains(&format!("_gate-record planned {recorded}")),
        "{:?}",
        run.gate
    );
    assert!(!workspace.path().join(earlier).exists());
}

/// A Pre-merge suite that failed with 100, nextest's status for failed
/// tests, which the suite's recipe gives once its doctests passed after a
/// red nextest run, still runs the exercise audit; with the audit green, it
/// is recorded with its nextest file, so its crates that passed in full
/// record green (the test-gate audit's option 4b). A red audit, or another
/// failure, records every member red, and the step stays red throughout.
#[rstest]
#[case::the_audit_passes(
    "",
    Some(100),
    true,
    "pre-merge fail --junit target/nextest/ci/junit.xml"
)]
#[case::the_audit_fails("features-audit=1\n", Some(1), true, "pre-merge fail")]
#[case::another_failure("_suite pre-merge all=3\n", Some(3), false, "pre-merge fail")]
fn a_pre_merge_suite_red_in_nextest_alone_still_runs_the_exercise_audit(
    #[case] audit: &str,
    #[case] code: Option<i32>,
    #[case] audited: bool,
    #[case] recorded: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("plan"), "previous\tnone\nhead\tplanned\n").unwrap();
    let suite = if audit.starts_with("_suite") {
        String::new()
    } else {
        "_suite pre-merge all=100\n".to_owned()
    };
    std::fs::write(stand_in.join("statuses"), format!("{suite}{audit}")).unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, code, "{}", run.stderr);
    assert_eq!(
        run.calls.iter().any(|call| call == "features-audit"),
        audited,
        "{:?}",
        run.calls
    );
    assert!(
        run.gate
            .contains(&format!("_gate-record planned {recorded}")),
        "{:?}",
        run.gate
    );
    assert!(
        !run.calls.iter().any(|call| call == "stress"),
        "{:?}",
        run.calls
    );
}

/// After a red nextest run, the Pre-merge suite's recipe still runs the
/// crates' doctests, then fails with 100 when they passed, and with their
/// status when they failed; Vitest, Playwright and pytest do not run after
/// it, as before (the test-gate audit's option 4b).
#[rstest]
#[case::the_doctests_pass(None, Some(100))]
#[case::the_doctests_fail(Some("101"), Some(101))]
fn after_a_red_nextest_run_the_pre_merge_suite_still_runs_the_doctests(
    #[case] doctests: Option<&str>,
    #[case] code: Option<i32>,
) {
    let fixture = crate::head_order::fixture();
    fixture.fail("insta", "1");
    if let Some(status) = doctests {
        fixture.fail("test", status);
    }
    fixture.workspace.commit_all();

    let run = fixture.test(&["pre-merge"]);

    assert_eq!(run.code, code, "{}", run.output);
    assert!(
        run.calls
            .iter()
            .any(|call| call.starts_with("cargo test --workspace --doc ")),
        "{:?}",
        run.calls
    );
    assert!(
        !run.calls
            .iter()
            .any(|call| call.starts_with("pnpm exec") || call.starts_with("uv run pytest")),
        "{:?}",
        run.calls
    );
}

/// A step beside the host's that ended before a host step failed keeps its
/// result, so the next run reuses it where the change since does not reach
/// it (the test-gate audit's option 3).
#[rstest]
#[case::passed("perf", "pass")]
#[case::failed("visual", "fail")]
fn a_step_beside_that_ended_before_a_host_step_failed_records_its_result(
    #[case] step: &str,
    #[case] status: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("plan"), "previous\tnone\nhead\tplanned\n").unwrap();
    if status == "fail" {
        std::fs::write(stand_in.join("failing"), step).unwrap();
    }
    // `lint` fails once the step beside has ended.
    std::fs::write(stand_in.join("late"), "lint").unwrap();
    std::fs::write(
        stand_in.join("awaiting"),
        format!("[{step}] Gate step {step}: ended at"),
    )
    .unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        !run.calls.iter().any(|call| call == "notices"),
        "the run went on past lint: {:?}",
        run.calls
    );
    assert!(
        run.gate
            .contains(&format!("_gate-record planned {step} {status}")),
        "{:?}\n{}",
        run.gate,
        run.stdout
    );
}

/// A step beside the host's still running when a host step fails is
/// stopped and records nothing, so the next run reruns it.
#[test]
fn a_step_beside_still_running_when_a_host_step_fails_records_nothing() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("plan"), "previous\tnone\nhead\tplanned\n").unwrap();
    std::fs::write(stand_in.join("lingering"), "perf").unwrap();
    std::fs::write(stand_in.join("failing"), "lint").unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    if let Ok(descendant) = std::fs::read_to_string(stand_in.join("descendant")) {
        let _ = std::process::Command::new("kill")
            .arg(descendant.trim())
            .status();
    }
    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        run.gate
            .contains(&"_gate-record planned lint fail".to_owned()),
        "{:?}",
        run.gate
    );
    assert!(
        !run.gate.iter().any(|call| call.contains(" perf ")),
        "{:?}",
        run.gate
    );
}

#[rstest]
#[case::what_changed("")]
#[case::quick("quick")]
fn only_the_whole_workspace_gate_plans_and_records(#[case] form: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("plan"), PLAN).unwrap();
    let args: &[&str] = if form.is_empty() { &[] } else { &[form] };

    let run = pre_merge(&workspace, &stand_in, args);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.gate.is_empty(), "{:?}", run.gate);
}

/// A run stopped during the exercise audit leaves the Pre-merge suite's
/// step unrecorded, so the next run reruns it (the review's F4).
#[test]
fn a_run_stopped_during_the_exercise_audit_records_no_result_for_the_suite() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("killing"), "features-audit").unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_ne!(run.code, Some(0));
    assert!(
        run.calls.iter().any(|call| call == "features-audit"),
        "{:?}",
        run.calls
    );
    assert!(
        !run.gate.iter().any(|call| call.contains(" pre-merge ")),
        "{:?}",
        run.gate
    );
}

/// One gate run at a time in a workspace: a second waits for none and
/// fails before any step while the first's process lives (the review's F6).
#[test]
fn a_gate_run_while_another_holds_the_records_fails_before_any_step() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    let mut holder = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    workspace.write(
        "target/gate-runs/lock/pid",
        holder.id().to_string().as_bytes(),
    );

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    holder.kill().unwrap();
    holder.wait().unwrap();
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(run.stderr.contains("another gate run"), "{}", run.stderr);
    assert!(
        run.calls.iter().all(|call| call == "_skips-report"),
        "{:?}",
        run.calls
    );
}

/// A gate run that narrows what runs or swaps the executables under test
/// fails before any step, so it records nothing a normal run would reuse
/// (the re-review's N1).
#[rstest]
#[case::selection("GOODFETCH_TEST_SELECTION")]
#[case::engine("GOODFETCH_TEST_ENGINE")]
#[case::order_seed("GOODFETCH_ORDER_SEED")]
fn a_whole_workspace_gate_run_that_narrows_what_runs_is_refused(#[case] variable: &str) {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);

    let run = pre_merge_with_env(&workspace, &stand_in, &["all"], &[(variable, "set")]);

    assert_eq!(run.code, Some(2), "{}", run.stderr);
    assert!(run.stderr.contains(variable), "{}", run.stderr);
    assert!(
        run.calls.iter().all(|call| call == "_skips-report") && run.gate.is_empty(),
        "{:?} {:?}",
        run.calls,
        run.gate
    );
}

/// Every gate run reruns the Vitest, Playwright and pytest tests of the
/// suites in the head's order, with every one of those runners' tests
/// (`--shuffled-all`), so a test that passed in an earlier run's order and
/// fails in the head's fails the gate (the owner's F8, option B).
#[test]
fn the_shuffling_runners_rerun_in_the_heads_order_and_their_failure_fails_the_gate() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(
        stand_in.join("plan"),
        "previous\tearlier
head\tplanned
members\tcrate test-support/typescript
unowned\t-
pre-merge\tpart\ttest-support/typescript\tVitest, Playwright and pytest rerun every member in the head's test order
",
    )
    .unwrap();
    let call = "test pre-merge --shuffled-all test-support/typescript";
    std::fs::write(stand_in.join("failing"), call).unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(run.calls.iter().any(|ran| ran == call), "{:?}", run.calls);
    assert!(
        run.gate
            .contains(&"_gate-record planned pre-merge fail".to_owned()),
        "{:?}",
        run.gate
    );
}

/// Whether the process `id` still runs: a zombie, which has ended and waits
/// only for its parent to reap it, does not.
fn runs(id: &str) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", id])
        .output()
        .unwrap();
    let state = String::from_utf8_lossy(&output.stdout);
    output.status.success() && !state.trim().is_empty() && !state.trim().starts_with('Z')
}

/// A host step that fails first stops each step beside it with every
/// process that step started, so none goes on building in the workspace
/// a later gate run uses (fix-queue 102: `xtask perf build` and its
/// `cargo bench` outlived a stopped `just perf` for 19 minutes).
#[test]
fn a_failed_host_step_stops_every_process_a_step_beside_it_started() {
    let workspace = Workspace::from_fixture("empty");
    let stand_in = stand_in_just(&workspace);
    std::fs::write(stand_in.join("lingering"), "perf").unwrap();
    std::fs::write(stand_in.join("failing"), "lint").unwrap();

    let run = pre_merge(&workspace, &stand_in, &["all"]);

    let descendant = std::fs::read_to_string(stand_in.join("descendant")).unwrap();
    let descendant = descendant.trim();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while runs(descendant) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let survived = runs(descendant);
    if survived {
        std::process::Command::new("kill")
            .arg(descendant)
            .status()
            .unwrap();
    }
    assert_eq!(run.code, Some(3), "{}", run.stderr);
    assert!(
        !survived,
        "the child of the stopped `just perf` still runs: {}",
        run.stderr
    );
}
