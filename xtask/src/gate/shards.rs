//! The hosted shards' check (issue 3378): a hosted run's Pre-merge unit of a
//! member holds only when its shards ran the gate's own selection of that
//! member's tests, each test once. The build job lists the archive it built,
//! with the suite's own skips, and each shard runs one hash partition of it;
//! the members that are no crate are the build job's runners' (Vitest,
//! Playwright and pytest, issue 3380). This check then reads that listing
//! and every shard's nextest JUnit file and finds:
//! - a member the plan runs that neither the archive holds nor the runners
//!   ran, or one of those the plan does not run (`members-differ`), and a
//!   listed test of no planned member (`not-planned`);
//! - a listed test no shard ran (`not-run`), one two shards ran
//!   (`run-twice`), and one a shard ran that the listing does not select
//!   (`not-selected`), so a shard set that ran fewer tests than the
//!   selection fails;
//! - an insta snapshot of a planned member that no shard's test read
//!   (`unreferenced-snapshot`), as `cargo insta test --unreferenced` judges
//!   it: an error, or a note while the quarantine list skips a test, as the
//!   local suite has it (TESTING.md, Snapshots).
//!
//! A JUnit file that cannot be read is an error, never an empty shard.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::affected::{Kind, WorkspaceMembers};
use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::{git, junit, output};

/// How an insta snapshot no test read counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Unreferenced {
    /// It fails the check.
    Reject,
    /// It is noted: the quarantine list skips a test that may read it.
    Warn,
}

/// What `xtask gate shards` takes.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// `cargo nextest list --message-format json`'s output for the archive,
    /// with the suite's skips.
    #[arg(long)]
    listing: PathBuf,
    /// The members the build job archived, space-separated.
    #[arg(long)]
    archived: String,
    /// The members that are no crate, whose Vitest, Playwright and pytest
    /// tests the build job ran in full, space-separated.
    #[arg(long, default_value = "")]
    runners: String,
    /// The members the gate's plan runs the Pre-merge suite of here, its
    /// `pre-merge.members` line.
    #[arg(long)]
    members: String,
    /// One shard's nextest JUnit file, repeated for every shard; none when
    /// the build archived no crate.
    #[arg(long)]
    junit: Vec<PathBuf>,
    /// One shard's insta references file (`INSTA_SNAPSHOT_REFERENCES_FILE`),
    /// repeatable; a shard whose tests read no snapshot writes none.
    #[arg(long)]
    references: Vec<PathBuf>,
    #[arg(long, value_enum, default_value = "reject")]
    unreferenced: Unreferenced,
}

/// One shard's run: its name, for the findings, and the tests its JUnit
/// file holds, as `<binary ID> <test name>`.
pub(super) struct Shard {
    pub(super) name: String,
    pub(super) tests: Vec<String>,
}

/// What the check judges.
pub(super) struct Judged<'a> {
    pub(super) planned: &'a BTreeSet<String>,
    pub(super) archived: &'a BTreeSet<String>,
    /// The members the build job's runners ran, whose tests no archive
    /// holds.
    pub(super) runners: &'a BTreeSet<String>,
    /// Each test the listing selects, with its package.
    pub(super) listing: &'a BTreeMap<String, String>,
    pub(super) shards: &'a [Shard],
    /// Each insta snapshot file in the repository, from its folder, with
    /// the crate that owns it.
    pub(super) snapshots: &'a BTreeMap<String, String>,
    /// The snapshot files the shards' tests read, from the repository's
    /// folder.
    pub(super) references: &'a BTreeSet<String>,
    pub(super) unreferenced: Unreferenced,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    let read = |path: &Path| {
        std::fs::read_to_string(environment.path(path))
            .with_context(|| format!("reading {}", path.display()))
    };
    let listing = listing(&read(&arguments.listing)?)?;
    let mut shards = Vec::new();
    for path in &arguments.junit {
        let text = read(path)?;
        let tests = junit::test_cases("nextest", &text)
            .with_context(|| format!("reading the JUnit file {}", path.display()))?
            .into_iter()
            .map(|case| case.test)
            .collect();
        shards.push(Shard {
            name: path.display().to_string(),
            tests,
        });
    }
    let mut references = BTreeSet::new();
    for path in &arguments.references {
        let full = environment.path(path);
        // A shard whose tests read no snapshot leaves no file.
        if !full.exists() {
            continue;
        }
        for line in read(path)?.lines().filter(|line| !line.is_empty()) {
            references.insert(relative(root, line));
        }
    }
    let members = WorkspaceMembers::read(&environment.cargo, root)?;
    let crates: BTreeSet<String> = members
        .all()
        .filter(|(_, kind)| *kind == Kind::Crate)
        .map(|(name, _)| name)
        .collect();
    let snapshots: BTreeMap<String, String> = git::line(root, &["ls-files", "--", "*.snap"])?
        .lines()
        .filter_map(|path| {
            members
                .owner(path)
                .filter(|owner| crates.contains(owner))
                .map(|owner| (path.to_owned(), owner))
        })
        .collect();
    let (errors, notes) = judge(&Judged {
        planned: &names(&arguments.members),
        archived: &names(&arguments.archived),
        runners: &names(&arguments.runners),
        listing: &listing,
        shards: &shards,
        snapshots: &snapshots,
        references: &references,
        unreferenced: arguments.unreferenced,
    });
    check::note_to(&notes, output::error);
    let ran: usize = shards.iter().map(|shard| shard.tests.len()).sum();
    output::error(&format!(
        "Shards: {} shards ran {ran} tests of the {} the listing selects",
        shards.len(),
        listing.len()
    ));
    Ok(check::report_to(&errors, output::error))
}

/// The members of a space-separated list; `-`, the plan's empty list, holds
/// none.
fn names(list: &str) -> BTreeSet<String> {
    list.split_whitespace()
        .filter(|name| *name != "-")
        .map(str::to_owned)
        .collect()
}

/// `path` from `root`'s folder when it lies in it, as `git ls-files` names
/// it.
fn relative(root: &Path, path: &str) -> String {
    Path::new(path).strip_prefix(root).map_or_else(
        |_| path.to_owned(),
        |relative| relative.display().to_string(),
    )
}

#[derive(Deserialize)]
struct Listing {
    #[serde(rename = "rust-suites")]
    suites: BTreeMap<String, Suite>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Suite {
    package_name: String,
    testcases: BTreeMap<String, Case>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Case {
    filter_match: FilterMatch,
}

#[derive(Deserialize)]
struct FilterMatch {
    status: String,
}

/// Each test `cargo nextest list --message-format json` printed as matching
/// its filters, as `<binary ID> <test name>`, with its package: the profile's
/// default filter, the suite's skips and the ignored tests leave theirs out.
pub(super) fn listing(json: &str) -> Result<BTreeMap<String, String>> {
    let listing: Listing =
        serde_json::from_str(json).context("reading cargo nextest list's output")?;
    Ok(listing
        .suites
        .into_iter()
        .flat_map(|(binary, suite)| {
            let package = suite.package_name;
            suite
                .testcases
                .into_iter()
                .filter(|(_, case)| case.filter_match.status == "matches")
                .map(move |(name, _)| (format!("{binary} {name}"), package.clone()))
        })
        .collect())
}

/// The check's errors, then its notes.
pub(super) fn judge(judged: &Judged<'_>) -> (Vec<Finding>, Vec<Finding>) {
    let mut errors = Vec::new();
    let mut notes = Vec::new();
    for member in judged.planned.difference(judged.archived) {
        if judged.runners.contains(member) {
            continue;
        }
        errors.push(Finding::new(
            "members-differ",
            member,
            "the plan runs its Pre-merge suite here, but the build neither archived it nor gave it to the runners",
        ));
    }
    for member in judged.runners.difference(judged.planned) {
        errors.push(Finding::new(
            "members-differ",
            member,
            "the runners ran it, but the plan does not run its Pre-merge suite here",
        ));
    }
    for member in judged.archived.difference(judged.planned) {
        errors.push(Finding::new(
            "members-differ",
            member,
            "the build archived it, but the plan does not run its Pre-merge suite here",
        ));
    }
    for (test, package) in judged.listing {
        if !judged.planned.contains(package) {
            errors.push(Finding::new(
                "not-planned",
                test,
                format!(
                    "the listing selects it, a test of {package}, which the plan does not run here"
                ),
            ));
        }
    }
    let mut ran: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for shard in judged.shards {
        for test in &shard.tests {
            ran.entry(test).or_default().push(&shard.name);
        }
    }
    for test in judged.listing.keys() {
        match ran.get(test.as_str()).map(Vec::as_slice) {
            None | Some([]) => errors.push(Finding::new(
                "not-run",
                test,
                "the listing selects it, and no shard ran it",
            )),
            Some([_]) => {}
            Some(shards) => errors.push(Finding::new(
                "run-twice",
                test,
                format!("{} ran it", shards.join(" and ")),
            )),
        }
    }
    for (test, shards) in &ran {
        if !judged.listing.contains_key(*test) {
            errors.push(Finding::new(
                "not-selected",
                *test,
                format!(
                    "{} ran it, which the listing does not select",
                    shards.join(" and ")
                ),
            ));
        }
    }
    for (snapshot, owner) in judged.snapshots {
        if judged.planned.contains(owner) && !judged.references.contains(snapshot) {
            let finding = Finding::new(
                "unreferenced-snapshot",
                snapshot,
                format!("a snapshot of {owner} that no shard's test read"),
            );
            match judged.unreferenced {
                Unreferenced::Reject => errors.push(finding),
                Unreferenced::Warn => notes.push(finding),
            }
        }
    }
    (errors, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROTOCOL: &str = "goodfetch-protocol";
    const CRAWL: &str = "goodfetch-crawl";
    const TYPESCRIPT: &str = "test-support/typescript";

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn selected() -> BTreeMap<String, String> {
        [
            ("goodfetch-protocol a", PROTOCOL),
            ("goodfetch-protocol b", PROTOCOL),
            ("goodfetch-crawl::it c", CRAWL),
        ]
        .into_iter()
        .map(|(test, package)| (test.to_owned(), package.to_owned()))
        .collect()
    }

    fn shard(name: &str, tests: &[&str]) -> Shard {
        Shard {
            name: name.to_owned(),
            tests: tests.iter().map(|test| (*test).to_owned()).collect(),
        }
    }

    /// A finding's kind and subject.
    type Found = Vec<(String, String)>;

    /// The findings' kinds and subjects, errors then notes.
    fn kinds(
        shards: &[Shard],
        planned: &[&str],
        archived: &[&str],
        references: &[&str],
        unreferenced: Unreferenced,
    ) -> (Found, Found) {
        let listing = selected();
        let snapshots: BTreeMap<String, String> = [(
            "crates/goodfetch-crawl/tests/snapshots/it__c.snap".to_owned(),
            CRAWL.to_owned(),
        )]
        .into_iter()
        .collect();
        let (errors, notes) = judge(&Judged {
            planned: &set(planned),
            archived: &set(archived),
            runners: &BTreeSet::new(),
            listing: &listing,
            shards,
            snapshots: &snapshots,
            references: &set(references),
            unreferenced,
        });
        let pairs = |findings: Vec<Finding>| {
            findings
                .into_iter()
                .map(|finding| (finding.kind.to_owned(), finding.subject))
                .collect()
        };
        (pairs(errors), pairs(notes))
    }

    const READ: &str = "crates/goodfetch-crawl/tests/snapshots/it__c.snap";

    #[test]
    fn shards_that_run_each_selected_test_once_pass() {
        let shards = [
            shard("1", &["goodfetch-protocol a", "goodfetch-crawl::it c"]),
            shard("2", &["goodfetch-protocol b"]),
        ];
        let (errors, notes) = kinds(
            &shards,
            &[PROTOCOL, CRAWL],
            &[PROTOCOL, CRAWL],
            &[READ],
            Unreferenced::Reject,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_selected_test_no_shard_ran_fails_naming_it() {
        let shards = [shard(
            "1",
            &["goodfetch-protocol a", "goodfetch-crawl::it c"],
        )];
        let (errors, _) = kinds(
            &shards,
            &[PROTOCOL, CRAWL],
            &[PROTOCOL, CRAWL],
            &[READ],
            Unreferenced::Reject,
        );
        assert_eq!(
            errors,
            [("not-run".to_owned(), "goodfetch-protocol b".to_owned())]
        );
    }

    #[test]
    fn a_test_two_shards_ran_or_one_the_listing_leaves_out_fails() {
        let shards = [
            shard("1", &["goodfetch-protocol a", "goodfetch-crawl::it c"]),
            shard(
                "2",
                &[
                    "goodfetch-protocol a",
                    "goodfetch-protocol b",
                    "goodfetch-protocol skipped",
                ],
            ),
        ];
        let (errors, _) = kinds(
            &shards,
            &[PROTOCOL, CRAWL],
            &[PROTOCOL, CRAWL],
            &[READ],
            Unreferenced::Reject,
        );
        assert_eq!(
            errors,
            [
                ("run-twice".to_owned(), "goodfetch-protocol a".to_owned()),
                (
                    "not-selected".to_owned(),
                    "goodfetch-protocol skipped".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn an_archive_of_other_members_than_the_plan_runs_fails() {
        let shards = [shard(
            "1",
            &["goodfetch-protocol a", "goodfetch-protocol b"],
        )];
        let (errors, _) = kinds(
            &shards,
            &[PROTOCOL],
            &[PROTOCOL, CRAWL],
            &[],
            Unreferenced::Reject,
        );
        assert_eq!(
            errors,
            [
                ("members-differ".to_owned(), CRAWL.to_owned()),
                ("not-planned".to_owned(), "goodfetch-crawl::it c".to_owned()),
                ("not-run".to_owned(), "goodfetch-crawl::it c".to_owned()),
            ]
        );
    }

    /// A member that is no crate is the runners' (issue 3380): it counts as
    /// run once the build gave it to them, and as missing otherwise.
    #[rstest::rstest]
    #[case::given_to_the_runners(&[TYPESCRIPT], &[])]
    #[case::given_to_no_one(&[], &[TYPESCRIPT])]
    fn a_planned_member_the_runners_run_needs_no_archive(
        #[case] runners: &[&str],
        #[case] missing: &[&str],
    ) {
        let shards = [
            shard("1", &["goodfetch-protocol a", "goodfetch-crawl::it c"]),
            shard("2", &["goodfetch-protocol b"]),
        ];
        let listing = selected();
        let snapshots = BTreeMap::new();
        let (errors, _) = judge(&Judged {
            planned: &set(&[PROTOCOL, CRAWL, TYPESCRIPT]),
            archived: &set(&[PROTOCOL, CRAWL]),
            runners: &set(runners),
            listing: &listing,
            shards: &shards,
            snapshots: &snapshots,
            references: &BTreeSet::new(),
            unreferenced: Unreferenced::Reject,
        });
        let expected: Vec<(String, String)> = missing
            .iter()
            .map(|member| ("members-differ".to_owned(), (*member).to_owned()))
            .collect();
        let found: Vec<(String, String)> = errors
            .into_iter()
            .map(|finding| (finding.kind.to_owned(), finding.subject))
            .collect();
        assert_eq!(found, expected);
    }

    #[rstest::rstest]
    #[case::rejected(Unreferenced::Reject, true)]
    #[case::warned_while_the_quarantine_skips_a_test(Unreferenced::Warn, false)]
    fn a_snapshot_no_shard_read_fails_unless_the_quarantine_skips_a_test(
        #[case] unreferenced: Unreferenced,
        #[case] fails: bool,
    ) {
        let shards = [
            shard("1", &["goodfetch-protocol a", "goodfetch-crawl::it c"]),
            shard("2", &["goodfetch-protocol b"]),
        ];
        let (errors, notes) = kinds(
            &shards,
            &[PROTOCOL, CRAWL],
            &[PROTOCOL, CRAWL],
            &[],
            unreferenced,
        );
        let found = [("unreferenced-snapshot".to_owned(), READ.to_owned())];
        let (expected_errors, expected_notes) = if fails {
            (&found[..], &[][..])
        } else {
            (&[][..], &found[..])
        };
        assert_eq!(errors, expected_errors);
        assert_eq!(notes, expected_notes);
    }

    #[test]
    fn the_listing_keeps_the_tests_that_match_its_filters() {
        let json = r#"{"rust-suites": {"goodfetch-protocol": {"package-name": "goodfetch-protocol",
            "testcases": {
              "a": {"kind": "test", "ignored": false, "filter-match": {"status": "matches"}},
              "skipped": {"kind": "test", "ignored": false, "filter-match": {"status": "mismatch", "reason": "string"}}
            }}}}"#;
        let listed = listing(json).expect("a listing");
        assert_eq!(
            listed.into_iter().collect::<Vec<_>>(),
            [("goodfetch-protocol a".to_owned(), PROTOCOL.to_owned())]
        );
    }
}
