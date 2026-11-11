//! The Performance budget of every change (ADR 0071, as amended by ADRs 0087
//! and 0133): `xtask perf compare` reads gungraun's summaries of the head,
//! measured against the base's saved baseline, and size-limit's reports of
//! the base and the head, then reports a figure past its threshold that the
//! pull request does not accept, and a Console build over its ceiling. Until
//! M1's optimization pass each is a note that fails nothing (ADR 0134, as
//! amended on 2026-10-22); a benchmark that fails its admission still fails.
//! The reports here are written the way gungraun (summary format 7) and
//! size-limit write them.
//! Covers: story:1-108

use rstest::rstest;
use serde_json::{Value, json};

use crate::github::GitHub;
use crate::workspace::{Outcome, Workspace, finding};

/// The benchmark every case measures, as `xtask perf` names it:
/// `<crate>/<module path>`.
const BENCHMARK: &str = "goodfetch-frontier/instructions::frontier::push";

/// One gungraun metric of a benchmark: its key in the summary, and the
/// head's (`new`) and the base's (`old`) values.
pub(crate) struct Metric<'a> {
    key: &'a str,
    head: Option<u64>,
    base: Option<u64>,
}

pub(crate) fn metric(key: &str, base: Option<u64>, head: Option<u64>) -> Metric<'_> {
    Metric { key, head, base }
}

/// The tool whose summary holds a metric.
fn tool_of(key: &str) -> &'static str {
    if key == "Ir" { "Callgrind" } else { "DHAT" }
}

/// One line of gungraun's `--output-format=json`: the summary of one
/// benchmark of the crate at `package_dir`, one profile per tool.
fn summary(package_dir: &str, module_path: &str, metrics: &[Metric<'_>]) -> String {
    bench_summary(package_dir, module_path, None, metrics)
}

/// The summary of one benchmark, with its `#[bench::id]` when it has one.
pub(crate) fn bench_summary(
    package_dir: &str,
    module_path: &str,
    id: Option<&str>,
    metrics: &[Metric<'_>],
) -> String {
    let mut profiles: Vec<(&str, serde_json::Map<String, Value>)> = Vec::new();
    for metric in metrics {
        let tool = tool_of(metric.key);
        let mut values = serde_json::Map::new();
        if let Some(head) = metric.head {
            values.insert("new".to_owned(), json!(head));
        }
        if let Some(base) = metric.base {
            values.insert("old".to_owned(), json!(base));
        }
        let entry = json!({ "change": null, "values": values });
        match profiles.iter_mut().find(|(name, _)| *name == tool) {
            Some((_, map)) => {
                map.insert(metric.key.to_owned(), entry);
            }
            None => profiles.push((
                tool,
                serde_json::Map::from_iter([(metric.key.to_owned(), entry)]),
            )),
        }
    }
    let profiles: Vec<Value> = profiles
        .into_iter()
        .map(|(tool, metrics)| {
            json!({
                "data": { "parts": [], "total": { "metrics": metrics, "regressions": [] } },
                "duration_ns": 1_000,
                "process_ns": 900,
                "started_at": "2026-10-19T08:00:00Z",
                "tool": tool,
            })
        })
        .collect();
    let (file, rest) = module_path.split_once("::").unwrap();
    let (group, function) = rest.split_once("::").unwrap();
    let mut summary = json!({
        "baselines": [null, "base"],
        "benchmark_exe": format!("target/release/deps/{file}-0123456789abcdef"),
        "benchmark_file": format!("{package_dir}/benches/{file}.rs"),
        "duration_ns": 2_000,
        "function_name": function,
        "group": group,
        "kind": "LibraryBenchmark",
        "module_path": module_path,
        "output_dir": format!("target/gungraun/{file}"),
        "package_dir": package_dir,
        "profiles": profiles,
        "project_root": "/work",
        "started_at": "2026-10-19T08:00:00Z",
        "version": "7",
    });
    if let Some(id) = id {
        summary["id"] = json!(id);
    }
    summary.to_string()
}

/// gungraun's report of the frontier's one benchmark.
fn write_gungraun(workspace: &Workspace, metrics: &[Metric<'_>]) {
    let line = summary(
        "crates/foxhound/goodfetch-frontier",
        "instructions::frontier::push",
        metrics,
    );
    workspace.write("perf/head.jsonl", format!("{line}\n").as_bytes());
}

pub(crate) fn compare(workspace: &Workspace, extra: &[&str]) -> Outcome {
    let mut args = vec!["perf", "compare", "--gungraun", "perf/head.jsonl"];
    args.extend_from_slice(extra);
    workspace.xtask(&args)
}

#[rstest]
#[case::instructions("Ir")]
#[case::total_allocations("TotalBlocks")]
#[case::total_bytes("TotalBytes")]
#[case::bytes_at_the_global_maximum("AtTGmaxBytes")]
fn figure_up_4_9_percent_on_its_base_passes(#[case] key: &str) {
    let workspace = Workspace::from_fixture("empty");
    write_gungraun(&workspace, &[metric(key, Some(1_000_000), Some(1_049_000))]);

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::instructions("Ir", "instructions")]
#[case::total_allocations("TotalBlocks", "total-allocations")]
#[case::total_bytes("TotalBytes", "total-bytes")]
#[case::bytes_at_the_global_maximum("AtTGmaxBytes", "global-maximum-bytes")]
fn figure_up_5_1_percent_on_its_base_is_noted_with_its_change_and_passes(
    #[case] key: &str,
    #[case] figure: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    write_gungraun(&workspace, &[metric(key, Some(1_000_000), Some(1_051_000))]);

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![finding(
            "performance-change",
            &format!("{BENCHMARK} {figure}")
        )]
    );
    assert!(
        note_line(&outcome, "performance-change").contains("+5.10%"),
        "{}",
        outcome.stdout
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// The note line of `kind` an outcome printed.
fn note_line<'a>(outcome: &'a Outcome, kind: &str) -> &'a str {
    outcome
        .stdout
        .lines()
        .find(|line| line.starts_with(&format!("note[{kind}] ")))
        .unwrap_or_default()
}

/// gungraun 0.20's own report of the Frontier's fixture benchmark, run
/// with `--baseline=base` by `xtask perf run` after a change that lengthened
/// every URL (Linux arm64, in a container).
#[test]
fn real_gungraun_report_of_a_lengthened_url_notes_three_figures() {
    let workspace = Workspace::from_fixture("empty");
    workspace.add_fixture_file("perf/gungraun-head.jsonl", "perf/head.jsonl");

    let outcome = compare(&workspace, &[]);

    let heap = "goodfetch-frontier/heap::frontier::queue_unique/thousand_urls";
    let instructions = "goodfetch-frontier/instructions::frontier::queue_unique/thousand_urls";
    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![
            finding(
                "performance-change",
                &format!("{heap} global-maximum-bytes")
            ),
            finding("performance-change", &format!("{heap} total-bytes")),
            finding(
                "performance-change",
                &format!("{instructions} instructions")
            ),
        ]
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn benchmark_with_no_base_is_reported_never_gated() {
    let workspace = Workspace::from_fixture("empty");
    write_gungraun(&workspace, &[metric("Ir", None, Some(9_000_000))]);

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// size-limit's `--json` report of `entries`, `(name, brotli bytes)`.
pub(crate) fn size_limit(entries: &[(&str, u64)]) -> String {
    let entries: Vec<Value> = entries
        .iter()
        .map(|(name, size)| json!({ "name": name, "passed": true, "size": size }))
        .collect();
    Value::Array(entries).to_string()
}

/// The base's and the head's size-limit reports, beside a head with no
/// benchmark.
fn write_bundles(workspace: &Workspace, base: &[(&str, u64)], head: &[(&str, u64)]) {
    workspace.write("perf/head.jsonl", b"");
    workspace.write("perf/size-base.json", size_limit(base).as_bytes());
    workspace.write("perf/size-head.json", size_limit(head).as_bytes());
}

const BUNDLES: [&str; 4] = [
    "--size-base",
    "perf/size-base.json",
    "--size-head",
    "perf/size-head.json",
];

#[rstest]
#[case::grown_by_4_9_kb(104_900, false)]
#[case::grown_by_exactly_5_kb(105_000, false)]
#[case::shrunk(90_000, false)]
#[case::grown_by_5_1_kb(105_100, true)]
fn entry_grown_past_5_kb_brotli_is_noted(#[case] head: u64, #[case] tripped: bool) {
    let workspace = Workspace::from_fixture("empty");
    write_bundles(
        &workspace,
        &[("collie-client", 100_000)],
        &[("collie-client", head)],
    );

    let outcome = compare(&workspace, &BUNDLES);

    let expected = if tripped {
        vec![finding(
            "performance-change",
            "size-limit/collie-client brotli",
        )]
    } else {
        vec![]
    };
    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.notes(), expected);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::collie("collie-console")]
#[case::foxhound("foxhound-console")]
fn console_build_over_1_mb_brotli_is_noted(#[case] entry: &str) {
    let workspace = Workspace::from_fixture("empty");
    write_bundles(&workspace, &[(entry, 999_000)], &[(entry, 1_000_001)]);

    let outcome = compare(&workspace, &BUNDLES);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![finding(
            "over-ceiling",
            &format!("size-limit/{entry} brotli")
        )]
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// A size over its ceiling never shows as at it: brotli kB round up.
#[test]
fn console_build_5_bytes_over_its_ceiling_shows_over_it() {
    let workspace = Workspace::from_fixture("empty");
    write_bundles(
        &workspace,
        &[("collie-console", 999_000)],
        &[("collie-console", 1_000_005)],
    );

    let outcome = compare(&workspace, &BUNDLES);

    let row = outcome
        .stdout
        .lines()
        .find(|line| line.starts_with("| size-limit/collie-console | brotli |"))
        .unwrap_or_default();
    assert!(row.contains("| 1000.01 kB |"), "{}", outcome.stdout);
}

#[test]
fn console_build_of_exactly_1_mb_brotli_passes() {
    let workspace = Workspace::from_fixture("empty");
    write_bundles(
        &workspace,
        &[("collie-console", 999_000)],
        &[("collie-console", 1_000_000)],
    );

    let outcome = compare(&workspace, &BUNDLES);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// Each Product's initial route (what loads before the Runs screen's first
/// render) has the ceiling the owner approved: 1.25 times its first build's
/// measure, rounded up (ADR 0071, as amended by ADR 0087): Collie's 111,823
/// bytes (#937), Foxhound's 128,939 (#1235).
#[rstest]
#[case::collie_at_its_ceiling("collie-console-initial-route", 139_779, false)]
#[case::collie_a_byte_over_it("collie-console-initial-route", 139_780, true)]
#[case::foxhound_at_its_ceiling("foxhound-console-initial-route", 161_174, false)]
#[case::foxhound_a_byte_over_it("foxhound-console-initial-route", 161_175, true)]
fn console_initial_route_over_its_ceiling_is_noted(
    #[case] entry: &str,
    #[case] head: u64,
    #[case] over: bool,
) {
    let workspace = Workspace::from_fixture("empty");
    write_bundles(&workspace, &[(entry, head)], &[(entry, head)]);

    let outcome = compare(&workspace, &BUNDLES);

    let expected = if over {
        vec![finding(
            "over-ceiling",
            &format!("size-limit/{entry} brotli"),
        )]
    } else {
        vec![]
    };
    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.notes(), expected, "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

pub(crate) const HEAD_SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER_SHA: &str = "fedcba9876543210fedcba9876543210fedcba98";
const PULL_REQUEST: &str = "/repos/goodfetch/goodfetch/pulls/7";
const COMMENTS: &str = "/repos/goodfetch/goodfetch/issues/7/comments?per_page=100&page=1";

/// A pull request whose head is `HEAD_SHA`, with `body` and one comment by
/// the owner.
pub(crate) fn pull_request(body: &str, comment: &str) -> GitHub {
    let pull_request = format!(
        r#"{{ "number": 7, "body": {}, "head": {{ "sha": "{HEAD_SHA}" }} }}"#,
        serde_json::to_string(body).unwrap()
    );
    let comments = format!(
        r#"[ {{ "body": {}, "author_association": "OWNER" }} ]"#,
        serde_json::to_string(comment).unwrap()
    );
    GitHub::serving(&[(PULL_REQUEST, &pull_request), (COMMENTS, &comments)])
}

/// Saves the pull request's acceptance, as the job does before its deny,
/// then compares with it.
pub(crate) fn compare_for_pull_request(
    workspace: &Workspace,
    github: &GitHub,
    extra: &[&str],
) -> Outcome {
    let saved = workspace.xtask(&[
        "perf",
        "pull-request",
        "--number",
        "7",
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
        "--save",
        "perf/pull-request.json",
    ]);
    assert_eq!(saved.code, 0, "{}", saved.stderr);
    let mut args = vec!["--pull-request", "perf/pull-request.json"];
    args.extend_from_slice(extra);
    compare(workspace, &args)
}

const SECTION: &str = "## Summary\n\nIndexes the Frontier.\n\n## Performance change\n\n\
    - goodfetch-frontier/instructions::frontier::push instructions: +5.10%, the new index\n\n## Evidence\n";

#[rstest]
#[case::full_head_sha(&format!("Performance change accepted for {HEAD_SHA}."))]
#[case::abbreviated_head_sha("Performance change accepted at 0123456789a")]
fn tripped_figure_listed_and_accepted_on_the_head_passes(#[case] comment: &str) {
    let workspace = Workspace::from_fixture("empty");
    write_gungraun(
        &workspace,
        &[metric("Ir", Some(1_000_000), Some(1_051_000))],
    );
    let github = pull_request(SECTION, comment);

    let outcome = compare_for_pull_request(&workspace, &github, &[]);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::acceptance_names_another_sha(
    SECTION,
    &format!("Performance change accepted for {OTHER_SHA}.")
)]
#[case::section_change_smaller_than_measured(
    "## Performance change\n\n- goodfetch-frontier/instructions::frontier::push instructions: +5.00%, the new index\n",
    &format!("Performance change accepted for {HEAD_SHA}.")
)]
#[case::section_lists_another_figure(
    "## Performance change\n\n- goodfetch-frontier/instructions::frontier::push total-bytes: +5.10%, the new index\n",
    &format!("Performance change accepted for {HEAD_SHA}.")
)]
#[case::no_section(
    "## Summary\n\n- goodfetch-frontier/instructions::frontier::push instructions: +5.10%\n",
    &format!("Performance change accepted for {HEAD_SHA}.")
)]
#[case::no_acceptance(SECTION, &format!("Looks fine at {HEAD_SHA}."))]
#[case::section_line_without_a_reason(
    "## Performance change\n\n- goodfetch-frontier/instructions::frontier::push instructions: +5.10%\n",
    &format!("Performance change accepted for {HEAD_SHA}.")
)]
fn tripped_figure_without_a_matching_acceptance_is_noted(
    #[case] body: &str,
    #[case] comment: &str,
) {
    let workspace = Workspace::from_fixture("empty");
    write_gungraun(
        &workspace,
        &[metric("Ir", Some(1_000_000), Some(1_051_000))],
    );
    let github = pull_request(body, comment);

    let outcome = compare_for_pull_request(&workspace, &github, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![finding(
            "performance-change",
            &format!("{BENCHMARK} instructions")
        )]
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn entry_grown_past_5_kb_listed_and_accepted_on_the_head_passes() {
    let workspace = Workspace::from_fixture("empty");
    write_bundles(
        &workspace,
        &[("collie-client", 100_000)],
        &[("collie-client", 106_200)],
    );
    let github = pull_request(
        "## Performance change\n\n- size-limit/collie-client brotli: +6.20 kB, the extractor's tables\n",
        &format!("Performance change accepted for {HEAD_SHA}."),
    );

    let outcome = compare_for_pull_request(&workspace, &github, &BUNDLES);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn console_build_over_1_mb_is_noted_whatever_the_pull_request_says() {
    let workspace = Workspace::from_fixture("empty");
    write_bundles(
        &workspace,
        &[("foxhound-console", 990_000)],
        &[("foxhound-console", 1_006_000)],
    );
    let github = pull_request(
        "## Performance change\n\n- size-limit/foxhound-console brotli: +16.00 kB, the Runs screen\n",
        &format!("Performance change accepted for {HEAD_SHA}."),
    );

    let outcome = compare_for_pull_request(&workspace, &github, &BUNDLES);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![finding(
            "over-ceiling",
            "size-limit/foxhound-console brotli"
        )]
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// The admission runs of the head's new benchmark, one gungraun report per
/// run of one build: every run counts `lowest` instructions but the last,
/// which counts `highest`.
fn write_admission(workspace: &Workspace, runs: usize, lowest: u64, highest: u64) {
    write_gungraun(workspace, &[metric("Ir", None, Some(lowest))]);
    for run in 1..=runs {
        let count = if run == runs { highest } else { lowest };
        let line = summary(
            "crates/foxhound/goodfetch-frontier",
            "instructions::frontier::push",
            &[metric("Ir", None, Some(count))],
        );
        workspace.write(
            &format!("perf/admission/run-{run:02}.jsonl"),
            line.as_bytes(),
        );
    }
}

const ADMISSION: [&str; 2] = ["--admission", "perf/admission"];

#[test]
fn new_benchmark_spreading_0_4_percent_over_20_runs_is_admitted() {
    let workspace = Workspace::from_fixture("empty");
    write_admission(&workspace, 20, 1_000_000, 1_004_000);

    let outcome = compare(&workspace, &ADMISSION);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn new_benchmark_spreading_0_6_percent_over_20_runs_fails_its_admission() {
    let workspace = Workspace::from_fixture("empty");
    write_admission(&workspace, 20, 1_000_000, 1_006_000);

    let outcome = compare(&workspace, &ADMISSION);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "admission-spread",
            &format!("{BENCHMARK} instructions")
        )]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[test]
fn new_benchmark_measuring_0_on_all_20_runs_is_admitted() {
    let workspace = Workspace::from_fixture("empty");
    write_admission(&workspace, 20, 0, 0);

    let outcome = compare(&workspace, &ADMISSION);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn new_benchmark_measuring_0_on_some_runs_only_fails_its_admission() {
    let workspace = Workspace::from_fixture("empty");
    write_admission(&workspace, 20, 0, 1);

    let outcome = compare(&workspace, &ADMISSION);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "admission-spread",
            &format!("{BENCHMARK} instructions")
        )]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[test]
fn new_benchmark_with_fewer_than_20_admission_runs_fails_its_admission() {
    let workspace = Workspace::from_fixture("empty");
    write_admission(&workspace, 19, 1_000_000, 1_000_000);

    let outcome = compare(&workspace, &ADMISSION);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "admission-runs",
            &format!("{BENCHMARK} instructions")
        )]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

/// A broken measurement still fails while the budget only notes: a figure
/// past its threshold beside a new benchmark that fails its admission.
#[test]
fn admission_failure_fails_beside_a_noted_figure_past_its_threshold() {
    let workspace = Workspace::from_fixture("empty");
    write_admission(&workspace, 20, 1_000_000, 1_006_000);
    let tripped = summary(
        "crates/foxhound/goodfetch-frontier",
        "heap::frontier::push",
        &[metric("TotalBytes", Some(1_000_000), Some(1_051_000))],
    );
    let new = workspace.read_text("perf/head.jsonl");
    workspace.write("perf/head.jsonl", format!("{new}\n{tripped}\n").as_bytes());

    let outcome = compare(&workspace, &ADMISSION);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "admission-spread",
            &format!("{BENCHMARK} instructions")
        )]
    );
    assert_eq!(
        outcome.notes(),
        vec![finding(
            "performance-change",
            "goodfetch-frontier/heap::frontier::push total-bytes"
        )]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

/// A run of every verdict: the frontier's figures within, past and below
/// their thresholds, a new benchmark with its admission runs, an entry
/// accepted and a Console build over its ceiling.
fn write_every_verdict(workspace: &Workspace) {
    let frontier = summary(
        "crates/foxhound/goodfetch-frontier",
        "instructions::frontier::push",
        &[metric("Ir", Some(2_000_000), Some(2_010_000))],
    );
    let heap = summary(
        "crates/foxhound/goodfetch-frontier",
        "heap::frontier::push",
        &[
            metric("TotalBlocks", Some(1_000), Some(1_200)),
            metric("TotalBytes", Some(64_000), Some(60_000)),
            metric("AtTGmaxBytes", Some(8_000), Some(8_000)),
        ],
    );
    let new = summary(
        "crates/goodfetch-engine",
        "instructions::dispatch::hello",
        &[metric("Ir", None, Some(500_000))],
    );
    workspace.write(
        "perf/head.jsonl",
        format!("{frontier}\n{heap}\n{new}\n").as_bytes(),
    );
    for run in 1..=20 {
        let count = if run == 20 { 501_000 } else { 500_000 };
        let line = summary(
            "crates/goodfetch-engine",
            "instructions::dispatch::hello",
            &[metric("Ir", None, Some(count))],
        );
        workspace.write(
            &format!("perf/admission/run-{run:02}.jsonl"),
            line.as_bytes(),
        );
    }
    workspace.write(
        "perf/size-base.json",
        size_limit(&[("collie-client", 100_000), ("collie-console", 990_000)]).as_bytes(),
    );
    workspace.write(
        "perf/size-head.json",
        size_limit(&[("collie-client", 106_200), ("collie-console", 1_000_400)]).as_bytes(),
    );
}

#[test]
fn table_lists_every_figure_with_its_base_head_change_and_verdict() {
    let workspace = Workspace::from_fixture("empty");
    write_every_verdict(&workspace);
    let github = pull_request(
        "## Performance change\n\n- size-limit/collie-client brotli: +6.20 kB, the extractor's tables\n",
        &format!("Performance change accepted for {HEAD_SHA}."),
    );

    let outcome = compare_for_pull_request(
        &workspace,
        &github,
        &[
            "--size-base",
            "perf/size-base.json",
            "--size-head",
            "perf/size-head.json",
            "--admission",
            "perf/admission",
        ],
    );

    insta::assert_snapshot!(outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn table_is_written_to_the_jobs_summary_page_too() {
    let workspace = Workspace::from_fixture("empty");
    write_gungraun(
        &workspace,
        &[metric("Ir", Some(1_000_000), Some(1_051_000))],
    );
    workspace.write("summary.md", b"## Earlier step\n");

    let outcome = compare(&workspace, &["--summary", "summary.md"]);

    let table = outcome
        .stdout
        .lines()
        .take_while(|line| !line.starts_with("note["))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        workspace.read_text("summary.md"),
        format!("## Earlier step\n{table}\n")
    );
}

const MERGE_COMMIT: &str = "abababababababababababababababababababab";

/// GitHub's pull requests of `MERGE_COMMIT`: `pulls` as it lists them, and
/// pull request 7 with the accepted section.
fn merged_pull_request(pulls: &str) -> GitHub {
    let pull_request = format!(
        r#"{{ "number": 7, "body": {}, "head": {{ "sha": "{HEAD_SHA}" }} }}"#,
        serde_json::to_string(SECTION).unwrap()
    );
    let comments = format!(
        r#"[ {{ "body": "Performance change accepted for {HEAD_SHA}.", "author_association": "OWNER" }} ]"#
    );
    let commit_pulls = format!("/repos/goodfetch/goodfetch/commits/{MERGE_COMMIT}/pulls");
    GitHub::serving(&[
        (&commit_pulls, pulls),
        (PULL_REQUEST, &pull_request),
        (COMMENTS, &comments),
    ])
}

/// Saves the acceptance of the pull request that merged `MERGE_COMMIT`, as a
/// Release suite cell does on `main`, then compares with it.
fn compare_for_merge_commit(workspace: &Workspace, github: &GitHub) -> Outcome {
    let saved = workspace.xtask(&[
        "perf",
        "pull-request",
        "--commit",
        MERGE_COMMIT,
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
        "--save",
        "perf/pull-request.json",
    ]);
    assert_eq!(saved.code, 0, "{}", saved.stderr);
    compare(workspace, &["--pull-request", "perf/pull-request.json"])
}

#[test]
fn tripped_figure_accepted_in_the_pull_request_that_merged_the_commit_passes() {
    let workspace = Workspace::from_fixture("empty");
    write_gungraun(
        &workspace,
        &[metric("Ir", Some(1_000_000), Some(1_051_000))],
    );
    let github = merged_pull_request(&format!(
        r#"[ {{ "number": 7, "merge_commit_sha": "{MERGE_COMMIT}" }} ]"#
    ));

    let outcome = compare_for_merge_commit(&workspace, &github);

    assert_eq!(outcome.findings(), vec![]);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn tripped_figure_of_a_commit_no_pull_request_merged_is_noted() {
    let workspace = Workspace::from_fixture("empty");
    write_gungraun(
        &workspace,
        &[metric("Ir", Some(1_000_000), Some(1_051_000))],
    );
    let github = merged_pull_request(&format!(
        r#"[ {{ "number": 7, "merge_commit_sha": "{OTHER_SHA}" }} ]"#
    ));

    let outcome = compare_for_merge_commit(&workspace, &github);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![finding(
            "performance-change",
            &format!("{BENCHMARK} instructions")
        )]
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn unreadable_pull_request_is_an_error_never_a_pass() {
    let workspace = Workspace::from_fixture("empty");
    let github = GitHub::serving(&[]);

    let outcome = workspace.xtask(&[
        "perf",
        "pull-request",
        "--number",
        "7",
        "--repository",
        "goodfetch/goodfetch",
        "--api-url",
        github.url(),
        "--save",
        "perf/pull-request.json",
    ]);

    assert_eq!(outcome.code, 2);
}
