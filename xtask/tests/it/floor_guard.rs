//! `xtask floor-guard`: the Quality register's Floor over a change, from the
//! merge base to its head (ADR 0140). Each move that lowers the bar has a known
//! violation here, a clean control, and a `Quality-change:` trailer accepting
//! it; an error of the guard's own is never a clean verdict.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

const TESTING: &str = "TESTING.md";

/// A register in the real one's shape, with the sections the guard reads.
const REGISTER: &str = "\
# Testing

## Quality register

Until the gate's spec builds them, this prose changes freely.

### Floor

- **No new suppression comment,** except these forms:
  - Rust: `#[expect(lint, reason = \"…\")]`.
- **No edit to the gate's own files** (its benchmarks, the floor guard) without a `Quality-change` trailer. The guard reads them from this line: `gate/guard.rs`, `gate/benches/`.

### Recorded changes

- **A change the Floor would flag carries a commit trailer.**

### Enforced

| Dimension | Rule | Checked by | Runs at |
|---|---|---|---|
| Rust lints | 0 warnings | `just lint` | merge gate |
| `F2.syncs` | ≤ 2 per attempt | `xtask counts` | implementer done |

### Measured, not yet enforced

The ratchet.

| Row | Metric, per unit | Contract | Today | Benchmark | Direction |
|---|---|---|---|---|---|
| `F1.parses` | parses per page | ≤ 1 (0 streaming) | 3 | B1 | must not grow |
| `F1.file_opens` | files per page | 2 | after #1583 | B1 | must not grow |
| `C3.commands` | commands | no cap yet | after #1583 | B2 | must not grow |
| `coverage.project` | line coverage | none: a ratchet | 62.5 | `just coverage all` | must not fall (0.5 %) |

### Exceptions

| ID | Rule | Path | Reason | Owner | Expires | Removed by |
|---|---|---|---|---|---|---|
| X1 | `F1.parses`, `F1.file_opens` | F1 | 3 parses | Jean-Baptiste Manillier | 2027-01-30 | #1577 |

### Rules

- **Tightening is silent; loosening is loud.**

## Security

Not the register.
";

const TEST: &str = "#[test]\nfn it_holds() {\n    assert!(true);\n}\n";

/// A repository holding the register and `files`, committed: the merge base.
fn repository(files: &[(&str, &str)]) -> (Workspace, String) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(TESTING, REGISTER.as_bytes());
    for (path, contents) in files {
        workspace.write(path, contents.as_bytes());
    }
    let base = workspace.commit_all();
    (workspace, base)
}

/// Commits the work tree with `message` and runs the guard from `base`.
fn guard(workspace: &Workspace, base: &str, message: &str) -> Outcome {
    workspace.git(&["add", "--all"]);
    workspace.git(&[
        "commit",
        "--quiet",
        "--no-verify",
        "--allow-empty",
        "--message",
        message,
    ]);
    workspace.xtask(&["floor-guard", "--base", base, "--head", "HEAD"])
}

/// Writes `added` as a new file, the guard's plainest input.
fn adding(path: &str, added: &str) -> Outcome {
    let (workspace, base) = repository(&[]);
    workspace.write(path, added.as_bytes());
    guard(&workspace, &base, "feat: add it")
}

fn assert_clean(outcome: &Outcome) {
    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// Each form of a new suppression, stub, empty catch or skipped test: flagged,
/// and accepted by a trailer that names its file.
#[rstest]
#[case::rust_allow("a.rs", "#[allow(dead_code)]\nfn a() {}\n", "suppression")]
#[case::rust_inner_allow("a.rs", "#![allow(dead_code)]\n", "suppression")]
#[case::rust_expect_with_no_reason("a.rs", "#[expect(dead_code)]\nfn a() {}\n", "suppression")]
#[case::rust_expect_split_with_no_reason(
    "a.rs",
    "#[expect(\n    dead_code,\n    clippy::all\n)]\nfn a() {}\n",
    "suppression"
)]
#[case::rust_conditional_allow("a.rs", "#[cfg_attr(miri, allow(dead_code))]\n", "suppression")]
#[case::typescript_ignore("a.ts", "// @ts-ignore\nlet a = 1;\n", "suppression")]
#[case::typescript_nocheck("a.ts", "// @ts-nocheck\n", "suppression")]
#[case::expect_error_outside_a_test("src/a.ts", "// @ts-expect-error -- why\n", "suppression")]
#[case::expect_error_with_no_reason("tests/a.ts", "// @ts-expect-error\n", "suppression")]
#[case::eslint_disable("a.js", "/* eslint-disable no-console */\n", "suppression")]
#[case::oxlint_with_no_reason("a.ts", "// oxlint-disable-next-line no-console\n", "suppression")]
#[case::oxlint_with_no_rule("a.ts", "// oxlint-disable-next-line -- why\n", "suppression")]
#[case::oxlint_single_hyphen(
    "a.ts",
    "// oxlint-disable-next-line no-console - why\n",
    "suppression"
)]
#[case::oxlint_whole_file("a.ts", "/* oxlint-disable no-console -- why */\n", "suppression")]
#[case::oxlint_this_line(
    "a.ts",
    "a(); // oxlint-disable-line no-console -- why\n",
    "suppression"
)]
#[case::python_noqa("a.py", "import os  # noqa: F401\n", "suppression")]
#[case::python_type_ignore_bare("tests/a.py", "a = 1  # type: ignore\n", "suppression")]
#[case::python_type_ignore_outside_a_test(
    "src/a.py",
    "a = 1  # type: ignore[assignment]\n",
    "suppression"
)]
#[case::pyright_bare("a.py", "a = 1  # pyright: ignore\n", "suppression")]
#[case::shellcheck("a.sh", "# shellcheck disable=SC2086\n", "suppression")]
#[case::secret_scanner("a.py", "a = 1  # gitleaks:allow\n", "suppression")]
#[case::rust_todo("a.rs", "fn a() {\n    todo!()\n}\n", "stub")]
#[case::rust_unimplemented("a.rs", "fn a() {\n    unimplemented!(\"later\")\n}\n", "stub")]
#[case::rust_ignored_error(
    "a.rs",
    "match a() {\n    Err(_) => {}\n    Ok(_) => b(),\n}\n",
    "stub"
)]
#[case::typescript_throw("a.ts", "throw new Error(\"Not implemented\");\n", "stub")]
#[case::typescript_empty_catch("a.ts", "try { a(); } catch {}\n", "stub")]
#[case::typescript_empty_catch_named("a.ts", "try { a(); } catch (error) { }\n", "stub")]
#[case::typescript_empty_catch_split("a.ts", "try {\n  a();\n} catch (error) {\n}\n", "stub")]
#[case::typescript_empty_promise_catch("a.ts", "a().catch(() => {});\n", "stub")]
#[case::python_not_implemented("a.py", "def a():\n    raise NotImplementedError\n", "stub")]
#[case::python_empty_except("a.py", "try:\n    a()\nexcept ValueError:\n    pass\n", "stub")]
#[case::rust_bare_ignore("a.rs", "#[test]\n#[ignore]\nfn a() {}\n", "skipped-test")]
#[case::rust_other_ignore("a.rs", "#[test]\n#[ignore = \"flaky\"]\nfn a() {}\n", "skipped-test")]
#[case::rust_conditional_ignore("a.rs", "#[cfg_attr(miri, ignore = \"slow\")]\n", "skipped-test")]
#[case::typescript_skip("a.test.ts", "test.skip(\"a\", () => {});\n", "skipped-test")]
#[case::typescript_conditional_skip("a.test.ts", "test.skip(a, \"why\");\n", "skipped-test")]
#[case::typescript_xit("a.test.ts", "xit(\"a\", () => {});\n", "skipped-test")]
#[case::python_skip("test_a.py", "@pytest.mark.skip(reason=\"later\")\n", "skipped-test")]
fn a_lowered_line_is_flagged_and_a_trailer_accepts_it(
    #[case] path: &str,
    #[case] added: &str,
    #[case] kind: &str,
) {
    let outcome = adding(path, added);

    assert_eq!(
        outcome.findings(),
        vec![finding(kind, path)],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);

    let (workspace, base) = repository(&[]);
    workspace.write(path, added.as_bytes());
    let accepted = guard(
        &workspace,
        &base,
        &format!("feat: add it\n\nQuality-change: {path}: the test needs it"),
    );
    assert_clean(&accepted);
}

/// The forms the Floor lists, and what only looks like a move.
#[rstest]
#[case::rust_expect(
    "a.rs",
    "#[expect(dead_code, reason = \"used by the macro\")]\nfn a() {}\n"
)]
#[case::rust_expect_split(
    "a.rs",
    "#![expect(\n    clippy::disallowed_methods,\n    reason = \"xtask is synchronous\"\n)]\n"
)]
#[case::child_test("a.rs", "#[test]\n#[ignore = \"run as a child by a::b\"]\nfn a() {}\n")]
#[case::doc_comment("a.rs", "/// An `#[ignore]`d test, or a `todo!()`.\nfn a() {}\n")]
#[case::line_comment("a.rs", "// #[allow(dead_code)] is banned.\nfn a() {}\n")]
#[case::string_in_rust("a.rs", "const A: &str = \"#[ignore]\";\n")]
#[case::type_test_expect_error("tests/a.ts", "// @ts-expect-error -- `set` takes no null\n")]
#[case::type_test_expect_error_colon("a.test.ts", "// @ts-expect-error: outside the catalog\n")]
#[case::oxlint_next_line("a.ts", "// oxlint-disable-next-line no-console -- it prints\n")]
#[case::oxlint_two_rules("a.ts", "// oxlint-disable-next-line a/b, c-d -- why\n")]
#[case::oxlint_in_jsx("a.tsx", "{/* oxlint-disable-next-line a/b -- why */}\n")]
#[case::python_type_ignore_code("tests/a.py", "a = 1  # type: ignore[assignment]\n")]
#[case::pyright_rule("a.py", "a = 1  # pyright: ignore[reportPrivateUsage]\n")]
#[case::catch_with_a_body("a.ts", "try { a(); } catch (error) { log(error); }\n")]
#[case::prose("a.md", "Never `#[allow(x)]`, `@ts-ignore` or `todo!()`.\n")]
#[case::vendored("vendor/w/a.rs", "#[allow(dead_code)]\nfn a() {\n    todo!()\n}\n")]
#[case::generated("sdks/x/generated/a.ts", "// @ts-nocheck\n")]
#[case::research("rnd/lab/a.py", "import os  # noqa\n")]
fn a_clean_line_passes(#[case] path: &str, #[case] added: &str) {
    assert_clean(&adding(path, added));
}

/// A line that was already there is not this change's: the guard is scoped to
/// the diff.
#[test]
fn an_existing_suppression_fails_nothing() {
    let (workspace, base) = repository(&[("a.rs", "#[allow(dead_code)]\nfn a() {}\n")]);
    workspace.append("a.rs", "fn b() {}\n");

    assert_clean(&guard(&workspace, &base, "feat: add it"));
}

/// A lowered line on a branch from an older merge base is still seen, and the
/// base's own later commits are not this change's.
#[test]
fn the_diff_starts_at_the_merge_base() {
    let (workspace, base) = repository(&[]);
    let branch = "change";
    workspace.git(&["checkout", "--quiet", "-b", branch]);
    workspace.write("a.rs", b"#[allow(dead_code)]\nfn a() {}\n");
    workspace.commit_all();
    workspace.git(&["checkout", "--quiet", &base]);
    workspace.write("b.rs", b"fn b() {\n    todo!()\n}\n");
    let moved_base = workspace.commit_all();
    workspace.git(&["checkout", "--quiet", branch]);

    let outcome = workspace.xtask(&["floor-guard", "--base", &moved_base, "--head", "HEAD"]);

    assert_eq!(outcome.findings(), vec![finding("suppression", "a.rs")]);
    assert_eq!(outcome.code, 1);
}

#[rstest]
#[case::whole_file("tests/a.rs", TEST, None)]
#[case::one_test_of_two(
    "tests/a.rs",
    "#[test]\nfn stays() {}\n\n#[test]\nfn goes() {}\n",
    Some("#[test]\nfn stays() {}\n")
)]
#[case::renamed_with_no_replacement_named(
    "tests/a.rs",
    "#[test]\nfn old_name() {}\n",
    Some("#[test]\nfn new_name() {}\n")
)]
#[case::rstest_case(
    "tests/a.rs",
    "#[rstest]\n#[case::one(1)]\n#[case::two(2)]\nfn a(#[case] b: u8) {}\n",
    Some("#[rstest]\n#[case::one(1)]\nfn a(#[case] b: u8) {}\n")
)]
#[case::attribute_alone("tests/a.rs", TEST, Some("fn it_holds() {\n    assert!(true);\n}\n"))]
#[case::typescript(
    "a.test.ts",
    "test(\"a\", () => {});\ntest(\"b\", () => {});\n",
    Some("test(\"a\", () => {});\n")
)]
#[case::python(
    "test_a.py",
    "def test_a():\n    pass\n\ndef test_b():\n    pass\n",
    Some("def test_a():\n    pass\n")
)]
fn a_deleted_test_is_flagged_and_a_trailer_accepts_it(
    #[case] path: &str,
    #[case] before: &str,
    #[case] after: Option<&str>,
) {
    let run = |message: &str| {
        let (workspace, base) = repository(&[(path, before)]);
        match after {
            Some(contents) => workspace.write(path, contents.as_bytes()),
            None => workspace.remove(path),
        }
        guard(&workspace, &base, message)
    };

    let outcome = run("test: drop it");
    assert_eq!(
        outcome.findings(),
        vec![finding("deleted-test", path)],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);

    assert_clean(&run(&format!(
        "test: drop it\n\nQuality-change: {path}: replaced by the module's interface test"
    )));
}

/// A test that moved keeps its name; a file with no test deletes none.
#[rstest]
#[case::moved_to_another_file("tests/a.rs", TEST, None, Some(("tests/b.rs", TEST)))]
#[case::helper_deleted("tests/a.rs", "pub fn helper() {}\n", None, None)]
#[case::body_edited(
    "tests/a.rs",
    TEST,
    Some("#[test]\nfn it_holds() {\n    assert_eq!(1, 1);\n}\n"),
    None
)]
#[case::became_rstest(
    "tests/a.rs",
    TEST,
    Some("#[rstest]\nfn it_holds() {\n    assert!(true);\n}\n"),
    None
)]
#[case::case_kept_for_one_platform(
    "tests/a.rs",
    "#[rstest]\n#[case::one(1)]\nfn a(#[case] b: u8) {}\n",
    Some("#[rstest]\n#[cfg_attr(unix, case::one(1))]\nfn a(#[case] b: u8) {}\n"),
    None
)]
#[case::case_arguments_edited(
    "tests/a.rs",
    "#[rstest]\n#[case::one(1)]\nfn a(#[case] b: u8) {}\n",
    Some("#[rstest]\n#[case::one(2)]\nfn a(#[case] b: u8) {}\n"),
    None
)]
fn a_kept_test_is_not_deleted(
    #[case] path: &str,
    #[case] before: &str,
    #[case] after: Option<&str>,
    #[case] other: Option<(&str, &str)>,
) {
    let (workspace, base) = repository(&[(path, before)]);
    match after {
        Some(contents) => workspace.write(path, contents.as_bytes()),
        None => workspace.remove(path),
    }
    if let Some((moved, contents)) = other {
        workspace.write(moved, contents.as_bytes());
    }

    assert_clean(&guard(&workspace, &base, "test: move it"));
}

/// Each way of weakening the register, against the register's own rules.
#[rstest]
#[case::contract_raised(
    "| ≤ 1 (0 streaming) | 3 |",
    "| ≤ 2 (0 streaming) | 3 |",
    "row-weakened",
    "F1.parses"
)]
#[case::contract_detail_changed(
    "| ≤ 1 (0 streaming) | 3 |",
    "| ≤ 1 (1 streaming) | 3 |",
    "row-weakened",
    "F1.parses"
)]
#[case::contract_lost_its_number(
    "| 2 | after #1583 |",
    "| no cap | after #1583 |",
    "row-weakened",
    "F1.file_opens"
)]
#[case::today_raised(
    "| ≤ 1 (0 streaming) | 3 |",
    "| ≤ 1 (0 streaming) | 4 |",
    "row-weakened",
    "F1.parses"
)]
#[case::today_lost_its_number(
    "| ≤ 1 (0 streaming) | 3 |",
    "| ≤ 1 (0 streaming) | later |",
    "row-weakened",
    "F1.parses"
)]
#[case::ratchet_lowered_where_it_must_not_fall(
    "| 62.5 |",
    "| 60 |",
    "row-weakened",
    "coverage.project"
)]
#[case::direction_changed(
    "| 3 | B1 | must not grow |",
    "| 3 | B1 | none |",
    "row-weakened",
    "F1.parses"
)]
#[case::measured_row_removed(
    "| `F1.file_opens` | files per page | 2 | after #1583 | B1 | must not grow |\n",
    "",
    "row-weakened",
    "F1.file_opens"
)]
#[case::enforced_row_removed(
    "| Rust lints | 0 warnings | `just lint` | merge gate |\n",
    "",
    "row-weakened",
    "Rust lints"
)]
#[case::enforced_limit_raised(
    "| ≤ 2 per attempt |",
    "| ≤ 3 per attempt |",
    "row-weakened",
    "F2.syncs"
)]
#[case::enforced_command_changed("| `xtask counts` |", "| `true` |", "row-weakened", "F2.syncs")]
#[case::enforced_moment_changed(
    "| `xtask counts` | implementer done |",
    "| `xtask counts` | never |",
    "row-weakened",
    "F2.syncs"
)]
#[case::enforced_demoted(
    "| `F2.syncs` | ≤ 2 per attempt | `xtask counts` | implementer done |\n",
    "",
    "row-weakened",
    "F2.syncs"
)]
#[case::exception_extended("| 2027-01-30 |", "| 2027-06-30 |", "exception-weakened", "X1")]
#[case::exception_widened(
    "| `F1.parses`, `F1.file_opens` | F1 |",
    "| `F1.parses`, `F1.file_opens`, `F2.syncs` | F1 |",
    "exception-weakened",
    "X1"
)]
#[case::exception_added(
    "| 2027-01-30 | #1577 |\n",
    "| 2027-01-30 | #1577 |\n| X2 | `F2.syncs` | F2 | gap | Owner | 2027-01-30 | #1577 |\n",
    "exception-added",
    "X2"
)]
#[case::floor_bullet_removed(
    "- **No new suppression comment,** except these forms:\n",
    "",
    "row-weakened",
    "Floor"
)]
#[case::floor_form_added(
    "  - Rust: `#[expect(lint, reason = \"…\")]`.\n",
    "  - Rust: `#[expect(lint, reason = \"…\")]`.\n  - Rust: `#[allow(lint)]`.\n",
    "row-weakened",
    "Floor"
)]
#[case::gate_files_narrowed(
    "`gate/guard.rs`, `gate/benches/`",
    "`gate/guard.rs`",
    "row-weakened",
    "Floor"
)]
#[case::recorded_changes_edited(
    "- **A change the Floor would flag carries a commit trailer.**\n",
    "- **A change the Floor would flag needs nothing.**\n",
    "row-weakened",
    "Recorded changes"
)]
#[case::rules_edited(
    "- **Tightening is silent; loosening is loud.**\n",
    "- **Tightening is silent; loosening is silent.**\n",
    "row-weakened",
    "Rules"
)]
fn a_weakened_register_is_flagged_and_a_trailer_accepts_it(
    #[case] from: &str,
    #[case] to: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let run = |message: &str| {
        let (workspace, base) = repository(&[]);
        workspace.edit(TESTING, from, to);
        guard(&workspace, &base, message)
    };

    let outcome = run("docs: change the register");
    assert_eq!(
        outcome.findings(),
        vec![finding(kind, subject)],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);

    assert_clean(&run(&format!(
        "docs: change the register\n\nQuality-change: {subject}: the owner's decision"
    )));
}

/// Tightening is silent.
#[rstest]
#[case::contract_lowered("| ≤ 1 (0 streaming) | 3 |", "| 0 | 3 |")]
#[case::contract_set_on_a_ratchet("| no cap yet |", "| ≤ 12 |")]
#[case::today_lowered("| ≤ 1 (0 streaming) | 3 |", "| ≤ 1 (0 streaming) | 2 |")]
#[case::first_figure("| 2 | after #1583 |", "| 2 | 2 |")]
#[case::figure_awaited_differently("| no cap yet | after #1583 |", "| no cap yet | from #1577 |")]
#[case::ratchet_raised_where_it_must_not_fall("| 62.5 |", "| 70 (the weekly job) |")]
#[case::row_added(
    "| `C3.commands` |",
    "| `C4.new` | reads | 1 | after #1583 | B2 | must not grow |\n| `C3.commands` |"
)]
#[case::enforced_row_added(
    "| Rust lints |",
    "| Python lints | 0 errors | `uv run ruff` | merge gate |\n| Rust lints |"
)]
#[case::enforced_limit_lowered("| ≤ 2 per attempt |", "| ≤ 1 per attempt |")]
#[case::exception_removed(
    "| X1 | `F1.parses`, `F1.file_opens` | F1 | 3 parses | Jean-Baptiste Manillier | 2027-01-30 | #1577 |\n",
    ""
)]
#[case::exception_shortened("| 2027-01-30 |", "| 2026-12-30 |")]
#[case::exception_narrowed("| `F1.parses`, `F1.file_opens` | F1 |", "| `F1.parses` | F1 |")]
#[case::exception_reworded("| 3 parses |", "| 3 parses, measured |")]
#[case::prose_above_the_floor("this prose changes freely", "this prose changed")]
#[case::prose_past_the_register("Not the register.", "Still not the register.")]
fn a_tightened_register_is_silent(#[case] from: &str, #[case] to: &str) {
    let (workspace, base) = repository(&[]);
    workspace.edit(TESTING, from, to);

    assert_clean(&guard(&workspace, &base, "docs: tighten the register"));
}

/// A measured row that left its table for the Enforced one was promoted.
#[test]
fn a_promoted_row_is_silent() {
    let (workspace, base) = repository(&[]);
    workspace.edit(
        TESTING,
        "| `F1.file_opens` | files per page | 2 | after #1583 | B1 | must not grow |\n",
        "",
    );
    workspace.edit(
        TESTING,
        "| Rust lints |",
        "| `F1.file_opens` | 2 per page | `xtask counts` | merge gate |\n| Rust lints |",
    );

    assert_clean(&guard(&workspace, &base, "docs: promote a row"));
}

#[rstest]
#[case::the_guard("gate/guard.rs", Some("fn loosened() {}\n"))]
#[case::a_benchmark("gate/benches/b1.rs", Some("fn smaller() {}\n"))]
#[case::the_guard_deleted("gate/guard.rs", None)]
fn an_edit_to_the_gates_own_files_is_flagged_and_a_trailer_accepts_it(
    #[case] path: &str,
    #[case] after: Option<&str>,
) {
    let run = |message: &str| {
        let (workspace, base) = repository(&[(path, "fn original() {}\n")]);
        match after {
            Some(contents) => workspace.write(path, contents.as_bytes()),
            None => workspace.remove(path),
        }
        guard(&workspace, &base, message)
    };

    let outcome = run("fix: change the gate");
    assert_eq!(
        outcome.findings(),
        vec![finding("gate-file-edited", path)],
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1);

    assert_clean(&run(&format!(
        "fix: change the gate\n\nQuality-change: {path}: the spec extends its fixture"
    )));
}

/// A new file of the gate adds to the bar, and a file the list does not name
/// is nobody's gate.
#[rstest]
#[case::a_new_benchmark("gate/benches/b2.rs")]
#[case::another_file("gatekeeper/guard.rs")]
fn a_new_gate_file_or_another_file_is_silent(#[case] path: &str) {
    let (workspace, base) = repository(&[("gatekeeper/guard.rs", "fn original() {}\n")]);
    workspace.write(path, b"fn added() {}\n");

    assert_clean(&guard(&workspace, &base, "feat: extend the gate"));
}

/// The list the guard obeys is the merge base's, so a change cannot free a file
/// by taking it off the list; that is flagged as the Floor's own edit.
#[test]
fn the_merge_bases_list_of_gate_files_decides() {
    let (workspace, base) = repository(&[("gate/guard.rs", "fn original() {}\n")]);
    workspace.edit(TESTING, "`gate/guard.rs`, ", "");
    workspace.write("gate/guard.rs", b"fn loosened() {}\n");

    let outcome = guard(&workspace, &base, "fix: free it");

    assert_eq!(
        outcome.findings(),
        vec![
            finding("gate-file-edited", "gate/guard.rs"),
            finding("row-weakened", "Floor"),
        ],
        "{}",
        outcome.stdout
    );
}

/// A trailer names what it accepts: not another file, and not without a reason.
#[rstest]
#[case::another_file("Quality-change: b.rs: the test needs it", &[("suppression", "a.rs")])]
#[case::no_reason(
    "Quality-change: a.rs:",
    &[("invalid-trailer", "a.rs:"), ("suppression", "a.rs")]
)]
#[case::no_subject(
    "Quality-change: a.rs",
    &[("invalid-trailer", "a.rs"), ("suppression", "a.rs")]
)]
fn a_trailer_accepts_only_what_it_names(#[case] trailer: &str, #[case] expected: &[(&str, &str)]) {
    let (workspace, base) = repository(&[]);
    workspace.write("a.rs", b"#[allow(dead_code)]\nfn a() {}\n");

    let outcome = guard(&workspace, &base, &format!("feat: add it\n\n{trailer}"));

    let expected: Vec<_> = expected
        .iter()
        .map(|(kind, subject)| finding(kind, subject))
        .collect();
    assert_eq!(outcome.findings(), expected, "{}", outcome.stdout);
    assert_eq!(outcome.code, 1);
}

/// A trailer on any commit of the change counts, and what it accepted is still
/// said, so a review can check each one.
#[test]
fn a_trailer_on_an_earlier_commit_counts_and_is_reported() {
    let (workspace, base) = repository(&[]);
    workspace.write("a.rs", b"#[allow(dead_code)]\nfn a() {}\n");
    guard(
        &workspace,
        &base,
        "feat: add it\n\nQuality-change: a.rs: the macro needs it",
    );
    workspace.append("a.rs", "fn b() {}\n");

    let outcome = guard(&workspace, &base, "feat: add more");

    assert_clean(&outcome);
    assert_eq!(outcome.notes(), vec![finding("quality-change", "a.rs")]);
}

/// A commit outside the change names nothing.
#[test]
fn a_trailer_from_before_the_merge_base_does_not_count() {
    let (workspace, _) = repository(&[]);
    guard(
        &workspace,
        "HEAD",
        "feat: earlier\n\nQuality-change: a.rs: long ago",
    );
    let base = workspace.git(&["rev-parse", "HEAD"]).trim().to_owned();
    workspace.write("a.rs", b"#[allow(dead_code)]\nfn a() {}\n");

    let outcome = guard(&workspace, &base, "feat: add it");

    assert_eq!(outcome.findings(), vec![finding("suppression", "a.rs")]);
}

/// A guard that could not run never reads as clean: exit 2 and no verdict.
#[rstest]
#[case::register_gone("## Quality register", "## Something else")]
#[case::column_gone("| Contract | Today |", "| Limit | Today |")]
#[case::row_with_too_few_cells("| `C3.commands` | commands | no cap yet |", "| `C3.commands` |")]
fn an_unreadable_register_is_an_error(#[case] from: &str, #[case] to: &str) {
    let (workspace, base) = repository(&[]);
    workspace.edit(TESTING, from, to);

    let outcome = guard(&workspace, &base, "docs: break it");

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(outcome.stderr.starts_with("error:"), "{}", outcome.stderr);
}

#[test]
fn an_unknown_base_is_an_error() {
    let (workspace, _) = repository(&[]);

    let outcome = workspace.xtask(&["floor-guard", "--base", "no-such-commit", "--head", "HEAD"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
}

/// A merge base from before the register has nothing to weaken, and no list of
/// gate files: this is how the guard's own pull request crosses it.
#[test]
fn a_merge_base_with_no_register_weakens_nothing() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(TESTING, b"# Testing\n");
    let base = workspace.commit_all();
    workspace.write(TESTING, REGISTER.as_bytes());

    assert_clean(&guard(&workspace, &base, "docs: add the register"));
}

/// The repository's own register reads, so a table reshaped for it fails here
/// and not at the next merge.
#[test]
fn the_real_register_reads() {
    let workspace = Workspace::from_fixture("empty");
    workspace.write(TESTING, include_str!("../../../TESTING.md").as_bytes());
    let base = workspace.commit_all();
    workspace.append(TESTING, "\nA line past the register.\n");

    assert_clean(&guard(&workspace, &base, "docs: add a line"));
}
