//! `xtask chromium expectations regenerate` (ADR 0074): the generated
//! `upstream-flaky` files, `gtest/generated/<suite>-<platform>.filter`, from
//! LUCI Analysis's record of Chromium's CI builds, which it answers without
//! a Google account.
//!
//! For each suite:
//! 1. `Clusters.QueryClusterSummaries` gives the failure clusters of the
//!    suite's tests, over the whole window and each week of it, across the
//!    fleet and on each platform's bots: it answers 200 clusters at most.
//! 2. A cluster of one test names it; `Clusters.QueryClusterFailures` lists
//!    the tests of the others (parameterised families and bug rules).
//!    Failure-reason clusters are left out: their failures also count in a
//!    test's own cluster.
//! 3. `TestHistory.QueryStats` gives each candidate's verdicts per day and
//!    variant, and `TestHistory.QueryVariants` each variant's builder and
//!    OS, which give its platform; bots of a configuration the Goodfetch
//!    Chromium never builds (sanitizers, `ChromeOS`, mobile, coverage) count
//!    nowhere.
//! 4. A test whose failed and flaky verdicts are the flake rate or more of
//!    its passed, failed and flaky ones on a platform, over 10 of them at
//!    least, is an entry of that platform's file.
//!
//! The approach is Brave's `update-upstream-flake-filters.py`
//! (`tools/chromium_tests_analysis/` in brave-core).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::environment::Environment;
use crate::github::{self, Api, Write};
use crate::issues::Label;
use crate::output;

/// LUCI Analysis's pRPC interface.
const LUCI_URL: &str = "https://analysis.api.luci.app";

/// The LUCI project of Chromium's CI.
const PROJECT: &str = "chromium";

/// The generated files, from the repository root.
const FOLDER: &str = "chromium/expectations/gtest/generated";

/// The fewest passed, failed and flaky verdicts a rate is read from.
const MIN_VERDICTS: u64 = 10;

/// The largest page `TestHistory` answers.
const PAGE_SIZE: u32 = 1000;

/// The largest answer read: 2000 cluster failures fit well within it.
const ANSWER_LIMIT: u64 = 64 << 20;

/// Each platform the Goodfetch Chromium ships on, and how its bots' OS
/// dimension starts.
const PLATFORMS: [(&str, &str); 3] = [("linux", "Ubuntu"), ("mac", "Mac"), ("windows", "Windows")];

/// Words of a builder's name, in lower case, for configurations the
/// Goodfetch Chromium never builds.
const OTHER_BUILDS: [&str; 12] = [
    "asan", "lsan", "msan", "tsan", "ubsan", "cfi", "chromeos", "android", "fuchsia", "cast",
    "ios", "coverage",
];

/// The days a discovery query covers at most: `QueryClusterSummaries` ranks
/// the top 200 clusters, so each week of the window gets its own query.
const WEEK: i64 = 7;

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The days of CI history read, up to today.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u16).range(1..))]
    days: u16,
    /// The flake rate, in percent, at or above which a test is an entry.
    #[arg(long, default_value_t = 1.0)]
    min_flake_rate: f64,
    /// Today's date in UTC, `YYYY-MM-DD`, where the window ends; the
    /// system's when left out.
    #[arg(long)]
    today: Option<Date>,
    /// LUCI Analysis, or a stand-in's address.
    #[arg(long, default_value = LUCI_URL)]
    luci_url: String,
    #[command(flatten)]
    github: github::Arguments,
    /// The suites, as `gn` names them, such as `unit_tests`.
    #[arg(required = true)]
    suites: Vec<String>,
}

/// A test's verdicts on one platform.
#[derive(Clone, Copy, Default, Deserialize)]
struct Counts {
    #[serde(default)]
    passed: u64,
    #[serde(default)]
    failed: u64,
    #[serde(default)]
    flaky: u64,
}

impl Counts {
    fn add(&mut self, other: Self) {
        self.passed += other.passed;
        self.failed += other.failed;
        self.flaky += other.flaky;
    }

    fn meaningful(self) -> u64 {
        self.passed + self.failed + self.flaky
    }

    /// The failed and flaky verdicts, in percent of the meaningful ones.
    #[expect(
        clippy::cast_precision_loss,
        reason = "verdict counts stay far below 2^52"
    )]
    fn rate(self) -> f64 {
        (self.failed + self.flaky) as f64 * 100.0 / self.meaningful() as f64
    }
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let today = arguments
        .today
        .unwrap_or_else(|| Timestamp::now().to_zoned(TimeZone::UTC).date());
    let latest = today.to_zoned(TimeZone::UTC)?.timestamp();
    let earliest = latest - SignedDuration::from_hours(i64::from(arguments.days) * 24);
    let luci = Luci {
        url: &arguments.luci_url,
    };
    let mut files = Vec::new();
    for suite in &arguments.suites {
        let tests = candidates(&luci, suite, earliest, latest)?;
        let entries = entries(&luci, &tests, earliest, latest, arguments.min_flake_rate)?;
        files.push((suite, entries));
    }
    let api = arguments.github.authenticated(environment.token())?;
    let folder = environment.root.join(FOLDER);
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    for (suite, entries) in files {
        let command = format!(
            "xtask chromium expectations regenerate --days {} --min-flake-rate {} {suite}",
            arguments.days, arguments.min_flake_rate
        );
        let mut written = BTreeSet::new();
        for (platform, tests) in entries {
            let name = format!("{suite}-{platform}.filter");
            let path = folder.join(&name);
            let issue = match tracking_issue(&path)? {
                Some(issue) => issue,
                None => open(&api, &arguments.github.repository, suite, platform, &name)?,
            };
            let mut text = format!(
                "# Generated by: {command}\n# Tracking issue: #{issue}\n# Do not edit: the command above rewrites this file from LUCI Analysis.\n"
            );
            for (test, counts) in &tests {
                writeln!(
                    text,
                    "\n# upstream-flaky rate={:.1}% passed={} failed={} flaky={}\n-{test}",
                    counts.rate(),
                    counts.passed,
                    counts.failed,
                    counts.flaky
                )?;
            }
            std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
            output::line(&format!("wrote {FOLDER}/{name}: {} entries", tests.len()));
            written.insert(name);
        }
        remove_stale(&folder, suite, &written)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// The suite's tests in the failure clusters of the window, by LUCI test id.
fn candidates(
    luci: &Luci<'_>,
    suite: &str,
    earliest: Timestamp,
    latest: Timestamp,
) -> Result<BTreeSet<String>> {
    let marker = format!(":{suite}!gtest");
    let fleet = format!("test_id:\"{marker}\"");
    let filters: Vec<String> = std::iter::once(fleet.clone())
        .chain(
            PLATFORMS
                .iter()
                .map(|(_, os)| format!("{fleet} variant.os:\"{os}\"")),
        )
        .collect();
    let mut windows = vec![(earliest, latest)];
    let mut start = earliest;
    while start < latest {
        let end = (start + SignedDuration::from_hours(WEEK * 24)).min(latest);
        windows.push((start, end));
        start = end;
    }
    let mut clusters = BTreeMap::new();
    for (from, to) in &windows {
        for filter in &filters {
            let answer: Summaries = luci.call(
                "luci.analysis.v1.Clusters/QueryClusterSummaries",
                &json!({
                    "project": PROJECT,
                    "failureFilter": filter,
                    "orderBy": "metrics.`failures`.value desc",
                    "timeRange": {"earliest": from.to_string(), "latest": to.to_string()},
                    "metrics": [format!("projects/{PROJECT}/metrics/failures")],
                }),
            )?;
            for summary in answer.cluster_summaries {
                if !summary.cluster_id.algorithm.starts_with("reason-") {
                    clusters.insert(
                        (summary.cluster_id.algorithm, summary.cluster_id.id),
                        summary.title,
                    );
                }
            }
        }
    }
    let mut tests = BTreeSet::new();
    for ((algorithm, id), title) in clusters {
        if algorithm.starts_with("testname-") && !title.contains('%') {
            // A one-test cluster's title is its test id, its backslash
            // escaped.
            let test = title.replace("\\\\", "\\");
            if test.contains(&marker) {
                tests.insert(normalized(&test));
            }
            continue;
        }
        let answer: Failures = luci.call(
            "luci.analysis.v1.Clusters/QueryClusterFailures",
            &json!({"parent": format!("projects/{PROJECT}/clusters/{algorithm}/{id}/failures")}),
        )?;
        for failure in answer.failures {
            if failure.test_id.contains(&marker) {
                tests.insert(normalized(&failure.test_id));
            }
        }
    }
    Ok(tests)
}

/// Each platform's entries: the gtest names of `tests` at or above
/// `min_rate` there, in order, with their verdicts.
fn entries(
    luci: &Luci<'_>,
    tests: &BTreeSet<String>,
    earliest: Timestamp,
    latest: Timestamp,
    min_rate: f64,
) -> Result<BTreeMap<&'static str, BTreeMap<String, Counts>>> {
    let mut platforms: BTreeMap<String, Option<&'static str>> = BTreeMap::new();
    let mut entries: BTreeMap<&'static str, BTreeMap<String, Counts>> = BTreeMap::new();
    for test in tests {
        let Some(name) = gtest_name(test) else {
            continue;
        };
        let groups = history(luci, test, earliest, latest)?;
        if groups
            .iter()
            .any(|group| !platforms.contains_key(&group.variant_hash))
        {
            for variant in variants(luci, test)? {
                platforms.insert(variant.variant_hash, platform(&variant.variant.def));
            }
            for group in &groups {
                platforms.entry(group.variant_hash.clone()).or_insert(None);
            }
        }
        let mut counts: BTreeMap<&'static str, Counts> = BTreeMap::new();
        for group in &groups {
            if let Some(Some(platform)) = platforms.get(&group.variant_hash) {
                counts
                    .entry(platform)
                    .or_default()
                    .add(group.verdict_counts);
            }
        }
        for (platform, counts) in counts {
            if counts.meaningful() >= MIN_VERDICTS && counts.rate() >= min_rate {
                entries
                    .entry(platform)
                    .or_default()
                    .insert(name.clone(), counts);
            }
        }
    }
    Ok(entries)
}

/// A test's verdicts per day and variant over the window, from CI builds.
fn history(
    luci: &Luci<'_>,
    test: &str,
    earliest: Timestamp,
    latest: Timestamp,
) -> Result<Vec<Group>> {
    let mut groups = Vec::new();
    let mut token = String::new();
    loop {
        let page: Stats = luci.call(
            "luci.analysis.v1.TestHistory/QueryStats",
            &json!({
                "project": PROJECT,
                "testId": test,
                "predicate": {
                    "subRealm": "ci",
                    "partitionTimeRange": {"earliest": earliest.to_string(), "latest": latest.to_string()},
                },
                "pageSize": PAGE_SIZE,
                "pageToken": token,
            }),
        )?;
        groups.extend(page.groups);
        match page.next_page_token {
            Some(next) if !next.is_empty() => token = next,
            _ => return Ok(groups),
        }
    }
}

/// The variants a test ran in on CI builds.
fn variants(luci: &Luci<'_>, test: &str) -> Result<Vec<Variant>> {
    let mut variants = Vec::new();
    let mut token = String::new();
    loop {
        let page: Variants = luci.call(
            "luci.analysis.v1.TestHistory/QueryVariants",
            &json!({
                "project": PROJECT,
                "testId": test,
                "subRealm": "ci",
                "pageSize": PAGE_SIZE,
                "pageToken": token,
            }),
        )?;
        variants.extend(page.variants);
        match page.next_page_token {
            Some(next) if !next.is_empty() => token = next,
            _ => return Ok(variants),
        }
    }
}

/// The platform a variant's bot tests, when the Goodfetch Chromium builds
/// its configuration.
fn platform(variant: &BTreeMap<String, String>) -> Option<&'static str> {
    let builder = variant.get("builder")?.to_lowercase();
    if OTHER_BUILDS.iter().any(|word| builder.contains(word)) {
        return None;
    }
    let os = variant.get("os")?;
    PLATFORMS
        .iter()
        .find(|(_, prefix)| os.starts_with(prefix))
        .map(|(platform, _)| *platform)
}

/// A `WebUI` test's JS sub-result, `<case>__<sub-result>`, as its gtest case,
/// the one a filter excludes.
fn normalized(test: &str) -> String {
    let Some((head, fine)) = test.split_once('#') else {
        return test.to_owned();
    };
    let (case, parameter) = match fine.split_once('/') {
        Some((case, parameter)) => (case, Some(parameter)),
        None => (fine, None),
    };
    let case = case.split("__").next().unwrap_or(case);
    match parameter {
        Some(parameter) => format!("{head}#{case}/{parameter}"),
        None => format!("{head}#{case}"),
    }
}

/// A LUCI gtest id's gtest name: `…!gtest::Suite#Case` is `Suite.Case`, and
/// `…!gtest::Suite#Case/Instance.Value` is `Instance/Suite.Case/Value`.
fn gtest_name(test: &str) -> Option<String> {
    let (_, fine) = test.split_once("!gtest::")?;
    let (suite, case) = fine.split_once('#')?;
    let Some((case, parameter)) = case.split_once('/') else {
        return Some(format!("{suite}.{case}"));
    };
    Some(match parameter.split_once('.') {
        Some((instance, value)) => format!("{instance}/{suite}.{case}/{value}"),
        None => format!("{suite}.{case}/{parameter}"),
    })
}

/// The tracking issue an existing file names.
fn tracking_issue(path: &Path) -> Result<Option<u64>> {
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(text.lines().find_map(|line| {
        line.strip_prefix("# Tracking issue: #")
            .and_then(|number| number.trim().parse().ok())
    }))
}

#[derive(Deserialize)]
struct Created {
    number: u64,
}

/// Opens the tracking issue of a new file, outside every milestone.
fn open(api: &Api<'_>, repository: &str, suite: &str, platform: &str, name: &str) -> Result<u64> {
    let created: Created = api.send(
        Write::Post,
        &format!("/repos/{repository}/issues"),
        &json!({
            "title": format!("Upstream-flaky exclusions: {suite} on {platform}"),
            "body": format!(
                "Tracks `{FOLDER}/{name}`: the `{suite}` tests LUCI Analysis finds failing or flaking on Chromium's {platform} CI bots, which the Chromium build gate excludes. Each Chromium pull request that changes the Chromium version regenerates the file with the command at its top (ADR 0074; TESTING.md, The Goodfetch Chromium).\n\nArea 8 (Stealth and Challenge solving). These exclusions block no milestone or release."
            ),
            "labels": [Label::NeedsTriage.name()],
        }),
    )?;
    output::line(&format!("opened #{} for {FOLDER}/{name}", created.number));
    Ok(created.number)
}

/// Removes the suite's files this run did not write: their platforms have
/// no entry left.
fn remove_stale(folder: &Path, suite: &str, written: &BTreeSet<String>) -> Result<()> {
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let platform = name
            .strip_prefix(suite)
            .and_then(|rest| rest.strip_prefix('-'))
            .and_then(|rest| rest.strip_suffix(".filter"));
        if platform.is_some_and(|platform| PLATFORMS.iter().any(|(known, _)| *known == platform))
            && !written.contains(name)
        {
            let issue = tracking_issue(&path)?;
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            output::line(&match issue {
                Some(issue) => {
                    format!("removed {FOLDER}/{name}: its tracking issue #{issue} can close")
                }
                None => format!("removed {FOLDER}/{name}"),
            });
        }
    }
    Ok(())
}

/// LUCI Analysis's pRPC interface, or a stand-in's address.
struct Luci<'a> {
    url: &'a str,
}

impl Luci<'_> {
    /// Calls `method` with `request` as JSON and parses the answer, which
    /// pRPC starts with `)]}'` against cross-site script inclusion.
    fn call<T: DeserializeOwned>(&self, method: &str, request: &Value) -> Result<T> {
        let url = format!("{}/prpc/{method}", self.url.trim_end_matches('/'));
        let text = ureq::post(&url)
            .header("Accept", "application/json")
            .header("User-Agent", "goodfetch-xtask")
            .send_json(request)
            .with_context(|| format!("asking {url}"))?
            .body_mut()
            .with_config()
            .limit(ANSWER_LIMIT)
            .read_to_string()
            .with_context(|| format!("reading {url}"))?;
        let json = text.strip_prefix(")]}'").unwrap_or(&text);
        serde_json::from_str(json).with_context(|| format!("reading {url}'s answer"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summaries {
    #[serde(default)]
    cluster_summaries: Vec<Summary>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    cluster_id: ClusterId,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
struct ClusterId {
    algorithm: String,
    id: String,
}

#[derive(Deserialize)]
struct Failures {
    #[serde(default)]
    failures: Vec<Failure>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Failure {
    test_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stats {
    #[serde(default)]
    groups: Vec<Group>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Group {
    variant_hash: String,
    #[serde(default)]
    verdict_counts: Counts,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Variants {
    #[serde(default)]
    variants: Vec<Variant>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Variant {
    variant_hash: String,
    #[serde(default)]
    variant: Definition,
}

#[derive(Default, Deserialize)]
struct Definition {
    #[serde(default)]
    def: BTreeMap<String, String>,
}
