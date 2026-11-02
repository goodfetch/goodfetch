//! The register check (ADR 0140; TESTING.md, Quality register): `xtask
//! register check` reads the register's tables and ADR 0140's inventory, and
//! fails a register that drifts from the ADR or from `xtask perf`'s ceilings.
//! One fixture register per failure, each the clean one with one edit.

use std::process::Command;

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

const TESTING: &str = "TESTING.md";
const ADR: &str = "docs/adr/0140-shared-paths.md";

/// A day before every expiry of the fixture register.
const TODAY: &str = "2026-11-01";

const CLEAN_ADR: &str = "\
# Shared paths

### The Shared paths

| Path | Unit | Owner module | Cost contract, per unit | Benchmark |
|---|---|---|---|---|
| **F1** Page analysis | one page | the analysis | ADR 0138's counts: at most 1 HTML parse (0 on the streaming path), 0 file opens; at most 2× its body's bytes allocated (ADR 0134); none adds 0 | B1 |
| **C1** CDP command | one command | the connection | 1 encode, 0 directories created, with no viewer, 0 work | B2 |

### What every path keeps
";

const CLEAN_TESTING: &str = "\
# Testing

## Quality register

### Enforced

| Dimension | Rule | Checked by | Runs at |
|---|---|---|---|
| Rust lints | 0 warnings | `cargo clippy --workspace` (`just lint`) | merge gate |
| Workflow security | 0 findings | `zizmor`, `xtask modules`, `xtask features check` and `ci/check.sh` | merge gate |
| Generated code | current | `xtask codegen --check` | merge gate |
| Dependency bans | 0 | `cargo deny check … bans sources` | merge gate |

### Measured, not yet enforced

| Row | Metric, per unit | Contract | Today | Benchmark | Direction |
|---|---|---|---|---|---|
| `F1.parses` | HTML parses per page | ≤ 1 (0 streaming) | after #1583 | B1 | must not grow |
| `F1.file_opens` | files opened per page | 0 | 0 | B1 | must not grow |
| `F1.bytes_allocated` | bytes allocated per crawled page, over its body's bytes | ≤ 2 (ADR 0134) | 234.45 | `heap::crawl::in_memory_pages` | must not grow |
| `C1.encodes` | encodes per CDP command | 1 | after #1583 | B2 | must not grow |
| `C1.file_opens` | directories created and files opened per CDP command | 0 | after #1583 | B2 | must not grow |
| `C1.instructions` | instructions per command | 1.10 × the figure after #1577 | after #1583 | B2 | must not grow |
| `frontier.heap_per_url` | heap bytes per URL | ≤ 8 (ADR 0134) | after #1583 | `goodfetch-frontier`'s `heap::frontier::enqueue` (`hundred_thousand_urls` less `no_urls`) | must not grow |
| `console.collie` | whole build, bytes brotli | ≤ 1,000,000 | after #1583 | `size-limit/collie-console` | must not grow |
| `console.collie_initial_route` | initial route, bytes brotli | ≤ 139,779 | after #1583 | `size-limit/collie-console-initial-route` | must not grow |
| `console.foxhound` | whole build, bytes brotli | ≤ 1,000,000 | after #1583 | `size-limit/foxhound-console` | must not grow |
| `console.foxhound_initial_route` | initial route, bytes brotli | ≤ 161,174 | after #1583 | `size-limit/foxhound-console-initial-route` | must not grow |
| `coverage.project` | project line coverage | none: a ratchet | the next job | `just coverage all` | must not fall |

### Exceptions

| ID | Rule | Path | Reason | Owner | Expires | Removed by |
|---|---|---|---|---|---|---|
| X1 | `F1.bytes_allocated` | F1 | 234.45 bytes per body byte | The owner | 2027-01-30 | #1577 |

### Rules
";

const JUSTFILE: &str = "\
# Lints.
lint: fmt-check
    cargo clippy --workspace
    ci/tool.sh zizmor zizmor --config zizmor.yml
    cargo deny check -A unused bans sources

fmt-check:
    cargo fmt --all --check
";

/// A workspace holding the clean register, ADR 0140's inventory, the justfile
/// and the script the register's commands name.
fn clean() -> Workspace {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(TESTING, CLEAN_TESTING.as_bytes());
    workspace.write(ADR, CLEAN_ADR.as_bytes());
    workspace.write("justfile", JUSTFILE.as_bytes());
    workspace.write("ci/check.sh", b"#!/bin/sh\n");
    workspace
}

fn check(workspace: &Workspace, today: &str) -> Outcome {
    workspace.xtask(&["register", "check", "--today", today])
}

#[test]
fn a_clean_register_passes() {
    let outcome = check(&clean(), TODAY);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::a_recipe_the_justfile_lacks("`just lint`", "`just no-such-recipe`", "Rust lints")]
#[case::a_command_xtask_lacks("`xtask modules`", "`xtask no-such-command`", "Workflow security")]
#[case::a_nested_command_xtask_lacks(
    "`xtask features check`",
    "`xtask features no-such`",
    "Workflow security"
)]
#[case::a_file_the_repository_lacks("`ci/check.sh`", "`ci/no-such.sh`", "Workflow security")]
#[case::a_tool_no_recipe_runs("`zizmor`", "`no-such-tool`", "Workflow security")]
fn an_enforced_row_whose_command_does_not_exist_fails(
    #[case] from: &str,
    #[case] to: &str,
    #[case] row: &str,
) {
    let workspace = clean();
    workspace.edit(TESTING, from, to);

    let outcome = check(&workspace, TODAY);

    assert_eq!(outcome.findings(), vec![finding("command-missing", row)]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_flag_the_command_lacks_fails() {
    let workspace = clean();
    workspace.edit(
        TESTING,
        "`xtask codegen --check`",
        "`xtask codegen --no-such`",
    );

    let outcome = check(&workspace, TODAY);

    assert_eq!(
        outcome.findings(),
        vec![finding("command-missing", "Generated code")]
    );
}

#[test]
fn text_the_justfile_lacks_fails_after_an_elision() {
    let workspace = clean();
    workspace.edit(TESTING, "… bans sources", "… bans no-such-check");

    let outcome = check(&workspace, TODAY);

    assert_eq!(
        outcome.findings(),
        vec![finding("command-missing", "Dependency bans")]
    );
}

#[test]
fn an_exception_passes_on_its_expiry_day() {
    let outcome = check(&clean(), "2027-01-30");

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn an_exception_fails_once_its_date_has_passed_with_no_other_change() {
    let outcome = check(&clean(), "2027-01-31");

    assert_eq!(outcome.findings(), vec![finding("exception-expired", "X1")]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_path_of_the_adr_with_no_row_fails() {
    let workspace = clean();
    workspace.edit(
        ADR,
        "\n### What every path keeps",
        "| **E7** Working copy | one update | the owner | 1 transaction | B3 |\n\n### What every path keeps",
    );

    let outcome = check(&workspace, TODAY);

    assert_eq!(outcome.findings(), vec![finding("path-without-row", "E7")]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn a_count_of_a_contract_with_no_row_fails() {
    let workspace = clean();
    workspace.edit(
        ADR,
        "0 file opens;",
        "0 file opens, 0 selector compilations;",
    );

    let outcome = check(&workspace, TODAY);

    assert_eq!(
        outcome.findings(),
        vec![finding("count-without-row", "F1/selector compilations")]
    );
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::its_contract_over_the_adrs_cap(
    "| ≤ 1 (0 streaming) |",
    "| ≤ 2 (0 streaming) |",
    "F1.parses"
)]
#[case::a_zero_cap_raised(
    "| files opened per page | 0 |",
    "| files opened per page | 1 |",
    "F1.file_opens"
)]
#[case::its_figure_over_its_contract_with_no_exception(
    "| ≤ 1 (0 streaming) | after #1583 |",
    "| ≤ 1 (0 streaming) | 3 |",
    "F1.parses"
)]
#[case::the_exception_naming_another_row(
    "| X1 | `F1.bytes_allocated` |",
    "| X1 | `F1.parses` |",
    "F1.bytes_allocated"
)]
fn a_row_above_its_contract_fails(#[case] from: &str, #[case] to: &str, #[case] row: &str) {
    let workspace = clean();
    workspace.edit(TESTING, from, to);

    let outcome = check(&workspace, TODAY);

    assert_eq!(outcome.findings(), vec![finding("row-above-contract", row)]);
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::a_per_url_ceiling(
    "| ≤ 8 (ADR 0134) |",
    "| ≤ 7 (ADR 0134) |",
    "goodfetch-frontier/heap::frontier::enqueue/hundred_thousand_urls"
)]
#[case::a_size_ceiling_with_a_longer_namesake(
    "| ≤ 139,779 |",
    "| ≤ 139,780 |",
    "size-limit/collie-console-initial-route"
)]
#[case::a_ceiling_with_no_row(
    "`size-limit/foxhound-console` |",
    "`size-limit/renamed` |",
    "size-limit/foxhound-console"
)]
fn a_difference_from_the_ceilings_of_xtask_perf_fails(
    #[case] from: &str,
    #[case] to: &str,
    #[case] subject: &str,
) {
    let workspace = clean();
    workspace.edit(TESTING, from, to);

    let findings = check(&workspace, TODAY).findings();

    assert!(
        findings.contains(&finding("ceiling-difference", subject)),
        "{findings:?}"
    );
}

#[test]
fn a_ceiling_row_of_a_benchmark_xtask_perf_does_not_hold_fails() {
    let workspace = clean();
    workspace.edit(
        TESTING,
        "| `coverage.project` |",
        "| `heap.other` | bytes | ≤ 3 | after #1583 | `heap::other::scenario` | must not grow |\n| `coverage.project` |",
    );

    let outcome = check(&workspace, TODAY);

    assert_eq!(
        outcome.findings(),
        vec![finding("ceiling-difference", "heap.other")]
    );
}

/// A section or table the check cannot read never passes as an empty one.
#[rstest]
#[case::no_enforced_table(TESTING, "### Enforced", "### Other", "TESTING.md")]
#[case::no_measured_table(TESTING, "| Row | Metric", "| Name | Metric", "TESTING.md")]
#[case::no_exceptions_table(TESTING, "### Exceptions", "### Other", "TESTING.md")]
#[case::no_inventory(ADR, "### The Shared paths", "### Other", "ADR 0140")]
#[case::an_expiry_that_is_no_date(TESTING, "| 2027-01-30 |", "| soon |", "X1")]
fn a_register_it_cannot_read_fails(
    #[case] file: &str,
    #[case] from: &str,
    #[case] to: &str,
    #[case] subject: &str,
) {
    let workspace = clean();
    workspace.edit(file, from, to);

    let outcome = check(&workspace, TODAY);

    assert!(
        outcome
            .findings()
            .contains(&finding("register-unreadable", subject)),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);
}

#[test]
fn the_register_of_this_repository_passes() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["register", "check", "--today", TODAY])
        .current_dir(repository)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `just lint`, which the merge gate runs, runs the check.
#[test]
fn just_lint_runs_the_check() {
    let justfile = include_str!("../../../justfile");
    let lint = justfile
        .split("\nlint:")
        .nth(1)
        .and_then(|recipe| recipe.split("\n\n").next())
        .unwrap();

    assert!(lint.contains("xtask -- register check"), "{lint}");
}
