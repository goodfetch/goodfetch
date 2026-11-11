//! The stress run before merge (ADR 0064): each new or changed test of a
//! pull request runs 20 times, and a test it takes off the quarantine list
//! 200 times. `xtask stress select` compares each runner's listing of the
//! fixture workspace at its head with the listing at the base; the runners
//! list the tests for real.
//! This spec's stories stay outside the Feature inventory (#325, Out of
//! Scope), so the module declares no Feature.

use std::path::Path;

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// How many times a new or changed test runs, and a test taken off the
/// quarantine list (ADR 0064).
const CHANGED: &str = "20";
const UNQUARANTINED: &str = "200";

/// The base: the fixture committed, with each runner's installed packages.
fn base() -> (Workspace, String) {
    let workspace = Workspace::from_fixture("stress");
    workspace.install_runners();
    let base = workspace.commit_all();
    (workspace, base)
}

/// The ID each runner lists its renamed test under at the base, which the
/// head no longer lists.
const RENAMED: [&str; 4] = [
    "stressed changed::tests::renamed_before",
    "changed > renamed before",
    "e2e/changed.spec.ts › changed › renamed before",
    "tests/test_changed.py::test_renamed_before",
];

/// In each runner: adds a test to the `changed` file and renames another
/// there, edits the helper the `imports` tests import, and takes the `flaky`
/// test off the quarantine list; then commits, with a `Quality-change`
/// trailer naming each renamed test.
fn change(workspace: &Workspace) {
    edit_each_runner(workspace);
    let trailers: Vec<String> = RENAMED
        .iter()
        .map(|test| format!("Quality-change: {test}: renamed"))
        .collect();
    workspace.commit_all_with(&format!("test: rename a test\n\n{}", trailers.join("\n")));
}

/// `change`'s edits, uncommitted. A runner whose folder the workspace lacks
/// is left alone.
fn edit_each_runner(workspace: &Workspace) {
    workspace.edit(
        "src/changed.rs",
        "fn renamed_before() {}",
        "fn renamed_after() {}\n\n    #[test]\n    fn added() {}",
    );
    let present = |file: &&str| workspace.path().join(file).is_file();
    for file in ["web/changed.test.ts", "e2e/changed.spec.ts"]
        .into_iter()
        .filter(present)
    {
        workspace.edit(
            file,
            "test(\"renamed before\"",
            "test(\"added\", () => {\n    expect(1).toBe(1);\n  });\n\n  test(\"renamed after\"",
        );
    }
    for file in ["web/helper.ts", "e2e/helper.ts"]
        .into_iter()
        .filter(present)
    {
        workspace.write(file, b"export const one = 2 - 1;\n");
    }
    workspace.edit(
        "tests/test_changed.py",
        "def test_renamed_before()",
        "def test_added() -> None:\n    \"\"\"Passes.\"\"\"\n\n\ndef test_renamed_after()",
    );
    workspace.write("quarantine.toml", b"");
}

/// `xtask stress select`'s selection, as `(count, runner, test ID)`, sorted.
fn select(workspace: &Workspace, args: &[&str]) -> Vec<(String, String, String)> {
    let outcome = workspace.xtask(&[&["stress", "select"], args].concat());
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let mut selection: Vec<(String, String, String)> = outcome
        .stdout
        .lines()
        .map(|line| {
            let mut fields = line.splitn(3, '\t').map(str::to_owned);
            (
                fields.next().unwrap(),
                fields.next().unwrap(),
                fields.next().unwrap(),
            )
        })
        .collect();
    selection.sort();
    selection
}

fn selected(count: &str, runner: &str, test: &str) -> (String, String, String) {
    (count.to_owned(), runner.to_owned(), test.to_owned())
}

/// What the change selects when every runner's base listing is right: the
/// `steady` files' `unchanged` tests are not run.
fn change_selection() -> Vec<(String, String, String)> {
    let mut expected = vec![
        selected(CHANGED, "nextest", "stressed changed::tests::added"),
        selected(CHANGED, "nextest", "stressed changed::tests::renamed_after"),
        selected(CHANGED, "nextest", "stressed changed::tests::kept"),
        selected(UNQUARANTINED, "nextest", "stressed steady::tests::flaky"),
        selected(CHANGED, "vitest", "changed > added"),
        selected(CHANGED, "vitest", "changed > renamed after"),
        selected(CHANGED, "vitest", "changed > kept"),
        selected(CHANGED, "vitest", "imports > helper"),
        selected(UNQUARANTINED, "vitest", "steady > flaky"),
        selected(
            CHANGED,
            "playwright",
            "e2e/changed.spec.ts › changed › added",
        ),
        selected(
            CHANGED,
            "playwright",
            "e2e/changed.spec.ts › changed › renamed after",
        ),
        selected(
            CHANGED,
            "playwright",
            "e2e/changed.spec.ts › changed › kept",
        ),
        selected(
            CHANGED,
            "playwright",
            "e2e/imports.spec.ts › imports › helper",
        ),
        selected(
            UNQUARANTINED,
            "playwright",
            "e2e/steady.spec.ts › steady › flaky",
        ),
        selected(CHANGED, "pytest", "tests/test_changed.py::test_added"),
        selected(
            CHANGED,
            "pytest",
            "tests/test_changed.py::test_renamed_after",
        ),
        selected(CHANGED, "pytest", "tests/test_changed.py::test_kept"),
        selected(UNQUARANTINED, "pytest", "tests/test_steady.py::test_flaky"),
    ];
    expected.sort();
    expected
}

/// No saved listing exists, so the base's listings are built from a
/// checkout of the base.
#[test]
fn new_renamed_and_changed_tests_run_20_times_and_unquarantined_ones_200() {
    let (workspace, base) = base();
    change(&workspace);

    let selection = select(&workspace, &["--base", &base]);

    assert_eq!(selection, change_selection());
}

/// The cache-warming job saves the base's listings. Its pytest listing is
/// then cut short of `test_unchanged`, which the selection takes as new: the
/// saved listing is what is read.
#[test]
fn listing_saved_for_the_base_commit_is_read_instead_of_listing_the_base() {
    let (workspace, base) = base();
    let saved = workspace.xtask(&["stress", "listings", "--out", "saved"]);
    assert_eq!(saved.code, 0, "{}", saved.stderr);
    workspace.edit(
        &format!("saved/{base}/pytest.txt"),
        "tests/test_steady.py::test_unchanged\n",
        "",
    );
    change(&workspace);

    let selection = select(&workspace, &["--base", &base, "--saved", "saved"]);

    let mut expected = change_selection();
    expected.push(selected(
        CHANGED,
        "pytest",
        "tests/test_steady.py::test_unchanged",
    ));
    expected.sort();
    assert_eq!(selection, expected);
}

/// A test still on the quarantine list runs in no suite, so not 20 times
/// either, though its file changed.
#[test]
fn quarantined_test_is_not_stressed() {
    let (workspace, base) = base();
    change(&workspace);
    workspace.write(
        "quarantine.toml",
        b"[[test]]\nrunner = \"pytest\"\nid = \"tests/test_changed.py::test_kept\"\nissue = 1601\ndate = 2026-10-18\n",
    );
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base]);

    let mut expected = change_selection();
    expected.retain(|(_, _, test)| test != "tests/test_changed.py::test_kept");
    assert_eq!(selection, expected);
}

/// `steady.rs`'s `flaky` test on the quarantine list, alone.
const FLAKY_ON_THE_LIST: &[u8] = b"[[test]]\nrunner = \"nextest\"\nid = \"stressed steady::tests::flaky\"\nissue = 1601\ndate = 2026-10-18\n";

/// A base whose quarantine list holds no test.
fn base_with_an_empty_list() -> (Workspace, String) {
    let workspace = Workspace::from_fixture("stress");
    workspace.install_runners();
    workspace.write("quarantine.toml", b"");
    let base = workspace.commit_all();
    (workspace, base)
}

/// Commits `flaky` onto the quarantine list, then commits it off.
fn quarantine_and_lift_flaky(workspace: &Workspace) {
    workspace.write("quarantine.toml", FLAKY_ON_THE_LIST);
    workspace.commit_all();
    workspace.write("quarantine.toml", b"");
    workspace.commit_all();
}

/// The base's list never held it and its file is as the base has it, so
/// nothing but the branch's commits say it was ever quarantined.
#[test]
fn a_test_quarantined_and_lifted_on_the_branch_runs_200_times_with_its_file_untouched() {
    let (workspace, base) = base_with_an_empty_list();
    quarantine_and_lift_flaky(&workspace);

    let selection = select(&workspace, &["--base", &base, "--within", "stressed"]);

    assert_eq!(
        selection,
        [selected(
            UNQUARANTINED,
            "nextest",
            "stressed steady::tests::flaky"
        )]
    );
}

/// Its changed file would give it 20 runs: the larger count holds. The
/// file's other test is only changed.
#[test]
fn a_test_quarantined_and_lifted_on_the_branch_runs_200_times_with_its_file_changed() {
    let (workspace, base) = base_with_an_empty_list();
    quarantine_and_lift_flaky(&workspace);
    workspace.edit(
        "src/steady.rs",
        "fn flaky() {}",
        "fn flaky() { let _ = 1; }",
    );
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base, "--within", "stressed"]);

    assert_eq!(
        selection,
        [
            selected(CHANGED, "nextest", "stressed steady::tests::unchanged"),
            selected(UNQUARANTINED, "nextest", "stressed steady::tests::flaky"),
        ]
    );
}

/// The merge leaves the list as both its parents have it, so Git's default
/// history of the file leaves out the side branch that held the entry.
#[test]
fn a_test_quarantined_and_lifted_on_a_merged_branch_runs_200_times() {
    let (workspace, base) = base_with_an_empty_list();
    workspace.git(&["switch", "--quiet", "--create", "side"]);
    quarantine_and_lift_flaky(&workspace);
    workspace.git(&["switch", "--quiet", "--detach", &base]);
    workspace.write("notes.txt", b"a commit the side branch lacks\n");
    workspace.commit_all();
    workspace.git(&["merge", "--quiet", "--no-ff", "--no-edit", "side"]);

    let selection = select(&workspace, &["--base", &base, "--within", "stressed"]);

    assert_eq!(
        selection,
        [selected(
            UNQUARANTINED,
            "nextest",
            "stressed steady::tests::flaky"
        )]
    );
}

/// The other workspace builds a test of its own after this one is copied and
/// frees its target folder first. Were this one to take that folder then, its
/// older sources would read as already built, and its listing would hold the
/// other's test.
#[test]
fn a_workspace_lists_its_own_tests_whatever_another_built_since_its_copy() {
    let (workspace, base) = base_with_an_empty_list();
    let (other, other_base) = base_with_an_empty_list();
    other.edit("src/changed.rs", "fn kept() {}", &rstest_cases(&["case_1"]));
    other.commit_all();
    select(&other, &["--base", &other_base, "--within", "stressed"]);
    drop(other);

    let selection = select(&workspace, &["--base", &base, "--within", "stressed"]);

    assert!(selection.is_empty(), "{selection:?}");
}

/// A runner may hold no test, as Playwright before the first Playwright
/// test exists: it lists none, and the others' tests are still chosen.
#[test]
fn runner_with_no_test_lists_none() {
    let workspace = Workspace::from_fixture("stress");
    workspace.remove_folder("e2e");
    workspace.install_runners();
    let base = workspace.commit_all();
    change(&workspace);

    let selection = select(&workspace, &["--base", &base]);

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner != "playwright");
    assert_eq!(selection, expected);
}

/// A test the base lists and the head does not fails the selection, in
/// every runner, unless a `Quality-change` trailer names its ID (the Floor):
/// the change renames one test in each runner and records none.
#[test]
fn a_test_the_head_no_longer_lists_fails_the_selection() {
    let (workspace, base) = base();
    edit_each_runner(&workspace);
    workspace.commit_all();

    let outcome = workspace.xtask(&["stress", "select", "--base", &base]);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    let mut expected: Vec<_> = RENAMED
        .iter()
        .map(|test| finding("deleted-test", test))
        .collect();
    let mut findings = outcome.stderr_findings();
    expected.sort_by(|a, b| a.subject.cmp(&b.subject));
    findings.sort_by(|a, b| a.subject.cmp(&b.subject));
    assert_eq!(findings, expected, "{}", outcome.stderr);
}

/// `steady.rs`'s `unchanged` test leaves it, and `changed.rs` gains a test
/// of that name: a removed test and an added one, which the floor guard's
/// reading of the diff's lines takes for a move.
fn move_unchanged(workspace: &Workspace) {
    workspace.edit(
        "src/steady.rs",
        "    #[test]\n    fn unchanged() {}\n\n",
        "",
    );
    workspace.edit(
        "src/changed.rs",
        "fn kept() {}",
        "fn kept() {}\n\n    #[test]\n    fn unchanged() {}",
    );
}

/// The test's ID is what the runner collects: one of the same name added in
/// another file does not stand in for it.
#[test]
fn a_test_moved_to_another_module_is_a_deleted_one() {
    let (workspace, base) = base();
    move_unchanged(&workspace);
    workspace.commit_all();

    let outcome = workspace.xtask(&["stress", "select", "--base", &base]);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.stderr_findings(),
        [finding("deleted-test", "stressed steady::tests::unchanged")],
        "{}",
        outcome.stderr
    );
}

/// A trailer naming the test's ID accepts it, as a `note[quality-change]`
/// line, and the selection is printed.
#[test]
fn a_trailer_naming_a_deleted_test_accepts_it() {
    let (workspace, base) = base();
    move_unchanged(&workspace);
    workspace.commit_all_with(
        "test: move a test\n\nQuality-change: stressed steady::tests::unchanged: moved to changed.rs",
    );

    let outcome = workspace.xtask(&["stress", "select", "--base", &base]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        outcome
            .stdout
            .lines()
            .any(|line| line == "20\tnextest\tstressed changed::tests::unchanged"),
        "{}",
        outcome.stdout
    );
    assert_eq!(
        outcome.stderr_notes(),
        [finding(
            "quality-change",
            "stressed steady::tests::unchanged"
        )],
        "{}",
        outcome.stderr
    );
}

/// The tests `changed.rs` gains in a module `reads`, as rstest generates
/// the cases of a test `reads`: one test per case, `case_<n>_<description>`
/// or `case_<n>` with none, `<n>` its position zero-padded to the width of
/// the case count. The fixture crate has no dependencies, so the module
/// spells them out.
fn rstest_cases(names: &[&str]) -> String {
    let tests: Vec<String> = names
        .iter()
        .map(|name| format!("        #[test]\n        fn {name}() {{}}\n"))
        .collect();
    format!(
        "fn kept() {{}}\n\n    mod reads {{\n{}    }}",
        tests.join("\n")
    )
}

/// A case's position is not its name: inserting a case renumbers the ones
/// after it, which the head still lists under their descriptions. A case
/// with no description has its position alone, and a renamed test any
/// other new name; each the head lacks is deleted.
#[rstest]
#[case::an_inserted_case_renumbers_the_ones_after_it(
    &["case_1_first", "case_2_second"],
    &["case_1_inserted", "case_2_first", "case_3_second"],
    &[]
)]
#[case::a_removed_case_is_deleted(
    &["case_1_first", "case_2_second"],
    &["case_1_second"],
    &["case_1_first"]
)]
#[case::a_renamed_test_is_deleted(&["before"], &["after"], &["before"])]
#[case::a_case_with_no_description_is_matched_on_its_position(
    &["case_1", "case_2", "case_3"],
    &["case_1", "case_2"],
    &["case_3"]
)]
fn a_test_case_is_deleted_when_the_head_lists_none_of_its_name(
    #[case] before: &[&str],
    #[case] after: &[&str],
    #[case] deleted: &[&str],
) {
    let workspace = Workspace::from_fixture("stress");
    workspace.edit("src/changed.rs", "fn kept() {}", &rstest_cases(before));
    workspace.install_runners();
    let base = workspace.commit_all();
    workspace.edit(
        "src/changed.rs",
        &rstest_cases(before),
        &rstest_cases(after),
    );
    workspace.commit_all();

    let outcome = workspace.xtask(&["stress", "select", "--base", &base, "--within", "stressed"]);

    let expected: Vec<_> = deleted
        .iter()
        .map(|test| {
            finding(
                "deleted-test",
                &format!("stressed changed::tests::reads::{test}"),
            )
        })
        .collect();
    assert_eq!(outcome.stderr_findings(), expected, "{}", outcome.stderr);
    assert_eq!(
        outcome.code,
        i32::from(!deleted.is_empty()),
        "{}",
        outcome.stderr
    );
}

/// The incremental gate (ADR 0135) stresses only the members the change
/// since an earlier gate run reaches: the crate `stressed` here, so only
/// nextest lists and only its new and changed tests run.
#[test]
fn within_members_only_their_new_and_changed_tests_run() {
    let (workspace, base) = base();
    change(&workspace);

    let selection = select(&workspace, &["--base", &base, "--within", "stressed"]);

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner == "nextest");
    assert_eq!(selection, expected);
}

/// The tests no workspace member owns (the fixture's `web/`, `e2e/` and
/// `tests/`) are stressed within `:unowned`, which every runner but nextest
/// lists.
#[test]
fn within_the_unowned_tests_their_new_and_changed_tests_run() {
    let (workspace, base) = base();
    change(&workspace);

    let selection = select(&workspace, &["--base", &base, "--within", ":unowned"]);

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner != "nextest");
    assert_eq!(selection, expected);
}

/// Every test the change since the earlier gate run changes runs 20 times
/// too, even one the pull request as a whole leaves as its base had it:
/// `steady.rs` changed in the earlier run's head and changed back since.
#[test]
fn a_test_the_change_since_the_earlier_run_changes_runs_20_times() {
    let (workspace, base) = base();
    change(&workspace);
    workspace.append("src/steady.rs", "// Changed.\n");
    let earlier = workspace.commit_all();
    workspace.edit("src/steady.rs", "// Changed.\n", "");
    workspace.commit_all();

    let selection = select(
        &workspace,
        &["--base", &base, "--within", "stressed", "--since", &earlier],
    );

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner == "nextest");
    expected.push(selected(
        CHANGED,
        "nextest",
        "stressed steady::tests::unchanged",
    ));
    expected.sort();
    assert_eq!(selection, expected);
}

/// An earned file as `xtask gate plan` writes it, holding `passes` clean passes
/// of `test`.
fn earned_file(workspace: &Workspace, passes: u32, test: &str) -> String {
    let file = workspace.path().join("target/run.earned");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    write_earned_file(&file, now, passes, test);
    file.display().to_string()
}

fn write_earned_file(file: &Path, at: u64, passes: u32, test: &str) {
    std::fs::write(file, format!("stressed\tearlier\t{at}\t{passes}\t{test}\n")).unwrap();
}

/// The plan checks the age of the passes it keeps; a day can pass before the
/// stress run reads them, so the selection checks it too, and a test whose
/// passes have aged runs its whole count.
#[test]
fn earned_passes_older_than_a_day_when_the_selection_reads_them_count_for_nothing() {
    let (workspace, base) = base();
    change(&workspace);
    let kept = "stressed changed::tests::added";
    let earned = earned_file(&workspace, 7, kept);
    write_earned_file(Path::new(&earned), 10, 7, kept);

    let selection = select(
        &workspace,
        &["--base", &base, "--within", "stressed", "--earned", &earned],
    );

    assert!(
        selection.contains(&selected("20", "nextest", kept)),
        "{selection:?}"
    );
}

/// The incremental gate's earned passes (#2233): a test with earlier clean
/// passes runs what its count still owes, and one that has its count runs no
/// more; a test with none runs its whole count.
#[rstest]
#[case::owes_the_rest(7, Some("13"))]
#[case::owes_one(19, Some("1"))]
#[case::has_its_count(20, None)]
fn a_test_with_earned_passes_runs_only_what_its_count_still_owes(
    #[case] passes: u32,
    #[case] owed: Option<&str>,
) {
    let (workspace, base) = base();
    change(&workspace);
    let kept = "stressed changed::tests::added";
    let earned = earned_file(&workspace, passes, kept);

    let selection = select(
        &workspace,
        &["--base", &base, "--within", "stressed", "--earned", &earned],
    );

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner == "nextest");
    expected.retain(|(_, _, test)| test != kept);
    if let Some(owed) = owed {
        expected.push(selected(owed, "nextest", kept));
    }
    expected.sort();
    assert_eq!(selection, expected);
}

/// The backstop's stress run covers merges `main` already holds, each judged at
/// its own gate. A test deleted since the base with no trailer in the range, as
/// a merge by hand could leave it, is noted and does not block; the test renamed
/// from it runs at its new name, as the change's new test; a member the
/// workspace no longer has is noted too. Without `--merged`, the selection still
/// refuses the deleted test.
#[test]
fn a_merged_range_notes_what_is_gone_and_stresses_the_rest() {
    let (workspace, base) = base();
    move_unchanged(&workspace);
    workspace.commit_all_with("test: move a test, with no trailer");

    let refused = workspace.xtask(&["stress", "select", "--base", &base, "--within", "stressed"]);
    let noted = workspace.xtask(&[
        "stress",
        "select",
        "--base",
        &base,
        "--within",
        "stressed",
        "--within",
        "a-crate-removed-since",
        "--merged",
    ]);

    assert_ne!(refused.code, 0, "{}", refused.stdout);
    assert_eq!(noted.code, 0, "{}", noted.stderr);
    assert!(
        noted
            .stdout
            .lines()
            .any(|line| line == "20\tnextest\tstressed changed::tests::unchanged"),
        "{}",
        noted.stdout
    );
    for gone in ["stressed steady::tests::unchanged", "a-crate-removed-since"] {
        assert!(noted.stderr.contains(gone), "{gone}: {}", noted.stderr);
    }
}

/// Commits the workspace's edits on a branch `name` with `message`, and merges
/// it into the branch checked out as `just merge` merges a pull request: with
/// a merge commit.
fn merge_edits(workspace: &Workspace, name: &str, message: &str) {
    workspace.git(&["switch", "--quiet", "--create", name]);
    workspace.commit_all_with(message);
    workspace.git(&["switch", "--quiet", "-"]);
    workspace.git(&["merge", "--quiet", "--no-ff", "--no-edit", name]);
}

/// Adds a test `name` to the `steady` file's Rust tests, uncommitted.
fn add_a_steady_test(workspace: &Workspace, name: &str) {
    workspace.edit(
        "src/steady.rs",
        "fn unchanged() {}",
        &format!("fn unchanged() {{}}\n\n    #[test]\n    fn {name}() {{}}"),
    );
}

/// `just merge`'s full gate hands the stress run the head of the last full
/// gate as its base (ADR 0135's amendment of 2026-11-10, #2455): the tests
/// two pull requests merged since added, as the quick gate merges them, run
/// 20 times with the one the gated pull request adds.
#[test]
fn from_the_last_full_gates_head_the_tests_merged_since_run_20_times_with_the_changes_own() {
    let (workspace, last_full_gate) = base();
    workspace.edit(
        "src/changed.rs",
        "fn kept() {}",
        "fn kept() {}\n\n    #[test]\n    fn first_quick_merge() {}",
    );
    merge_edits(&workspace, "first-quick-merge", "test: add a test");
    workspace.edit(
        "tests/test_changed.py",
        "def test_kept()",
        "def test_second_quick_merge() -> None:\n    \"\"\"Passes.\"\"\"\n\n\ndef test_kept()",
    );
    merge_edits(&workspace, "second-quick-merge", "test: add another test");
    add_a_steady_test(&workspace, "third_pull_request");
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &last_full_gate]);

    for (runner, test) in [
        ("nextest", "stressed changed::tests::first_quick_merge"),
        ("pytest", "tests/test_changed.py::test_second_quick_merge"),
        ("nextest", "stressed steady::tests::third_pull_request"),
    ] {
        assert!(
            selection.contains(&selected(CHANGED, runner, test)),
            "{runner} {test} not in {selection:?}"
        );
    }
}

/// The next pull request to take a full gate, on a base where a pull request
/// merged since the last full gate renamed the test `renamed_before`, its
/// commit saying `message`. The repository and the last full gate's head.
fn a_merge_since_the_last_full_gate_renamed_a_test(message: &str) -> (Workspace, String) {
    let (workspace, last_full_gate) = base();
    workspace.edit(
        "src/changed.rs",
        "fn renamed_before() {}",
        "fn renamed_after() {}",
    );
    merge_edits(&workspace, "quick-merge", message);
    add_a_steady_test(&workspace, "next_pull_request");
    workspace.commit_all();
    (workspace, last_full_gate)
}

/// The deleted tests are read from that same range, and so are the trailers
/// that accept them: a test a pull request merged since the last full gate
/// renamed is accepted, at the next full gate, by the `Quality-change` trailer
/// of that pull request's commit.
#[test]
fn a_test_deleted_by_a_merge_since_the_last_full_gate_is_accepted_by_that_merges_trailer() {
    let (workspace, last_full_gate) = a_merge_since_the_last_full_gate_renamed_a_test(
        "test: rename a test\n\nQuality-change: stressed changed::tests::renamed_before: renamed",
    );

    let outcome = workspace.xtask(&[
        "stress",
        "select",
        "--base",
        &last_full_gate,
        "--within",
        "stressed",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stderr_notes(),
        [finding(
            "quality-change",
            "stressed changed::tests::renamed_before"
        )],
        "{}",
        outcome.stderr
    );
}

/// Without that trailer the renamed test fails the selection at the next full
/// gate, as a test the gated pull request deleted itself would.
#[test]
fn a_test_deleted_by_a_merge_since_the_last_full_gate_fails_the_selection_without_a_trailer() {
    let (workspace, last_full_gate) =
        a_merge_since_the_last_full_gate_renamed_a_test("test: rename a test");

    let outcome = workspace.xtask(&[
        "stress",
        "select",
        "--base",
        &last_full_gate,
        "--within",
        "stressed",
    ]);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.stderr_findings(),
        [finding(
            "deleted-test",
            "stressed changed::tests::renamed_before"
        )],
        "{}",
        outcome.stderr
    );
}

/// A test taken off the quarantine list owes the rest of its 200.
#[test]
fn a_test_taken_off_the_quarantine_list_owes_the_rest_of_its_200() {
    let (workspace, base) = base();
    change(&workspace);
    let test = "stressed steady::tests::flaky";
    let earned = earned_file(&workspace, 20, test);

    let selection = select(
        &workspace,
        &["--base", &base, "--within", "stressed", "--earned", &earned],
    );

    assert!(
        selection.contains(&selected("180", "nextest", test)),
        "{selection:?}"
    );
}

/// An earned file that is missing or cannot be read earns nothing: every test
/// runs its whole count.
#[rstest]
#[case::missing(None)]
#[case::unreadable(Some("not a line\n"))]
fn an_earned_file_that_cannot_be_read_owes_every_whole_count(#[case] text: Option<&str>) {
    let (workspace, base) = base();
    change(&workspace);
    let file = workspace.path().join("target/run.earned");
    if let Some(text) = text {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
    }

    let selection = select(
        &workspace,
        &[
            "--base",
            &base,
            "--within",
            "stressed",
            "--earned",
            &file.to_string_lossy(),
        ],
    );

    let mut expected = change_selection();
    expected.retain(|(_, runner, _)| runner == "nextest");
    assert_eq!(selection, expected);
}

/// A branch behind its base, as one that has not merged its target lately:
/// the base's Vitest config imports an entry of a workspace package that the
/// head does not have. The base is listed with its own packages, so its
/// config loads, and the test the change edits is chosen.
#[test]
fn changed_vitest_test_is_selected_when_the_base_config_imports_what_the_head_lacks() {
    let (workspace, base) = base();
    workspace.edit(
        "web-support/package.json",
        r#""./include": "./include.mjs""#,
        r#""./tests": "./include.mjs""#,
    );
    workspace.edit(
        "vitest.config.mjs",
        r#""web-support/include""#,
        r#""web-support/tests""#,
    );
    workspace.edit(
        "web/changed.test.ts",
        r#"test("kept""#,
        "test(\"added\", () => {\n    expect(1).toBe(1);\n  });\n\n  test(\"kept\"",
    );
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base]);

    assert!(
        selection.contains(&selected(CHANGED, "vitest", "changed > added")),
        "{selection:?}"
    );
}

/// A parameterized test is one test case per parameter, each run under its
/// own name: Vitest's `test.for` formats its title, pytest's `parametrize`
/// adds the parameter's ID to the node ID, and Playwright's loop titles each
/// test. The change adds one in each runner's `changed` file, and each case
/// is chosen by the name it runs under.
#[test]
fn each_case_of_a_parameterized_test_is_selected_by_the_name_it_runs_under() {
    let (workspace, base) = base();
    workspace.edit(
        "web/changed.test.ts",
        r#"test("kept""#,
        "test.for([\"a.log\", \"b\"])(\"reads %s\", (name) => {\n    expect(name).toBeTruthy();\n  });\n\n  test(\"kept\"",
    );
    workspace.edit(
        "e2e/changed.spec.ts",
        r#"test("kept""#,
        "for (const name of [\"a.log\", \"b\"]) {\n    test(`reads ${name}`, () => {\n      expect(name).toBeTruthy();\n    });\n  }\n\n  test(\"kept\"",
    );
    workspace.edit(
        "tests/test_changed.py",
        "def test_kept()",
        "import pytest\n\n\n@pytest.mark.parametrize(\"name\", [\"a.log\", \"b\"])\ndef test_reads(name: str) -> None:\n    \"\"\"Passes.\"\"\"\n    assert name\n\n\ndef test_kept()",
    );
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base]);

    for (runner, test) in [
        ("vitest", "changed > reads a.log"),
        ("vitest", "changed > reads b"),
        ("playwright", "e2e/changed.spec.ts › changed › reads a.log"),
        ("playwright", "e2e/changed.spec.ts › changed › reads b"),
        ("pytest", "tests/test_changed.py::test_reads[a.log]"),
        ("pytest", "tests/test_changed.py::test_reads[b]"),
    ] {
        assert!(
            selection.contains(&selected(CHANGED, runner, test)),
            "{runner} {test} not in {selection:?}"
        );
    }
    assert!(
        !selection.iter().any(|(_, _, test)| test.contains("%s")),
        "{selection:?}"
    );
}

/// A branch behind its base, as above: the base's `conftest.py` imports a
/// name of a workspace package that the head does not have. The base is
/// listed with its own Python environment, so it collects, and the test the
/// change adds is chosen.
#[test]
fn changed_pytest_test_is_selected_when_the_base_conftest_imports_what_the_head_lacks() {
    let (workspace, base) = base();
    workspace.edit(
        "py-support/src/py_support/__init__.py",
        "included = True",
        "supported = True",
    );
    workspace.edit(
        "tests/conftest.py",
        "from py_support import included\n\nassert included",
        "from py_support import supported\n\nassert supported",
    );
    workspace.edit(
        "tests/test_changed.py",
        "def test_kept()",
        "def test_added() -> None:\n    \"\"\"Passes.\"\"\"\n\n\ndef test_kept()",
    );
    workspace.commit_all();

    let selection = select(&workspace, &["--base", &base]);

    assert!(
        selection.contains(&selected(
            CHANGED,
            "pytest",
            "tests/test_changed.py::test_added"
        )),
        "{selection:?}"
    );
}

/// A test that fails on its second run alone: it counts its runs in the
/// crate's `runs` file.
const RED_ON_ITS_SECOND_RUN: &str = r#"
#[cfg(test)]
mod counted {
    #[test]
    fn red_on_its_second_run() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/runs");
        let run = std::fs::read_to_string(path).map_or(0, |text| text.trim().parse::<u32>().unwrap()) + 1;
        std::fs::write(path, run.to_string()).unwrap();
        assert_ne!(run, 2, "red on its second run");
    }
}
"#;

/// The repository's justfile and `ci/` in the `quarantine` fixture, whose
/// `sample` crate's tests nextest runs for real, with
/// `counted::red_on_its_second_run` added. cargo is a stand-in for what
/// the recipes ask of xtask (`stress select` prints `selection`, `repeat`
/// runs the real xtask) and is cargo for the rest; pnpm and uv are
/// stand-ins. Each records its arguments.
#[cfg(unix)]
struct Recipes {
    workspace: Workspace,
    calls: std::path::PathBuf,
    path: String,
}

#[cfg(unix)]
fn recipes(selection: &str) -> Recipes {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = Workspace::from_fixture("quarantine");
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    std::fs::copy(
        repository.join("justfile"),
        workspace.path().join("justfile"),
    )
    .unwrap();
    std::os::unix::fs::symlink(repository.join("ci"), workspace.path().join("ci")).unwrap();
    workspace.append("src/lib.rs", RED_ON_ITS_SECOND_RUN);
    workspace.write(
        ".config/nextest.toml",
        b"[profile.ci]\nfail-fast = false\n\n[profile.ci.junit]\npath = \"junit.xml\"\n",
    );
    workspace.write(".gitignore", b"target\nstand-ins\nruns\n");
    workspace.write("selection.tsv", selection.as_bytes());
    let stand_ins = workspace.path().join("stand-ins");
    std::fs::create_dir_all(&stand_ins).unwrap();
    let calls = stand_ins.join("calls");
    let outer = std::env::var("PATH").unwrap();
    let cargo = format!(
        r#"#!/bin/sh
echo "cargo $*" >>"{calls}"
case " $* " in
  *" -- stress select "*) cat "{selection}"; exit 0 ;;
  *" -- test-tools "*) mkdir -p target; echo '{{}}' >target/test-tools.json; exit 0 ;;
esac
if [ "$1" = run ]; then
  while [ "$1" != -- ]; do shift; done
  shift
  exec "{xtask}" "$@"
fi
PATH="{outer}" exec cargo "$@"
"#,
        calls = calls.display(),
        selection = workspace.path().join("selection.tsv").display(),
        xtask = env!("CARGO_BIN_EXE_xtask"),
    );
    let recorder =
        |name: &str| format!("#!/bin/sh\necho \"{name} $*\" >>\"{}\"\n", calls.display());
    for (name, script) in [
        ("cargo", cargo),
        ("pnpm", recorder("pnpm")),
        ("uv", recorder("uv")),
    ] {
        let path = stand_ins.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    workspace.commit_all();
    Recipes {
        workspace,
        calls,
        path: format!("{}:{outer}", stand_ins.display()),
    }
}

#[cfg(unix)]
struct Ran {
    code: Option<i32>,
    output: String,
    calls: Vec<String>,
}

/// A JUnit file's tests, as `(name, passed runs, failed runs)`.
#[cfg(unix)]
type Tests = Vec<(String, usize, usize)>;

#[cfg(unix)]
impl Recipes {
    /// Runs the real `just <args>` with the stand-ins first on PATH.
    fn just(&self, args: &[&str]) -> Ran {
        let output = self
            .workspace
            .command("just")
            .args(args)
            .env("PATH", &self.path)
            .env("HOME", self.workspace.path())
            .output()
            .unwrap();
        Ran {
            code: output.status.code(),
            output: format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            calls: std::fs::read_to_string(&self.calls)
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect(),
        }
    }

    /// How many times `counted::red_on_its_second_run` ran.
    fn runs(&self) -> u32 {
        self.workspace.read("runs").map_or(0, |text| {
            String::from_utf8(text).unwrap().trim().parse().unwrap()
        })
    }

    /// The tests of the nextest JUnit files in `folder`, with their runs
    /// in all the files.
    fn junit(&self, folder: &str) -> Tests {
        let Ok(entries) = std::fs::read_dir(self.workspace.path().join(folder)) else {
            return Vec::new();
        };
        let mut tests: std::collections::BTreeMap<String, (usize, usize)> =
            std::collections::BTreeMap::new();
        for entry in entries {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if !name.starts_with("nextest") {
                continue;
            }
            let text = self.workspace.read_text(&format!("{folder}/{name}"));
            for case in text.split("<testcase ").skip(1) {
                let name = case
                    .split("name=\"")
                    .nth(1)
                    .unwrap()
                    .split('"')
                    .next()
                    .unwrap();
                let body = case.split("</testcase>").next().unwrap();
                let entry = tests.entry(name.to_owned()).or_default();
                if body.contains("<failure") {
                    entry.1 += 1;
                } else {
                    entry.0 += 1;
                }
            }
        }
        tests
            .into_iter()
            .map(|(test, (passed, failed))| (test, passed, failed))
            .collect()
    }
}

/// A red stress run stops at the end of its first red iteration (the
/// test-gate audit's option 2, ADR 0064's amendment of 2026-10-28): nextest
/// runs that iteration through, every selected test once, and nothing runs
/// after it: neither the count's later iterations, nor its later runners,
/// nor the 200 runs. The run says nextest's command ended as a failing test
/// ends it.
#[cfg(unix)]
#[test]
fn a_red_stress_run_stops_at_the_end_of_its_first_red_iteration() {
    let recipes = recipes(
        "20\tnextest\tsample counted::red_on_its_second_run\n\
         20\tnextest\tsample tests::steady\n\
         20\tpytest\ttests/test_sample.py::test_flaky\n\
         200\tnextest\tsample tests::steady_twin\n",
    );

    let ran = recipes.just(&["stress", "HEAD"]);

    assert_ne!(ran.code, Some(0), "{}", ran.output);
    assert_eq!(recipes.runs(), 2, "{}", ran.output);
    // Two iterations, each through: the red test passed once and failed
    // once, and the test beside it passed twice.
    assert_eq!(
        recipes.junit("target/junit/repeat-20"),
        [
            ("counted::red_on_its_second_run".to_owned(), 1, 1),
            ("tests::steady".to_owned(), 2, 0),
        ],
        "{}",
        ran.output
    );
    assert_eq!(recipes.junit("target/junit/repeat-200"), []);
    assert!(
        !ran.calls.iter().any(|call| call.starts_with("uv ")),
        "{:?}",
        ran.calls
    );
    assert_eq!(
        recipes.workspace.read_text(RED_COMMANDS),
        "nextest\t100\n",
        "{}",
        ran.output
    );
}

/// How many times a run built the test tool manifest.
#[cfg(unix)]
fn test_tools_builds(ran: &Ran) -> usize {
    ran.calls
        .iter()
        .filter(|call| call.contains(" -- test-tools "))
        .count()
}

/// A green stress run runs every iteration of each count the selection gives,
/// 200 for a test taken off the quarantine list among them, so no count can
/// be left out of a green job; it builds the test tool manifest once, for its
/// listings and every count's runs alike.
#[cfg(unix)]
#[test]
fn a_green_stress_run_runs_every_iteration_of_each_count_on_one_test_tools_build() {
    let recipes =
        recipes("20\tnextest\tsample tests::steady\n200\tnextest\tsample tests::steady\n");

    let ran = recipes.just(&["stress", "HEAD"]);

    assert_eq!(ran.code, Some(0), "{}", ran.output);
    assert_eq!(
        recipes.junit("target/junit/repeat-20"),
        [("tests::steady".to_owned(), 20, 0)],
        "{}",
        ran.output
    );
    assert_eq!(
        recipes.junit("target/junit/repeat-200"),
        [("tests::steady".to_owned(), 200, 0)],
        "{}",
        ran.output
    );
    assert_eq!(test_tools_builds(&ran), 1, "{:?}", ran.calls);
}

/// #2233: the recipe runs what the selection owes each test, smallest count
/// first, and hands `xtask stress select` the earned file. A green run reports
/// no red command.
#[cfg(unix)]
#[test]
fn a_stress_run_runs_each_tests_owed_count_and_hands_on_the_earned_file() {
    let recipes =
        recipes("3\tnextest\tsample tests::steady\n20\tnextest\tsample tests::steady_twin\n");

    let ran = recipes.just(&["stress", "HEAD", "", "the.earned", "", "sample"]);

    assert_eq!(ran.code, Some(0), "{}", ran.output);
    assert_eq!(recipes.workspace.read(RED_COMMANDS), None, "{}", ran.output);
    assert_eq!(
        recipes.junit("target/junit/repeat-3"),
        [("tests::steady".to_owned(), 3, 0)],
        "{}",
        ran.output
    );
    assert_eq!(
        recipes.junit("target/junit/repeat-20"),
        [("tests::steady_twin".to_owned(), 20, 0)],
        "{}",
        ran.output
    );
    assert!(
        ran.calls
            .iter()
            .any(|call| call.contains(" -- stress select ")
                && call.contains("--within sample")
                && call.contains("--earned the.earned")
                && !call.contains("--merged")),
        "{:?}",
        ran.calls
    );
}

/// The backstop's stress run says the range is on `main` already.
#[cfg(unix)]
#[test]
fn a_stress_run_of_merges_hands_on_that_they_are_merged() {
    let recipes = recipes("");

    let ran = recipes.just(&["stress", "HEAD", "", "", "merged", "sample"]);

    assert_eq!(ran.code, Some(0), "{}", ran.output);
    assert!(
        ran.calls
            .iter()
            .any(|call| call.contains(" -- stress select ") && call.contains("--merged")),
        "{:?}",
        ran.calls
    );
}

/// Where the stress run says which runner's command went red and how it ended,
/// which `xtask gate record` judges a red run's crates by.
#[cfg(unix)]
const RED_COMMANDS: &str = "target/stress-red-commands.tsv";

/// A test that owes one pass runs one command, and a red one still stops the
/// run: no other runner runs after it.
#[cfg(unix)]
#[test]
fn a_red_stress_run_that_owes_one_pass_stops_at_it() {
    let recipes = recipes(
        "1\tnextest\tsample counted::red_on_its_second_run\n\
         1\tpytest\ttests/test_sample.py::test_flaky\n",
    );
    recipes.workspace.write("runs", b"1\n");

    let ran = recipes.just(&["stress", "HEAD", "", "the.earned", "", "sample"]);

    assert_ne!(ran.code, Some(0), "{}", ran.output);
    assert!(
        !ran.calls.iter().any(|call| call.starts_with("uv ")),
        "{:?}",
        ran.calls
    );
    assert_eq!(
        recipes.workspace.read_text(RED_COMMANDS),
        "nextest\t100\n",
        "{}",
        ran.output
    );
}

/// `just repeat` builds the test tool manifest once before its runs.
#[cfg(unix)]
#[test]
fn a_repeat_run_builds_the_test_tools_once() {
    let recipes = recipes("");

    let ran = recipes.just(&["repeat", "1", "--test", "nextest=sample tests::steady"]);

    assert_eq!(ran.code, Some(0), "{}", ran.output);
    assert_eq!(test_tools_builds(&ran), 1, "{:?}", ran.calls);
}

/// `just repeat` outside the stress run, as a diagnostic run and the daily
/// test-health job run it, runs every one of its runs past a red one.
#[cfg(unix)]
#[test]
fn repeat_runs_every_run_past_a_red_one() {
    let recipes = recipes("");

    let ran = recipes.just(&[
        "repeat",
        "3",
        "--test",
        "nextest=sample counted::red_on_its_second_run",
    ]);

    assert_ne!(ran.code, Some(0), "{}", ran.output);
    assert_eq!(recipes.runs(), 3, "{}", ran.output);
}
