//! `xtask counts` (ADR 0140): each row's figure against the limit the Quality
//! register holds, over a stand-in benchmark that prints what a real one does.
//! The real benchmark's run is `just counts`, and its own tests are
//! `goodfetch-foxhound`'s `in_memory_crawl`.

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// A register whose `F1.parses` holds `today` under a contract of 1.
fn register(today: &str) -> String {
    format!(
        "# Testing\n\n## Quality register\n\n### Measured, not yet enforced\n\n\
         | Row | Metric, per unit | Contract | Today | Benchmark | Direction | Checked by | Runs at |\n\
         |---|---|---|---|---|---|---|---|\n\
         | `F1.parses` | HTML parses per page | ≤ 1 (0 streaming) | {today} | B1 | must not grow | `xtask counts` | implementer done |\n"
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

    let outcome = workspace.xtask(&["counts", "--path", "F1", "--executable", &executable]);

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
        "--path",
        "F1",
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

    let outcome = workspace.xtask(&["counts", "--path", "F1", "--executable", &executable]);

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
}

#[test]
fn a_cell_with_a_feature_off_is_held_to_the_same_limit() {
    let (workspace, executable) = workspace(
        "8",
        &benchmark(&[("default", 104), ("a_feature_off", 105)], false),
    );

    let outcome = workspace.xtask(&["counts", "--path", "F1", "--executable", &executable]);

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

    let outcome = workspace.xtask(&["counts", "--path", "F1", "--executable", &executable]);

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
        "--path",
        "F1",
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
        "--path",
        "F1",
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

    let outcome = workspace.xtask(&["counts", "--path", "F1", "--executable", &executable]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
}

/// A register of F2's rows, each of contract 1, and a benchmark printing them
/// with no Effect, whose plant of `F2.effect_syncs` counts one each page.
fn effectless() -> (Workspace, String) {
    let rows = [
        "transactions",
        "syncs",
        "prepares",
        "writer_reads",
        "configuration_reads",
        "prunes",
        "effect_syncs",
    ]
    .iter()
    .map(|work| format!("| `F2.{work}` | it, per unit | 1 | after #1583 | B1 | must not grow | `xtask counts` | implementer done |"))
    .collect::<Vec<_>>()
    .join("\n");
    let register = format!(
        "# Testing\n\n## Quality register\n\n### Measured, not yet enforced\n\n\
         | Row | Metric, per unit | Contract | Today | Benchmark | Direction | Checked by | Runs at |\n\
         |---|---|---|---|---|---|---|---|\n{rows}"
    );
    let line = "{\"cell\":\"default\",\"units\":{\"pages\":13,\"attempts\":13,\"runs\":1,\"effects\":0},\"counts\":{\"F2.transactions\":13,\"F2.syncs\":13,\"F2.prepares\":0,\"F2.writer_reads\":0,\"F2.configuration_reads\":1,\"F2.prunes\":0,\"F2.effect_syncs\":'$extra'}}";
    let script =
        format!("#!/bin/sh\nextra=0\n[ \"$2\" = F2.effect_syncs ] && extra=13\necho '{line}'\n");
    let (workspace, executable) = workspace("8", &script);
    workspace.write("TESTING.md", register.as_bytes());
    (workspace, executable)
}

/// A row whose unit the fixture has none of, such as an Effect's until the
/// Effect journal lands: nothing done passes and says so, and a planted unit
/// still fails, naming the row.
#[test]
fn a_row_with_no_units_passes_at_zero_and_fails_when_planted() {
    let (workspace, executable) = effectless();

    let clean = workspace.xtask(&["counts", "--path", "F2", "--executable", &executable]);
    let planted = workspace.xtask(&[
        "counts",
        "--path",
        "F2",
        "--plant",
        "F2.effect_syncs",
        "--executable",
        &executable,
    ]);

    assert_eq!(
        (clean.code, clean.findings()),
        (0, vec![]),
        "{}",
        clean.stderr
    );
    assert!(
        clean
            .stdout
            .contains("F2.effect_syncs [default]: no effects in this cell"),
        "{}",
        clean.stdout
    );
    assert_eq!(planted.code, 1, "{}", planted.stderr);
    assert_eq!(
        planted.findings(),
        vec![finding("over-limit", "F2.effect_syncs")]
    );
    assert!(
        planted
            .stdout
            .contains("13 with no effects in the cell `default`, over its limit of 1"),
        "{}",
        planted.stdout
    );
}
