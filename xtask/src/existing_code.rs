//! An issue's `Existing code` list, checked against `main`, and the sweep
//! that sets aside a `ready-for-agent` issue whose evidence moved (ADR 0144;
//! docs/agents/triage-labels.md). The check proves that each cited path,
//! symbol and line is on `main` as it was read; it does not prove a statement
//! true. The sweep runs with the merge gate's backstop, and `just
//! existing-code <issue>` checks one issue on demand.

mod evidence;
mod list;

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use jiff::Timestamp;
use jiff::civil::Date;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::github::{self, Api, List, Write};
use crate::issues::Label;
use crate::{git, output};
use evidence::{Broken, Holds, Repository};
use list::Citation;

/// `xtask existing-code`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Checks one issue's `Existing code` list against `main`: fails on a
    /// spec with no list, a list with no known read commit on `main`, an
    /// item that cites nothing, and a cited path, symbol or line that moved.
    Check {
        /// The issue's number.
        issue: u64,
        #[command(flatten)]
        main: MainArguments,
        #[command(flatten)]
        github: github::Arguments,
    },
    /// Checks every open `ready-for-agent` issue, but those whose spec's run
    /// has started (a commit on `spec/<n>`), and sets aside each that fails:
    /// one comment with what moved, `ready-for-agent` removed and
    /// `needs-triage` applied. Without `--apply` it only says what it would
    /// do.
    Sweep {
        /// Writes the comments and labels; without it, nothing is written.
        #[arg(long)]
        apply: bool,
        #[command(flatten)]
        main: MainArguments,
        #[command(flatten)]
        github: github::Arguments,
    },
}

/// Where `main` and the specs' branches are read.
#[derive(clap::Args)]
pub(crate) struct MainArguments {
    /// The remote whose `main` the lists are checked against and whose
    /// `spec/<n>` branches, beside the local ones, show a run started.
    #[arg(long, default_value = "origin")]
    remote: String,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    match command {
        Command::Check {
            issue,
            main,
            github,
        } => {
            let api = api(&github, environment)?;
            let issue: Issue = api.get(&format!("/repos/{}/issues/{issue}", github.repository))?;
            if issue.pull_request.is_some() {
                bail!("#{} is a pull request, not an issue", issue.number);
            }
            let checker = Checker::new(root, &main.remote)?;
            let problems = checker.check(&issue)?;
            let (moved, failed): (Vec<_>, Vec<_>) =
                problems.iter().partition(|problem| problem.only_moved);
            check::note(
                &moved
                    .iter()
                    .map(|problem| problem.finding(issue.number))
                    .collect::<Vec<_>>(),
            );
            Ok(check::report(
                &failed
                    .iter()
                    .map(|problem| problem.finding(issue.number))
                    .collect::<Vec<_>>(),
            ))
        }
        Command::Sweep {
            apply,
            main,
            github,
        } => {
            let api = api(&github, environment)?;
            sweep(
                &Checker::new(root, &main.remote)?,
                &api,
                &github.repository,
                apply,
            )?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// The API, with the token the issues are read with: without one, a private
/// repository answers 404 for every issue.
fn api<'a>(github: &'a github::Arguments, environment: &'a Environment) -> Result<Api<'a>> {
    if environment.token().is_none() {
        bail!(
            "no GitHub token to read the issues of {} with: set GITHUB_TOKEN, or log the GitHub CLI in (`gh auth login`)",
            github.repository
        );
    }
    Ok(github.api(environment.token()))
}

/// An issue as GitHub's REST interface answers it.
#[derive(Deserialize)]
struct Issue {
    number: u64,
    title: String,
    body: Option<String>,
    created_at: Timestamp,
    #[serde(default)]
    labels: Vec<NamedLabel>,
    sub_issues_summary: Option<SubIssues>,
    parent_issue_url: Option<String>,
    /// Present when the "issue" is a pull request.
    pull_request: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct NamedLabel {
    name: String,
}

#[derive(Deserialize)]
struct SubIssues {
    total: u64,
}

#[derive(Deserialize)]
struct Comment {
    body: Option<String>,
}

/// From this day, a spec carries its list from its publication (ADR 0144).
const LISTS_FROM: Date = jiff::civil::date(2026, 11, 10);

/// The file whose specs carry a list before their runs start.
const ROLLOUT_ORDER: &str = "docs/agents/rollout-order.md";

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
    remote: &'a str,
    /// `docs/agents/rollout-order.md` on `main`, empty when it has none.
    rollout_order: String,
}

/// One thing a list fails on, or a cited line that only moved.
struct Problem {
    kind: &'static str,
    /// The item it is about, from 1, and the citation, when it has one.
    item: Option<(usize, String, Option<String>)>,
    says: String,
    /// A cited line that moved and holds: a note, no failure.
    only_moved: bool,
}

impl Problem {
    fn of_issue(kind: &'static str, says: String) -> Self {
        Self {
            kind,
            item: None,
            says,
            only_moved: false,
        }
    }

    fn finding(&self, issue: u64) -> Finding {
        match &self.item {
            None => Finding::new(self.kind, format!("#{issue}"), self.says.clone()),
            Some((index, statement, citation)) => Finding::new(
                self.kind,
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
            Some((_, statement, Some(citation))) => {
                format!("- {statement}\n  `{citation}`: {}", self.says)
            }
            Some((_, statement, None)) => format!("- {statement}\n  {}", self.says),
        }
    }
}

impl<'a> Checker<'a> {
    fn new(root: &'a Path, remote: &'a str) -> Result<Self> {
        let main = git::commit(root, &format!("{remote}/main")).with_context(|| {
            format!(
                "reading {remote}/main, the `main` the lists are checked against: fetch it first"
            )
        })?;
        let repository = Repository { root, main };
        let rollout_order = repository
            .file(&repository.main, ROLLOUT_ORDER)?
            .unwrap_or_default();
        Ok(Self {
            repository,
            remote,
            rollout_order,
        })
    }

    /// Whether the run of spec `spec` has started: its branch, local or the
    /// remote's, holds a commit `main` does not.
    fn started(&self, spec: u64) -> Result<bool> {
        let root = self.repository.root;
        for branch in [
            format!("refs/heads/spec/{spec}"),
            format!("refs/remotes/{}/spec/{spec}", self.remote),
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
                    "existing-code-missing-list",
                    format!(
                        "this spec has no `Existing code` list, which a spec published on or after {LISTS_FROM} or listed in {ROLLOUT_ORDER} carries before its run starts (ADR 0144)"
                    ),
                )]
            } else {
                Vec::new()
            });
        };
        let Some(read) = list.read_at else {
            return Ok(vec![Problem::of_issue(
                "existing-code-missing-read-commit",
                "the `Existing code` list's first line is not \"Read at `<commit>`.\", with a commit of at least 7 hexadecimal digits".to_owned(),
            )]);
        };
        let root = self.repository.root;
        let Ok(read) = git::commit(root, &read) else {
            return Ok(vec![Problem::of_issue(
                "existing-code-unknown-read-commit",
                format!("the list's read commit {read} is no commit this repository knows"),
            )]);
        };
        if !git::succeeds(
            root,
            &["merge-base", "--is-ancestor", &read, &self.repository.main],
        )? {
            return Ok(vec![Problem::of_issue(
                "existing-code-read-commit-off-main",
                format!(
                    "the list's read commit {} is not on {}/main",
                    &read[..9],
                    self.remote
                ),
            )]);
        }
        let mut problems = Vec::new();
        for (index, item) in (1..).zip(&list.items) {
            let mut cited = false;
            for quoted in &item.quoted {
                let Some(citation) = self.citation(quoted, &read)? else {
                    continue;
                };
                cited = true;
                if let Some((kind, says, only_moved)) = self.judge(&citation, &read)? {
                    problems.push(Problem {
                        kind,
                        item: Some((index, item.statement.clone(), Some(citation.written))),
                        says,
                        only_moved,
                    });
                }
            }
            if !cited && !item.says_nothing_exists() {
                problems.push(Problem {
                    kind: "existing-code-no-citation",
                    item: Some((index, item.statement.clone(), None)),
                    says: "the item cites no path of the repository".to_owned(),
                    only_moved: false,
                });
            }
        }
        Ok(problems)
    }

    /// Whether a spec must carry a list: published on or after
    /// [`LISTS_FROM`] or listed in `rollout-order.md`, its run not started.
    fn needs_list(&self, issue: &Issue) -> Result<bool> {
        let published = issue.created_at.to_zoned(jiff::tz::TimeZone::UTC).date();
        Ok(issue.is_spec()
            && (published >= LISTS_FROM || self.rollout_order_lists(issue.number))
            && !self.started(issue.number)?)
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
    fn judge(
        &self,
        citation: &Citation,
        read: &str,
    ) -> Result<Option<(&'static str, String, bool)>> {
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
                    "existing-code-moved",
                    format!("the cited lines only moved: they are {path}:{lines} on main"),
                    true,
                )));
            }
            Err(broken) => broken,
        };
        if broken == Broken::UnknownLines {
            return Ok(Some((
                "existing-code-unknown-lines",
                format!("{path} has no such lines at the read commit {}", &read[..9]),
                false,
            )));
        }
        let since = match repository.moved_by(citation, read)? {
            Some(merge) => format!("first in {merge}"),
            None => format!("as at the read commit {}", &read[..9]),
        };
        let (kind, what) = match broken {
            Broken::MissingPath => ("existing-code-missing-path", format!("{path} is gone")),
            Broken::MissingSymbol => (
                "existing-code-missing-symbol",
                format!(
                    "{path} no longer names `{}`",
                    citation.symbol.as_deref().unwrap_or_default()
                ),
            ),
            Broken::ChangedLines | Broken::UnknownLines => (
                "existing-code-changed-lines",
                "the cited lines changed".to_owned(),
            ),
        };
        Ok(Some((
            kind,
            format!("the evidence moved: {what}, {since}"),
            false,
        )))
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

/// The issues the sweep sets aside, each with its comment.
struct Aside<'a> {
    issue: &'a Issue,
    comment: String,
    /// Whether the issue already holds that comment, from an earlier sweep
    /// that failed before its labels moved.
    commented: bool,
}

/// Sets aside every open `ready-for-agent` issue whose list fails, but those
/// whose spec's run has started. Every read comes before the first write, so a
/// tracker that fails a read moves no label; each issue's comment comes before
/// its labels, which move in one write, so a sweep that fails between them
/// leaves the issue listed, and the next one finds its comment.
fn sweep(checker: &Checker<'_>, api: &Api<'_>, repository: &str, apply: bool) -> Result<()> {
    let issues: Vec<Issue> = api
        .get_all(
            &format!("/repos/{repository}/issues?labels={READY_FOR_AGENT}&state=open"),
            List::Array,
        )
        .context("listing the open ready-for-agent issues")?;
    let mut examined = 0;
    let mut aside = Vec::new();
    for issue in issues.iter().filter(|issue| issue.pull_request.is_none()) {
        if let Some(spec) = issue.spec()
            && checker.started(spec)?
        {
            output::line(&format!(
                "skipped #{}: the run of spec #{spec} has started, and its implementers read the code itself",
                issue.number
            ));
            continue;
        }
        examined += 1;
        let problems = checker.check(issue)?;
        let failed: Vec<&Problem> = problems
            .iter()
            .filter(|problem| !problem.only_moved)
            .collect();
        if failed.is_empty() {
            continue;
        }
        check::note(
            &failed
                .iter()
                .map(|problem| problem.finding(issue.number))
                .collect::<Vec<_>>(),
        );
        aside.push(Aside {
            issue,
            comment: comment(&failed),
            commented: false,
        });
    }
    if apply {
        for aside in &mut aside {
            let comments: Vec<Comment> = api.get_all(
                &format!("/repos/{repository}/issues/{}/comments", aside.issue.number),
                List::Array,
            )?;
            aside.commented = comments
                .iter()
                .any(|comment| comment.body.as_deref() == Some(aside.comment.as_str()));
        }
        for aside in &aside {
            set_aside(api, repository, aside)?;
            output::line(&format!("set aside #{}", aside.issue.number));
        }
    } else {
        for Aside { issue, .. } in &aside {
            output::line(&format!(
                "would set aside #{}: a dry run, which `--apply` makes",
                issue.number
            ));
        }
    }
    output::line(&format!(
        "checked {examined} ready-for-agent issues: {} {}",
        aside.len(),
        if apply { "set aside" } else { "to set aside" }
    ));
    Ok(())
}

const READY_FOR_AGENT: &str = "ready-for-agent";

/// Comments on the issue, unless it holds the comment already, then moves its
/// labels in one write: `ready-for-agent` out, `needs-triage` in.
fn set_aside(api: &Api<'_>, repository: &str, aside: &Aside<'_>) -> Result<()> {
    let issue = format!("/repos/{repository}/issues/{}", aside.issue.number);
    if !aside.commented {
        let _: Value = api.send(
            Write::Post,
            &format!("{issue}/comments"),
            &json!({ "body": aside.comment }),
        )?;
    }
    let triage = Label::NeedsTriage.name();
    let mut labels: Vec<&str> = aside
        .issue
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .filter(|name| *name != READY_FOR_AGENT)
        .collect();
    if !labels.contains(&triage) {
        labels.push(triage);
    }
    let _: Value = api.send(Write::Patch, &issue, &json!({ "labels": labels }))?;
    Ok(())
}

/// The sweep's comment: what moved, item by item, and how the label comes
/// back. It names no commit of `main`, so a later sweep writes the same text.
fn comment(problems: &[&Problem]) -> String {
    let lines: Vec<String> = problems
        .iter()
        .map(|problem| problem.comment_lines())
        .collect();
    format!(
        "The `Existing code` list of this issue no longer holds on `main`: the evidence moved (ADR 0144). The check reads where each statement was read, not whether it still holds; that is for whoever reads the code again.\n\n{}\n\nThe sweep took `{READY_FOR_AGENT}` and applied `{}`. Read the cited code again, correct the list and its \"Read at\" commit, and put `{READY_FOR_AGENT}` back (`triage`, \"Verify the claim\").",
        lines.join("\n"),
        Label::NeedsTriage.name()
    )
}
