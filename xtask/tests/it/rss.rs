//! The resident-memory check (ADRs 0071, 0094 and 0134): `xtask rss check`
//! holds each scenario's peak, measured on one Product's executable on one
//! platform, to the committed ceiling for that Product, platform and
//! scenario, and reports without gating while no ceilings file exists;
//! `xtask rss propose` turns 20 runs into ceilings at 1.5 times the highest
//! peak, rounded up to a whole MiB. Peaks and ceilings are fixtures here.

use std::process::Stdio;
use std::time::{Duration, Instant};

use goodfetch_test_support::Tool;
use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

const MIB: u64 = 1024 * 1024;

/// The ceilings file the check reads by default.
const CEILINGS: &str = "ci/rss-ceilings.toml";

/// One peak of the peaks file, as `xtask rss measure` writes it.
fn peak(product: &str, platform: &str, scenario: &str, bytes: u64) -> String {
    format!(
        "{{\"product\":\"{product}\",\"platform\":\"{platform}\",\"scenario\":\"{scenario}\",\"bytes\":{bytes}}}\n"
    )
}

fn write_peaks(workspace: &Workspace, peaks: &[String]) {
    workspace.write("target/rss/peaks.jsonl", peaks.concat().as_bytes());
}

fn check(workspace: &Workspace) -> Outcome {
    workspace.xtask(&["rss", "check", "--peaks", "target/rss/peaks.jsonl"])
}

#[rstest]
#[case::under_its_ceiling(5 * MIB)]
#[case::at_its_ceiling(6 * MIB)]
fn a_peak_at_or_under_its_ceiling_passes(#[case] bytes: u64) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[collie.macos-arm64]\nidle-attached-engine = 6\n",
    );
    write_peaks(
        &workspace,
        &[peak("collie", "macos-arm64", "idle-attached-engine", bytes)],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn a_peak_over_its_ceiling_fails() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[collie.macos-arm64]\nidle-attached-engine = 6\n",
    );
    write_peaks(
        &workspace,
        &[peak(
            "collie",
            "macos-arm64",
            "idle-attached-engine",
            6 * MIB + 1,
        )],
    );

    let outcome = check(&workspace);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "rss-ceiling",
            "collie idle-attached-engine macos-arm64"
        )],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[test]
fn with_no_ceilings_file_the_check_reports_and_gates_nothing() {
    let workspace = Workspace::from_fixture("empty");
    write_peaks(
        &workspace,
        &[
            peak("collie", "linux-x64", "idle-attached-engine", 3 * MIB),
            peak("foxhound", "linux-x64", "idle-attached-engine", 9 * MIB),
        ],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    insta::assert_snapshot!(outcome.stdout, @"
    ## Resident memory

    No ceilings file at `ci/rss-ceilings.toml`: the check reports and gates nothing until the owner approves the ceilings (ADR 0071).

    | Product | Scenario | Platform | Peak | Ceiling | Verdict | Performance target |
    |---|---|---|---:|---:|---|---|
    | collie | idle-attached-engine | linux-x64 | 3.00 MiB | none | not gated | 8.00 MiB: met |
    | foxhound | idle-attached-engine | linux-x64 | 9.00 MiB | none | not gated | 8.00 MiB: missed by 1.00 MiB |

    A Performance target informs the owner test and never fails a change (ADR 0134).
    ");
}

#[rstest]
#[case::collie_under_its_own_ceiling("collie", vec![])]
#[case::foxhound_over_its_own_ceiling(
    "foxhound",
    vec![finding("rss-ceiling", "foxhound idle-attached-engine macos-arm64")]
)]
fn each_peak_is_held_to_its_own_products_ceiling(
    #[case] product: &str,
    #[case] expected: Vec<crate::workspace::Finding>,
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[collie.macos-arm64]\nidle-attached-engine = 20\n\n[foxhound.macos-arm64]\nidle-attached-engine = 4\n",
    );
    write_peaks(
        &workspace,
        &[peak(
            product,
            "macos-arm64",
            "idle-attached-engine",
            10 * MIB,
        )],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), expected, "{}", outcome.stderr);
}

#[test]
fn a_collie_peak_is_never_judged_against_foxhounds_ceiling() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[foxhound.macos-arm64]\nidle-attached-engine = 4\n",
    );
    write_peaks(
        &workspace,
        &[peak(
            "collie",
            "macos-arm64",
            "idle-attached-engine",
            10 * MIB,
        )],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn a_peak_is_held_to_the_ceiling_of_the_platform_it_was_measured_on() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        CEILINGS,
        b"[collie.linux-x64]\nidle-attached-engine = 4\n\n[collie.windows-x64]\nidle-attached-engine = 20\n",
    );
    write_peaks(
        &workspace,
        &[peak(
            "collie",
            "windows-x64",
            "idle-attached-engine",
            10 * MIB,
        )],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
}

#[test]
fn the_check_saves_each_peak_with_its_ceiling_for_the_drift_check() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(CEILINGS, b"[collie.linux-x64]\nidle-attached-engine = 6\n");
    write_peaks(
        &workspace,
        &[
            peak("collie", "linux-x64", "idle-attached-engine", 3 * MIB),
            peak("foxhound", "linux-x64", "idle-attached-engine", 5 * MIB),
        ],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let figures: serde_json::Value =
        serde_json::from_str(&workspace.read_text("target/junit/rss/perf-figures.json")).unwrap();
    assert_eq!(
        figures,
        serde_json::json!({
            "figures": [
                {
                    "subject": "rss/collie/idle-attached-engine",
                    "figure": "resident-memory",
                    "value": 3_145_728,
                    "ceiling": 6_291_456,
                },
                {
                    "subject": "rss/foxhound/idle-attached-engine",
                    "figure": "resident-memory",
                    "value": 5_242_880,
                },
            ],
        })
    );
}

#[test]
fn the_check_writes_each_peak_as_a_junit_test_case_with_its_figures() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(CEILINGS, b"[collie.linux-x64]\nidle-attached-engine = 6\n");
    write_peaks(
        &workspace,
        &[
            peak("collie", "linux-x64", "idle-attached-engine", 7 * MIB),
            peak("foxhound", "linux-x64", "idle-attached-engine", 5 * MIB),
        ],
    );

    let outcome = check(&workspace);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    insta::assert_snapshot!(workspace.read_text("target/junit/rss.xml"), @r#"
    <?xml version="1.0" encoding="UTF-8"?>
    <testsuites name="rss" tests="2" failures="1">
      <testsuite name="rss" tests="2" failures="1">
        <testcase classname="rss" name="collie idle-attached-engine" time="0">
          <properties>
            <property name="platform" value="linux-x64"/>
            <property name="peak-bytes" value="7340032"/>
            <property name="ceiling-bytes" value="6291456"/>
            <property name="target-bytes" value="8388608"/>
          </properties>
          <failure message="peak of 7.00 MiB over its ceiling of 6.00 MiB">peak of 7.00 MiB over its ceiling of 6.00 MiB</failure>
        </testcase>
        <testcase classname="rss" name="foxhound idle-attached-engine" time="0">
          <properties>
            <property name="platform" value="linux-x64"/>
            <property name="peak-bytes" value="5242880"/>
            <property name="target-bytes" value="8388608"/>
          </properties>
        </testcase>
      </testsuite>
    </testsuites>
    "#);
}

/// Collie's 20 runs on macOS arm64, the highest 3,500,000 bytes and not the
/// last.
const COLLIE_RUNS: [u64; 20] = [
    3_100_000, 3_200_000, 3_150_000, 3_300_000, 3_500_000, 3_120_000, 3_180_000, 3_220_000,
    3_260_000, 3_240_000, 3_110_000, 3_190_000, 3_230_000, 3_270_000, 3_210_000, 3_130_000,
    3_170_000, 3_250_000, 3_160_000, 3_140_000,
];

/// Foxhound's 20 runs on macOS arm64, the highest exactly 10 MiB.
const FOXHOUND_RUNS: [u64; 20] = [
    9_000_000, 9_100_000, 9_200_000, 9_300_000, 9_400_000, 9_500_000, 9_600_000, 9_700_000,
    9_800_000, 9_900_000, 10_000_000, 10_100_000, 10_200_000, 10_300_000, 10_400_000, 10_485_760,
    10_000_000, 9_000_000, 9_500_000, 9_900_000,
];

fn runs(product: &str, bytes: &[u64]) -> Vec<String> {
    bytes
        .iter()
        .map(|bytes| peak(product, "macos-arm64", "idle-attached-engine", *bytes))
        .collect()
}

#[test]
fn propose_gives_one_and_a_half_times_the_highest_of_twenty_peaks_rounded_up() {
    let workspace = Workspace::from_fixture("empty");
    write_peaks(
        &workspace,
        &[
            runs("collie", &COLLIE_RUNS),
            runs("foxhound", &FOXHOUND_RUNS),
        ]
        .concat(),
    );

    let outcome = workspace.xtask(&["rss", "propose", "--peaks", "target/rss/peaks.jsonl"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    insta::assert_snapshot!(outcome.stdout, @"
    # Proposed by `xtask rss propose`: 1.5 times the highest peak of 20 runs, rounded up to a whole MiB (ADR 0071).

    [collie.macos-arm64]
    idle-attached-engine = 6

    [foxhound.macos-arm64]
    idle-attached-engine = 15
    ");
}

#[test]
fn propose_refuses_fewer_than_twenty_runs() {
    let workspace = Workspace::from_fixture("empty");
    write_peaks(
        &workspace,
        &[
            runs("collie", &COLLIE_RUNS),
            runs("foxhound", &FOXHOUND_RUNS[..19]),
        ]
        .concat(),
    );

    let outcome = workspace.xtask(&["rss", "propose", "--peaks", "target/rss/peaks.jsonl"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "rss-runs",
            "foxhound idle-attached-engine macos-arm64"
        )],
        "{}",
        outcome.stderr
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

/// A child that holds `bytes` of memory, touched, before it exits: `dd`
/// reads one block of that size from `/dev/zero`.
#[cfg(unix)]
fn allocating(bytes: u64) -> Vec<String> {
    ["dd", "if=/dev/zero", "of=/dev/null", "count=1"]
        .into_iter()
        .map(str::to_owned)
        .chain([format!("bs={bytes}")])
        .collect()
}

/// A child that holds `bytes` of memory, touched, before it exits: a
/// PowerShell byte array written once per page.
#[cfg(windows)]
fn allocating(bytes: u64) -> Vec<String> {
    vec![
        "powershell".to_owned(),
        "-NoProfile".to_owned(),
        "-Command".to_owned(),
        format!(
            "$b = New-Object byte[] {bytes}; for ($i = 0; $i -lt $b.Length; $i += 4096) {{ $b[$i] = 1 }}"
        ),
    ]
}

/// Seam B: the peak reader, through the probe that runs one child.
#[rstest]
#[case::a_quarter_gibibyte(256 * MIB, 256 * MIB, 1024 * MIB)]
#[case::one_kibibyte(1024, 0, 256 * MIB)]
fn the_peak_reader_returns_a_known_allocations_order_of_magnitude(
    #[case] allocated: u64,
    #[case] at_least: u64,
    #[case] under: u64,
) {
    let workspace = Workspace::from_fixture("empty");
    let command = allocating(allocated);
    let mut arguments = vec!["rss", "probe", "--"];
    arguments.extend(command.iter().map(String::as_str));

    let outcome = workspace.xtask(&arguments);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let peak = probed(&outcome, "peak-bytes").unwrap();
    assert!(
        (at_least..under).contains(&peak),
        "{peak} bytes, outside {at_least}..{under}"
    );
}

/// The figure the probe printed as `<name>=<n>`.
fn probed(outcome: &Outcome, name: &str) -> Option<u64> {
    outcome
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
        .map(|figure| figure.parse().unwrap())
}

/// Seam B on Linux: beside the peak, the probe reads the executable's own
/// file-backed pages from the child's mappings while it runs: some, and
/// far fewer than a peak that allocated 64 MiB.
#[cfg(target_os = "linux")]
#[test]
fn the_probe_reads_the_executables_own_pages_apart_from_the_peak() {
    let workspace = Workspace::from_fixture("empty");
    // `dd` holds its 64 MiB block for the time it takes to read it; `sh`
    // keeps it alive for the sampler to read it once in between.
    let outcome = workspace.xtask(&[
        "rss",
        "probe",
        "--",
        "sh",
        "-c",
        "dd if=/dev/zero of=/dev/null count=1 bs=67108864 && sleep 0.3",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let peak = probed(&outcome, "peak-bytes").unwrap();
    let executable = probed(&outcome, "executable-bytes").unwrap();
    assert!(
        executable > 0 && executable < 8 * MIB && peak >= 64 * MIB,
        "{executable} of {peak} bytes"
    );
}

/// `cat` answers `hello` with `hello` itself, which no engine sends.
#[cfg(unix)]
#[test]
fn the_idle_attached_engine_scenario_fails_on_a_child_that_does_not_accept_hello() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = workspace.xtask(&[
        "rss",
        "probe",
        "--scenario",
        "idle-attached-engine",
        "--needs",
        "collie",
        "--",
        "cat",
    ]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "");
}

/// An engine stops on a Goodfetch variable it does not know, so a stray one
/// of the shell running the check never reaches it. The child here answers
/// `hello` only when no such variable reached it.
#[cfg(unix)]
#[test]
fn the_idle_attached_engine_scenario_runs_the_engine_without_the_shell_s_goodfetch_variables() {
    let workspace = Workspace::from_fixture("empty");
    let engine = r#"test -z "$GOODFETCH_TEST_STRAY" || exit 3; read hello; echo '{"jsonrpc":"2.0","id":0,"result":{}}'; cat > /dev/null"#;

    let outcome = workspace.xtask_with_env(
        &[
            "rss",
            "probe",
            "--scenario",
            "idle-attached-engine",
            "--needs",
            "collie",
            "--",
            "sh",
            "-c",
            engine,
        ],
        &[("GOODFETCH_TEST_STRAY", std::ffi::OsStr::new("1"))],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        outcome.stdout.starts_with("peak-bytes="),
        "{}",
        outcome.stdout
    );
}

#[test]
fn measure_refuses_a_manifest_without_an_executable_the_scenario_runs_on() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "target/test-tools.json",
        b"{\"collie\": \"/nonexistent/goodfetch-collie\"}\n",
    );

    let outcome = workspace.xtask(&[
        "rss",
        "measure",
        "--tools",
        "target/test-tools.json",
        "--out",
        "target/rss/peaks.jsonl",
    ]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert_eq!(workspace.read("target/rss/peaks.jsonl"), None);
}

/// The test tool manifest of the commit's executables the scenarios run,
/// which `just test` builds, as `xtask rss measure` reads it.
fn real_tools(workspace: &Workspace) {
    let tools = goodfetch_test_support::tools().unwrap();
    let manifest = serde_json::json!({
        "collie": tools.path(Tool::Collie).unwrap(),
        "foxhound": tools.path(Tool::Foxhound).unwrap(),
        "proving_ground": tools.path(Tool::ProvingGround).unwrap(),
    });
    workspace.write("target/test-tools.json", manifest.to_string().as_bytes());
}

fn measure(workspace: &Workspace, scenario: &str) -> Outcome {
    workspace.xtask(&[
        "rss",
        "measure",
        scenario,
        "--tools",
        "target/test-tools.json",
        "--out",
        "target/rss/peaks.jsonl",
    ])
}

/// The peaks file's lines as (Product, scenario, bytes).
fn measured(workspace: &Workspace) -> Vec<(String, String, u64)> {
    workspace
        .read_text("target/rss/peaks.jsonl")
        .lines()
        .map(|line| {
            let peak: serde_json::Value = serde_json::from_str(line).unwrap();
            (
                peak["product"].as_str().unwrap().to_owned(),
                peak["scenario"].as_str().unwrap().to_owned(),
                peak["bytes"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn subjects(peaks: &[(String, String, u64)]) -> Vec<(&str, &str)> {
    peaks
        .iter()
        .map(|(product, scenario, _)| (product.as_str(), scenario.as_str()))
        .collect()
}

/// Each Product's Daemon is started, waited for until it prints its
/// runtime file, stopped over its own endpoint and measured once it exited:
/// the probe fails on a Daemon that is not stopped cleanly.
#[test]
fn the_idle_daemon_scenario_measures_each_products_daemon_once_stopped() {
    let workspace = Workspace::from_fixture("empty");
    real_tools(&workspace);

    let outcome = measure(&workspace, "idle-daemon");

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let peaks = measured(&workspace);
    assert_eq!(
        subjects(&peaks),
        [("collie", "idle-daemon"), ("foxhound", "idle-daemon")]
    );
    // A running Daemon holds well over a mebibyte.
    assert!(peaks.iter().all(|(_, _, bytes)| *bytes > MIB), "{peaks:?}");
}

/// A child that prints no runtime file is no Daemon to measure.
#[cfg(unix)]
#[test]
fn the_idle_daemon_scenario_fails_on_a_child_that_prints_no_runtime_file() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = workspace.xtask(&[
        "rss",
        "probe",
        "--scenario",
        "idle-daemon",
        "--needs",
        "collie",
        "--",
        "sh",
        "-c",
        "echo not-a-runtime-file",
    ]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "");
}

/// How long a probe whose step failed may take to return: well under the
/// stand-in Daemon's own life, so a probe that waits for the Daemon to end
/// by itself fails here, and well over the scenario's idle time and the
/// Daemon's stop deadline.
#[cfg(unix)]
const PROBE_RETURNS_WITHIN: Duration = Duration::from_secs(60);

/// How long a stand-in Daemon lives, in seconds, unless it is killed.
#[cfg(unix)]
const STAND_IN_LIFE: u64 = 120;

/// Runs `xtask rss probe` of the idle-Daemon scenario on a stand-in that
/// prints `runtime_file` and then lives on, as a keep-alive Daemon does,
/// and returns its outcome and how long it took.
#[cfg(unix)]
fn probe_a_live_stand_in(
    workspace: &Workspace,
    runtime_file: &serde_json::Value,
) -> (Outcome, Duration) {
    let script = format!("printf '%s\\n' '{runtime_file}'; exec sleep {STAND_IN_LIFE}");
    let started = Instant::now();
    let outcome = workspace.xtask(&[
        "rss",
        "probe",
        "--scenario",
        "idle-daemon",
        "--needs",
        "collie",
        "--",
        "sh",
        "-c",
        &script,
    ]);
    (outcome, started.elapsed())
}

/// A stand-in's runtime file: its endpoint the Unix socket at `socket`.
#[cfg(unix)]
fn runtime_file(socket: &std::path::Path) -> serde_json::Value {
    serde_json::json!({
        "endpoint": { "unix": socket },
        "token": "t",
        "version": "0.0.0",
        "pid": 1,
    })
}

/// A Daemon whose endpoint refuses the probe's connection is still running:
/// the probe stops it and reports the failure, rather than wait for it to
/// end by itself, which a keep-alive Daemon never does.
#[cfg(unix)]
#[test]
fn the_idle_daemon_probe_stops_a_live_daemon_whose_connection_failed() {
    let workspace = Workspace::from_fixture("empty");

    let (outcome, took) = probe_a_live_stand_in(
        &workspace,
        &runtime_file(&workspace.path().join("absent.sock")),
    );

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("connecting to the Daemon"),
        "{}",
        outcome.stderr
    );
    assert!(took < PROBE_RETURNS_WITHIN, "the probe took {took:?}");
}

/// A Daemon that answers `daemon.stop` but never exits fails the probe once
/// the stop deadline passed, and is killed.
#[cfg(unix)]
#[test]
fn the_idle_daemon_probe_fails_on_a_daemon_that_does_not_exit_once_stopped() {
    let workspace = Workspace::from_fixture("empty");
    let socket = workspace.path().join("daemon.sock");
    let _answering = answer_every_line(&socket);

    let (outcome, took) = probe_a_live_stand_in(&workspace, &runtime_file(&socket));

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("did not exit"),
        "{}",
        outcome.stderr
    );
    assert!(took < PROBE_RETURNS_WITHIN, "the probe took {took:?}");
}

/// A stand-in Daemon's endpoint at `socket`, which answers each line of
/// each connection with a success, `hello` and `daemon.stop` alike.
#[cfg(unix)]
#[expect(
    clippy::disallowed_types,
    reason = "a test's stand-in for the Daemon binds its own Unix socket; ADR 0060's sockets govern product code"
)]
fn answer_every_line(socket: &std::path::Path) -> std::os::unix::net::UnixListener {
    use std::io::{BufRead as _, BufReader, Write as _};

    let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
    let serving = listener.try_clone().unwrap();
    std::thread::spawn(move || {
        for stream in serving.incoming() {
            let Ok(mut stream) = stream else { return };
            let reader = BufReader::new(stream.try_clone().unwrap());
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let answer = serde_json::json!({
                    "jsonrpc": "2.0", "id": request["id"], "result": {},
                });
                if writeln!(stream, "{answer}").is_err() {
                    break;
                }
            }
        }
    });
    listener
}

/// Two idle-Daemon scenarios run at once, each Daemon in a home of its own:
/// neither holds anything the other needs.
#[test]
fn two_idle_daemon_scenarios_run_at_once() {
    let workspace = Workspace::from_fixture("empty");
    let tools = goodfetch_test_support::tools().unwrap();
    let collie = tools.path(Tool::Collie).unwrap();
    let probe = || {
        workspace
            .command(env!("CARGO_BIN_EXE_xtask"))
            .args([
                "rss",
                "probe",
                "--scenario",
                "idle-daemon",
                "--needs",
                "collie",
                "--",
            ])
            .arg(collie)
            .args(["daemon", "launched", "--keep-alive"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };

    let probes = [probe(), probe()];

    for probe in probes {
        let output = probe.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with("peak-bytes="),
            "{stderr}"
        );
    }
}

/// A crawl that ends complete having fetched fewer pages than the site
/// holds measures no 1,000-page crawl: the probe fails.
#[test]
fn the_crawl_probe_fails_on_a_crawl_that_fetched_fewer_pages_than_the_site_holds() {
    let workspace = Workspace::from_fixture("empty");
    let tools = goodfetch_test_support::tools().unwrap();
    let foxhound = tools.path(Tool::Foxhound).unwrap();
    let address = serve_one_page();

    let outcome = workspace.xtask(&[
        "rss",
        "probe",
        "--scenario",
        "crawl",
        "--needs",
        "foxhound",
        "--address",
        &address,
        "--",
        foxhound.to_str().unwrap(),
        "attached-engine",
    ]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(outcome.stderr.contains("1000 pages"), "{}", outcome.stderr);
}

/// A stand-in site whose first page, the 1,000-page site's, links nowhere,
/// every other path not found; returns its `<ip>:<port>`.
#[expect(
    clippy::disallowed_types,
    reason = "a test's stand-in site binds its own loopback socket; ADR 0060's sockets govern product code"
)]
fn serve_one_page() -> String {
    use std::io::{BufRead as _, BufReader, Write as _};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                continue;
            }
            // The rest of the head, up to its blank line.
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let path = request.split(' ').nth(1).unwrap_or_default();
            let (status, body) = if path == "/sites/1000/1" {
                (
                    "200 OK",
                    "<!doctype html><html><body><p>One page.</p></body></html>",
                )
            } else {
                ("404 Not Found", "")
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    address
}

/// Foxhound's executable crawls the 1,000-page site of a Proving ground
/// `measure` serves outside the probe, so the peak is the engine's alone;
/// the probe fails on a crawl that does not finish complete.
#[test]
fn the_crawl_scenario_measures_foxhounds_crawl_of_the_proving_ground() {
    let workspace = Workspace::from_fixture("empty");
    real_tools(&workspace);

    let outcome = measure(&workspace, "crawl");

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let peaks = measured(&workspace);
    assert_eq!(subjects(&peaks), [("foxhound", "crawl")]);
    assert!(peaks[0].2 > MIB, "{peaks:?}");
}

#[test]
fn the_crawl_scenario_refuses_a_manifest_without_the_proving_ground() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "target/test-tools.json",
        b"{\"collie\": \"/nonexistent/goodfetch-collie\", \"foxhound\": \"/nonexistent/goodfetch-foxhound\"}\n",
    );

    let outcome = measure(&workspace, "crawl");

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
    assert!(
        outcome.stderr.contains("Proving ground"),
        "{}",
        outcome.stderr
    );
    assert_eq!(workspace.read("target/rss/peaks.jsonl"), None);
}
