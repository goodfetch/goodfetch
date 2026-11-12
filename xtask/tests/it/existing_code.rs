//! An issue's `Existing code` list and the sweep that sets aside a
//! `ready-for-agent` issue whose evidence moved (ADR 0144). Issues come from
//! a local stand-in for GitHub; the citations are read from a fixture
//! repository whose `origin/main` each test sets.

use rstest::rstest;
use serde_json::{Value, json};

use crate::github::{GitHub, Request};
use crate::workspace::{Outcome, Workspace, finding};

const ISSUE: &str = "/repos/goodfetch/goodfetch/issues/7";
const READY: &str =
    "/repos/goodfetch/goodfetch/issues?labels=ready-for-agent&state=open&per_page=100&page=1";
const COMMENTS: &str = "/repos/goodfetch/goodfetch/issues/7/comments?per_page=100&page=1";
const POST_COMMENT: &str = "POST /repos/goodfetch/goodfetch/issues/7/comments";
const PATCH_ISSUE: &str = "PATCH /repos/goodfetch/goodfetch/issues/7";

/// `keep` is lines 3 to 5.
const LIBRARY: &str = "fn first() {}\n\nfn keep() {\n    1\n}\n";

/// A repository holding a library, a root file and a path with no
/// extension, and the commit they were read at.
fn repository() -> (Workspace, String) {
    let workspace = Workspace::from_fixture("empty");
    workspace.write("src/lib.rs", LIBRARY.as_bytes());
    workspace.write("Cargo.toml", b"[workspace]\n");
    workspace.write("ci/musl/cargo", b"#!/bin/sh\n");
    let read = workspace.commit_all();
    (workspace, read)
}

/// Commits the workspace as a merge of `main` and points `origin/main` at
/// it; returns the commit.
fn merge(workspace: &Workspace, subject: &str) -> String {
    let commit = workspace.commit_all_with(subject);
    workspace.git(&["update-ref", "refs/remotes/origin/main", &commit]);
    commit
}

/// A commit on top of `origin/main`, which `main` does not hold.
fn commit_off_main(workspace: &Workspace) -> String {
    workspace
        .git(&[
            "commit-tree",
            "origin/main^{tree}",
            "-p",
            "origin/main",
            "-m",
            "Off main",
        ])
        .trim()
        .to_owned()
}

/// A body whose `Existing code` list was read at `read` and holds `items`.
fn listed(read: &str, items: &[&str]) -> String {
    let items: String = items
        .iter()
        .map(|item| ["- ", item, "\n"].concat())
        .collect();
    format!(
        "## What to build\n\nSomething.\n\n## Existing code\n\nRead at `{}`.\n\n{items}",
        &read[..9]
    )
}

struct Issue<'a> {
    number: u64,
    title: &'a str,
    body: String,
    created_at: &'a str,
    sub_issues: u64,
    parent: Option<u64>,
    labels: &'a [&'a str],
}

impl Issue<'_> {
    fn json(&self) -> Value {
        json!({
            "number": self.number,
            "title": self.title,
            "body": self.body,
            "state": "open",
            "created_at": self.created_at,
            "labels": self.labels.iter().map(|name| json!({ "name": name })).collect::<Vec<_>>(),
            "sub_issues_summary": { "total": self.sub_issues, "completed": 0, "percent_completed": 0 },
            "parent_issue_url": self.parent.map(|parent| format!("https://api.github.com/repos/goodfetch/goodfetch/issues/{parent}")),
        })
    }
}

/// Ticket #7, ready for an agent, with `body`.
fn ticket(body: String) -> Issue<'static> {
    Issue {
        number: 7,
        title: "A ticket",
        body,
        created_at: "2026-11-11T09:00:00Z",
        sub_issues: 0,
        parent: None,
        labels: &["enhancement", "ready-for-agent"],
    }
}

fn check(workspace: &Workspace, issue: &Issue<'_>) -> Outcome {
    let issue = issue.json().to_string();
    let github = GitHub::serving(&[(ISSUE, &issue)]);
    workspace.xtask_with_env(
        &["existing-code", "check", "7", "--api-url", github.url()],
        &[("GITHUB_TOKEN", "a-read-token".as_ref())],
    )
}

fn sweep(workspace: &Workspace, github: &GitHub, apply: bool) -> Outcome {
    let mut arguments = vec!["existing-code", "sweep", "--api-url", github.url()];
    if apply {
        arguments.push("--apply");
    }
    workspace.xtask_with_env(&arguments, &[("GITHUB_TOKEN", "a-write-token".as_ref())])
}

#[test]
fn a_list_whose_citations_hold_passes() {
    let (workspace, read) = repository();
    workspace.write("README.md", b"Another file.\n");
    merge(&workspace, "Unrelated (#10)");
    let body = listed(
        &read,
        &[
            "The library keeps one thing. `src/lib.rs:3-5#keep`",
            "The workspace has a manifest. `Cargo.toml`",
            "Builds for musl go through a wrapper. `ci/musl/cargo`",
            "The library starts with `first`. `src/lib.rs:1#first` `src/lib.rs`",
        ],
    );

    let outcome = check(&workspace, &ticket(body));

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// Neither a root file nor a path with no extension is skipped as no
/// citation: each is checked, and fails once it is gone.
#[test]
fn a_root_file_and_a_path_with_no_extension_are_citations() {
    let (workspace, read) = repository();
    workspace.remove("Cargo.toml");
    workspace.remove("ci/musl/cargo");
    merge(&workspace, "Remove the wrapper (#10)");
    let body = listed(
        &read,
        &[
            "The workspace has a manifest. `Cargo.toml`",
            "Builds for musl go through a wrapper. `ci/musl/cargo`",
        ],
    );

    let outcome = check(&workspace, &ticket(body));

    assert_eq!(
        outcome.findings(),
        vec![
            finding("existing-code-missing-path", "#7 Cargo.toml"),
            finding("existing-code-missing-path", "#7 ci/musl/cargo"),
        ]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[test]
fn cited_lines_that_only_moved_pass_naming_their_new_lines() {
    let (workspace, read) = repository();
    workspace.write(
        "src/lib.rs",
        format!("// A header.\n// Two lines.\n{LIBRARY}").as_bytes(),
    );
    merge(&workspace, "Add a header (#10)");
    let body = listed(
        &read,
        &["The library keeps one thing. `src/lib.rs:3-5#keep`"],
    );

    let outcome = check(&workspace, &ticket(body));

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![finding("existing-code-moved", "#7 src/lib.rs:3-5#keep")]
    );
    assert!(
        outcome.stdout.contains("src/lib.rs:5-7"),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// The merge named is the first on `main` after the read commit that changed
/// the cited lines, not `main`'s head.
#[test]
fn cited_lines_that_changed_fail_naming_the_merge_that_changed_them() {
    let (workspace, read) = repository();
    workspace.edit("src/lib.rs", "    1\n", "    2\n");
    let changed = merge(&workspace, "Keep two (#10)");
    workspace.write("README.md", b"Another file.\n");
    merge(&workspace, "Unrelated (#11)");
    let body = listed(
        &read,
        &["The library keeps one thing. `src/lib.rs:3-5#keep`"],
    );

    let outcome = check(&workspace, &ticket(body));

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "existing-code-changed-lines",
            "#7 src/lib.rs:3-5#keep"
        )]
    );
    assert!(
        outcome.stdout.contains(&changed[..9]) && outcome.stdout.contains("Keep two (#10)"),
        "{}",
        outcome.stdout
    );
    assert!(
        outcome.stdout.contains("the evidence moved"),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[rstest]
#[case::path("`src/gone.rs:1`", "existing-code-missing-path", "#7 src/gone.rs:1")]
#[case::symbol(
    "`src/lib.rs#kept`",
    "existing-code-missing-symbol",
    "#7 src/lib.rs#kept"
)]
#[case::lines(
    "`src/lib.rs:9-12`",
    "existing-code-unknown-lines",
    "#7 src/lib.rs:9-12"
)]
#[case::nothing("The library is small.", "existing-code-no-citation", "#7 item 1")]
fn an_item_whose_citation_does_not_hold_fails(
    #[case] item: &str,
    #[case] kind: &str,
    #[case] subject: &str,
) {
    let (workspace, read) = repository();
    merge(&workspace, "Nothing (#10)");
    let body = listed(&read, &[&format!("The library holds it. {item}")]);

    let outcome = check(&workspace, &ticket(body));

    assert_eq!(outcome.findings(), vec![finding(kind, subject)]);
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

/// Words in backticks that name no path of the repository are not
/// citations, nor is a lone word that names a root folder, as an event named
/// `console` beside a `console/` folder; a path below one of its folders is,
/// even when it is gone, and so is a folder written with its `/`.
#[test]
fn only_paths_of_the_repository_are_citations() {
    let (workspace, read) = repository();
    workspace.remove("ci/musl/cargo");
    merge(&workspace, "Remove the ci folder (#10)");
    let body = listed(
        &read,
        &[
            "It answers `text/html` through `page.on`, `ready-for-agent`, `ci` and `xtask`. `src/lib.rs#keep`",
            "A typo is still a citation. `src/lbi.rs`",
            "The sources are one folder. `src/`",
        ],
    );

    let outcome = check(&workspace, &ticket(body));

    assert_eq!(
        outcome.findings(),
        vec![finding("existing-code-missing-path", "#7 src/lbi.rs")]
    );
}

#[test]
fn a_list_that_says_nothing_exists_yet_passes() {
    let (workspace, read) = repository();
    merge(&workspace, "Nothing (#10)");
    let body = listed(&read, &["None: nothing this spec changes exists yet."]);

    let outcome = check(&workspace, &ticket(body));

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::missing(
    "## Existing code\n\n- It holds. `src/lib.rs`\n",
    "existing-code-missing-read-commit"
)]
#[case::unknown(
    "## Existing code\n\nRead at `0123456789`.\n\n- It holds. `src/lib.rs`\n",
    "existing-code-unknown-read-commit"
)]
fn a_list_without_a_known_read_commit_fails(#[case] body: &str, #[case] kind: &str) {
    let (workspace, _) = repository();
    merge(&workspace, "Nothing (#10)");

    let outcome = check(&workspace, &ticket(body.to_owned()));

    assert_eq!(outcome.findings(), vec![finding(kind, "#7")]);
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[test]
fn a_read_commit_main_does_not_hold_fails() {
    let (workspace, _) = repository();
    merge(&workspace, "Nothing (#10)");
    let off_main = commit_off_main(&workspace);
    let body = listed(&off_main, &["It holds. `src/lib.rs`"]);

    let outcome = check(&workspace, &ticket(body));

    assert_eq!(
        outcome.findings(),
        vec![finding("existing-code-read-commit-off-main", "#7")]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

/// A spec: titled "Spec:" or holding sub-issues, published on or after
/// 2026-11-10 or listed in `docs/agents/rollout-order.md`.
#[rstest]
#[case::titled("Spec: something", "2026-11-10T00:00:00Z", 0, None)]
#[case::with_sub_issues("Something", "2026-11-12T00:00:00Z", 3, None)]
#[case::older_spec_a_step_opens_with(
    "Spec: something",
    "2026-11-01T00:00:00Z",
    0,
    Some(ROLLOUT_ORDER_LISTS_7)
)]
#[case::older_spec_linked_under_its_title(
    "Spec: something",
    "2026-11-01T00:00:00Z",
    0,
    Some(
        "2. **The tickets first:**\n   - [#9](https://github.com/goodfetch/goodfetch/issues/9), of [Spec: something](https://github.com/goodfetch/goodfetch/issues/7).\n"
    )
)]
fn a_spec_with_no_list_fails(
    #[case] title: &str,
    #[case] created_at: &str,
    #[case] sub_issues: u64,
    #[case] rollout_order: Option<&str>,
) {
    let (workspace, _) = repository();
    if let Some(rollout_order) = rollout_order {
        workspace.write("docs/agents/rollout-order.md", rollout_order.as_bytes());
    }
    merge(&workspace, "Nothing (#10)");
    let spec = Issue {
        title,
        created_at,
        sub_issues,
        ..ticket("## What to build\n\nSomething.\n".to_owned())
    };

    let outcome = check(&workspace, &spec);

    assert_eq!(
        outcome.findings(),
        vec![finding("existing-code-missing-list", "#7")]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[rstest]
#[case::ticket("A ticket", "2026-11-12T00:00:00Z", None, false)]
#[case::older_spec_not_listed("Spec: something", "2026-11-01T00:00:00Z", None, false)]
#[case::older_spec_only_mentioned(
    "Spec: something",
    "2026-11-01T00:00:00Z",
    Some(
        "1. **[Spec: other](https://github.com/goodfetch/goodfetch/issues/8)**: it\n   follows #7 and [spec #7](https://github.com/goodfetch/goodfetch/issues/7).\n"
    ),
    false
)]
#[case::listed_spec_whose_run_started(
    "Spec: something",
    "2026-11-01T00:00:00Z",
    Some(ROLLOUT_ORDER_LISTS_7),
    true
)]
fn an_issue_that_needs_no_list_passes_without_one(
    #[case] title: &str,
    #[case] created_at: &str,
    #[case] rollout_order: Option<&str>,
    #[case] started: bool,
) {
    let (workspace, _) = repository();
    if let Some(rollout_order) = rollout_order {
        workspace.write("docs/agents/rollout-order.md", rollout_order.as_bytes());
    }
    merge(&workspace, "Nothing (#10)");
    if started {
        let run = commit_off_main(&workspace);
        workspace.git(&["update-ref", "refs/heads/spec/7", &run]);
    }
    let issue = Issue {
        title,
        created_at,
        ..ticket("## What to build\n\nSomething.\n".to_owned())
    };

    let outcome = check(&workspace, &issue);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// A rollout order whose step 1 is spec #7.
const ROLLOUT_ORDER_LISTS_7: &str =
    "1. **[Spec: something](https://github.com/goodfetch/goodfetch/issues/7)**\n";

/// A repository whose `main` changed the lines ticket #7 cites, and the
/// merge that changed them.
fn moved_evidence() -> (Workspace, Issue<'static>, String) {
    let (workspace, read) = repository();
    workspace.edit("src/lib.rs", "    1\n", "    2\n");
    let changed = merge(&workspace, "Keep two (#10)");
    let body = listed(
        &read,
        &["The library keeps one thing. `src/lib.rs:3-5#keep`"],
    );
    (workspace, ticket(body), changed)
}

fn ready(issues: &[&Issue<'_>]) -> String {
    Value::Array(issues.iter().map(|issue| issue.json()).collect()).to_string()
}

fn written(requests: &[Request], method: &str) -> Vec<Value> {
    requests
        .iter()
        .filter(|request| request.method == method)
        .map(|request| serde_json::from_str(&request.body).unwrap())
        .collect()
}

#[test]
fn the_sweep_without_apply_writes_nothing() {
    let (workspace, issue, _) = moved_evidence();
    let listing = ready(&[&issue]);
    let github = GitHub::serving(&[(READY, &listing), (COMMENTS, "[]")]);

    let outcome = sweep(&workspace, &github, false);

    assert_eq!(github.writes(), vec![]);
    assert!(
        outcome.stdout.contains("would set aside #7"),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// It comments once with the item, its citation and the merge, then moves the
/// labels in one write; on the state that leaves, a second sweep, which no
/// longer lists the issue, writes nothing.
#[test]
fn the_sweep_sets_a_failing_issue_aside_once() {
    let (workspace, issue, changed) = moved_evidence();
    let listing = ready(&[&issue]);
    let github = GitHub::serving(&[
        (READY, &listing),
        (COMMENTS, "[]"),
        (POST_COMMENT, "{}"),
        (PATCH_ISSUE, "{}"),
    ]);

    let outcome = sweep(&workspace, &github, true);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let writes = github.writes();
    assert_eq!(
        writes
            .iter()
            .map(|write| (write.method.as_str(), write.path.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("POST", "/repos/goodfetch/goodfetch/issues/7/comments"),
            ("PATCH", "/repos/goodfetch/goodfetch/issues/7"),
        ]
    );
    let comment = written(&writes, "POST")[0]["body"]
        .as_str()
        .unwrap()
        .to_owned();
    for expected in [
        "The library keeps one thing.",
        "`src/lib.rs:3-5#keep`",
        &changed[..9],
        "the evidence moved",
    ] {
        assert!(comment.contains(expected), "{expected:?} in {comment}");
    }
    assert!(!comment.contains("false"), "{comment}");
    assert_eq!(
        written(&writes, "PATCH")[0],
        json!({ "labels": ["enhancement", "needs-triage"] })
    );

    let after = GitHub::serving(&[(READY, "[]")]);
    let again = sweep(&workspace, &after, true);

    assert_eq!(again.code, 0, "{}", again.stderr);
    assert_eq!(after.writes(), vec![]);
}

/// A sweep that commented and then failed before the labels moved leaves the
/// issue listed: the next one moves the labels without a second comment.
#[test]
fn the_sweep_comments_no_second_time() {
    let (workspace, issue, _) = moved_evidence();
    let listing = ready(&[&issue]);
    let first = GitHub::serving(&[(READY, &listing), (COMMENTS, "[]"), (POST_COMMENT, "{}")]);
    let failed = sweep(&workspace, &first, true);
    assert_eq!(failed.code, 2, "{}", failed.stderr);
    let comment = written(&first.writes(), "POST")[0]["body"].clone();
    let comments = json!([{ "body": comment }]).to_string();
    let second = GitHub::serving(&[
        (READY, &listing),
        (COMMENTS, &comments),
        (POST_COMMENT, "{}"),
        (PATCH_ISSUE, "{}"),
    ]);

    let outcome = sweep(&workspace, &second, true);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert_eq!(
        second
            .writes()
            .iter()
            .map(|write| write.method.as_str())
            .collect::<Vec<_>>(),
        vec!["PATCH"]
    );
}

/// From the first commit on `spec/<n>`, a spec's implementers change the
/// cited code on purpose: the sweep leaves the spec and its tickets alone.
#[rstest]
#[case::the_spec("refs/heads/spec/7", None)]
#[case::a_ticket_of_the_spec("refs/remotes/origin/spec/5", Some(5))]
fn the_sweep_leaves_alone_an_issue_whose_specs_run_started(
    #[case] branch: &str,
    #[case] parent: Option<u64>,
) {
    let (workspace, issue, _) = moved_evidence();
    let run = commit_off_main(&workspace);
    workspace.git(&["update-ref", branch, &run]);
    let issue = Issue {
        title: if parent.is_none() {
            "Spec: something"
        } else {
            "A ticket"
        },
        parent,
        ..issue
    };
    let listing = ready(&[&issue]);
    let github = GitHub::serving(&[
        (READY, &listing),
        (COMMENTS, "[]"),
        (POST_COMMENT, "{}"),
        (PATCH_ISSUE, "{}"),
    ]);

    let outcome = sweep(&workspace, &github, true);

    assert_eq!(github.writes(), vec![]);
    assert!(outcome.stdout.contains("skipped #7"), "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// The listing, the comments read, or the comment written fails: the sweep
/// fails loudly and no label moves.
#[rstest]
#[case::listing(&[])]
#[case::comments_read(&[(READY, "listing")])]
#[case::comment_written(&[(READY, "listing"), (COMMENTS, "[]")])]
fn a_tracker_failure_fails_the_sweep_and_moves_no_label(#[case] routes: &[(&str, &str)]) {
    let (workspace, issue, _) = moved_evidence();
    let listing = ready(&[&issue]);
    let routes: Vec<(&str, &str)> = routes
        .iter()
        .map(|&(path, body)| {
            (
                path,
                if body == "listing" {
                    listing.as_str()
                } else {
                    body
                },
            )
        })
        .chain([(PATCH_ISSUE, "{}")])
        .collect();
    let github = GitHub::serving(&routes);

    let outcome = sweep(&workspace, &github, true);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(outcome.stderr.starts_with("error:"), "{}", outcome.stderr);
    assert!(
        github.writes().iter().all(|write| write.method != "PATCH"),
        "{:?}",
        github.writes()
    );
}
