//! `xtask release`'s release line on a fixture repository (ADR 0032): a
//! minor's cut, the backports its branch lacks, each release's git-cliff
//! range and its changelog entries, by the Products each commit reaches
//! (ADR 0094), and the gates a release and a promotion pass (ADRs 0064,
//! 0066, 0067 and 0072), against local stand-ins of GitHub's interface, the
//! private fuzzing repository's included.

use std::ffi::OsStr;

use rstest::rstest;

use crate::github::{GitHub, Request};
use crate::workspace::{Finding, Outcome, Workspace, finding};

/// A Feature report whose gate is ready on a Release run of `main`.
const READY: &str = r#"{"release_commit": "abc123", "gate": {"ready": true, "untested": []}}"#;

const FORMAT: &str =
    r#"{"name": "protocol", "schema": {"type": "object"}, "storage": "jsonl", "version": "0.1"}"#;

/// The history every test starts from, `main` at `feat: the second
/// Feature`:
///
/// ```text
/// main          first ─ fix ─ second ─ other-fix
///                 │      ╲       │
///                 │       ╲      1.4-branch
///  release/1.3    1.3-branch ─ v1.3.0 ─ fix (cherry-picked)
/// ```
///
/// `fix` and `other-fix` carry the backport label; only `fix` is
/// cherry-picked.
struct Line {
    workspace: Workspace,
    fix: String,
    second: String,
    other_fix: String,
}

fn line() -> Line {
    let workspace = Workspace::from_fixture("release");
    workspace.write("quarantine.toml", b"# The test quarantine list.\n");
    workspace.write("compat/protocol/head/format.json", FORMAT.as_bytes());
    workspace.commit_all_with("feat: the first Feature");
    workspace.git(&["branch", "--move", "main"]);
    for (key, value) in [
        ("user.name", "Releaser"),
        ("user.email", "releaser@example.invalid"),
    ] {
        workspace.git(&["config", key, value]);
    }
    workspace.git(&["tag", "1.3-branch"]);
    workspace.git(&["branch", "release/1.3"]);

    workspace.git(&["switch", "--quiet", "release/1.3"]);
    workspace.commit_all_with("chore(release): 1.3.0");
    workspace.git(&["tag", "v1.3.0"]);

    workspace.git(&["switch", "--quiet", "main"]);
    workspace.write("crates/goodfetch-engine/src/lib.rs", b"// fixed\n");
    let fix = workspace.commit_all_with("fix: the crash");
    workspace.write("crates/goodfetch-engine/src/second.rs", b"// second\n");
    let second = workspace.commit_all_with("feat: the second Feature");
    workspace.git(&["tag", "1.4-branch"]);
    workspace.git(&["branch", "release/1.4"]);
    workspace.write("crates/goodfetch-engine/src/other.rs", b"// other fix\n");
    let other_fix = workspace.commit_all_with("fix: the other crash");

    workspace.git(&["switch", "--quiet", "release/1.3"]);
    workspace.git(&["cherry-pick", "-x", "--allow-empty", &fix]);
    workspace.git(&["switch", "--quiet", "main"]);
    workspace.write("features.json", READY.as_bytes());
    Line {
        workspace,
        fix,
        second,
        other_fix,
    }
}

fn subjects(workspace: &Workspace, range: &str) -> Vec<String> {
    workspace
        .git(&["log", "--format=%s", range])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[rstest]
#[case::first_release("1.3.0-rc.1", "release/1.3", "release/1.3")]
#[case::first_candidate("1.4.0-rc.1", "release/1.4", "1.3-branch..release/1.4")]
#[case::later_candidate("1.4.0-rc.2", "release/1.4", "v1.4.0-rc.1..release/1.4")]
#[case::stable("1.4.0", "release/1.4", "1.3-branch..release/1.4")]
#[case::patch("1.3.1", "release/1.3", "v1.3.0..release/1.3")]
fn range_starts_at_the_previous_minors_cut_point_or_the_previous_release_tag(
    #[case] version: &str,
    #[case] to: &str,
    #[case] range: &str,
) {
    let Line { workspace, .. } = line();
    workspace.git(&["tag", "v1.4.0-rc.1", "release/1.4"]);

    let outcome = workspace.xtask(&["release", "range", "--version", version, "--to", to]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, format!("{range}\n"));
}

#[test]
fn a_cherry_picked_fix_is_listed_under_its_patch_and_again_under_the_next_minor() {
    let Line { workspace, .. } = line();

    let patch = workspace.xtask(&[
        "release",
        "range",
        "--version",
        "1.3.1",
        "--to",
        "release/1.3",
    ]);
    let minor = workspace.xtask(&[
        "release",
        "range",
        "--version",
        "1.4.0-rc.1",
        "--to",
        "release/1.4",
    ]);

    assert_eq!(patch.code, 0, "{}", patch.stderr);
    assert_eq!(minor.code, 0, "{}", minor.stderr);
    assert_eq!(
        subjects(&workspace, patch.stdout.trim()),
        ["fix: the crash"]
    );
    assert_eq!(
        subjects(&workspace, minor.stdout.trim()),
        ["feat: the second Feature", "fix: the crash"]
    );
}

#[test]
fn range_refuses_a_candidate_whose_previous_candidate_is_not_tagged() {
    let Line { workspace, .. } = line();

    let outcome = workspace.xtask(&["release", "range", "--version", "1.4.0-rc.3"]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(outcome.stderr.contains("v1.4.0-rc.2"), "{}", outcome.stderr);
}

/// GitHub's pull requests of each commit `main` has since `1.4-branch`'s
/// parent minor was cut.
fn pulls(line: &Line) -> Vec<(String, String)> {
    let labelled = |number: u32, label: &str| {
        format!(r#"[{{"number": {number}, "labels": [{{"name": "{label}"}}]}}]"#)
    };
    vec![
        (
            format!("/repos/goodfetch/goodfetch/commits/{}/pulls", line.fix),
            labelled(11, "backport"),
        ),
        (
            format!("/repos/goodfetch/goodfetch/commits/{}/pulls", line.second),
            labelled(12, "feature"),
        ),
        (
            format!(
                "/repos/goodfetch/goodfetch/commits/{}/pulls",
                line.other_fix
            ),
            labelled(13, "backport"),
        ),
    ]
}

fn serving(routes: &[(String, String)]) -> GitHub {
    let routes: Vec<(&str, &str)> = routes
        .iter()
        .map(|(path, body)| (path.as_str(), body.as_str()))
        .collect();
    GitHub::serving(&routes)
}

fn with_token(workspace: &Workspace, args: &[&str]) -> Outcome {
    workspace.xtask_with_env(args, &[("GITHUB_TOKEN", OsStr::new("token"))])
}

#[test]
fn backports_lists_the_labelled_commits_the_branch_lacks() {
    let line = line();
    let github = serving(&pulls(&line));

    let outcome = with_token(
        &line.workspace,
        &[
            "release",
            "backports",
            "--minor",
            "1.3",
            "--api-url",
            github.url(),
        ],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        outcome.stdout,
        format!("{} #13 fix: the other crash\n", line.other_fix)
    );
}

#[test]
fn backports_counts_a_fix_cherry_picked_with_changes_as_on_the_branch() {
    let line = line();
    let workspace = &line.workspace;
    workspace.git(&["switch", "--quiet", "release/1.3"]);
    workspace.write(
        "crates/goodfetch-engine/src/other.rs",
        b"// other fix, as the branch takes it\n",
    );
    workspace.git(&["add", "--all", "--", ".", ":!features.json"]);
    workspace.git(&[
        "commit",
        "--quiet",
        "--no-verify",
        "--message",
        &format!(
            "fix: the other crash\n\n(cherry picked from commit {})",
            line.other_fix
        ),
    ]);
    workspace.git(&["switch", "--quiet", "main"]);
    let github = serving(&pulls(&line));

    let outcome = with_token(
        workspace,
        &[
            "release",
            "backports",
            "--minor",
            "1.3",
            "--api-url",
            github.url(),
        ],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(outcome.stdout, "");
}

#[test]
fn backports_refuses_a_minor_never_cut() {
    let line = line();
    let github = serving(&pulls(&line));

    let outcome = with_token(
        &line.workspace,
        &[
            "release",
            "backports",
            "--minor",
            "1.5",
            "--api-url",
            github.url(),
        ],
    );

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(outcome.stderr.contains("1.5-branch"), "{}", outcome.stderr);
    assert_eq!(github.requests(), [] as [Request; 0]);
}

/// A bare repository `origin` that holds `main`.
fn remote(workspace: &Workspace) -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().to_str().unwrap();
    workspace.git(&["init", "--quiet", "--bare", path]);
    workspace.git(&["remote", "add", "origin", path]);
    workspace.git(&["push", "--quiet", "origin", "main"]);
    folder
}

fn remote_ref(workspace: &Workspace, name: &str) -> String {
    workspace
        .git(&["ls-remote", "origin", name])
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned()
}

const PULL_CREATED: &str =
    r#"{"number": 41, "html_url": "https://github.com/goodfetch/goodfetch/pull/41"}"#;

fn cut(workspace: &Workspace, github: &GitHub) -> Outcome {
    with_token(
        workspace,
        &[
            "release",
            "cut",
            "--minor",
            "1.5",
            "--feature-report",
            "features.json",
            "--api-url",
            github.url(),
        ],
    )
}

#[test]
fn cut_branches_tags_main_and_opens_the_first_candidates_pull_request() {
    let Line { workspace, .. } = line();
    let _origin = remote(&workspace);
    let main = workspace.git(&["rev-parse", "main"]).trim().to_owned();
    let github = GitHub::serving(&[("POST /repos/goodfetch/goodfetch/pulls", PULL_CREATED)]);

    let outcome = cut(&workspace, &github);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(remote_ref(&workspace, "refs/heads/release/1.5"), main);
    assert_eq!(remote_ref(&workspace, "refs/tags/1.5-branch"), main);
    assert_eq!(remote_ref(&workspace, "refs/heads/main"), main);
    assert_eq!(workspace.git(&["rev-parse", "main"]).trim(), main);
    let candidate = remote_ref(&workspace, "refs/heads/release-pr/1.5.0-rc.1");
    workspace.git(&["fetch", "--quiet", "origin", "release-pr/1.5.0-rc.1"]);
    assert_eq!(
        workspace
            .git(&["rev-parse", &format!("{candidate}^")])
            .trim(),
        main
    );
    assert_eq!(
        workspace.git(&[
            "log",
            "-1",
            "--format=%s%n%(trailers:key=Signed-off-by)",
            &candidate
        ]),
        "chore(release): 1.5.0-rc.1\nSigned-off-by: Releaser <releaser@example.invalid>\n\n"
    );
    let manifest = workspace.git(&[
        "show",
        &format!("{candidate}:crates/goodfetch-collie/Cargo.toml"),
    ]);
    assert!(manifest.contains("version = \"1.5.0-rc.1\""), "{manifest}");
    assert_eq!(
        workspace.git(&["status", "--porcelain", "--untracked-files=no"]),
        ""
    );
    let writes = github.writes();
    assert_eq!(writes.len(), 1);
    let body: serde_json::Value = serde_json::from_str(&writes[0].body).unwrap();
    assert_eq!(body["base"], "release/1.5");
    assert_eq!(body["head"], "release-pr/1.5.0-rc.1");
    assert_eq!(body["title"], "chore(release): 1.5.0-rc.1");
    assert!(outcome.stdout.contains("pull/41"), "{}", outcome.stdout);
}

/// What a refused cut leaves: no branch, no tag, no pull request.
fn assert_nothing_cut(workspace: &Workspace, github: &GitHub) {
    assert_eq!(remote_ref(workspace, "refs/tags/1.5-branch"), "");
    assert_eq!(
        remote_ref(workspace, "refs/heads/release-pr/1.5.0-rc.1"),
        ""
    );
    assert_eq!(workspace.git(&["tag", "--list", "1.5-branch"]), "");
    assert_eq!(github.writes(), [] as [Request; 0]);
}

#[test]
fn cut_refuses_a_minor_whose_branch_exists() {
    let Line { workspace, .. } = line();
    let _origin = remote(&workspace);
    workspace.git(&["push", "--quiet", "origin", "main:refs/heads/release/1.5"]);
    let github = GitHub::serving(&[("POST /repos/goodfetch/goodfetch/pulls", PULL_CREATED)]);

    let outcome = cut(&workspace, &github);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(outcome.stderr.contains("release/1.5"), "{}", outcome.stderr);
    assert_nothing_cut(&workspace, &github);
}

const QUARANTINED: &str = r#"
[[test]]
runner = "nextest"
id = "goodfetch-engine::it dispatch::route_waits_for_its_frame"
issue = 1601
date = 2026-10-18
"#;

const UNTESTED: &str =
    r#"{"release_commit": "abc123", "gate": {"ready": false, "untested": ["story:1-7"]}}"#;

#[rstest]
#[case::quarantine(
    "quarantine.toml",
    QUARANTINED,
    finding("quarantine-not-empty", "quarantine.toml")
)]
#[case::untested("features.json", UNTESTED, finding("feature-untested", "story:1-7"))]
#[case::no_release_run(
    "features.json",
    r#"{"release_commit": null, "gate": {"ready": false, "untested": []}}"#,
    finding("no-release-run", "features.json")
)]
fn cut_refuses_while_a_release_gate_is_closed(
    #[case] file: &str,
    #[case] contents: &str,
    #[case] refusal: Finding,
) {
    let Line { workspace, .. } = line();
    let _origin = remote(&workspace);
    workspace.write(file, contents.as_bytes());
    let github = GitHub::serving(&[("POST /repos/goodfetch/goodfetch/pulls", PULL_CREATED)]);

    let outcome = cut(&workspace, &github);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), [refusal]);
    assert_nothing_cut(&workspace, &github);
}

/// `release/1.4` with its first candidate tagged, then `xtask release
/// prepare`'s stable version commit, plus whatever `extra` writes.
fn promoted(extra: &[(&str, &str)]) -> Workspace {
    let Line { workspace, .. } = line();
    workspace.git(&["switch", "--quiet", "release/1.4"]);
    workspace.git(&["tag", "v1.4.0-rc.1"]);
    let prepared = workspace.xtask(&["release", "prepare", "--version", "1.4.0"]);
    assert_eq!(prepared.code, 0, "{}", prepared.stderr);
    for (path, contents) in extra {
        workspace.write(path, contents.as_bytes());
    }
    workspace.git(&["add", "--all", "--", ".", ":!features.json"]);
    workspace.git(&[
        "commit",
        "--quiet",
        "--no-verify",
        "--message",
        "chore(release): 1.4.0",
    ]);
    workspace.write("features.json", READY.as_bytes());
    workspace
}

/// The issues GitHub and the private fuzzing repository list, none open.
fn no_issues() -> Vec<(String, String)> {
    vec![
        (
            "/repos/goodfetch/goodfetch/issues?labels=live-check&state=open&per_page=100&page=1"
                .to_owned(),
            "[]".to_owned(),
        ),
        (
            "/repos/goodfetch/goodfetch-fuzzing/issues?state=open&per_page=100&page=1".to_owned(),
            "[]".to_owned(),
        ),
    ]
}

fn check(workspace: &Workspace, github: &GitHub, version: &str, at: &str) -> Outcome {
    with_token(
        workspace,
        &[
            "release",
            "check",
            "--version",
            version,
            "--feature-report",
            "features.json",
            "--api-url",
            github.url(),
            "--at",
            at,
        ],
    )
}

const MONDAY: &str = "2026-11-23T10:00:00Z";

#[test]
fn check_passes_a_stable_commit_that_only_sets_versions_and_adds_the_release_files() {
    let workspace = promoted(&[
        ("CHANGELOG.md", "# Changelog\n"),
        ("website/release-notes/1.4.md", "# 1.4\n"),
        ("bench/results/1.4.0/summary.json", "{}\n"),
    ]);
    let github = serving(&no_issues());

    let outcome = check(&workspace, &github, "1.4.0", MONDAY);

    assert_eq!(outcome.code, 0, "{}\n{}", outcome.stdout, outcome.stderr);
}

#[rstest]
#[case::code("crates/goodfetch-engine/src/lib.rs", "// changed\n")]
#[case::test("crates/goodfetch-engine/tests/it.rs", "// a test\n")]
#[case::build_input("Cargo.toml", "[workspace]\n")]
#[case::other_results("bench/results/1.3.0/summary.json", "{}\n")]
fn check_rejects_a_stable_commit_that_changes_a_code_test_or_build_input(
    #[case] path: &str,
    #[case] contents: &str,
) {
    let workspace = promoted(&[(path, contents)]);
    let github = serving(&no_issues());

    let outcome = check(&workspace, &github, "1.4.0", MONDAY);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), [finding("stable-commit-changes", path)]);
}

#[test]
fn check_rejects_a_stable_commit_that_adds_more_than_one_site_page() {
    let workspace = promoted(&[
        ("website/release-notes/1.4.md", "# 1.4\n"),
        ("website/other.mdx", "# Other\n"),
    ]);
    let github = serving(&no_issues());

    let outcome = check(&workspace, &github, "1.4.0", MONDAY);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [
            finding("stable-commit-changes", "website/other.mdx"),
            finding("stable-commit-changes", "website/release-notes/1.4.md"),
        ]
    );
}

#[test]
fn check_rejects_a_stable_commit_not_on_the_last_candidate() {
    let workspace = promoted(&[]);
    workspace.git(&["tag", "v1.4.0-rc.2", "HEAD"]);
    let github = serving(&no_issues());

    let outcome = check(&workspace, &github, "1.4.0", MONDAY);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(
        outcome.findings(),
        [finding("not-the-last-candidate", "v1.4.0-rc.2")]
    );
}

fn live_check(created_at: &str, comments: &str) -> Vec<(String, String)> {
    let mut routes = no_issues();
    routes[0].1 = format!(
        r#"[{{"number": 70, "created_at": "{created_at}"}}, {{"number": 71, "created_at": "2026-11-01T00:00:00Z", "pull_request": {{}}}}]"#
    );
    routes.push((
        "/repos/goodfetch/goodfetch/issues/70/comments?per_page=100&page=1".to_owned(),
        comments.to_owned(),
    ));
    routes
}

const FUZZ_CRASH: &str = r#"[{"number": 5, "title": "Crash in protocol_frame (2026-11-20)"}, {"number": 6, "title": "Fuzzing runner setup"}]"#;

#[rstest]
#[case::live_check_unexplained(
    live_check("2026-11-18T09:00:00Z", r#"[{"body": "Looking."}]"#),
    finding("live-check-unexplained", "goodfetch/goodfetch#70")
)]
#[case::fuzz_crash(
    {
        let mut routes = no_issues();
        routes[1].1 = FUZZ_CRASH.to_owned();
        routes
    },
    finding("fuzz-crash-open", "goodfetch/goodfetch-fuzzing#5")
)]
fn check_refuses_promotion_while_a_live_check_is_unexplained_or_a_fuzz_crash_open(
    #[case] routes: Vec<(String, String)>,
    #[case] blocker: Finding,
) {
    let workspace = promoted(&[]);
    let github = serving(&routes);

    let outcome = check(&workspace, &github, "1.4.0", MONDAY);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), [blocker]);
}

#[rstest]
#[case::within_two_business_days(live_check("2026-11-20T09:00:00Z", "[]"), MONDAY)]
#[case::opened_at_the_weekend(live_check("2026-11-21T10:00:00Z", "[]"), "2026-11-24T12:00:00Z")]
#[case::cause_stated(
    live_check(
        "2026-11-18T09:00:00Z",
        r#"[{"body": "Looking."}, {"body": "Cause: the vendor's outage, upstream report linked."}]"#
    ),
    MONDAY
)]
fn check_promotes_past_a_live_check_issue_within_its_time_or_with_its_cause(
    #[case] routes: Vec<(String, String)>,
    #[case] at: &str,
) {
    let workspace = promoted(&[]);
    let github = serving(&routes);

    let outcome = check(&workspace, &github, "1.4.0", at);

    assert_eq!(outcome.code, 0, "{}\n{}", outcome.stdout, outcome.stderr);
}

/// `release/1.3`'s tip, whose patch the check judges, after `edit`.
fn patch(edit: impl FnOnce(&Workspace)) -> Workspace {
    let Line { workspace, .. } = line();
    workspace.git(&["switch", "--quiet", "release/1.3"]);
    edit(&workspace);
    workspace.git(&["add", "--all", "--", ".", ":!features.json"]);
    workspace.git(&[
        "commit",
        "--quiet",
        "--no-verify",
        "--allow-empty",
        "--message",
        "fix: on the branch",
    ]);
    workspace
}

#[test]
fn check_holds_a_patch_by_neither_live_checks_nor_fuzz_crashes() {
    let workspace = patch(|_| {});
    let mut routes = live_check("2026-11-02T09:00:00Z", "[]");
    routes[1].1 = FUZZ_CRASH.to_owned();
    let github = serving(&routes);

    let outcome = check(&workspace, &github, "1.3.1", MONDAY);

    assert_eq!(outcome.code, 0, "{}\n{}", outcome.stdout, outcome.stderr);
    assert_eq!(github.requests(), [] as [Request; 0]);
}

#[rstest]
#[case::version(
    r#"{"name": "protocol", "schema": {"type": "object"}, "storage": "jsonl", "version": "0.2"}"#
)]
#[case::schema(
    r#"{"name": "protocol", "schema": {"type": "array"}, "storage": "jsonl", "version": "0.1"}"#
)]
fn check_fails_a_patch_that_changes_a_formats_version_or_schema(#[case] format: &str) {
    let workspace = patch(|workspace| {
        workspace.write("compat/protocol/head/format.json", format.as_bytes());
    });
    let github = serving(&[]);

    let outcome = check(&workspace, &github, "1.3.1", MONDAY);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), [finding("format-changed", "protocol")]);
}

#[test]
fn prepare_writes_nothing_for_a_patch_that_changes_a_format() {
    let workspace = patch(|workspace| {
        workspace.write(
            "compat/protocol/head/format.json",
            FORMAT.replace("0.1", "0.2").as_bytes(),
        );
    });

    let outcome = workspace.xtask(&["release", "prepare", "--version", "1.3.1"]);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), [finding("format-changed", "protocol")]);
    assert_eq!(
        workspace.git(&["status", "--porcelain", "--untracked-files=no"]),
        ""
    );
}

#[test]
fn check_fails_a_patch_that_adds_a_format() {
    let workspace = patch(|workspace| {
        workspace.write("compat/export/head/format.json", FORMAT.as_bytes());
    });
    let github = serving(&[]);

    let outcome = check(&workspace, &github, "1.3.1", MONDAY);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), [finding("format-changed", "export")]);
}

#[rstest]
#[case::quarantine(
    "quarantine.toml",
    QUARANTINED,
    finding("quarantine-not-empty", "quarantine.toml")
)]
#[case::untested("features.json", UNTESTED, finding("feature-untested", "story:1-7"))]
fn check_refuses_a_patch_while_a_release_gate_is_closed(
    #[case] file: &str,
    #[case] contents: &str,
    #[case] refusal: Finding,
) {
    let workspace = patch(|_| {});
    workspace.write(file, contents.as_bytes());
    let github = serving(&[]);

    let outcome = check(&workspace, &github, "1.3.1", MONDAY);

    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
    assert_eq!(outcome.findings(), [refusal]);
}

/// Commits `paths`, each written anew, and nothing else, with `message`.
fn commit_paths(workspace: &Workspace, message: &str, paths: &[&str]) {
    for path in paths {
        workspace.write(path, format!("// {message}\n").as_bytes());
    }
    workspace.git(&[&["add", "--"], paths].concat());
    workspace.git(&["commit", "--quiet", "--no-verify", "--message", message]);
}

/// An earlier release's entries, which the next release's go above.
const EARLIER: &str = "# Changelog\n\n## 1.3.0 (2026-10-01)\n\n### Both Products\n\n#### Added\n\n- The first Feature\n";

/// `1.4.0-rc.1`'s entries as written before, above [`EARLIER`]'s.
const FIRST_CANDIDATE: &str = "# Changelog\n\n## 1.4.0-rc.1 (2026-11-19)\n\n### Both Products\n\n#### Fixed\n\n- The crash\n\n## 1.3.0 (2026-10-01)\n\n### Both Products\n\n#### Added\n\n- The first Feature\n";

/// `1.4.0`'s two candidates' entries above [`EARLIER`]'s.
const CANDIDATES: &str = "# Changelog\n\n## 1.4.0-rc.2 (2026-11-19)\n\n### Foxhound only\n\n#### Added\n\n- **foxhound:** Crawl to a depth\n\n## 1.4.0-rc.1 (2026-11-18)\n\n### Both Products\n\n#### Fixed\n\n- The crash\n\n## 1.3.0 (2026-10-01)\n\n### Both Products\n\n#### Added\n\n- The first Feature\n";

/// `version`'s changelog, from the repository's `cliff.toml`, once
/// `release/1.4` has merged a branch's commit on Foxhound's side alone,
/// and has a commit on both sides and a chore, over `main`'s fix and
/// Feature, with `earlier` as `CHANGELOG.md` before it, snapshotted as
/// `name`. A merge is left out: its branch's commit is listed.
///
/// Covers: story:1-78
#[rstest]
#[case::the_first_release("changelog_of_the_first_release", "1.4.0-rc.1", None)]
#[case::after_an_earlier_release("changelog_after_an_earlier_release", "1.4.0-rc.1", Some(EARLIER))]
#[case::written_again_over_its_own_section(
    "changelog_after_an_earlier_release",
    "1.4.0-rc.1",
    Some(FIRST_CANDIDATE)
)]
#[case::a_stable_minor_over_its_candidates(
    "changelog_of_a_stable_minor_over_its_candidates",
    "1.4.0",
    Some(CANDIDATES)
)]
fn changelog_lists_a_foxhound_only_commit_under_foxhound_and_a_commit_on_both_sides_under_both(
    #[case] name: &str,
    #[case] version: &str,
    #[case] earlier: Option<&str>,
) {
    let Line { workspace, .. } = line();
    workspace.git(&["switch", "--quiet", "--create", "depth", "release/1.4"]);
    commit_paths(
        &workspace,
        "feat(foxhound): crawl to a depth",
        &[
            "crates/foxhound/goodfetch-foxhound/src/depth.rs",
            "sdks/python/foxhound/depth.py",
        ],
    );
    workspace.git(&["switch", "--quiet", "release/1.4"]);
    workspace.git(&[
        "merge",
        "--quiet",
        "--no-ff",
        "--no-verify",
        "--message",
        "feat(foxhound): merge the depth crawl (#9)",
        "depth",
    ]);
    commit_paths(
        &workspace,
        "fix: time out shared requests",
        &[
            "crates/foxhound/goodfetch-foxhound/src/timeout.rs",
            "crates/goodfetch-engine/src/timeout.rs",
        ],
    );
    commit_paths(
        &workspace,
        "chore(foxhound): tidy the crawl",
        &["crates/foxhound/goodfetch-foxhound/src/depth.rs"],
    );
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    workspace.write(
        "cliff.toml",
        &std::fs::read(repository.join("cliff.toml")).unwrap(),
    );
    if let Some(earlier) = earlier {
        workspace.write("CHANGELOG.md", earlier.as_bytes());
    }

    let outcome = workspace.xtask(&[
        "release",
        "changelog",
        "--version",
        version,
        "--at",
        "2026-11-20T12:00:00Z",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    insta::assert_snapshot!(
        name,
        std::fs::read_to_string(workspace.path().join("CHANGELOG.md")).unwrap()
    );
}
