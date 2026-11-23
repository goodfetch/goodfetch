//! The scan of the results store after each ingest (ADR 0064; TESTING.md,
//! After merge and Live checks): a test that passed and failed on the same
//! tree hash and configuration (OS, architecture, runtime), in any tiers or
//! attempts, is flaky and gets one `flaky-test` issue, reopened when it
//! recurs after the issue closed; a Live check (the `live` tier) whose last
//! result failed, or whose last three were inconclusive, opens or updates
//! its one `live-check` issue. A new issue goes to the spec whose ticket
//! added the test or declared the Live check (ADR 0136), or to triage when
//! none is found.

#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::num::NonZeroU8;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::github::Api;
use crate::issues::{Issues, Label};
use crate::output;
use crate::owners::{Added, Owners};
use crate::results::{self, Outcome, TestRun};

/// The tier Live checks report under (ADR 0064).
const LIVE_TIER: &str = "live";

/// Inconclusive results in a row that open a Live check's issue (ADR 0064).
const INCONCLUSIVE_IN_A_ROW: usize = 3;

/// A test, as its runner names it.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TestKey {
    runner: String,
    test: String,
}

/// The inputs a flaky test's results share: the tree and the configuration.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Inputs {
    tree: String,
    os: String,
    arch: String,
    runtime: Option<String>,
}

/// The Live checks and their owning areas, numbered as ADR 0037 numbers them.
pub(crate) const LIVE_CHECKS: &str = "ci/live-checks.toml";

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct LiveChecks {
    #[serde(default, rename = "live-check")]
    checks: Vec<LiveCheck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveCheck {
    /// The check's ID, as its results name it.
    id: String,
    area: NonZeroU8,
}

impl LiveChecks {
    /// The list in the workspace at `root`; none when the file is absent.
    fn read(root: &Path) -> Result<Self> {
        let path = root.join(LIVE_CHECKS);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    fn area(&self, id: &str) -> Option<NonZeroU8> {
        self.checks
            .iter()
            .find(|check| check.id == id)
            .map(|check| check.area)
    }
}

/// Scans the store at `store` and opens, updates or reopens the issues
/// through `api` in `repository`. Fails on a Live check that
/// `ci/live-checks.toml` does not declare, after keeping its issue.
pub(crate) fn scan(
    root: &Path,
    store: &Path,
    api: &Api<'_>,
    repository: &str,
) -> Result<Vec<Finding>> {
    let live_checks = LiveChecks::read(root)?;
    let mut findings = Vec::new();
    let attempts = results::read_all(store)?;
    let test_runs: Vec<&TestRun> = attempts
        .iter()
        .flat_map(|attempt| &attempt.test_runs)
        .collect();
    let mut issues = Issues::new(api, repository);
    let owners = Owners::new(api, repository);
    for (test, groups) in flaky_tests(&test_runs) {
        let latest = groups
            .values()
            .flatten()
            .map(|test_run| test_run.started_at)
            .max();
        let Some(latest) = latest else { continue };
        let title = format!("Flaky test: {} ({})", test.test, test.runner);
        let key = format!("{} {}", test.runner, test.test);
        let body = flaky_test_body(&test, &groups);
        issues.keep(Label::FlakyTest, &key, &title, &body, latest, || {
            added_test(&test).map_or(Ok(None), |added| owners.of_added(root, &added))
        })?;
    }
    for (check, last) in failing_live_checks(&test_runs) {
        let area = if let Some(area) = live_checks.area(&check) {
            format!("area {area}")
        } else {
            findings.push(Finding::new(
                "undeclared-live-check",
                check.as_str(),
                format!("declare the Live check and its owning area in {LIVE_CHECKS}"),
            ));
            "area undeclared".to_owned()
        };
        let title = format!("Live check: {check} ({area})");
        let body = live_check_body(&check, &area, &last);
        let latest = last.iter().map(|test_run| test_run.started_at).max();
        let Some(latest) = latest else { continue };
        issues.keep(Label::LiveCheck, &check, &title, &body, latest, || {
            let added = Added {
                text: format!("\"{check}\""),
                files: Some(LIVE_CHECKS.to_owned()),
            };
            owners.of_added(root, &added)
        })?;
    }
    for action in issues.actions() {
        output::line(action);
    }
    Ok(findings)
}

/// The text that first wrote `test` in the source: a nextest test's function
/// (an rstest case's leaf is its case, under its function), a pytest test's
/// function without its parameters, a Vitest or Playwright test's own title,
/// the last of its titles, searched in the runner's test files, so an
/// earlier mention elsewhere does not count. None for another runner.
fn added_test(test: &TestKey) -> Option<Added> {
    let (written, files) = match test.runner.as_str() {
        "nextest" => {
            let path = test
                .test
                .split_once(' ')
                .map_or(test.test.as_str(), |(_, path)| path);
            let mut segments = path.rsplit("::");
            let leaf = segments.next()?;
            let is_case = leaf
                .strip_prefix("case_")
                .is_some_and(|rest| rest.starts_with(|first: char| first.is_ascii_digit()));
            let function = if is_case { segments.next()? } else { leaf };
            (format!("fn {function}("), ":(glob)**/*.rs")
        }
        "pytest" => {
            let leaf = test.test.rsplit("::").next()?;
            let function = leaf.split_once('[').map_or(leaf, |(function, _)| function);
            (format!("def {function}("), ":(glob)**/test_*.py")
        }
        "vitest" => (own_title(&test.test).to_owned(), ":(glob)**/*.test.*"),
        "playwright" => (own_title(&test.test).to_owned(), ":(glob)**/*.spec.*"),
        _ => return None,
    };
    (!written.is_empty()).then(|| Added {
        text: written,
        files: Some(files.to_owned()),
    })
}

/// A Vitest or Playwright test's own title: the last of its titles.
fn own_title(test: &str) -> &str {
    [" › ", " > "].iter().fold(test, |title, separator| {
        title.rsplit(separator).next().unwrap_or(title)
    })
}

/// Each flaky test, with the results of each of its inputs that both passed
/// and failed.
fn flaky_tests<'a>(
    test_runs: &[&'a TestRun],
) -> BTreeMap<TestKey, BTreeMap<Inputs, Vec<&'a TestRun>>> {
    let mut by_test: BTreeMap<TestKey, BTreeMap<Inputs, Vec<&TestRun>>> = BTreeMap::new();
    for test_run in test_runs {
        if test_run.tier == LIVE_TIER {
            continue;
        }
        let test = TestKey {
            runner: test_run.runner.clone(),
            test: test_run.test.clone(),
        };
        let inputs = Inputs {
            tree: test_run.tree.clone(),
            os: test_run.os.clone(),
            arch: test_run.arch.clone(),
            runtime: test_run.runtime.clone(),
        };
        by_test
            .entry(test)
            .or_default()
            .entry(inputs)
            .or_default()
            .push(test_run);
    }
    by_test
        .into_iter()
        .filter_map(|(test, groups)| {
            let flaky: BTreeMap<_, _> = groups
                .into_iter()
                .filter(|(_, runs)| {
                    let has = |outcome| runs.iter().any(|run| run.outcome == outcome);
                    has(Outcome::Passed) && has(Outcome::Failed)
                })
                .collect();
            (!flaky.is_empty()).then_some((test, flaky))
        })
        .collect()
}

/// Each Live check whose last result failed or whose last three were
/// inconclusive, with its last results, oldest first.
fn failing_live_checks<'a>(test_runs: &[&'a TestRun]) -> BTreeMap<String, Vec<&'a TestRun>> {
    let mut by_check: BTreeMap<String, Vec<&TestRun>> = BTreeMap::new();
    for test_run in test_runs.iter().filter(|run| run.tier == LIVE_TIER) {
        by_check
            .entry(test_run.test.clone())
            .or_default()
            .push(test_run);
    }
    by_check
        .into_iter()
        .filter_map(|(check, mut results)| {
            results.sort_by_key(|run| (run.started_at, run.run, run.attempt));
            let last = results.last()?;
            let tail = &results[results.len().saturating_sub(INCONCLUSIVE_IN_A_ROW)..];
            let inconclusive = tail.len() == INCONCLUSIVE_IN_A_ROW
                && tail.iter().all(|run| run.outcome == Outcome::Inconclusive);
            (last.outcome == Outcome::Failed || inconclusive).then(|| (check, tail.to_vec()))
        })
        .collect()
}

fn flaky_test_body(test: &TestKey, groups: &BTreeMap<Inputs, Vec<&TestRun>>) -> String {
    let mut body = format!(
        "`{}` ({}) passed and failed with the same inputs: the same tree hash and configuration (ADR 0064).\n\n\
         Quarantine it the same day: add it to `quarantine.toml` with this issue (TESTING.md, A flaky test found after merge).\n\n\
         | Tree | OS | Architecture | Runtime | Run | Attempt | Tier | Job | Outcome | Runs | Order seed | `Entropy` seed | Commit |\n\
         |---|---|---|---|---|---|---|---|---|---|---|---|---|\n",
        test.test, test.runner
    );
    for (inputs, test_runs) in groups {
        let mut rows: BTreeMap<_, usize> = BTreeMap::new();
        for run in test_runs {
            let row = (
                run.started_at,
                run.run,
                run.attempt,
                run.tier.as_str(),
                run.job.as_str(),
                run.outcome.name(),
                run.order_seed.as_deref().unwrap_or("-"),
                run.entropy_seed.as_deref().unwrap_or("-"),
                run.commit.as_str(),
            );
            *rows.entry(row).or_default() += 1;
        }
        for ((_, run, attempt, tier, job, outcome, order, entropy, commit), count) in rows {
            let _ = writeln!(
                body,
                "| `{}` | {} | {} | {} | {run} | {attempt} | {tier} | {job} | {outcome} | {count} | {order} | {entropy} | `{commit}` |",
                inputs.tree,
                inputs.os,
                inputs.arch,
                inputs.runtime.as_deref().unwrap_or("-"),
            );
        }
    }
    body
}

fn live_check_body(check: &str, area: &str, last: &[&TestRun]) -> String {
    let mut body = format!(
        "The Live check `{check}`, owned by {area} (ADR 0037), failed, or was inconclusive {INCONCLUSIVE_IN_A_ROW} times in a row (ADR 0064).\n\n\
         State its cause in a comment within two business days. This issue closes once the cause is fixed in Goodfetch, \
         or attributed to an external party with a linked upstream report.\n\n\
         | Started | Run | Attempt | Job | Outcome | Commit |\n|---|---|---|---|---|---|\n"
    );
    for run in last {
        let _ = writeln!(
            body,
            "| {} | {} | {} | {} | {} | `{}` |",
            run.started_at,
            run.run,
            run.attempt,
            run.job,
            run.outcome.name(),
            run.commit
        );
    }
    body
}
