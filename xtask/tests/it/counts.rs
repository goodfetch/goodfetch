//! `xtask counts` (ADR 0140): each row's figure against the limit the Quality
//! register holds, over a stand-in benchmark that prints what a real one does,
//! run as the family B1 so that no other family's benchmark is built.
//! The real benchmark's run is `just counts`, and its own tests are
//! `goodfetch-foxhound`'s `in_memory_crawl`.

use std::path::Path;
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
| `C6.file_opens` | files opened per Live view frame | 0 | not run yet (#959) | B2 | must not grow | `just counts --path C6` | implementer done |
| `C6.compilations` | compilations per Live view frame | 0 | not run yet (#959) | B2 | must not grow | `just counts --path C6` | implementer done |
| `C6.spawns` | tasks spawned per Live view frame | 0 | not run yet (#959) | B2 | must not grow | `just counts --path C6` | implementer done |
| `C6.acknowledgements` | acknowledgements per Live view frame | 1 | not run yet (#959) | B2 | must not grow | `just counts --path C6` | implementer done |
| `C6.seals` | seals per fragment of a relayed viewer's frame | 1 | not run yet (#959) | B2 | must not grow | `just counts --path C6` | implementer done |
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

/// The register is found by its heading line, the part `xtask affected`
/// reads it as: prose that quotes the heading earlier in the file is no
/// register.
#[test]
fn the_register_is_found_by_its_heading_line_not_a_quote_of_it() {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", 104)], false));
    let quoted = register("8").replace(
        "# Testing\n\n",
        "# Testing\n\n## Commands\n\nA reader names `\"## Quality register\"`.\n\n",
    );
    workspace.write("TESTING.md", quoted.as_bytes());

    let outcome = workspace.xtask(&["counts", "--row", "F1.parses", "--executable", &executable]);

    assert_eq!(
        (outcome.code, outcome.findings()),
        (0, vec![]),
        "{}",
        outcome.stderr
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

/// A register whose `F1.parses` was promoted to the Enforced section's table
/// of counted work, with `today` as its figure.
fn promoted_register(today: &str) -> String {
    format!(
        "# Testing\n\n## Quality register\n\n### Enforced\n\n\
         | Dimension | Rule | Checked by | Runs at |\n\
         |---|---|---|---|\n\
         | Rust lints | 0 warnings | `just lint` | merge gate |\n\n\
         #### Counted work\n\n\
         | Row | Metric, per unit | Contract | Today | Benchmark | Direction | Checked by | Runs at |\n\
         |---|---|---|---|---|---|---|---|\n\
         | `F1.parses` | HTML parses per page | ≤ 1 (0 streaming) | {today} | B1 | must not grow | `just counts` | implementer done, merge gate |\n\n\
         ### Measured, not yet enforced\n\n\
         | Row | Metric, per unit | Contract | Today | Benchmark | Direction | Checked by | Runs at |\n\
         |---|---|---|---|---|---|---|---|\n\
         {NOT_RUN_YET}"
    )
}

/// A row the first measure promoted to Enforced keeps its figure as its limit.
#[rstest]
#[case::at_its_figure(104, 0)]
#[case::one_parse_over(105, 1)]
fn a_row_promoted_to_enforced_is_held_to_its_figure(#[case] count: u64, #[case] code: i32) {
    let (workspace, executable) = workspace("8", &benchmark(&[("default", count)], false));
    workspace.write("TESTING.md", promoted_register("8").as_bytes());

    let outcome = workspace.xtask(&["counts", "--row", "F1.parses", "--executable", &executable]);

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
    assert!(
        outcome.stdout.contains("limit 8 ("),
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
}

/// The rows left in the Measured table are read beside the promoted ones.
#[test]
fn a_row_left_in_the_measured_table_is_read_beside_the_promoted_ones() {
    let (workspace, _) = workspace("8", &benchmark(&[("default", 104)], false));
    workspace.write("TESTING.md", promoted_register("8").as_bytes());

    let outcome = workspace.xtask(&["counts", "--path", "F5"]);

    assert_eq!(outcome.code, 0, "{}\n{}", outcome.stdout, outcome.stderr);
    assert!(
        outcome
            .stdout
            .contains("F5.calls: not run yet, limit 1, until #1481 builds its path"),
        "{}",
        outcome.stdout
    );
}

/// An enforced row of the register's counted work that `xtask counts` has no
/// counter for would go uncounted, and pass.
#[test]
fn an_enforced_row_with_no_counter_fails_naming_it() {
    let (workspace, _) = workspace("8", &benchmark(&[("default", 104)], false));
    let register = promoted_register("8").replace(
        "| implementer done, merge gate |\n\n",
        "| implementer done, merge gate |\n\
         | `F1.uncounted` | parses no counter sees per page | 0 | 0 | B1 | must not grow | `just counts` | implementer done, merge gate |\n\n",
    );
    workspace.write("TESTING.md", register.as_bytes());

    let outcome = workspace.xtask(&["counts", "--row", "F1.uncounted"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("row-without-counter", "F1.uncounted")],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
}

/// A counter of `xtask counts` whose row the register lacks fails, naming it.
#[test]
fn a_counter_whose_row_the_register_lacks_fails_naming_it() {
    let (workspace, executable) = whole_b1(104, "");
    let rows = OTHER_B1_ROWS.replace("| `F1.readability_runs` |", "| `F1.renamed` |");
    workspace.write("TESTING.md", format!("{}{rows}", register("8")).as_bytes());

    let outcome = workspace.xtask(&["counts", "--executable", &executable]);

    assert_eq!(
        outcome.findings(),
        vec![finding("counter-without-row", "F1.readability_runs")],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
}

/// So does a counter read under Valgrind, before any Valgrind run.
#[test]
fn a_valgrind_counter_whose_row_the_register_lacks_fails_naming_it() {
    let (workspace, _) = workspace("8", &benchmark(&[("default", 104)], false));

    let outcome = workspace.xtask(&["counts", "--valgrind", "--row", "E1.instructions"]);

    assert_eq!(
        outcome.findings(),
        vec![finding("counter-without-row", "E1.instructions")],
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
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
     | `E1.decodes` | decodes of its params per request | 1 | 2 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E1.instructions` | instructions per message, small params | 1.10 × the figure after #1577 (ADR 0134) | 1000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.instructions_large` | instructions per message, large params | 1.10 × the figure after #1577 (ADR 0134) | 5000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.allocations` | allocations per message, small params | 1.10 × the figure after #1577 (ADR 0134) | 1000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.allocations_large` | allocations per message, large params | 1.10 × the figure after #1577 (ADR 0134) | 5000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.bytes_allocated` | bytes per message, small params | 1.10 × the figure after #1577 (ADR 0134) | 1000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E1.bytes_allocated_large` | bytes per message, large params | 1.10 × the figure after #1577 (ADR 0134) | 5000 | B3 | must not grow (0.5 %) | `xtask counts --valgrind` | merge gate |\n\
     | `E7.merge_round_trips` | Merge host round trips per Working-copy update | 1 | not run yet (#1998) | B3 | must not grow | `just counts --path E7` | implementer done |\n\
     | `E7.document_loads` | whole-document loads per Working-copy update | 0 | not run yet (#1689) | B3 | must not grow | `just counts --path E7` | implementer done |\n\
     | `E7.document_encodes` | whole-document encodes per Working-copy update | 0 | not run yet (#1689) | B3 | must not grow | `just counts --path E7` | implementer done |\n\
     | `E7.opens` | store opens per Working-copy update | 0 | not run yet (#1689) | B3 | must not grow | `just counts --path E7` | implementer done |\n\
     | `E7.prepares` | statement preparations per Working-copy update | 0 | not run yet (#1689) | B3 | must not grow | `just counts --path E7` | implementer done |\n\
     | `E7.transactions` | transactions per Working-copy update | 1 | not run yet (#1689) | B3 | must not grow | `just counts --path E7` | implementer done |\n\
     | `E7.syncs` | syncs per Working-copy update | 0 | not run yet (#1689) | B3 | must not grow | `just counts --path E7` | implementer done |\n\
     | `E7.encodes` | encodes per Working-copy update | 1 | not run yet (#1689) | B3 | must not grow | `just counts --path E7` | implementer done |\n\
     | `E2.configuration_reads` | configuration reads per Audit entry | 0 | 1 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E2.log_opens` | store opens per Audit log | 1 | 1 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E2.log_transactions` | transactions per Audit log | ≤ 2 | 2 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E2.log_prepares` | statement preparations per Audit log | 30 | 30 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E2.opens` | store opens per Audit entry | 0 | 1 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E2.prepares` | statement preparations per Audit entry | 0 | 14 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E2.prunes` | prune passes per Audit entry | 0 | 1 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E2.transactions` | transactions per Audit entry | 1 | 2.05 | B3 | must not grow | `xtask counts` | implementer done |\n\
     | `E2.syncs` | syncs and head-file writes per Audit entry | no cap (ratchet) | 5.2 | B3 | must not grow | `xtask counts` | implementer done |\n";

fn executable_script(workspace: &Workspace, name: &str, script: &str) -> std::path::PathBuf {
    workspace.write(name, script.as_bytes());
    let path = workspace.path().join(name);
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    path
}

/// B3's counted rows each divide by their own outcome: copies by the messages
/// in, encodes by the messages out, decodes by the requests among the
/// messages in, an Audit entry's transactions by its entries, a log's store
/// opens by its logs.
#[rstest]
#[case::each_within_its_limit(11, 20, 10, 41, 1, 0, &[])]
#[case::copies_over_per_message_in(12, 20, 10, 41, 1, 1, &["E1.deep_copies"])]
#[case::encodes_over_per_message_out(11, 21, 10, 41, 1, 1, &["E1.encodes"])]
#[case::decodes_over_per_request(11, 20, 11, 41, 1, 1, &["E1.decodes"])]
#[case::transactions_over_per_audit_entry(11, 20, 10, 42, 1, 1, &["E2.transactions"])]
#[case::log_opens_over_per_audit_log(11, 20, 10, 41, 2, 1, &["E2.log_opens"])]
fn each_b3_row_divides_by_the_outcome_that_did_its_work(
    #[case] copies: u64,
    #[case] encodes: u64,
    #[case] decodes: u64,
    #[case] transactions: u64,
    #[case] log_opens: u64,
    #[case] code: i32,
    #[case] over: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("TESTING.md", B3_REGISTER.as_bytes());
    let path = executable_script(
        &workspace,
        "benchmark.sh",
        &format!(
            "#!/bin/sh\necho '{{\"cell\":\"default\",\"units\":{{\"incoming_messages\":10,\"answers_and_events\":10,\"requests\":5,\"audit_entries\":20,\"audit_logs\":1,\"later_audit_entries\":19}},\"counts\":{{\"E1.deep_copies\":{copies},\"E1.encodes\":{encodes},\"E1.decodes\":{decodes},\"E2.configuration_reads\":20,\"E2.log_opens\":{log_opens},\"E2.log_transactions\":2,\"E2.log_prepares\":30,\"E2.opens\":20,\"E2.prepares\":266,\"E2.prunes\":0,\"E2.transactions\":{transactions},\"E2.syncs\":104}}}}'\n"
        ),
    );

    let executable = format!("B3={}", path.display());
    let on_path =
        |path: &str| workspace.xtask(&["counts", "--path", path, "--executable", &executable]);
    let (messages, audit) = (on_path("E1"), on_path("E2"));

    assert_eq!(
        messages.code.max(audit.code),
        code,
        "{}\n{}\n{}\n{}",
        messages.stdout,
        messages.stderr,
        audit.stdout,
        audit.stderr
    );
    assert_eq!(
        [messages.findings(), audit.findings()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        over.iter()
            .map(|row| finding("over-limit", row))
            .collect::<Vec<_>>()
    );
    assert!(
        audit
            .stdout
            .contains("E2.opens [default]: 1 per audit_entry, limit 1 (20 over 20)"),
        "{}",
        audit.stdout
    );
}

/// B3's run holds the Working-copy update's rows, not run yet, to their
/// absence: the ticket that builds the path cannot count its work on B3 and
/// leave them saying they have not run.
#[test]
fn a_b3_benchmark_that_does_a_working_copy_updates_work_fails_its_row() {
    let workspace = Workspace::from_fixture("empty");
    // The whole family runs, so the register holds B3's Usage charge and
    // governor sample rows too.
    let register = format!(
        "{B3_REGISTER}\
         | `E3.statements` | SQL statements per charge the held Slices cover | 0 | 1 | B3 | must not grow | `just counts` | implementer done |\n\
         | `E3.slice_transactions` | transactions and syncs per Slice taken | 1 | 2.25 | B3 | must not grow | `just counts` | implementer done |\n\
         | `E3.opens` | store opens per charge or Slice | 0 | 0.125 | B3 | must not grow | `just counts` | implementer done |\n\
         | `E3.prepares` | statement preparations per Slice taken | 0 | 19.75 | B3 | must not grow | `just counts` | implementer done |\n\
         | `E4.configuration_reads` | configuration reads per governor sample | 0 | 2 | B3 | must not grow | `just counts` | implementer done |\n\
         | `E4.meminfo_opens` | `/proc/meminfo` opens per sample | ≤ 1 | 3 | B3 | must not grow | `just counts` | implementer done |\n\
         | `E4.static_opens` | `self/cgroup` and `self/mountinfo` opens per sample | 0 | 2 | B3 | must not grow | `just counts` | implementer done |\n\
         | `E4.cpu_stat_opens` | `cpu.stat` opens per sample | ≤ 1 | not run yet (#1306) | B3 | must not grow | `just counts --path E4` | implementer done |\n\
         | `E4.process_reads` | memory reads per process per sample | 1 | 1.5 | B3 | must not grow | `just counts` | implementer done |\n"
    );
    workspace.write("TESTING.md", register.as_bytes());
    let path = executable_script(
        &workspace,
        "benchmark.sh",
        "#!/bin/sh\necho '{\"cell\":\"default\",\"units\":{\"incoming_messages\":10,\"answers_and_events\":10,\"requests\":5,\"covered_charges\":4,\"slices\":4,\"charges\":8,\"audit_entries\":20,\"audit_logs\":1,\"later_audit_entries\":19,\"samples\":4,\"process_samples\":8},\"counts\":{\"E1.deep_copies\":11,\"E1.encodes\":20,\"E1.decodes\":10,\"E2.configuration_reads\":20,\"E2.log_opens\":1,\"E2.log_transactions\":2,\"E2.log_prepares\":30,\"E2.opens\":20,\"E2.prepares\":266,\"E2.prunes\":0,\"E2.transactions\":41,\"E2.syncs\":104,\"E3.statements\":4,\"E3.slice_transactions\":9,\"E3.opens\":1,\"E3.prepares\":79,\"E4.configuration_reads\":8,\"E4.meminfo_opens\":12,\"E4.static_opens\":8,\"E4.process_reads\":12,\"E7.encodes\":3}}'\n",
    );

    let outcome = workspace.xtask(&[
        "counts",
        "--family",
        "B3",
        "--executable",
        &format!("B3={}", path.display()),
    ]);

    assert_eq!(
        (outcome.code, outcome.findings()),
        (1, vec![finding("row-listed-as-not-run", "E7.encodes")]),
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    assert!(
        outcome
            .stdout
            .contains("E7.merge_round_trips: not run yet, limit 1, until #1998 builds its path"),
        "{}",
        outcome.stdout
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
/// limit and a command that says it has not run, with nothing to build. The
/// rows, their limits and their tickets are the register's and `xtask
/// counts`' own, so only their lines' shape is held here; the fixture tests
/// hold their values.
#[rstest]
#[case::hook_stage("F5")]
#[case::sink_delivery("F6")]
#[case::working_copy_update("E7")]
#[case::live_view_frame("C6")]
fn a_path_not_built_yet_reports_its_rows_as_not_run(#[case] path: &str) {
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
    // `<path>.<row>: not run yet, limit <limit>, until #<ticket> builds its path`
    let not_run = |line: &str| {
        line.strip_prefix(&format!("{path}."))
            .and_then(|line| line.split_once(": not run yet, limit "))
            .and_then(|(_, line)| line.split_once(", until #"))
            .is_some_and(|(_, line)| line.ends_with(" builds its path"))
    };
    assert!(
        !stdout.is_empty() && stdout.lines().all(not_run),
        "{stdout}"
    );
}

/// B1's counted rows other than `F1.parses`, at the figures [`whole_b1`]'s
/// benchmark gives them, so a run that names no row holds every counted row.
const OTHER_B1_ROWS: &str = "\
| `F1.selector_compilations` | selector compilations per page | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F1.readability_runs` | readability runs per page | ≤ 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F1.file_opens` | files opened per page | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F1.locale_form_builds` | locale-form builds per page | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F1.plugin_serializations` | serializations per page | ≤ 1 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F1.plugin_answer_parses` | answer parses per page | ≤ 1 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F1.tree_copies` | tree copies per page | ≤ 1 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F1.share_readability_runs` | share readability runs per page | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F1.block_classifications` | Block classifications per page | ≤ 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F1.analysis_handoffs` | analysis hand-offs per page | at most 1 | B1 2, B4 3 | B1, B4 | must not grow | `just counts` | implementer done |
| `F1.analysis_handoffs.handler` | a handler's analysis hand-offs per page | one per read | 0 | B1 | must not grow | `just counts` | implementer done |
| `F4.declaration_reads` | declaration reads per Record | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F4.validator_builds` | validator builds per Record | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F4.validations` | validations per Record | ≤ 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F4.serializations` | serializations per Record | ≤ 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F4.deep_copies` | deep copies per Record | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F4.constant_lookups` | constant lookups per Record | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F4.transactions` | transactions per Record | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F2.transactions` | its work | 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F2.syncs` | its work | 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F2.prepares` | its work | 1 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F2.writer_reads` | its work | 1 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F2.page_waits` | its work | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F2.page_waits.full` | its work | ≤ 2 | 2 | B1 | must not grow | `just counts` | implementer done |
| `F2.batches` | its work | ≤ 2 | 2 | B1 | must not grow | `just counts` | implementer done |
| `F2.evidence_reads` | its work | 1 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F2.admission_evidence` | its work | 1 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F2.page_learning` | its work | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F2.learning_off_evidence` | its work | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F2.configuration_reads` | its work | 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F2.prunes` | its work | 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F2.effect_syncs` | its work | 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.invariant_reads` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.invariant_reads.hop` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.invariant_reads.sitemap` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.compilations` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.compilations.hop` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.compilations.sitemap` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.compilations.payload` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.prepares` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.prepares.hop` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.prepares.sitemap` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.transactions` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.transactions.hop` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.transactions.sitemap` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.payload_serializations` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.validations` | its work | 0 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.actor_round_trips` | its work | 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.hop_round_trips` | its work | 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.hop_round_trips.end` | its work | 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `F3.in_flight_serializations` | its work | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `F3.counted_work_constructions` | its work | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `E5.file_opens` | folders created and files opened per entry | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `E5.spawns` | tasks spawned per entry | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `E5.formatting_off` | entries formatted per entry of a kind off | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `E6.transactions` | Politeness-file transactions per attempt | ≤ 2 | 2 | B1 | must not grow | `just counts` | implementer done |
| `E6.opens` | opens and statement preparations per permit | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `E6.script_sends` | script sends per permit | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `E6.robots_parses` | robots.txt parses per origin per Run | 1 | 1 | B1 | must not grow | `just counts` | implementer done |
| `E6.robots_reads` | reads of a robots.txt copy per request | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
| `E6.off_work` | Politeness work per attempt with Politeness and robots.txt off | 0 | 0 | B1 | must not grow | `just counts` | implementer done |
";

/// A workspace whose register holds every counted row of B1, and whose
/// benchmark prints over 13 pages and their 13 Records `parses` parses, one
/// readability run and two analysis hand-offs a page, one validation and one
/// serialization a Record, one Payload serialized and validated over one links batch, one transaction,
/// sync and configuration read over one attempt and one Run, two Politeness
/// transactions an attempt and one robots.txt parse an origin, over 13
/// Run-trace entries none of their rows' work, none of the other rows' work,
/// then `extra` as it is, and 13 more of the row it is told to plant.
fn whole_b1(parses: u64, extra: &str) -> (Workspace, String) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "TESTING.md",
        format!("{}{OTHER_B1_ROWS}", register("8")).as_bytes(),
    );
    let counts: Vec<String> = [
        ("F1.parses", parses),
        ("F1.selector_compilations", 0),
        ("F1.readability_runs", 13),
        ("F1.file_opens", 0),
        ("F1.locale_form_builds", 0),
        ("F1.plugin_serializations", 0),
        ("F1.plugin_answer_parses", 0),
        ("F1.tree_copies", 0),
        ("F1.share_readability_runs", 0),
        ("F1.block_classifications", 13),
        ("F1.analysis_handoffs", 26),
        ("F1.analysis_handoffs.handler", 0),
        ("F4.declaration_reads", 0),
        ("F4.validator_builds", 0),
        ("F4.validations", 13),
        ("F4.serializations", 13),
        ("F4.deep_copies", 0),
        ("F4.constant_lookups", 0),
        ("F4.transactions", 0),
        ("F2.transactions", 1),
        ("F2.syncs", 1),
        ("F2.prepares", 0),
        ("F2.writer_reads", 0),
        ("F2.page_waits", 0),
        ("F2.page_waits.full", 0),
        ("F2.batches", 2),
        ("F2.evidence_reads", 0),
        ("F2.admission_evidence", 0),
        ("F2.page_learning", 0),
        ("F2.learning_off_evidence", 0),
        ("F2.configuration_reads", 1),
        ("F2.prunes", 0),
        ("F2.effect_syncs", 0),
        ("F3.invariant_reads", 0),
        ("F3.invariant_reads.hop", 0),
        ("F3.invariant_reads.sitemap", 0),
        ("F3.compilations", 0),
        ("F3.compilations.hop", 0),
        ("F3.compilations.sitemap", 0),
        ("F3.compilations.payload", 0),
        ("F3.prepares", 0),
        ("F3.prepares.hop", 0),
        ("F3.prepares.sitemap", 0),
        ("F3.transactions", 0),
        ("F3.transactions.hop", 0),
        ("F3.transactions.sitemap", 0),
        ("F3.payload_serializations", 1),
        ("F3.validations", 1),
        ("F3.actor_round_trips", 1),
        ("F3.hop_round_trips", 1),
        ("F3.hop_round_trips.end", 1),
        ("F3.in_flight_serializations", 0),
        ("F3.counted_work_constructions", 0),
        ("E5.file_opens", 0),
        ("E5.spawns", 0),
        ("E5.formatting_off", 0),
        ("E6.transactions", 2),
        ("E6.opens", 0),
        ("E6.script_sends", 0),
        ("E6.robots_parses", 1),
        ("E6.robots_reads", 0),
        ("E6.off_work", 0),
    ]
    .iter()
    .map(|(row, count)| format!("\"{row}\":'$(({count} + $(planted {row})))'"))
    .collect();
    let script = format!(
        "#!/bin/sh\nplant=$2\nplanted() {{ if [ \"$1\" = \"$plant\" ]; then echo 13; else echo 0; fi; }}\n\
         echo '{{\"cell\":\"default\",\"units\":{{\"pages\":13,\"added_pages\":13,\"records\":13,\"links_batches\":1,\"hops\":1,\"chains\":1,\"sitemaps\":1,\"payloads\":1,\"attempts\":1,\"attempts_ended\":1,\"runs\":1,\"effects\":0,\"entries\":13,\"permits\":1,\"requests\":1,\"origins\":1,\"off_attempts\":0,\"learning_off_pages\":0,\"batched_pages\":13,\"full_pages\":0}},\"counts\":{{{}{extra}}}}}'\n",
        counts.join(",")
    );
    let path = executable_script(&workspace, "benchmark.sh", &script);
    let executable = format!("B1={}", path.display());
    (workspace, executable)
}

#[test]
fn a_whole_run_holds_the_counted_rows_and_reports_the_rest_as_not_run() {
    let (workspace, executable) = whole_b1(105, "");

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
    let (workspace, executable) = whole_b1(104, ",\"F5.calls\":13");

    let outcome = workspace.xtask(&["counts", "--executable", &executable]);

    assert_eq!(
        outcome.findings(),
        vec![finding("row-listed-as-not-run", "F5.calls")]
    );
}

#[test]
fn proving_leaves_a_row_not_run_yet_out() {
    let (workspace, executable) = whole_b1(104, "");

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

/// B2's rows, as the register holds them: C1's per command, C2's per read or
/// per document read, C2's two rows whose reads are not built yet, C7's
/// walks and opening commands per page opened, C4's per navigation, and C5's
/// per request, pause or refusal.
const B2_ROWS: &str = "\
| `C1.encodes` | encodes per CDP command | 1 | after #1583 | B2 | must not grow | `just counts` | implementer done |
| `C1.sends` | sends per CDP command | 1 | after #1583 | B2 | must not grow | `just counts` | implementer done |
| `C1.file_opens` | opens per CDP command | 0 | 2 | B2 | must not grow | `just counts` | implementer done |
| `C1.log_sends_off` | sends per CDP command | 0 | 1 | B2 | must not grow | `just counts` | implementer done |
| `C2.commands` | CDP commands per page read | 1 | 2 | B2 | must not grow | `just counts` | implementer done |
| `C2.world_creations` | worlds per page read, past a document's first | 0 | 1 | B2 | must not grow | `just counts` | implementer done |
| `C2.document_worlds` | worlds per document and frame read | ≤ 1 | 2 | B2 | must not grow | `just counts` | implementer done |
| `C2.serialization_commands` | CDP commands per serialization | 2 | 2 | B2 | must not grow | `just counts` | implementer done |
| `C2.closed_root_commands` | CDP commands per read in a closed root's document | ≤ 3 | not run yet (#726) | B2 | must not grow | `just counts --path C2` | implementer done |
| `C2.live_view_commands` | CDP commands per Live view read | no cap (ratchet) | not run yet (#1683) | B2 | must not grow | `just counts --path C2` | implementer done |
| `C7.pool_walks` | walks per page opened | 0 | after #1583 | B2 | must not grow | `just counts` | implementer done |
| `C7.opening_commands` | CDP commands awaited per page opened | no cap (ratchet) | 12 | B2 | must not grow | `just counts` | implementer done |
| `C4.detection_reads` | detection reads per navigation | ≤ 1 | 4 | B2 | must not grow | `just counts` | implementer done |
| `C4.compilations` | glob and regex compilations per navigation | 0 | 1 | B2 | must not grow | `just counts` | implementer done |
| `C4.script_compilations` | script compilations per navigation | 0 | after #1583 | B2 | must not grow | `just counts` | implementer done |
| `C4.provenance_entries` | Provenance entries per navigation | 1 | 1.334 | B2 | must not grow | `just counts` | implementer done |
| `C5.fetch_messages_no_rule` | `Fetch` messages per request with no rule | 0 | 3.25 | B2 | must not grow | `xtask counts` | implementer done |
| `C5.spawns` | tasks spawned per refused request | 0 | 1 | B2 | must not grow | `xtask counts` | implementer done |
| `C5.file_opens` | files opened per refused request | 0 | 2 | B2 | must not grow | `xtask counts` | implementer done |
| `C5.matches` | Filter-list matches per paused request | 1 | after #1583 | B2 | must not grow | `xtask counts` | implementer done |
| `C5.answers` | CDP answers per paused request | 1 | after #1583 | B2 | must not grow | `xtask counts` | implementer done |
| `C5.compilations` | compilations per paused request | 0 | after #1583 | B2 | must not grow | `xtask counts` | implementer done |
| `C5.entries` | entries per refused request | 1 | after #1583 | B2 | must not grow | `xtask counts` | implementer done |
| `C5.cache_lookups` | Response cache lookups per request | 1 | not run yet (#793) | B2 | must not grow | `xtask counts --path C5` | implementer done |
| `C5.cache_file_opens` | body files opened per cache hit | 1 | not run yet (#793) | B2 | must not grow | `xtask counts --path C5` | implementer done |
| `C5.body_reads` | body reads per recorded response | 1 | not run yet (#1291) | B2 | must not grow | `xtask counts --path C5` | implementer done |
| `C5.charset_refetches` | second GETs per document on the charset path | ≤ 1 | not run yet (#1783) | B2 | must not grow | `xtask counts --path C5` | implementer done |
";

/// A stand-in of B2 over 20 commands, 4 reads, 2 documents, 2 serializations,
/// 1 page opened,
/// 4 requests paused 6 times, 2 of them refused, one check and attempt of each
/// action kind and 3 navigations, each row at its limit, which counts `extra`
/// more of `planted` when told to plant it.
fn b2_benchmark(planted: &str, extra: u64) -> String {
    let counts = [
        ("C1.encodes", 20),
        ("C1.sends", 20),
        ("C1.file_opens", 30),
        ("C1.log_sends_off", 20),
        ("C2.commands", 8),
        ("C2.world_creations", 4),
        ("C2.document_worlds", 4),
        ("C2.serialization_commands", 4),
        ("C7.pool_walks", 0),
        ("C7.opening_commands", 12),
        ("C5.fetch_messages_no_rule", 13),
        ("C5.spawns", 2),
        ("C5.file_opens", 4),
        ("C5.matches", 4),
        ("C5.answers", 6),
        ("C5.compilations", 0),
        ("C5.entries", 2),
        ("C3.script_sends", 1),
        ("C3.check_commands", 1),
        ("C3.commands.click", 6),
        ("C3.commands.fill", 2),
        ("C3.commands.press", 4),
        ("C4.detection_reads", 12),
        ("C4.compilations", 0),
        ("C4.script_compilations", 0),
        ("C4.provenance_entries", 4),
    ]
    .iter()
    .map(|(row, count)| format!("\"{row}\":'$(({count} + $(planted {row})))'"))
    .collect::<Vec<_>>()
    .join(",");
    format!(
        "#!/bin/sh\nplant=$2\n\
         planted() {{ if [ \"$1\" = \"$plant\" ] && [ \"$1\" = {planted} ]; then echo {extra}; else echo 0; fi; }}\n\
         echo '{{\"cell\":\"default\",\"units\":{{\"commands\":20,\"reads\":4,\"documents\":2,\"serializations\":2,\"pages_opened\":1,\"requests\":4,\"pauses\":6,\"refusals\":2,\"checks\":1,\"click_attempts\":1,\"fill_attempts\":1,\"press_attempts\":1,\"navigations\":3}},\"counts\":{{{counts}}}}}'\n"
    )
}

/// Every F3 row, in what a benchmark prints and the register holds.
const FRONTIER_ROWS: [&str; 20] = [
    "F3.invariant_reads",
    "F3.invariant_reads.hop",
    "F3.invariant_reads.sitemap",
    "F3.compilations",
    "F3.compilations.hop",
    "F3.compilations.sitemap",
    "F3.compilations.payload",
    "F3.prepares",
    "F3.prepares.hop",
    "F3.prepares.sitemap",
    "F3.transactions",
    "F3.transactions.hop",
    "F3.transactions.sitemap",
    "F3.payload_serializations",
    "F3.validations",
    "F3.actor_round_trips",
    "F3.hop_round_trips",
    "F3.hop_round_trips.end",
    "F3.in_flight_serializations",
    "F3.counted_work_constructions",
];

/// The register's F3 rows, each with the limit 1.
fn frontier_register() -> String {
    let rows = FRONTIER_ROWS
        .map(|row| {
            format!(
                "| `{row}` | its work | 99 | 1 | B1 | must not grow | `xtask counts` | implementer done |"
            )
        })
        .join("\n");
    format!(
        "# Testing\n\n## Quality register\n\n### Measured, not yet enforced\n\n\
         | Row | Metric, per unit | Contract | Today | Benchmark | Direction | Checked by | Runs at |\n\
         |---|---|---|---|---|---|---|---|\n{rows}"
    )
}

/// A benchmark that prints every F3 count, each outcome one unit and each
/// count 1, except `counted`, whose count is 5.
fn frontier_benchmark(counted: &str) -> String {
    let counts: Vec<String> = FRONTIER_ROWS
        .iter()
        .map(|name| format!("\"{name}\":{}", if *name == counted { 5 } else { 1 }))
        .collect();
    format!(
        "#!/bin/sh\necho '{{\"cell\":\"default\",\"units\":{{\"links_batches\":1,\"hops\":1,\"chains\":1,\"sitemaps\":1,\"payloads\":1,\"attempts_ended\":1,\"added_pages\":1}},\"counts\":{{{}}}}}'\n",
        counts.join(",")
    )
}

/// A hop's work is its own row: over its limit it fails, divided by its hops
/// and not by the batches.
#[test]
fn a_hops_work_fails_its_own_row_divided_by_its_hops() {
    let (workspace, executable) = workspace("1", &frontier_benchmark("F3.compilations.hop"));
    workspace.write("TESTING.md", frontier_register().as_bytes());

    let outcome = workspace.xtask(&["counts", "--path", "F3", "--executable", &executable]);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        vec![finding("over-limit", "F3.compilations.hop")]
    );
    assert!(
        outcome
            .stdout
            .contains("F3.compilations.hop [default]: 5 per hop, limit 1 (5 over 1)"),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome
            .stdout
            .contains("5 per hop in the cell `default`, over its limit of 1"),
        "{}",
        outcome.stdout
    );
}

/// Every outcome at its limit passes, and each of the six rows is printed.
#[test]
fn the_frontiers_rows_pass_at_their_limit_and_each_is_printed() {
    let (workspace, executable) = workspace("1", &frontier_benchmark("none"));
    workspace.write("TESTING.md", frontier_register().as_bytes());

    let outcome = workspace.xtask(&["counts", "--path", "F3", "--executable", &executable]);

    assert_eq!(
        (outcome.code, outcome.findings()),
        (0, vec![]),
        "{}",
        outcome.stderr
    );
    for row in [
        "F3.invariant_reads [default]: 1 per links batch",
        "F3.compilations [default]",
        "F3.prepares [default]",
        "F3.transactions [default]",
        "F3.payload_serializations [default]",
        "F3.validations [default]",
    ] {
        assert!(outcome.stdout.contains(row), "{row}: {}", outcome.stdout);
    }
}

/// A plant reaches only its own row's family: B1's benchmark knows no plant of
/// B2's row, and fails when asked for one.
#[test]
fn a_plant_goes_to_the_family_of_its_row_alone() {
    let (workspace, _) = whole_b1(104, "");
    workspace.write(
        "TESTING.md",
        format!("{}{OTHER_B1_ROWS}{B2_ROWS}{C3_ROWS}", register("8")).as_bytes(),
    );
    let b1 = "#!/bin/sh\n[ -n \"$1\" ] && exit 3\nexec \"$(dirname \"$0\")/benchmark.sh\"\n";
    let b2 = b2_benchmark("C1.sends", 20);
    let executables: Vec<String> = [("B1", b1), ("B2", b2.as_str())]
        .iter()
        .map(|(family, script)| {
            let path = executable_script(&workspace, &format!("{family}.sh"), script);
            format!("{family}={}", path.display())
        })
        .collect();

    let outcome = workspace.xtask(&[
        "counts",
        "--executable",
        &executables[0],
        "--executable",
        &executables[1],
        "--plant",
        "C1.sends",
    ]);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), vec![finding("over-limit", "C1.sends")]);
    assert!(
        outcome
            .stdout
            .contains("C5.cache_lookups: not run yet, limit 1, until #793 builds its path"),
        "{}",
        outcome.stdout
    );
}

/// B2's benchmark holds the Live view frame's rows as not run, so the ticket
/// that gives B2 its viewer cannot leave them saying so.
#[test]
fn b2_doing_a_live_view_frames_work_fails_while_its_rows_say_not_run() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "TESTING.md",
        format!("{}{B2_ROWS}{C3_ROWS}", register("8")).as_bytes(),
    );
    let b2 = executable_script(
        &workspace,
        "B2.sh",
        "#!/bin/sh\necho '{\"cell\":\"default\",\"units\":{\"commands\":4,\"reads\":4,\"documents\":2,\"serializations\":2,\"pages_opened\":1,\"requests\":4,\"pauses\":6,\"refusals\":2,\"checks\":1,\"click_attempts\":1,\"fill_attempts\":1,\"press_attempts\":1,\"navigations\":3},\"counts\":{\"C1.encodes\":4,\"C1.sends\":4,\"C1.file_opens\":6,\"C1.log_sends_off\":4,\"C2.commands\":8,\"C2.world_creations\":4,\"C2.document_worlds\":4,\"C2.serialization_commands\":4,\"C7.pool_walks\":0,\"C7.opening_commands\":12,\"C5.fetch_messages_no_rule\":13,\"C5.spawns\":2,\"C5.file_opens\":4,\"C5.matches\":4,\"C5.answers\":6,\"C5.compilations\":0,\"C5.entries\":2,\"C3.script_sends\":1,\"C3.check_commands\":1,\"C3.commands.click\":6,\"C3.commands.fill\":2,\"C3.commands.press\":4,\"C4.detection_reads\":12,\"C4.compilations\":0,\"C4.script_compilations\":0,\"C4.provenance_entries\":4,\"C6.acknowledgements\":0}}'\n\
         echo '{\"cell\":\"no_viewer\",\"units\":{\"commands\":4,\"reads\":4,\"documents\":2,\"serializations\":2,\"pages_opened\":1,\"requests\":4,\"pauses\":6,\"refusals\":2,\"checks\":1,\"click_attempts\":1,\"fill_attempts\":1,\"press_attempts\":1,\"navigations\":3},\"counts\":{\"C1.encodes\":4,\"C1.sends\":4,\"C1.file_opens\":6,\"C1.log_sends_off\":4,\"C2.commands\":8,\"C2.world_creations\":4,\"C2.document_worlds\":4,\"C2.serialization_commands\":4,\"C7.pool_walks\":0,\"C7.opening_commands\":12,\"C5.fetch_messages_no_rule\":13,\"C5.spawns\":2,\"C5.file_opens\":4,\"C5.matches\":4,\"C5.answers\":6,\"C5.compilations\":0,\"C5.entries\":2,\"C3.script_sends\":1,\"C3.check_commands\":1,\"C3.commands.click\":6,\"C3.commands.fill\":2,\"C3.commands.press\":4,\"C4.detection_reads\":12,\"C4.compilations\":0,\"C4.script_compilations\":0,\"C4.provenance_entries\":4,\"C6.acknowledgements\":1}}'\n",
    );

    let outcome = workspace.xtask(&[
        "counts",
        "--family",
        "B2",
        "--executable",
        &format!("B2={}", b2.display()),
    ]);

    assert_eq!(
        outcome.findings(),
        vec![finding("row-listed-as-not-run", "C6.acknowledgements")],
        "{}",
        outcome.stderr
    );
    assert!(
        outcome
            .stdout
            .contains("C6.acknowledgements: not run yet, limit 1, until #959 builds its path"),
        "{}",
        outcome.stdout
    );
}

/// C2's rows on a stand-in of B2: its read rows divide by the reads or the
/// documents read, and its rows not run yet are held as they stand, a ratchet
/// with no figure included.
#[test]
fn page_read_rows_divide_by_their_reads_and_a_ratchet_not_run_yet_passes() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "TESTING.md",
        format!(
            "# Testing\n\n## Quality register\n\n### Measured, not yet enforced\n\n\
             | Row | Metric, per unit | Contract | Today | Benchmark | Direction | Checked by | Runs at |\n\
             |---|---|---|---|---|---|---|---|\n{B2_ROWS}"
        )
        .as_bytes(),
    );
    let path = executable_script(&workspace, "B2.sh", &b2_benchmark("C2.document_worlds", 1));
    let executable = format!("B2={}", path.display());

    let held = workspace.xtask(&["counts", "--path", "C2", "--executable", &executable]);
    let planted = workspace.xtask(&[
        "counts",
        "--path",
        "C2",
        "--executable",
        &executable,
        "--plant",
        "C2.document_worlds",
    ]);

    assert_eq!((held.code, held.findings()), (0, vec![]), "{}", held.stderr);
    for line in [
        "C2.commands [default]: 2 per read, limit 2 (8 over 4)",
        "C2.document_worlds [default]: 2 per document, limit 2 (4 over 2)",
        "C2.serialization_commands [default]: 2 per serialization, limit 2 (4 over 2)",
        "C2.closed_root_commands: not run yet, limit 3, until #726 builds its path",
        "C2.live_view_commands: not run yet, a ratchet with no figure yet, until #1683 builds its path",
    ] {
        assert!(held.stdout.contains(line), "{line}\n{}", held.stdout);
    }
    assert_eq!(
        planted.findings(),
        vec![finding("over-limit", "C2.document_worlds")]
    );
}

/// A register of F2's rows, each of contract 1, and a benchmark printing them
/// with no Effect, whose plant of `F2.effect_syncs` counts one each page.
fn effectless() -> (Workspace, String) {
    let rows = [
        "transactions",
        "syncs",
        "prepares",
        "writer_reads",
        "page_waits",
        "page_waits.full",
        "batches",
        "evidence_reads",
        "admission_evidence",
        "page_learning",
        "learning_off_evidence",
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
    let line = "{\"cell\":\"default\",\"units\":{\"pages\":13,\"added_pages\":6,\"attempts\":13,\"runs\":1,\"effects\":0,\"batched_pages\":13,\"full_pages\":0,\"learning_off_pages\":0},\"counts\":{\"F2.transactions\":13,\"F2.syncs\":13,\"F2.prepares\":0,\"F2.writer_reads\":0,\"F2.page_waits\":0,\"F2.page_waits.full\":0,\"F2.batches\":1,\"F2.evidence_reads\":0,\"F2.admission_evidence\":0,\"F2.page_learning\":0,\"F2.learning_off_evidence\":0,\"F2.configuration_reads\":1,\"F2.prunes\":0,\"F2.effect_syncs\":'$extra'}}";
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

/// A feature off may take a row's whole outcome away: it has no figure then,
/// and fails nothing.
#[test]
fn a_feature_off_that_removes_the_outcome_has_no_figure() {
    let script = "#!/bin/sh\n\
         echo '{\"cell\":\"default\",\"units\":{\"pages\":13},\"counts\":{\"F1.parses\":104}}'\n\
         echo '{\"cell\":\"a_feature_off\",\"units\":{\"pages\":0},\"counts\":{\"F1.parses\":0}}'\n";
    let (workspace, executable) = workspace("8", script);

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
            .contains("F1.parses [a_feature_off]: no pages"),
        "{}",
        outcome.stdout
    );
}

/// C3's rows as the register holds them: `C3.commands` carries a figure per
/// action kind, each its own limit.
const C3_ROWS: &str = "\
| `C3.script_sends` | sends of `check.js`'s source per check | 0 | 1 | B2 | must not grow | `just counts` | implementer done |
| `C3.check_commands` | CDP commands per check | 1 | after #1583 | B2 | must not grow | `just counts` | implementer done |
| `C3.commands` | CDP commands per attempt, per action kind | no cap yet (ADR 0140, open question 2) | click 6, fill 2, press 4; human-like input and typing not run yet (#1018, #1022) | B2 | must not grow | `just counts` | implementer done |
";

/// A B2 benchmark over 3 checks and one attempt of each kind, its click's
/// commands `click`.
fn c3_benchmark(workspace: &Workspace, click: u64) -> String {
    let path = executable_script(
        workspace,
        "B2.sh",
        &format!(
            "#!/bin/sh\necho '{{\"cell\":\"default\",\"units\":{{\"commands\":20,\"checks\":3,\"click_attempts\":1,\"fill_attempts\":1,\"press_attempts\":1}},\"counts\":{{\"C3.script_sends\":3,\"C3.check_commands\":3,\"C3.commands.click\":{click},\"C3.commands.fill\":2,\"C3.commands.press\":4}}}}'\n"
        ),
    );
    format!("B2={}", path.display())
}

/// Each action kind's figure of `C3.commands` is held to its own first
/// figure, and named by its kind.
#[rstest]
#[case::each_kind_at_its_figure(6, 0, &[])]
#[case::a_click_over_its_figure(7, 1, &["C3.commands.click"])]
fn each_action_kind_is_held_to_its_own_figure(
    #[case] click: u64,
    #[case] code: i32,
    #[case] over: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "TESTING.md",
        format!("{}{C3_ROWS}", register("8")).as_bytes(),
    );
    let executable = c3_benchmark(&workspace, click);

    let outcome = workspace.xtask(&[
        "counts",
        "--row",
        "C3.commands",
        "--executable",
        &executable,
    ]);

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        over.iter()
            .map(|row| finding("over-limit", row))
            .collect::<Vec<_>>()
    );
    assert!(
        outcome
            .stdout
            .contains("C3.commands.fill [default]: 2 per fill_attempt, limit 2 (2 over 1)"),
        "{}",
        outcome.stdout
    );
}

/// The rows of the extraction path (spec #2164): its hand-offs, which the
/// register puts on two families with a figure for each, its clean-up's
/// instructions, its selectors' compilations' and its generic set's build's.
const EXTRACTION_ROWS: &str = "\
| `F1.analysis_handoffs` | hand-offs to the analysis pool per page | at most 1, and 0 when no reader needs a tree | B1 2, B4 3; until #2189 | B1, B4 | must not grow | `just counts` | implementer done |
| `F1.analysis_handoffs.handler` | a handler's analysis hand-offs per page | one per read | 0 | B1 | must not grow | `just counts` | implementer done |
| `F1.clean_up_instructions` | instructions in the clean-up's removal per page | no cap (ratchet) | 1000 | B4 | must not grow | `just counts --family B4` | implementer done |
| `F1.selector_compilation_instructions` | instructions in a crawl's selector compilations per crawl | no cap (ratchet) | 1000 | B4 | must not grow | `just counts --family B4` | implementer done |
| `F1.generic_set_instructions` | instructions in the generic set's build per crawl | no cap (ratchet) | 1000 | B4 | must not grow | `just counts --family B4` | implementer done |
";

/// A benchmark of `family` whose default cell counts `handoffs` hand-offs
/// over 13 pages, and `instructions` in the clean-up over one page and in
/// the selectors' compilations and the generic set's build over one crawl.
fn extraction_benchmark(
    workspace: &Workspace,
    family: &str,
    handoffs: u64,
    instructions: u64,
) -> String {
    // B1 also counts a handler's own hand-offs, a row of B1's alone.
    let handler = if family == "B1" {
        ",\"F1.analysis_handoffs.handler\":0"
    } else {
        ""
    };
    let path = executable_script(
        workspace,
        &format!("{family}.sh"),
        &format!(
            "#!/bin/sh\necho '{{\"cell\":\"default\",\"units\":{{\"pages\":13,\"callgrind_pages\":1,\"callgrind_crawls\":1}},\"counts\":{{\"F1.analysis_handoffs\":{handoffs},\"F1.clean_up_instructions\":{instructions},\"F1.selector_compilation_instructions\":{instructions},\"F1.generic_set_instructions\":{instructions}{handler}}}}}'\n"
        ),
    );
    format!("{family}={}", path.display())
}

/// A row the register puts on two families, with a figure for each, is
/// counted on both, each held to its own family's figure: B1's two
/// hand-offs a page pass B1's 2, B4's three its 3, and B1 at B4's figure
/// fails.
#[rstest]
#[case::each_at_its_figure(26, 0, &[])]
#[case::b1_at_b4s_figure(39, 1, &["F1.analysis_handoffs"])]
fn a_row_two_families_count_is_held_on_each_to_its_own_figure(
    #[case] b1: u64,
    #[case] code: i32,
    #[case] over: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "TESTING.md",
        format!("{}{EXTRACTION_ROWS}", register("8")).as_bytes(),
    );
    let on_b1 = extraction_benchmark(&workspace, "B1", b1, 0);
    let on_b4 = extraction_benchmark(&workspace, "B4", 39, 0);

    let outcome = workspace.xtask(&[
        "counts",
        "--row",
        "F1.analysis_handoffs",
        "--executable",
        &on_b1,
        "--executable",
        &on_b4,
    ]);

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        over.iter()
            .map(|row| finding("over-limit", row))
            .collect::<Vec<_>>()
    );
    assert!(
        outcome
            .stdout
            .contains("F1.analysis_handoffs [default]: 3 per page, limit 3 (39 over 13)"),
        "{}",
        outcome.stdout
    );
    assert_eq!(
        outcome
            .stdout
            .contains("F1.analysis_handoffs [default]: 2 per page, limit 2 (26 over 13)"),
        over.is_empty(),
        "{}",
        outcome.stdout
    );
}

/// The clean-up's instructions, the selectors' compilations' and the generic
/// set's build's are read under Callgrind by B4's own run, and vary a little from run to run: each
/// row is held within the 0.5 % of a figure read under Valgrind (the
/// register's ratchet), and fails past it.
#[rstest]
#[case::within_the_tolerance(1005, 0, false)]
#[case::past_it(1006, 1, true)]
fn b4s_instructions_are_held_within_the_valgrind_tolerance(
    #[case] instructions: u64,
    #[case] code: i32,
    #[case] past: bool,
    #[values(
        "F1.clean_up_instructions",
        "F1.selector_compilation_instructions",
        "F1.generic_set_instructions"
    )]
    row: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "TESTING.md",
        format!("{}{EXTRACTION_ROWS}", register("8")).as_bytes(),
    );
    let executable = extraction_benchmark(&workspace, "B4", 39, instructions);

    let outcome = workspace.xtask(&["counts", "--row", row, "--executable", &executable]);

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        if past {
            vec![finding("over-limit", row)]
        } else {
            Vec::new()
        }
    );
}

/// A stand-in for B1 under `strace` (ADR 0140's cross-check): it creates a
/// Run-trace folder twice and opens two of its files, the work it counts,
/// syncs twice the file its counting VFS holds, reads a trace file and writes
/// a file outside the trace, which no row counts, then does the bypass it is
/// told to plant, unless `blind`.
#[cfg(target_os = "linux")]
fn traced_benchmark(blind: bool) -> String {
    let bypass = if blind {
        ""
    } else {
        "case \"$3\" in\n\
         open) : >> \"$d/trace/requests.jsonl\" ;;\n\
         directory) mkdir \"$d/trace\" 2>/dev/null ;;\n\
         sync) sync \"$d/run.sqlite\" ;;\n\
         esac\n"
    };
    format!(
        "#!/bin/sh\n\
         [ \"$1\" = --strace ] || exit 3\n\
         d=$(mktemp -d)\n\
         mkdir \"$d/trace\"\n\
         : >> \"$d/trace/requests.jsonl\"\n\
         mkdir \"$d/trace\" 2>/dev/null\n\
         : >> \"$d/trace/provenance.jsonl\"\n\
         : > \"$d/run.sqlite\"\n\
         sync \"$d/run.sqlite\"\n\
         sync --data \"$d/run.sqlite\"\n\
         cat \"$d/trace/requests.jsonl\" > /dev/null\n\
         {bypass}\
         echo '{{\"cell\":\"default\",\"units\":{{\"pages\":13}},\"counts\":{{\"E5.file_opens\":3,\"C1.file_opens\":1,\"F2.syncs\":2}},\"vfs_files\":[\"'$d'/run.sqlite\"]}}'\n\
         rm -rf \"$d\"\n"
    )
}

#[cfg(target_os = "linux")]
fn traced_workspace(blind: bool) -> (Workspace, String) {
    let workspace = Workspace::from_fixture("empty");
    let path = executable_script(&workspace, "benchmark.sh", &traced_benchmark(blind));
    (workspace, format!("B1={}", path.display()))
}

/// What `strace` sees of the Run trace's folders and files and of the counted
/// files' syncs is what the counters counted: the cell passes, printing both.
#[cfg(target_os = "linux")]
#[test]
fn the_strace_cell_passes_when_strace_sees_what_the_counters_count() {
    let (workspace, executable) = traced_workspace(false);

    let outcome = workspace.xtask(&["counts", "--strace", "--executable", &executable]);

    assert_eq!(
        (outcome.code, outcome.findings()),
        (0, vec![]),
        "{}\n{}",
        outcome.stdout,
        outcome.stderr
    );
    for line in [
        "B1 opens and folders: strace 4, counted 4 (E5.file_opens, C1.file_opens)",
        "B1 syncs: strace 2, counted 2 (F2.syncs)",
    ] {
        assert!(outcome.stdout.contains(line), "{line}: {}", outcome.stdout);
    }
}

/// A sync the benchmark's own setup does, such as its Limit's, through a
/// counting VFS of its own, is counted beside the row's: no row's work, and
/// still no bypass.
#[cfg(target_os = "linux")]
#[test]
fn the_fixtures_own_counted_syncs_are_no_bypass() {
    let workspace = Workspace::from_fixture("empty");
    let script = "#!/bin/sh\n\
         d=$(mktemp -d)\n\
         : > \"$d/usage-ledger.sqlite\"\n\
         sync \"$d/usage-ledger.sqlite\"\n\
         sync \"$d/usage-ledger.sqlite\"\n\
         echo '{\"cell\":\"default\",\"units\":{},\"counts\":{\"F2.syncs\":1},\"setup_syncs\":1,\"vfs_files\":[\"'$d'/usage-ledger.sqlite\"]}'\n\
         rm -rf \"$d\"\n";
    let path = executable_script(&workspace, "benchmark.sh", script);

    let outcome = workspace.xtask(&[
        "counts",
        "--strace",
        "--executable",
        &format!("B1={}", path.display()),
    ]);

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
            .contains("B1 syncs: strace 2, counted 2 (F2.syncs, and 1 of the fixture's setup)"),
        "{}",
        outcome.stdout
    );
}

/// A raw open, folder creation or sync beside what the counters see fails
/// the cell, naming the family and the work strace saw more of.
#[cfg(target_os = "linux")]
#[rstest]
#[case::a_raw_open(
    "open",
    "B1.opens",
    "strace saw 5 Run-trace opens and folder creations, the counters 4"
)]
#[case::a_raw_folder_creation(
    "directory",
    "B1.opens",
    "strace saw 5 Run-trace opens and folder creations, the counters 4"
)]
#[case::a_raw_sync(
    "sync",
    "B1.syncs",
    "strace saw 3 syncs of the counted files, the counters 2"
)]
fn a_planted_bypass_fails_the_strace_cell(
    #[case] bypass: &str,
    #[case] subject: &str,
    #[case] message: &str,
) {
    let (workspace, executable) = traced_workspace(false);

    let outcome = workspace.xtask(&[
        "counts",
        "--strace",
        "--executable",
        &executable,
        "--bypass",
        bypass,
    ]);

    assert_eq!(outcome.code, 1, "{}\n{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), vec![finding("strace-differs", subject)]);
    assert!(outcome.stdout.contains(message), "{}", outcome.stdout);
}

/// Proving plants each bypass in turn and passes only when each fails the
/// cell; a benchmark blind to its bypasses fails, naming each.
#[cfg(target_os = "linux")]
#[rstest]
#[case::each_bypass_caught(false, 0, &[])]
#[case::a_benchmark_blind_to_its_bypasses(true, 1, &["B1.open", "B1.directory", "B1.sync"])]
fn proving_the_strace_cell_plants_each_bypass(
    #[case] blind: bool,
    #[case] code: i32,
    #[case] blind_to: &[&str],
) {
    let (workspace, executable) = traced_workspace(blind);

    let outcome = workspace.xtask(&["counts", "--strace", "--prove", "--executable", &executable]);

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
    let expected: Vec<_> = blind_to
        .iter()
        .map(|subject| finding("strace-blind", subject))
        .collect();
    assert_eq!(outcome.findings(), expected);
    if !blind {
        assert!(
            outcome.stdout.contains("B1 sync: one planted bypass fails"),
            "{}",
            outcome.stdout
        );
    }
}

/// Without `strace` a local run lists the cell as skipped, with its reason,
/// as ADR 0061 asks; CI runs every part, so there it fails.
#[cfg(target_os = "linux")]
#[rstest]
#[case::locally(None, 0)]
#[case::in_ci(Some("true"), 2)]
fn without_strace_the_cell_lists_itself_as_skipped(#[case] ci: Option<&str>, #[case] code: i32) {
    let (workspace, executable) = traced_workspace(false);
    let empty = workspace.path().join("no-tools");
    std::fs::create_dir(&empty).unwrap();
    let mut variables = vec![("PATH", empty.as_os_str())];
    if let Some(ci) = ci {
        variables.push(("CI", std::ffi::OsStr::new(ci)));
    }

    let outcome = workspace.xtask_with_env(
        &["counts", "--strace", "--executable", &executable],
        &variables,
    );

    assert_eq!(outcome.code, code, "{}\n{}", outcome.stdout, outcome.stderr);
    if ci.is_some() {
        assert!(
            outcome
                .stderr
                .contains("strace is not installed on this host, and CI runs every part"),
            "{}",
            outcome.stderr
        );
    } else {
        assert!(
            outcome
                .stderr
                .contains("skipped: xtask counts --strace (strace is not installed on this host)"),
            "{}",
            outcome.stderr
        );
        assert!(
            workspace
                .read_text("target/skipped.txt")
                .contains("xtask counts --strace: skipped, strace is not installed on this host"),
        );
    }
}

/// Off Linux there is no `strace`: the cell lists itself as skipped and
/// passes; the merge gate runs it in the Linux CI image.
#[cfg(not(target_os = "linux"))]
#[test]
fn off_linux_the_strace_cell_lists_itself_as_skipped() {
    let workspace = Workspace::from_fixture("empty");

    let outcome = workspace.xtask(&["counts", "--strace"]);

    assert_eq!(outcome.code, 0, "{}\n{}", outcome.stdout, outcome.stderr);
    assert!(
        outcome
            .stderr
            .contains("skipped: xtask counts --strace (strace runs on Linux only)"),
        "{}",
        outcome.stderr
    );
}

/// `cargo bench` runs a benchmark in its package's folder, so gungraun's
/// folder is given as an absolute path under the root's `target/`: a
/// relative one would land in the package's own `target/`, outside every
/// build folder Git ignores, and leave the gate's tree unclean.
#[test]
fn gungraun_writes_under_the_roots_target_folder() {
    let script = stand_in_cargo(1000, 5000, 0).replacen(
        "#!/bin/sh\n",
        "#!/bin/sh\necho \"$GUNGRAUN_HOME\" >>\"$PWD/gungraun-homes\"\n",
        1,
    );
    let (workspace, cargo) = valgrind_workspace(&script);

    let outcome = workspace.xtask_with_env(
        &["counts", "--family", "B3", "--valgrind"],
        &[("CARGO", cargo.as_os_str())],
    );

    assert_eq!(outcome.code, 0, "{}\n{}", outcome.stdout, outcome.stderr);
    let homes = workspace.read_text("gungraun-homes");
    let expected = std::path::absolute(workspace.path().join("target/counts/gungraun")).unwrap();
    assert!(
        homes.lines().all(|home| Path::new(home) == expected),
        "{homes}"
    );
}
