//! `xtask release`'s release line on a fixture repository (ADR 0032): a
//! minor's cut, behind every release's gates (ADRs 0064 and 0072), the
//! backports its branch lacks and each release's git-cliff range, against
//! local stand-ins of GitHub's interface.

use std::ffi::OsStr;

use rstest::rstest;

use crate::github::{GitHub, Request};
use crate::workspace::{Finding, Outcome, Workspace, finding};

/// A Feature report whose gate is ready on a Release run of `main`.
const READY: &str = r#"{"release_commit": "abc123", "gate": {"ready": true, "untested": []}}"#;

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
fn backports_counts_a_fix_cherry_picked_with_changes_as_the_branchs() {
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
