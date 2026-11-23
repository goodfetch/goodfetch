//! The Release suite's private-period run (ADRs 0061 and 0108): which cells a
//! manual dispatch starts under the 600-minute ceiling of minutes that bill,
//! counted from the month's recorded Release jobs in Linux-equivalent
//! minutes (Linux 1, Windows 2, macOS 11), with standard runners of a public
//! repository counting none. GitHub's interface is a local stand-in.
//! Covers: story:1-54, story:1-98

use std::collections::BTreeMap;

use rstest::rstest;

use crate::github::{GitHub, Request};
use crate::workspace::{Outcome, Workspace, finding};

const REPOSITORY: &str = "/repos/goodfetch/goodfetch";
const MAIN: &str = "refs/heads/main";
const TOPIC: &str = "refs/heads/topic";
const HEAD: &str = "1111111111111111111111111111111111111111";
const OLDER: &str = "2222222222222222222222222222222222222222";
const ISSUES: &str = "/repos/goodfetch/goodfetch/issues";
const TRIAGE_ISSUES: &str =
    "/repos/goodfetch/goodfetch/issues?labels=needs-triage&state=all&per_page=100&page=1";
const MILESTONES: &str = "/repos/goodfetch/goodfetch/milestones?state=open&per_page=100&page=1";
/// The marker line of the ceiling's `needs-triage` issue.
const CEILING_MARKER: &str = "<!-- goodfetch-results: needs-triage release-ceiling -->";

/// A job of a run, as GitHub's jobs API lists it with `filter=all`.
struct Job {
    name: String,
    label: String,
    attempt: u32,
    started_at: String,
    /// `None` while the job runs.
    completed_at: Option<String>,
    conclusion: Option<String>,
    /// Whether its test step failed (ADR 0064).
    failed_test_step: bool,
}

impl Job {
    fn json(&self) -> String {
        let quoted = |value: &Option<String>| {
            value
                .as_ref()
                .map_or("null".to_owned(), |v| format!("\"{v}\""))
        };
        let test = if self.failed_test_step {
            "failure"
        } else {
            "success"
        };
        format!(
            r#"{{"name": "{}", "labels": ["{}"], "run_attempt": {}, "started_at": "{}", "completed_at": {}, "conclusion": {}, "steps": [{{"name": "Test: Rust", "conclusion": "{test}"}}]}}"#,
            self.name,
            self.label,
            self.attempt,
            self.started_at,
            quoted(&self.completed_at),
            quoted(&self.conclusion),
        )
    }

    fn in_attempt(mut self, attempt: u32) -> Self {
        self.attempt = attempt;
        self
    }

    fn with_failed_test_step(mut self) -> Self {
        self.failed_test_step = true;
        self
    }
}

/// A job that ran for `minutes` from midnight of `date` and ended so.
fn ran(name: &str, label: &str, date: &str, minutes: u32, conclusion: &str) -> Job {
    Job {
        name: name.to_owned(),
        label: label.to_owned(),
        attempt: 1,
        started_at: format!("{date}T00:00:00Z"),
        completed_at: Some(format!(
            "{date}T{:02}:{:02}:00Z",
            minutes / 60,
            minutes % 60
        )),
        conclusion: Some(conclusion.to_owned()),
        failed_test_step: false,
    }
}

/// This run's own `plan` job, running on a hosted Linux runner.
fn plan_job() -> Job {
    Job {
        name: "plan".to_owned(),
        label: "ubuntu-24.04".to_owned(),
        attempt: 1,
        started_at: "2026-10-18T09:00:10Z".to_owned(),
        completed_at: None,
        conclusion: None,
        failed_test_step: false,
    }
}

/// Every hosted cell's job, and the owner's machine's, each `minutes` long.
fn full_run(date: &str, minutes: u32) -> Vec<Job> {
    vec![
        ran("plan", "ubuntu-24.04", date, 1, "success"),
        ran("linux-x64-gnu", "ubuntu-24.04", date, minutes, "success"),
        ran("linux-x64-musl", "ubuntu-24.04", date, minutes, "success"),
        ran(
            "linux-arm64-gnu",
            "ubuntu-24.04-arm",
            date,
            minutes,
            "success",
        ),
        ran(
            "linux-arm64-musl",
            "ubuntu-24.04-arm",
            date,
            minutes,
            "success",
        ),
        ran("windows-x64", "windows-2025", date, minutes, "success"),
        ran("macos-arm64", "owner-machine", date, minutes, "success"),
        ran(
            "macos-x64 (Rosetta 2)",
            "owner-machine",
            date,
            minutes,
            "success",
        ),
    ]
}

/// A run of the Release workflow and its jobs.
struct Run {
    id: u64,
    created_at: String,
    branch: String,
    jobs: Vec<Job>,
}

fn run(id: u64, created_at: &str, branch: &str, jobs: Vec<Job>) -> Run {
    Run {
        id,
        created_at: created_at.to_owned(),
        branch: branch.to_owned(),
        jobs,
    }
}

/// The repository's visibility, which GitHub gives each run's `repository`
/// as `private`.
#[derive(Clone, Copy)]
enum Visibility {
    Private,
    Public,
}

impl Run {
    fn json(&self, visibility: Visibility) -> String {
        let private = matches!(visibility, Visibility::Private);
        format!(
            r#"{{"id": {}, "created_at": "{}", "head_branch": "{}", "head_sha": "{HEAD}", "repository": {{"private": {private}}}}}"#,
            self.id, self.created_at, self.branch
        )
    }
}

/// This run (ID 7, created 2026-10-18 on `branch`) and the earlier runs of
/// a private repository, newest first, served as GitHub's Actions interface
/// lists them, with any `extra` routes.
fn serve(branch: &str, earlier: Vec<Run>, extra: &[(&str, &str)]) -> GitHub {
    serve_in(Visibility::Private, branch, earlier, extra)
}

fn serve_in(
    visibility: Visibility,
    branch: &str,
    earlier: Vec<Run>,
    extra: &[(&str, &str)],
) -> GitHub {
    let mut runs = vec![run(7, "2026-10-18T09:00:00Z", branch, vec![plan_job()])];
    runs.extend(earlier);
    let listed: Vec<String> = runs.iter().map(|run| run.json(visibility)).collect();
    let mut routes = vec![
        (
            format!("{REPOSITORY}/actions/runs/7"),
            runs[0].json(visibility),
        ),
        (
            format!("{REPOSITORY}/actions/workflows/release-suite.yml/runs?per_page=100&page=1"),
            format!(
                r#"{{"total_count": {}, "workflow_runs": [{}]}}"#,
                runs.len(),
                listed.join(", ")
            ),
        ),
    ];
    routes.extend(jobs_routes(&runs));
    let mut borrowed: Vec<(&str, &str)> = routes
        .iter()
        .map(|(path, body)| (path.as_str(), body.as_str()))
        .collect();
    borrowed.extend_from_slice(extra);
    GitHub::serving(&borrowed)
}

fn plan(workspace: &Workspace, github: &GitHub, git_ref: &str, extra: &[&str]) -> Outcome {
    let mut args = vec![
        "release-suite",
        "plan",
        "--repository",
        "goodfetch/goodfetch",
        "--run-id",
        "7",
        "--ref",
        git_ref,
        "--outputs",
        "outputs.txt",
        "--summary",
        "summary.md",
        "--api-url",
        github.url(),
    ];
    args.extend_from_slice(extra);
    workspace.xtask(&args)
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

/// The `key=value` lines `plan` wrote for the workflow's later jobs.
fn outputs(workspace: &Workspace) -> BTreeMap<String, String> {
    let text = String::from_utf8(workspace.read("outputs.txt").unwrap_or_default()).unwrap();
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[test]
fn first_run_on_main_starts_every_cell_on_the_commit() {
    let workspace = Workspace::from_fixture("empty");
    let github = serve("main", vec![], &[]);

    let outcome = plan(&workspace, &github, MAIN, &[]);

    assert_eq!(outcome.code, 0);
    let outputs = outputs(&workspace);
    assert_eq!(outputs["commit"], HEAD);
    assert_eq!(
        outputs["cells"],
        r#"["linux-x64-gnu","linux-x64-musl","linux-arm64-gnu","linux-arm64-musl","windows-x64","macos-arm64","macos-x64"]"#
    );
    assert_eq!(
        json(&outputs["linux-x64"]),
        json(
            r#"[{"cell":"linux-x64-gnu","target":"x86_64-unknown-linux-gnu","runtimes":"release-linux-x64-gnu"},{"cell":"linux-x64-musl","target":"x86_64-unknown-linux-musl","runtimes":"release-linux"}]"#
        )
    );
    assert_eq!(
        json(&outputs["linux-arm64"]),
        json(
            r#"[{"cell":"linux-arm64-gnu","target":"aarch64-unknown-linux-gnu","runtimes":"release-linux"},{"cell":"linux-arm64-musl","target":"aarch64-unknown-linux-musl","runtimes":"release-linux"}]"#
        )
    );
    assert_eq!(outputs["held"], "false");
    assert_eq!(outputs["estimate"], "270");
}

#[test]
fn run_on_main_past_the_ceiling_holds_the_hosted_cells_and_starts_the_owners_machines() {
    let workspace = Workspace::from_fixture("empty");
    // Earlier this month: 1 + 4 x 80 + 2 x 80 = 481 minutes; this plan job, 1.
    let earlier = run(
        5,
        "2026-10-05T00:00:00Z",
        "main",
        full_run("2026-10-05", 80),
    );
    let github = serve("main", vec![earlier], &[]);

    let outcome = plan(&workspace, &github, MAIN, &[]);

    assert_eq!(outcome.code, 0);
    let outputs = outputs(&workspace);
    assert_eq!(outputs["held"], "true");
    assert_eq!(outputs["cells"], r#"["macos-arm64","macos-x64"]"#);
    assert_eq!(outputs["linux-x64"], "[]");
    assert_eq!(outputs["linux-arm64"], "[]");
    assert_eq!(outputs["used"], "482");
    assert_eq!(outputs["estimate"], "480");
    assert_eq!(outputs["needed"], "962");
    assert_eq!(outputs["month-turns"], "2026-11-01");
}

#[test]
fn estimate_comes_from_each_cells_latest_recorded_run_at_its_rate() {
    let workspace = Workspace::from_fixture("empty");
    let older = run(
        3,
        "2026-09-20T00:00:00Z",
        "main",
        full_run("2026-09-20", 40),
    );
    let newer = run(
        4,
        "2026-09-25T00:00:00Z",
        "topic",
        vec![
            ran("windows-x64", "windows-2025", "2026-09-25", 30, "failure"),
            ran("linux-x64-gnu", "ubuntu-24.04", "2026-09-25", 0, "skipped"),
        ],
    );
    let github = serve("main", vec![newer, older], &[]);

    let outcome = plan(&workspace, &github, MAIN, &[]);

    assert_eq!(outcome.code, 0);
    let outputs = outputs(&workspace);
    // Windows' failed run of 30 minutes at 2, and 40 minutes for each Linux
    // cell from the older run, since the newer one skipped Linux x64.
    assert_eq!(outputs["estimate"], "220");
    assert_eq!(outputs["used"], "1");
}

#[rstest]
#[case::room_left_for_a_run_on_main(
    vec![],
    "false",
    r#"["linux-x64-gnu","linux-x64-musl","linux-arm64-gnu","linux-arm64-musl","windows-x64"]"#
)]
#[case::no_room_left_for_a_run_on_main(
    vec![run(6, "2026-10-12T00:00:00Z", "other", full_run("2026-10-12", 30))],
    "true",
    "[]"
)]
fn dispatch_off_main_starts_its_hosted_cells_only_when_a_run_on_main_still_fits(
    #[case] more: Vec<Run>,
    #[case] held: &str,
    #[case] cells: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    // Each run: 1 + 4 x 30 + 2 x 30 = 181 minutes. Room left: this run's 180,
    // then a run on `main` of 180, after 182 used, fits (542); after another
    // branch run (363 used), it does not (723).
    let mut earlier = more;
    earlier.push(run(
        5,
        "2026-10-05T00:00:00Z",
        "main",
        full_run("2026-10-05", 30),
    ));
    let github = serve("topic", earlier, &[]);

    let outcome = plan(&workspace, &github, TOPIC, &[]);

    assert_eq!(outcome.code, 0);
    let outputs = outputs(&workspace);
    assert_eq!(outputs["held"], held);
    assert_eq!(outputs["cells"], cells);
}

#[test]
fn dispatch_off_main_never_starts_the_owners_machine() {
    let workspace = Workspace::from_fixture("empty");
    let github = serve("topic", vec![], &[]);

    let outcome = plan(
        &workspace,
        &github,
        TOPIC,
        &["--cells", "macos-arm64,macos-x64"],
    );

    assert_eq!(outcome.code, 0);
    assert_eq!(outputs(&workspace)["cells"], "[]");
}

#[test]
fn chosen_cells_alone_start_and_are_estimated() {
    let workspace = Workspace::from_fixture("empty");
    let github = serve("main", vec![], &[]);

    let outcome = plan(
        &workspace,
        &github,
        MAIN,
        &["--cells", "windows-x64,macos-arm64"],
    );

    assert_eq!(outcome.code, 0);
    let outputs = outputs(&workspace);
    assert_eq!(outputs["cells"], r#"["windows-x64","macos-arm64"]"#);
    assert_eq!(outputs["linux-x64"], "[]");
    assert_eq!(outputs["estimate"], "90");
}

#[test]
fn unknown_cell_reaches_no_plan() {
    let workspace = Workspace::from_fixture("empty");
    let github = serve("main", vec![], &[]);

    let outcome = plan(&workspace, &github, MAIN, &["--cells", "macos-x64-native"]);

    assert_eq!(outcome.code, 2);
    assert_eq!(workspace.read("outputs.txt"), None);
}

/// A run whose Windows cell took 400 minutes, so 800 at its rate: past the
/// ceiling on its own.
fn over_the_ceiling(id: u64, created_at: &str, date: &str) -> Run {
    run(
        id,
        created_at,
        "main",
        vec![ran("windows-x64", "windows-2025", date, 400, "success")],
    )
}

#[test]
fn months_first_run_on_main_over_the_ceiling_on_its_own_starts_and_opens_an_issue() {
    let workspace = Workspace::from_fixture("empty");
    let last_month = over_the_ceiling(3, "2026-09-20T00:00:00Z", "2026-09-20");
    let github = serve(
        "main",
        vec![last_month],
        &[
            (TRIAGE_ISSUES, "[]"),
            (
                MILESTONES,
                r#"[{"number": 1, "open_issues": 3, "due_on": null}]"#,
            ),
            (ISSUES, r#"{"number": 9}"#),
        ],
    );

    let outcome = plan(&workspace, &github, MAIN, &["--cells", "windows-x64"]);

    assert_eq!(outcome.code, 0);
    assert_eq!(outputs(&workspace)["held"], "false");
    let writes = github.writes();
    assert_eq!(
        writes
            .iter()
            .map(|request| (request.method.as_str(), request.path.as_str()))
            .collect::<Vec<_>>(),
        vec![("POST", ISSUES)]
    );
    assert_eq!(json(&writes[0].body)["labels"], json(r#"["needs-triage"]"#));
    assert_eq!(
        json(&writes[0].body)["body"]
            .as_str()
            .unwrap()
            .lines()
            .next(),
        Some(CEILING_MARKER)
    );
}

#[test]
fn run_over_the_ceiling_on_its_own_updates_the_open_issue() {
    let workspace = Workspace::from_fixture("empty");
    let last_month = over_the_ceiling(3, "2026-09-20T00:00:00Z", "2026-09-20");
    let open = format!(
        r#"[{{"number": 12, "state": "open", "closed_at": null, "body": "{CEILING_MARKER}\nAn older run."}}]"#
    );
    let issue = "/repos/goodfetch/goodfetch/issues/12";
    let github = serve(
        "main",
        vec![last_month],
        &[(TRIAGE_ISSUES, &open), (issue, r#"{"number": 12}"#)],
    );

    let outcome = plan(&workspace, &github, MAIN, &["--cells", "windows-x64"]);

    assert_eq!(outcome.code, 0);
    let writes: Vec<Request> = github.writes();
    assert_eq!(
        writes
            .iter()
            .map(|request| (request.method.as_str(), request.path.as_str()))
            .collect::<Vec<_>>(),
        vec![("PATCH", issue)]
    );
}

#[test]
fn second_run_on_main_over_the_ceiling_on_its_own_is_held() {
    let workspace = Workspace::from_fixture("empty");
    let this_month = over_the_ceiling(5, "2026-10-02T00:00:00Z", "2026-10-02");
    let github = serve("main", vec![this_month], &[]);

    let outcome = plan(&workspace, &github, MAIN, &["--cells", "windows-x64"]);

    assert_eq!(outcome.code, 0);
    assert_eq!(outputs(&workspace)["held"], "true");
    assert_eq!(github.writes(), vec![]);
}

/// Standard GitHub-hosted runners are free in a public repository and draw
/// on no included minutes (ADR 0108's note of 2026-11-23), so a run on them
/// alone starts whatever its estimate.
#[test]
fn in_a_public_repository_a_run_on_standard_runners_alone_starts_whatever_its_estimate() {
    let workspace = Workspace::from_fixture("empty");
    let this_month = over_the_ceiling(5, "2026-10-02T00:00:00Z", "2026-10-02");
    let github = serve_in(Visibility::Public, "main", vec![this_month], &[]);

    let outcome = plan(&workspace, &github, MAIN, &["--cells", "windows-x64"]);

    assert_eq!(outcome.code, 0);
    let outputs = outputs(&workspace);
    assert_eq!(
        (
            outputs["held"].as_str(),
            outputs["cells"].as_str(),
            outputs["used"].as_str(),
            outputs["estimate"].as_str(),
        ),
        ("false", r#"["windows-x64"]"#, "0", "0")
    );
    assert_eq!(github.writes(), vec![]);
}

/// A larger runner bills in a public repository too, so its minutes still
/// count against the ceiling, at its rate.
#[test]
fn in_a_public_repository_a_larger_runners_minutes_still_count() {
    let workspace = Workspace::from_fixture("empty");
    let mut jobs = full_run("2026-10-05", 80);
    jobs.push(ran(
        "bench",
        "ubuntu-24.04-16core",
        "2026-10-05",
        700,
        "success",
    ));
    let this_month = run(5, "2026-10-05T00:00:00Z", "main", jobs);
    let github = serve_in(Visibility::Public, "main", vec![this_month], &[]);

    let outcome = plan(&workspace, &github, MAIN, &[]);

    assert_eq!(outcome.code, 0);
    let outputs = outputs(&workspace);
    // The larger runner's 700 minutes count; the standard runners' 481 and
    // this plan job's do not. This run's cells bill nothing, so it starts.
    assert_eq!(
        (
            outputs["used"].as_str(),
            outputs["estimate"].as_str(),
            outputs["held"].as_str(),
        ),
        ("700", "0", "false")
    );
}

#[rstest]
#[case::ahead("ahead", 0)]
#[case::diverged("diverged", 1)]
#[case::behind("behind", 1)]
fn commit_off_mains_history_is_refused_before_any_cell(#[case] status: &str, #[case] code: i32) {
    let workspace = Workspace::from_fixture("empty");
    let compare = format!("{REPOSITORY}/compare/{OLDER}...{HEAD}");
    let body = format!(r#"{{"status": "{status}"}}"#);
    let github = serve("main", vec![], &[(compare.as_str(), body.as_str())]);

    let outcome = plan(&workspace, &github, MAIN, &["--commit", OLDER]);

    assert_eq!(outcome.code, code);
    if code == 0 {
        assert_eq!(outputs(&workspace)["commit"], OLDER);
    } else {
        assert_eq!(
            outcome.findings(),
            vec![finding("commit-not-on-main", OLDER)]
        );
        assert_eq!(workspace.read("outputs.txt"), None);
    }
}

#[test]
fn commit_chosen_off_main_is_refused() {
    let workspace = Workspace::from_fixture("empty");
    let github = serve("topic", vec![], &[]);

    let outcome = plan(&workspace, &github, TOPIC, &["--commit", OLDER]);

    assert_eq!(outcome.code, 1);
    assert_eq!(outcome.findings(), vec![finding("commit-off-main", OLDER)]);
}

/// The runs of `HEAD`, newest first, as the runs API lists them by commit.
/// The route of each run's jobs, as GitHub's jobs API lists them with
/// `filter=all`.
fn jobs_routes(runs: &[Run]) -> Vec<(String, String)> {
    runs.iter()
        .map(|run| {
            let jobs: Vec<String> = run.jobs.iter().map(Job::json).collect();
            (
                format!(
                    "{REPOSITORY}/actions/runs/{}/jobs?filter=all&per_page=100&page=1",
                    run.id
                ),
                format!(
                    r#"{{"total_count": {}, "jobs": [{}]}}"#,
                    jobs.len(),
                    jobs.join(", ")
                ),
            )
        })
        .collect()
}

fn serve_commit(runs: &[Run]) -> GitHub {
    let listed: Vec<String> = runs
        .iter()
        .map(|run| run.json(Visibility::Private))
        .collect();
    let mut routes = vec![(
        format!(
            "{REPOSITORY}/actions/workflows/release-suite.yml/runs?head_sha={HEAD}&per_page=100&page=1"
        ),
        format!(
            r#"{{"total_count": {}, "workflow_runs": [{}]}}"#,
            runs.len(),
            listed.join(", ")
        ),
    )];
    routes.extend(jobs_routes(runs));
    let borrowed: Vec<(&str, &str)> = routes
        .iter()
        .map(|(path, body)| (path.as_str(), body.as_str()))
        .collect();
    GitHub::serving(&borrowed)
}

fn verdict(workspace: &Workspace, github: &GitHub) -> Outcome {
    workspace.xtask(&[
        "release-suite",
        "verdict",
        "--repository",
        "goodfetch/goodfetch",
        "--commit",
        HEAD,
        "--api-url",
        github.url(),
    ])
}

/// The hosted cells' jobs of a dispatch, with the owner's machine's skipped,
/// as a dispatch that did not reach it lists them.
fn hosted_cells(date: &str, conclusion: &str) -> Vec<Job> {
    vec![
        ran("linux-x64-gnu", "ubuntu-24.04", date, 30, conclusion),
        ran("linux-x64-musl", "ubuntu-24.04", date, 30, conclusion),
        ran("linux-arm64-gnu", "ubuntu-24.04-arm", date, 30, conclusion),
        ran("linux-arm64-musl", "ubuntu-24.04-arm", date, 30, conclusion),
        ran("windows-x64", "windows-2025", date, 30, conclusion),
        ran("macos-arm64", "owner-machine", date, 0, "skipped"),
    ]
}

fn owner_machine_cells(date: &str) -> Vec<Job> {
    vec![
        ran("macos-arm64", "owner-machine", date, 30, "success"),
        ran(
            "macos-x64 (Rosetta 2)",
            "owner-machine",
            date,
            30,
            "success",
        ),
    ]
}

#[test]
fn commit_whose_every_cell_is_green_across_dispatches_passes() {
    let workspace = Workspace::from_fixture("empty");
    let github = serve_commit(&[
        run(
            9,
            "2026-10-19T00:00:00Z",
            "main",
            owner_machine_cells("2026-10-19"),
        ),
        run(
            8,
            "2026-10-18T00:00:00Z",
            "main",
            hosted_cells("2026-10-18", "success"),
        ),
    ]);

    let outcome = verdict(&workspace, &github);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0);
}

#[test]
fn cell_with_a_failed_test_step_in_any_attempt_leaves_the_commit_red() {
    let workspace = Workspace::from_fixture("empty");
    let mut jobs: Vec<Job> = hosted_cells("2026-10-18", "success")
        .into_iter()
        .map(|job| job.in_attempt(2))
        .collect();
    jobs.push(
        ran("windows-x64", "windows-2025", "2026-10-18", 30, "failure").with_failed_test_step(),
    );
    jobs.extend(owner_machine_cells("2026-10-18"));
    let github = serve_commit(&[run(8, "2026-10-18T00:00:00Z", "main", jobs)]);

    let outcome = verdict(&workspace, &github);

    assert_eq!(outcome.findings(), vec![finding("cell-red", "windows-x64")]);
    assert_eq!(outcome.code, 1);
}

#[test]
fn cell_not_run_or_failed_without_a_failed_test_step_is_not_green_yet() {
    let workspace = Workspace::from_fixture("empty");
    let mut jobs = hosted_cells("2026-10-18", "success");
    jobs[4] = ran("windows-x64", "windows-2025", "2026-10-18", 3, "failure");
    let github = serve_commit(&[run(8, "2026-10-18T00:00:00Z", "main", jobs)]);

    let outcome = verdict(&workspace, &github);

    assert_eq!(
        outcome.findings(),
        vec![
            finding("cell-failed", "windows-x64"),
            finding("cell-not-run", "macos-arm64"),
            finding("cell-not-run", "macos-x64 (Rosetta 2)"),
        ]
    );
    assert_eq!(outcome.code, 1);
}
