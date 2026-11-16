//! The absolute ceilings of the Performance budget (ADR 0134): `xtask perf
//! compare` holds each ceiling's figure of the head, read from gungraun's and
//! size-limit's reports, to the ceiling, beside the comparison with the base.
//! No "Performance change" section passes a figure over its ceiling; until
//! M1's optimization pass a figure over its ceiling is a note that fails
//! nothing (ADR 0134, as amended on 2026-10-22). A ceiling whose benchmark
//! the base or the head measured fails when the head has no figure for it.
//! Covers: story:1-108

use rstest::rstest;

use crate::perf::{
    HEAD_SHA, bench_summary, compare, compare_for_pull_request, metric, pull_request,
};
use crate::workspace::{Workspace, finding};

const FRONTIER: &str = "crates/foxhound/goodfetch-frontier";

/// The Frontier's 100,000-URL heap scenario and its zero-URL baseline, as
/// `xtask perf` names them.
const HUNDRED_THOUSAND_URLS: &str =
    "goodfetch-frontier/heap::frontier::enqueue/hundred_thousand_urls";

/// One heap scenario of the Frontier's, with its bytes at the global
/// maximum on the base and on the head.
fn enqueue(id: &str, base: Option<u64>, head: u64) -> String {
    bench_summary(
        FRONTIER,
        "heap::frontier::enqueue",
        Some(id),
        &[metric("AtTGmaxBytes", base, Some(head))],
    )
}

/// gungraun's report of the Frontier's two heap scenarios, new in the head.
fn write_frontier(workspace: &Workspace, hundred_thousand_urls: u64, no_urls: u64) {
    let full = enqueue("hundred_thousand_urls", None, hundred_thousand_urls);
    let empty = enqueue("no_urls", None, no_urls);
    workspace.write("perf/head.jsonl", format!("{full}\n{empty}\n").as_bytes());
}

/// One heap scenario of the Frontier's benchmark `bench`, new in the head,
/// with its bytes at the global maximum.
fn frontier_heap(bench: &str, id: &str, head: u64) -> String {
    bench_summary(
        FRONTIER,
        bench,
        Some(id),
        &[metric("AtTGmaxBytes", None, Some(head))],
    )
}

/// gungraun's report of the Frontier's heap scenarios `scenarios`, each a
/// benchmark, its scenario and its bytes, new in the head.
fn write_frontier_heap(workspace: &Workspace, scenarios: &[(&str, &str, u64)]) {
    let lines: String = scenarios
        .iter()
        .map(|(bench, id, bytes)| frontier_heap(bench, id, *bytes) + "\n")
        .collect();
    workspace.write("perf/head.jsonl", lines.as_bytes());
}

/// The Frontier's heap at a 1 MiB budget (ADR 0142), its 1,000,000 URLs
/// read against its 100,000.
const GROW: &str = "heap::frontier::grow";

/// The Frontier's heap at the default budget before any spill (ADR 0142),
/// its 100,000 URLs read against none.
const HOLD: &str = "heap::frontier::hold";

/// The Run state's heap at the default budget past the spill (ADR 0142),
/// its 1,000,000 URLs read against none.
const SPILL: &str = "heap::frontier::spill";

/// Foxhound's crawl heap scenario, as `xtask perf` names it.
const CRAWL: &str = "goodfetch-foxhound/heap::crawl::in_memory_pages";

/// The file in which the crawl's fixture records its total body bytes.
const PAGES: &str = "crates/foxhound/goodfetch-foxhound/benches/pages.source.toml";

/// gungraun's report of the crawl scenario, new in the head, with its total
/// bytes allocated, and its fixture's record of `body_bytes`.
fn write_crawl(workspace: &Workspace, total_bytes: u64, body_bytes: u64) {
    let crawl = bench_summary(
        "crates/foxhound/goodfetch-foxhound",
        "heap::crawl::in_memory_pages",
        None,
        &[metric("TotalBytes", None, Some(total_bytes))],
    );
    workspace.write("perf/head.jsonl", format!("{crawl}\n").as_bytes());
    workspace.write(
        PAGES,
        format!(
            "generator = \"goodfetch-test-support pages\"\nseed = 7\nbody-bytes = {body_bytes}\n"
        )
        .as_bytes(),
    );
}

#[rstest]
#[case::at_twice_the_body_bytes(2_000_000)]
#[case::under_twice_the_body_bytes(1_500_000)]
fn crawl_at_or_under_twice_its_body_bytes_passes(#[case] total_bytes: u64) {
    let workspace = Workspace::from_fixture("empty");
    write_crawl(&workspace, total_bytes, 1_000_000);

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn crawl_over_twice_its_body_bytes_is_noted() {
    let workspace = Workspace::from_fixture("empty");
    write_crawl(&workspace, 2_000_001, 1_000_000);

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![finding(
            "over-ceiling",
            &format!("{CRAWL} bytes-per-body-byte")
        )]
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn crawl_whose_fixture_records_no_body_bytes_is_an_error_never_a_pass() {
    let workspace = Workspace::from_fixture("empty");
    write_crawl(&workspace, 2_000_000, 1_000_000);
    workspace.write(
        PAGES,
        b"generator = \"goodfetch-test-support pages\"\nseed = 7\n",
    );

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.code, 2, "{}", outcome.stderr);
}

/// The derived figures, from hand-computed values: (1,742,501 − 1,000,000)
/// / 100,000 = 7.42501 bytes per URL, shown rounded up as 7.43; 1,234,567 /
/// 1,000,000 = 1.234567 times the body bytes, shown as 1.24.
#[test]
fn summary_lists_each_ceiling_with_its_derived_head_figure() {
    let workspace = Workspace::from_fixture("empty");
    write_crawl(&workspace, 1_234_567, 1_000_000);
    let crawl = workspace.read_text("perf/head.jsonl");
    let full = enqueue("hundred_thousand_urls", None, 1_742_501);
    let empty = enqueue("no_urls", None, 1_000_000);
    workspace.write(
        "perf/head.jsonl",
        format!("{crawl}{full}\n{empty}\n").as_bytes(),
    );

    let outcome = compare(&workspace, &[]);

    let ceilings: String = outcome
        .stdout
        .split_once("#### Absolute ceilings\n\n")
        .and_then(|(_, rest)| rest.split_once("\n#### Reference build"))
        .map(|(ceilings, _)| ceilings.to_owned())
        .unwrap_or_default();
    insta::assert_snapshot!(ceilings, @r#"
    | Benchmark or entry | Figure | Head | Ceiling | Verdict |
    |---|---|---:|---:|---|
    | goodfetch-frontier/heap::frontier::enqueue/hundred_thousand_urls | heap-bytes-per-url | 7.43 bytes | ≤ 8 bytes | within |
    | goodfetch-frontier/heap::frontier::grow/million_urls | heap-bytes-per-url | — | ≤ 8 bytes | not run yet |
    | goodfetch-frontier/heap::frontier::hold/hundred_thousand_urls | heap-bytes-per-url | — | ≤ 95 bytes | not run yet |
    | goodfetch-frontier/heap::frontier::spill/million_urls | heap-percent-of-budget | — | ≤ 110% of 67108864 bytes | not run yet |
    | goodfetch-foxhound/heap::crawl::in_memory_pages | bytes-per-body-byte | 1.24× | ≤ 2× | within |

    A "Performance change" section never passes a ceiling; only the owner moves one (ADR 0134).
    "#);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::at_8_bytes(1_800_000)]
#[case::under_8_bytes(1_500_000)]
fn frontier_at_or_under_8_bytes_per_url_passes(#[case] hundred_thousand_urls: u64) {
    let workspace = Workspace::from_fixture("empty");
    write_frontier(&workspace, hundred_thousand_urls, 1_000_000);

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    assert_eq!(outcome.notes(), vec![], "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[test]
fn frontier_over_8_bytes_per_url_is_noted() {
    let workspace = Workspace::from_fixture("empty");
    write_frontier(&workspace, 1_800_001, 1_000_000);

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![finding(
            "over-ceiling",
            &format!("{HUNDRED_THOUSAND_URLS} heap-bytes-per-url")
        )]
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

#[rstest]
#[case::renamed_in_the_head(
    &[("hundred_thousand_urls", 1_700_000), ("no_urls", 1_000_000)],
    &[("hundred_thousand", 1_700_000), ("no_urls", 1_000_000)]
)]
#[case::removed_from_the_head(
    &[("hundred_thousand_urls", 1_700_000), ("no_urls", 1_000_000)],
    &[("no_urls", 1_000_000)]
)]
#[case::half_added_in_the_head(&[], &[("hundred_thousand_urls", 1_700_000)])]
fn frontier_ceiling_with_a_benchmark_the_head_lacks_fails(
    #[case] base: &[(&str, u64)],
    #[case] head: &[(&str, u64)],
) {
    let workspace = Workspace::from_fixture("empty");
    let lines = |scenarios: &[(&str, u64)]| -> String {
        scenarios
            .iter()
            .map(|(id, bytes)| enqueue(id, None, *bytes) + "\n")
            .collect()
    };
    workspace.write("perf/base.jsonl", lines(base).as_bytes());
    workspace.write("perf/head.jsonl", lines(head).as_bytes());

    let outcome = compare(&workspace, &["--gungraun-base", "perf/base.jsonl"]);

    assert_eq!(
        outcome.findings(),
        vec![finding(
            "ceiling-not-measured",
            &format!("{HUNDRED_THOUSAND_URLS} heap-bytes-per-url")
        )]
    );
    assert_eq!(outcome.code, 1, "{}", outcome.stderr);
}

#[rstest]
#[case::listing_the_tripped_figure(
    "## Performance change\n\n\
     - goodfetch-frontier/heap::frontier::enqueue/hundred_thousand_urls global-maximum-bytes: +5.89%, the URL index\n"
)]
#[case::listing_the_ceiling_too(
    "## Performance change\n\n\
     - goodfetch-frontier/heap::frontier::enqueue/hundred_thousand_urls global-maximum-bytes: +5.89%, the URL index\n\
     - goodfetch-frontier/heap::frontier::enqueue/hundred_thousand_urls heap-bytes-per-url: +8.01 bytes, the URL index\n"
)]
fn frontier_over_8_bytes_per_url_is_noted_whatever_the_pull_request_says(#[case] body: &str) {
    let workspace = Workspace::from_fixture("empty");
    let full = enqueue("hundred_thousand_urls", Some(1_700_000), 1_800_100);
    let empty = enqueue("no_urls", Some(1_000_000), 1_000_000);
    workspace.write("perf/head.jsonl", format!("{full}\n{empty}\n").as_bytes());
    let github = pull_request(
        body,
        &format!("Performance change accepted for {HEAD_SHA}."),
    );

    let outcome = compare_for_pull_request(&workspace, &github, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stdout);
    assert_eq!(
        outcome.notes(),
        vec![finding(
            "over-ceiling",
            &format!("{HUNDRED_THOUSAND_URLS} heap-bytes-per-url")
        )]
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// ADR 0142's heap growth from 100,000 to 1,000,000 URLs at a 1 MiB test
/// budget, from hand-computed values: (8,200,000 − 1,000,000) / 900,000 =
/// 8 bytes per URL is at the ceiling, and one byte more is over it.
#[rstest]
#[case::at_8_bytes(8_200_000, &[])]
#[case::over_8_bytes(
    8_200_001,
    &["goodfetch-frontier/heap::frontier::grow/million_urls heap-bytes-per-url"]
)]
fn the_frontiers_heap_grows_at_most_8_bytes_per_url_past_100_000_at_a_1_mib_budget(
    #[case] million_urls: u64,
    #[case] noted: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    write_frontier_heap(
        &workspace,
        &[
            (GROW, "hundred_thousand_urls", 1_000_000),
            (GROW, "million_urls", million_urls),
        ],
    );

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    let expected: Vec<_> = noted.iter().map(|id| finding("over-ceiling", id)).collect();
    assert_eq!(outcome.notes(), expected, "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// ADR 0142's heap before any spill, at 100,000 URLs and the default budget,
/// from hand-computed values: (10,500,000 − 1,000,000) / 100,000 = 95 bytes
/// per URL is at the ceiling, and one byte more is over it.
#[rstest]
#[case::at_95_bytes(10_500_000, &[])]
#[case::over_95_bytes(
    10_500_001,
    &["goodfetch-frontier/heap::frontier::hold/hundred_thousand_urls heap-bytes-per-url"]
)]
fn the_frontier_holds_at_most_95_bytes_per_url_before_any_spill(
    #[case] hundred_thousand_urls: u64,
    #[case] noted: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    write_frontier_heap(
        &workspace,
        &[
            (HOLD, "no_urls", 1_000_000),
            (HOLD, "hundred_thousand_urls", hundred_thousand_urls),
        ],
    );

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    let expected: Vec<_> = noted.iter().map(|id| finding("over-ceiling", id)).collect();
    assert_eq!(outcome.notes(), expected, "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}

/// ADR 0142's Run state's heap at the default budget of 67,108,864 bytes and
/// 1,000,000 URLs, from hand-computed values: 1.10 × 67,108,864 =
/// 73,819,750.4 bytes, so 73,819,750 over the empty scenario's 1,000,000 is
/// within the ceiling, and one byte more is over it.
#[rstest]
#[case::at_the_budget_plus_10_percent(74_819_750, &[])]
#[case::over_the_budget_plus_10_percent(
    74_819_751,
    &["goodfetch-frontier/heap::frontier::spill/million_urls heap-percent-of-budget"]
)]
fn the_run_states_heap_past_the_spill_is_at_most_its_budget_plus_10_percent(
    #[case] million_urls: u64,
    #[case] noted: &[&str],
) {
    let workspace = Workspace::from_fixture("empty");
    write_frontier_heap(
        &workspace,
        &[
            (SPILL, "no_urls", 1_000_000),
            (SPILL, "million_urls", million_urls),
        ],
    );

    let outcome = compare(&workspace, &[]);

    assert_eq!(outcome.findings(), vec![], "{}", outcome.stderr);
    let expected: Vec<_> = noted.iter().map(|id| finding("over-ceiling", id)).collect();
    assert_eq!(outcome.notes(), expected, "{}", outcome.stdout);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
}
