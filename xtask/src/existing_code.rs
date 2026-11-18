//! An issue's `Existing code` list, checked against `main` (ADR 0144;
//! docs/agents/triage-labels.md): each cited path, symbol and line is on
//! `main` as it was read. The check does not prove a statement true.
//! `just existing-code <issue>` checks one issue; the sweep checks every
//! `ready-for-agent` issue.

mod evidence;
mod list;
mod sweep;

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use jiff::Timestamp;
use jiff::civil::Date;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::git;
use crate::github;
use evidence::{Broken, Holds, Repository};
use list::Citation;

/// `xtask existing-code`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Checks one issue's `Existing code` list against `origin/main`: fails
    /// on a spec with no list, a list with no known read commit on `main`, an
    /// item that cites nothing, and a cited path, symbol or line that moved.
    Check {
        /// The issue's number.
        issue: u64,
        #[command(flatten)]
        github: github::Arguments,
    },
    /// Checks every open `ready-for-agent` issue, but those whose spec's run
    /// has started (a commit on `spec/<n>`), and sets aside each that fails:
    /// one comment with what moved, `needs-triage` applied and
    /// `ready-for-agent` removed. Without `--apply` it only says what it
    /// would do.
    Sweep {
        /// Writes the comments and labels; without it, nothing is written.
        #[arg(long)]
        apply: bool,
        #[command(flatten)]
        github: github::Arguments,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    match command {
        Command::Check { issue, github } => {
            let api = github.authenticated(environment.token())?;
            let issue: Issue = api.get(&format!("/repos/{}/issues/{issue}", github.repository))?;
            if issue.pull_request.is_some() {
                bail!("#{} is a pull request, not an issue", issue.number);
            }
            let problems = Checker::new(root)?.check(&issue)?;
            let (moved, failed): (Vec<_>, Vec<_>) =
                problems.iter().partition(|problem| problem.only_moved());
            let findings = |problems: Vec<&Problem>| -> Vec<Finding> {
                problems
                    .iter()
                    .map(|problem| problem.finding(issue.number))
                    .collect()
            };
            check::note(&findings(moved));
            Ok(check::report(&findings(failed)))
        }
        Command::Sweep { apply, github } => {
            let api = github.authenticated(environment.token())?;
            sweep::sweep(&Checker::new(root)?, &api, &github.repository, apply)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// An issue as GitHub's REST interface answers it.
#[derive(Deserialize)]
struct Issue {
    number: u64,
    title: String,
    body: Option<String>,
    created_at: Timestamp,
    sub_issues_summary: Option<SubIssues>,
    parent_issue_url: Option<String>,
    /// Present when the "issue" is a pull request.
    pull_request: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct SubIssues {
    total: u64,
}

/// From this day, a spec carries its list from its publication (ADR 0144).
const LISTS_FROM: Date = jiff::civil::date(2026, 11, 10);

/// The file whose specs carry a list before their runs start.
const ROLLOUT_ORDER: &str = "docs/agents/rollout-order.md";

/// The remote whose `main` the lists are checked against and whose
/// `spec/<n>` branches, beside the local ones, show a run started.
const REMOTE: &str = "origin";

impl Issue {
    /// An issue with sub-issues, or one whose title starts with "Spec:".
    fn is_spec(&self) -> bool {
        self.title.starts_with("Spec:")
            || self
                .sub_issues_summary
                .as_ref()
                .is_some_and(|summary| summary.total > 0)
    }

    /// The spec it belongs to: itself, or its parent.
    fn spec(&self) -> Option<u64> {
        if self.is_spec() {
            return Some(self.number);
        }
        self.parent_issue_url
            .as_deref()
            .and_then(|url| url.rsplit('/').next())
            .and_then(|number| number.parse().ok())
    }
}

/// Checks issues against one commit of `main`.
struct Checker<'a> {
    repository: Repository<'a>,
    /// `docs/agents/rollout-order.md` on `main`, empty when it has none.
    rollout_order: String,
}

/// What a list fails on, or the one note it may get.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    MissingList,
    MissingReadCommit,
    UnknownReadCommit,
    ReadCommitOffMain,
    NoCitation,
    MissingPath,
    MissingSymbol,
    UnknownLines,
    ChangedLines,
    /// Cited lines that moved and hold: a note, no failure.
    Moved,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::MissingList => "existing-code-missing-list",
            Self::MissingReadCommit => "existing-code-missing-read-commit",
            Self::UnknownReadCommit => "existing-code-unknown-read-commit",
            Self::ReadCommitOffMain => "existing-code-read-commit-off-main",
            Self::NoCitation => "existing-code-no-citation",
            Self::MissingPath => "existing-code-missing-path",
            Self::MissingSymbol => "existing-code-missing-symbol",
            Self::UnknownLines => "existing-code-unknown-lines",
            Self::ChangedLines => "existing-code-changed-lines",
            Self::Moved => "existing-code-moved",
        }
    }
}

/// One thing a list fails on, or cited lines that only moved.
struct Problem {
    kind: Kind,
    /// The item it is about, when it is about one.
    item: Option<About>,
    says: String,
}

/// The item a problem is about.
struct About {
    /// From 1.
    index: usize,
    statement: String,
    /// The citation, as written, when the problem is one citation's.
    citation: Option<String>,
}

impl Problem {
    fn of_issue(kind: Kind, says: String) -> Self {
        Self {
            kind,
            item: None,
            says,
        }
    }

    fn only_moved(&self) -> bool {
        self.kind == Kind::Moved
    }

    fn finding(&self, issue: u64) -> Finding {
        let kind = self.kind.name();
        match &self.item {
            None => Finding::new(kind, format!("#{issue}"), self.says.clone()),
            Some(About {
                index,
                statement,
                citation,
            }) => Finding::new(
                kind,
                match citation {
                    Some(citation) => format!("#{issue} {citation}"),
                    None => format!("#{issue} item {index}"),
                },
                format!("{}; item {index}: {statement}", self.says),
            ),
        }
    }

    /// Its lines in the sweep's comment.
    fn comment_lines(&self) -> String {
        match &self.item {
            None => format!("- {}", self.says),
            Some(About {
                statement,
                citation: Some(citation),
                ..
            }) => format!("- {statement}\n  `{citation}`: {}", self.says),
            Some(About {
                statement,
                citation: None,
                ..
            }) => format!("- {statement}\n  {}", self.says),
        }
    }
}

impl<'a> Checker<'a> {
    fn new(root: &'a Path) -> Result<Self> {
        let main = git::commit(root, &format!("{REMOTE}/main")).with_context(|| {
            format!(
                "reading {REMOTE}/main, the `main` the lists are checked against: fetch it first"
            )
        })?;
        let repository = Repository { root, main };
        let rollout_order = repository
            .file(&repository.main, ROLLOUT_ORDER)?
            .unwrap_or_default();
        Ok(Self {
            repository,
            rollout_order,
        })
    }

    /// Whether the run of spec `spec` has started: its branch, local or the
    /// remote's, holds a commit `main` does not.
    fn started(&self, spec: u64) -> Result<bool> {
        let root = self.repository.root;
        for branch in [
            format!("refs/heads/spec/{spec}"),
            format!("refs/remotes/{REMOTE}/spec/{spec}"),
        ] {
            if git::succeeds(root, &["rev-parse", "--verify", "--quiet", &branch])? {
                let ahead = git::line(
                    root,
                    &[
                        "rev-list",
                        "--count",
                        &format!("{}..{branch}", self.repository.main),
                    ],
                )?;
                if ahead != "0" {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Whether `docs/agents/rollout-order.md` lists spec `number`: it links
    /// to it under its title, `[Spec: …](…/issues/<number>)`, or opens a
    /// numbered step with a link to it, `N. **[…](…/issues/<number>)**`. An
    /// issue the text only mentions, `#<number>` or `[#<number>](…)`, is not
    /// listed.
    fn rollout_order_lists(&self, number: u64) -> bool {
        let target = format!("/issues/{number}");
        self.rollout_order.lines().any(|line| {
            let opens_a_step = line.split_once(". **[").is_some_and(|(step, _)| {
                !step.is_empty() && step.bytes().all(|byte| byte.is_ascii_digit())
            });
            links(line).enumerate().any(|(index, (text, url))| {
                url.ends_with(&target) && (text.starts_with("Spec:") || opens_a_step && index == 0)
            })
        })
    }

    /// What `issue`'s list fails on, and the cited lines that only moved.
    fn check(&self, issue: &Issue) -> Result<Vec<Problem>> {
        let Some(list) = list::find(issue.body.as_deref().unwrap_or_default()) else {
            return Ok(if self.needs_list(issue)? {
                vec![Problem::of_issue(
                    Kind::MissingList,
                    format!(
                        "this spec has no `Existing code` list, which a spec published on or after {LISTS_FROM} carries, and one listed in {ROLLOUT_ORDER} carries before its run starts (ADR 0144)"
                    ),
                )]
            } else {
                Vec::new()
            });
        };
        let Some(read) = list.read_at else {
            return Ok(vec![Problem::of_issue(
                Kind::MissingReadCommit,
                "the `Existing code` list's first line is not \"Read at `<commit>`.\", with a commit of at least 7 hexadecimal digits".to_owned(),
            )]);
        };
        let root = self.repository.root;
        let read = match git::commit(root, &read) {
            Ok(read) => read,
            Err(git::Error::Refused { .. }) => {
                return Ok(vec![Problem::of_issue(
                    Kind::UnknownReadCommit,
                    format!("the list's read commit {read} is no commit this repository knows"),
                )]);
            }
            Err(error) => return Err(error.into()),
        };
        if !git::succeeds(
            root,
            &["merge-base", "--is-ancestor", &read, &self.repository.main],
        )? {
            return Ok(vec![Problem::of_issue(
                Kind::ReadCommitOffMain,
                format!(
                    "the list's read commit {} is not on {REMOTE}/main",
                    &read[..9]
                ),
            )]);
        }
        let mut problems = Vec::new();
        for (index, item) in (1..).zip(&list.items) {
            let about = |citation| {
                Some(About {
                    index,
                    statement: item.statement.clone(),
                    citation,
                })
            };
            let mut cited = false;
            for quoted in &item.quoted {
                let Some(citation) = self.citation(quoted, &read)? else {
                    continue;
                };
                cited = true;
                if let Some((kind, says)) = self.judge(&citation, &read)? {
                    problems.push(Problem {
                        kind,
                        item: about(Some(citation.written)),
                        says,
                    });
                }
            }
            if !cited && !item.says_nothing_exists() {
                problems.push(Problem {
                    kind: Kind::NoCitation,
                    item: about(None),
                    says: "the item cites no path of the repository".to_owned(),
                });
            }
        }
        Ok(problems)
    }

    /// Whether a spec must carry a list: one published on or after
    /// [`LISTS_FROM`] always does, and one listed in `rollout-order.md` does
    /// until its run starts.
    fn needs_list(&self, issue: &Issue) -> Result<bool> {
        if !issue.is_spec() {
            return Ok(false);
        }
        let published = issue.created_at.to_zoned(jiff::tz::TimeZone::UTC).date();
        Ok(published >= LISTS_FROM
            || self.rollout_order_lists(issue.number) && !self.started(issue.number)?)
    }

    /// `quoted` as a citation: of the citation's form, and its path below a
    /// root file or top folder the repository has at `read` or on `main`. A
    /// path below one that is gone is still a citation, and fails.
    fn citation(&self, quoted: &str, read: &str) -> Result<Option<Citation>> {
        let Some(citation) = Citation::parse(quoted) else {
            return Ok(None);
        };
        let repository = &self.repository;
        // A lone word, `console`, is a citation only as a root file: a root
        // folder is cited as `console/`, so an event or a setting that shares
        // a folder's name is not one.
        let lone_word = !citation.path.contains('/')
            && citation.lines.is_none()
            && citation.symbol.is_none()
            && !citation.written.ends_with('/');
        let ours = if lone_word {
            repository.file(read, &citation.path)?.is_some()
                || repository.file(&repository.main, &citation.path)?.is_some()
        } else {
            repository.exists(read, citation.top())?
                || repository.exists(&repository.main, citation.top())?
        };
        Ok(ours.then_some(citation))
    }

    /// What `citation` fails on, or the note of lines that only moved; `None`
    /// when it holds as read.
    fn judge(&self, citation: &Citation, read: &str) -> Result<Option<(Kind, String)>> {
        let repository = &self.repository;
        let path = &citation.path;
        let broken = match repository.holds(citation, read, &repository.main)? {
            Ok(Holds::Unchanged) => return Ok(None),
            Ok(Holds::Moved { first, last }) => {
                let lines = if first == last {
                    first.to_string()
                } else {
                    format!("{first}-{last}")
                };
                return Ok(Some((
                    Kind::Moved,
                    format!("the cited lines only moved: they are {path}:{lines} on main"),
                )));
            }
            Err(broken) => broken,
        };
        let (kind, what) = match broken {
            Broken::UnknownLines => {
                return Ok(Some((
                    Kind::UnknownLines,
                    format!("{path} has no such lines at the read commit {}", &read[..9]),
                )));
            }
            Broken::MissingPath => (Kind::MissingPath, format!("{path} is gone")),
            Broken::MissingSymbol => (
                Kind::MissingSymbol,
                format!(
                    "{path} no longer names `{}`",
                    citation.symbol.as_deref().unwrap_or_default()
                ),
            ),
            Broken::ChangedLines => (Kind::ChangedLines, "the cited lines changed".to_owned()),
        };
        let since = match repository.moved_by(citation, read)? {
            Some(merge) => format!("first in {merge}"),
            None => format!("as at the read commit {}", &read[..9]),
        };
        Ok(Some((kind, format!("the evidence moved: {what}, {since}"))))
    }
}

/// The Markdown links of `line`, as their text and target.
fn links(line: &str) -> impl Iterator<Item = (&str, &str)> {
    line.split('[').skip(1).filter_map(|after| {
        let (text, rest) = after.split_once("](")?;
        let (url, _) = rest.split_once(')')?;
        Some((text, url))
    })
}
