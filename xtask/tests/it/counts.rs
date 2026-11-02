//! `xtask counts` (ADR 0140): each row's figure against the limit the Quality
//! register holds, over a stand-in benchmark that prints what a real one does.
//! The real benchmark's run is `just counts`, and its own tests are
//! `goodfetch-foxhound`'s `in_memory_crawl`.

use std::process::Command;

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// The rows of the paths whose code does not exist yet, as the register holds
/// them.
const NOT_RUN_YET: &str = "\
| `F5.messages_no_hook` | messages per stage with no Hook on it | 0 | not run yet (#1481) | B1 | must not grow | `just counts --path F5` | implementer done |
| `F5.calls` | requests out per Hook called | 1 each | not run yet (#1481) | B1 | must not grow | `just counts --path F5` | implementer done |
| `F5.builds` | chain builds per stage | 0 | not run yet (#1481) | B1 | must not grow | `just counts --path F5` | implementer done |
| `F6.opens` | store opens per delivered batch | 0 | not run yet (#1121) | B1 | must not grow | `just counts --path F6` | implementer done |
| `F6.transactions` | transactions per delivered batch | 1 | not run yet (#1121) | B1 | must not grow | `just counts --path F6` | implementer done |
| `F6.writer_reads` | writer reads per delivered batch | 0 | not run yet (#1121) | B1 | must not grow | `just counts --path F6` | implementer done |
";

/// A register whose `F1.parses` holds `today` under a contract of 1.
fn register(today: &str) -> String {
    format!(
        "# Testing\n\n## Quality register\n\n### Measured, not yet enforced\n\n\
         | Row | Metric, per unit | Contract | Today | Benchmark | Direction | Checked by | Runs at |\n\
         |---|---|---|---|---|---|---|---|\n\
         | `F1.parses` | HTML parses per page | ≤ 1 (0 streaming) | {today} | B1 | must not grow | `xtask counts` | implementer done |\n\
         {NOT_RUN_YET}"
    )
}

/// A benchmark that prints each `(cell, count)` of 13 pages, and one more parse
/// per page in each when it is told to plant `F1.parses`, unless `blind`.
fn benchmark(cells: &[(&str, u64)], blind: bool) -> String {
    let lines: Vec<String> = cells
        .iter()
        .map(|(cell, count)| {
            format!(
                "echo '{{\"cell\":\"{cell}\",\"units\":{{\"pages\":13}},\"counts\":{{\"F1.parses\":'$(({count} + extra))'}}}}'"
            )
        })
        .collect();
    let plant = if blind {
        ""
    } else {
        "[ \"$2\" = F1.parses ] && extra=13\n"
    };
    format!("#!/bin/sh\nextra=0\n{plant}{}\n", lines.join("\n"))
}

fn workspace(today: &str, script: &str) -> (Workspace, String) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("TESTING.md", register(today).as_bytes());
    workspace.write("benchmark.sh", script.as_bytes());
    let path = workspace.path().join("benchmark.sh");
    let executable = format!("B1={}", path.display());
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    (workspace, executable)
}

#[test]
fn a_figure_at_its_limit_passes_and_is_printed() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 104)], false));

    let outcome = workspace.xtask(&["counts", "--row", "F1.parses", "--executable", &executable]);

    assert_eq!(
        (outcome.code, outcome.findings()),
        (0, vec![]),
        "{}",
        outcome.stderr
    );
    assert!(
        outcome
            .stdout
            .contains("F1.parses [default]: 8 per page, limit 8 (104 over 13)"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn a_planted_parse_fails_naming_the_row_its_figure_and_its_limit() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 104)], false));

    let outcome = workspace.xtask(&[
        "counts",
        "--row",
        "F1.parses",
        "--executable",
        &executable,
        "--plant",
        "F1.parses",
    ]);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), vec![finding("over-limit", "F1.parses")]);
    assert!(
        outcome
            .stdout
            .contains("9 per page in the cell `default`, over its limit of 8"),
        "{}",
        outcome.stdout
    );
}

/// A row with no measured figure yet is held to its Contract; one measured, to
/// the figure, zero tolerance, so one parse more than a whole page fails.
#[rstest]
#[case::not_measured_within_its_contract("after #1583", 13, 0)]
#[case::not_measured_over_its_contract("after #1583", 14, 1)]
#[case::measured_at_its_figure("8", 104, 0)]
#[case::measured_one_parse_over("8", 105, 1)]
#[case::measured_with_decimals("1.077", 14, 0)]
#[case::measured_with_decimals_one_over("1.077", 15, 1)]
fn the_limit_is_todays_figure_else_the_contract(
    #[case] today: &str,
    #[case] count: u64,
    #[case] code: i32,
) {
    let (workspace, executable) = workspace(today, &benchmark(&[("default", count)], false));

    let outcome = workspace.xtask(&["counts", "--row", "F1.parses", "--executable", &executable]);

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
}

#[test]
fn a_cell_with_a_feature_off_is_held_to_the_same_limit() {
    let (workspace, executable) = workspace(
        "8",
        &benchmark(&[("default", 104), ("a_feature_off", 105)], false),
    );

    let outcome = workspace.xtask(&["counts", "--row", "F1.parses", "--executable", &executable]);

    assert_eq!(outcome.findings(), vec![finding("over-limit", "F1.parses")]);
    assert!(
        outcome.stdout.contains("`a_feature_off`"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn a_benchmark_that_did_none_of_the_work_fails() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 0)], false));

    let outcome = workspace.xtask(&["counts", "--row", "F1.parses", "--executable", &executable]);

    assert_eq!(
        outcome.findings(),
        vec![finding("work-not-done", "F1.parses")]
    );
}

#[test]
fn proving_passes_when_each_counter_sees_its_plant() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 104)], false));

    let outcome = workspace.xtask(&[
        "counts",
        "--row",
        "F1.parses",
        "--executable",
        &executable,
        "--prove",
    ]);

    assert_eq!(
        (outcome.code, outcome.findings()),
        (0, vec![]),
        "{}",
        outcome.stderr
    );
}

#[test]
fn proving_fails_a_counter_blind_to_its_plant() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 104)], true));

    let outcome = workspace.xtask(&[
        "counts",
        "--row",
        "F1.parses",
        "--executable",
        &executable,
        "--prove",
    ]);

    assert_eq!(
        outcome.findings(),
        vec![finding("counter-blind", "F1.parses")]
    );
}

#[test]
fn a_register_without_the_row_reaches_no_verdict() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 104)], false));
    workspace.write("TESTING.md", b"# Testing\n\n## Quality register\n");

    let outcome = workspace.xtask(&["counts", "--row", "F1.parses", "--executable", &executable]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
}

/// A register with B3's rows, its counted rows at their first figures.
const B3_REGISTER: &str = "# Testing\n\n## Quality register\n\n### Measured, not yet enforced\n\n\
     | Row | Metric, per unit | Contract | Today | Benchmark | Direction | Checked by | Runs at |\n\
     |---|---|---|---|---|---|---|---|\n\
     | `E1.deep_copies` | deep copies per incoming message | 0 | 1.1 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E1.encodes` | encodes per answer or event | 1 | 2 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E1.instructions` | instructions per message, small params | 1.10 × the figure after #1577 (ADR 0134) | 1000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.instructions_large` | instructions per message, large params | 1.10 × the figure after #1577 (ADR 0134) | 5000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.allocations` | allocations per message, small params | 1.10 × the figure after #1577 (ADR 0134) | 1000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.allocations_large` | allocations per message, large params | 1.10 × the figure after #1577 (ADR 0134) | 5000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.bytes_allocated` | bytes per message, small params | 1.10 × the figure after #1577 (ADR 0134) | 1000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.bytes_allocated_large` | bytes per message, large params | 1.10 × the figure after #1577 (ADR 0134) | 5000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n";

fn executable_script(workspace: &Workspace, name: &str, script: &str) -> std::path::PathBuf {
    workspace.write(name, script.as_bytes());
    let path = workspace.path().join(name);
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    path
}

/// B3's counted rows each divide by their own outcome: copies by the messages
/// in, encodes by the messages out.
#[rstest]
#[case::each_within_its_limit(11, 20, 0, &[])]
#[case::copies_over_per_message_in(12, 20, 1, &["E1.deep_copies"])]
#[case::encodes_over_per_message_out(11, 21, 1, &["E1.encodes"])]
fn each_b3_row_divides_by_the_outcome_that_did_its_work(
    #[case] copies: u64,
    #[case] encodes: u64,
    #[case] code: i32,
    #[case] over: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("TESTING.md", B3_REGISTER.as_bytes());
    let path = executable_script(
        &workspace,
        "benchmark.sh",
        &format!(
            "#!/bin/sh\necho '{{\"cell\":\"default\",\"units\":{{\"incoming_messages\":10,\"outgoing_messages\":10}},\"counts\":{{\"E1.deep_copies\":{copies},\"E1.encodes\":{encodes}}}}}'\n"
        ),
    );

    let outcome = workspace.xtask(&[
        "counts",
        "--family",
        "B3",
        "--executable",
        &format!("B3={}", path.display()),
    ]);

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        over.iter()
            .map(|row| finding("over-limit", row))
            .collect::<Vec<_>>()
    );
}

/// One bench target's gungraun summaries: for each cell, `no_messages` at 500
/// and each scenario `per_message` over it per message of a hundred.
fn summaries(bench: &str, metrics: &[&str], small: u64, large: u64) -> String {
    let mut lines = Vec::new();
    for suffix in ["", "_log_off"] {
        for (id, per_message) in [
            ("no_messages", 0),
            ("small_params", small),
            ("large_params", large),
        ] {
            let figures: Vec<String> = metrics
                .iter()
                .map(|metric| {
                    format!(
                        "\"{metric}\":{{\"values\":{{\"new\":{}}}}}",
                        500 + per_message * 100
                    )
                })
                .collect();
            lines.push(format!(
                "{{\"version\":\"7\",\"module_path\":\"{bench}::engine::dispatch_messages\",\"id\":\"{id}{suffix}\",\"package_dir\":\"/x/goodfetch-engine\",\"profiles\":[{{\"data\":{{\"total\":{{\"metrics\":{{{}}}}}}}}}]}}",
                figures.join(",")
            ));
        }
    }
    lines.join("\n")
}

/// A stand-in `cargo` whose `bench` prints each target's summaries, `small`
/// and `large` per message, and `planted` more when told to plant anything.
fn stand_in_cargo(small: u64, large: u64, planted: u64) -> String {
    let target = |bench: &str, metrics: &[&str]| {
        format!(
            "case \"$*\" in *\"--bench {bench}\"*)\n  if [ -n \"$GOODFETCH_COUNTS_PLANT\" ]; then cat <<'END'\n{}\nEND\n  else cat <<'END'\n{}\nEND\n  fi ;;\nesac\n",
            summaries(bench, metrics, small + planted, large + planted),
            summaries(bench, metrics, small, large),
        )
    };
    format!(
        "#!/bin/sh\n{}{}",
        target("instructions", &["Ir"]),
        target("heap", &["TotalBlocks", "TotalBytes"])
    )
}

fn valgrind_workspace(script: &str) -> (Workspace, std::path::PathBuf) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("TESTING.md", B3_REGISTER.as_bytes());
    let cargo = executable_script(&workspace, "cargo.sh", script);
    (workspace, cargo)
}

/// Each figure is the difference with the run of no messages over a hundred
/// messages, and may pass its limit by 0.5 % at most.
#[rstest]
#[case::at_its_limit(1000, 5000, 0, &[])]
#[case::within_its_tolerance(1005, 5025, 0, &[])]
#[case::small_over_its_tolerance(1006, 5000, 1, &["E1.instructions", "E1.allocations", "E1.bytes_allocated"])]
#[case::large_over_its_tolerance(1000, 5026, 1, &["E1.instructions_large", "E1.allocations_large", "E1.bytes_allocated_large"])]
fn a_valgrind_row_is_read_from_its_benchmark_within_its_tolerance(
    #[case] small: u64,
    #[case] large: u64,
    #[case] code: i32,
    #[case] over: &[&str],
) {
    let (workspace, cargo) = valgrind_workspace(&stand_in_cargo(small, large, 0));

    let outcome = workspace.xtask_with_env(
        &["counts", "--family", "B3", "--valgrind"],
        &[("CARGO", cargo.as_os_str())],
    );

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
    assert!(
        outcome.stdout.contains("E1.instructions [log_off]: "),
        "{}",
        outcome.stdout
    );
    let mut found = outcome.findings();
    found.dedup();
    assert_eq!(
        found,
        over.iter()
            .map(|row| finding("over-limit", row))
            .collect::<Vec<_>>()
    );
}

/// A row with no plain number in its Contract and no figure yet has no limit:
/// the `1.10` of `1.10 × the figure` is no count of instructions.
#[test]
fn a_contract_with_no_plain_number_gives_no_limit_until_a_figure() {
    let (workspace, cargo) = valgrind_workspace(&stand_in_cargo(1, 1, 0));
    let register = B3_REGISTER.replace(
        "small params | 1.10 × the figure after #1577 (ADR 0134) | 1000 |",
        "small params | 1.10 × the figure after #1577 (ADR 0134) | after #1583 |",
    );
    workspace.write("TESTING.md", register.as_bytes());

    let outcome = workspace.xtask_with_env(
        &["counts", "--family", "B3", "--valgrind"],
        &[("CARGO", cargo.as_os_str())],
    );

    assert_eq!(outcome.code, 2, "{}\n{}", outcome.stdout, outcome.stderr);
    assert!(
        outcome.stderr.contains("no row E1.instructions"),
        "{}",
        outcome.stderr
    );
}

#[test]
fn proving_a_valgrind_row_expects_the_planted_unit_to_fail_it() {
    let (workspace, cargo) = valgrind_workspace(&stand_in_cargo(1, 1, 100_000));

    let outcome = workspace.xtask_with_env(
        &["counts", "--family", "B3", "--valgrind", "--prove"],
        &[("CARGO", cargo.as_os_str())],
    );

    assert_eq!(
        (outcome.code, outcome.findings()),
        (0, vec![]),
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome
            .stdout
            .contains("E1.instructions_large: one planted unit fails"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn proving_fails_a_valgrind_row_blind_to_its_plant() {
    let (workspace, cargo) = valgrind_workspace(&stand_in_cargo(1, 1, 0));

    let outcome = workspace.xtask_with_env(
        &["counts", "--family", "B3", "--valgrind", "--prove"],
        &[("CARGO", cargo.as_os_str())],
    );

    assert!(
        outcome
            .findings()
            .contains(&finding("counter-blind", "E1.allocations")),
        "{}",
        outcome.stdout
    );
}

/// The real register: each row of a path whose code does not exist yet has its
/// limit and a command that says it has not run, with nothing to build.
#[rstest]
#[case::hook_stage("F5", &[
    "F5.messages_no_hook: not run yet, limit 0, until #1481 builds its path",
    "F5.calls: not run yet, limit 1, until #1481 builds its path",
    "F5.builds: not run yet, limit 0, until #1481 builds its path",
])]
#[case::sink_delivery("F6", &[
    "F6.opens: not run yet, limit 0, until #1121 builds its path",
    "F6.transactions: not run yet, limit 1, until #1121 builds its path",
    "F6.writer_reads: not run yet, limit 0, until #1121 builds its path",
])]
fn a_path_not_built_yet_reports_its_rows_as_not_run(#[case] path: &str, #[case] lines: &[&str]) {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["counts", "--path", path])
        .current_dir(repository)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout.lines().collect::<Vec<_>>(), lines);
}

#[test]
fn a_whole_run_holds_the_counted_rows_and_reports_the_rest_as_not_run() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 105)], false));

    let outcome = workspace.xtask(&["counts", "--executable", &executable]);

    assert_eq!(outcome.findings(), vec![finding("over-limit", "F1.parses")]);
    assert!(
        outcome
            .stdout
            .contains("F6.opens: not run yet, limit 0, until #1121 builds its path"),
        "{}",
        outcome.stdout
    );
}

#[test]
fn a_row_not_run_yet_cannot_be_planted() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 104)], false));

    let outcome = workspace.xtask(&["counts", "--executable", &executable, "--plant", "F5.calls"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(outcome.stderr.contains("#1481"), "{}", outcome.stderr);
}

#[test]
fn a_row_not_run_yet_missing_from_the_register_reaches_no_verdict() {
    let (workspace, _) = workspace("8", &benchmark(&[("default", 104)], false));
    let without = register("8").replace("| `F6.opens` |", "| `F6.renamed` |");
    workspace.write("TESTING.md", without.as_bytes());

    let outcome = workspace.xtask(&["counts", "--path", "F6"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(outcome.stderr.contains("F6.opens"), "{}", outcome.stderr);
}

/// The ticket that builds the path cannot leave its row saying it has not run.
#[test]
fn a_benchmark_that_does_the_work_of_a_row_not_run_yet_fails() {
    let script = "#!/bin/sh\n\
        echo '{\"cell\":\"default\",\"units\":{\"pages\":13},\"counts\":{\"F1.parses\":104,\"F5.calls\":13}}'\n";
    let (workspace, executable) = workspace("8", script);

    let outcome = workspace.xtask(&["counts", "--executable", &executable]);

    assert_eq!(
        outcome.findings(),
        vec![finding("row-listed-as-not-run", "F5.calls")]
    );
}

#[test]
fn proving_leaves_a_row_not_run_yet_out() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 104)], false));

    let outcome = workspace.xtask(&["counts", "--executable", &executable, "--prove"]);

    assert_eq!(
        (outcome.code, outcome.findings()),
        (0, vec![]),
        "{}",
        outcome.stderr
    );
    assert!(
        outcome.stdout.contains("F5.calls: not run yet"),
        "{}",
        outcome.stdout
    );
}
