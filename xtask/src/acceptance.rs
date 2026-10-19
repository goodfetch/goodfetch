//! A pull request's acceptance of a tripped budget (ADR 0057's rule, as
//! amended by ADR 0133; [owner's answer](https://github.com/goodfetch/goodfetch/issues/130#issuecomment-6143249424)),
//! which `xtask size` ("Size increase") and `xtask perf` ("Performance
//! change") share. A tripped item passes only when
//!
//! - the body's section of the rule's heading (to the next heading of the
//!   same or a higher level) has a line naming the item's identifier, as the
//!   check's finding prints it, followed by a change at least the measured
//!   one, in the check's own unit;
//! - and a comment by someone with write access (owner, member or
//!   collaborator) says the rule's phrase and names the pull request's head
//!   SHA, in full or abbreviated to at least 7 characters.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::github::{Api, List};

pub(crate) struct PullRequest<'a> {
    pub(crate) api: Api<'a>,
    /// `owner/name`.
    pub(crate) repository: &'a str,
    pub(crate) number: u64,
}

/// Which section and which comment accept a check's tripped items.
pub(crate) struct Rule {
    /// The section's heading, compared without case.
    pub(crate) heading: &'static str,
    /// What the accepting comment says, compared without case.
    pub(crate) phrase: &'static str,
}

/// What the pull request accepts. It is saved as JSON when a job reads it
/// before denying outbound traffic (`xtask perf pull-request`).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Acceptance {
    /// The section's lines.
    section: Vec<String>,
    /// Whether a comment accepts the section on the pull request's head.
    accepted_on_head: bool,
}

/// The comment authors GitHub reports as having write access.
const WRITERS: [&str; 3] = ["OWNER", "MEMBER", "COLLABORATOR"];
/// Git's shortest abbreviation of an object name.
const SHORTEST_SHA: usize = 7;
const FULL_SHA: usize = 40;

#[derive(Deserialize)]
struct PullRequestBody {
    body: Option<String>,
    head: Head,
}

#[derive(Deserialize)]
struct Head {
    sha: String,
}

#[derive(Deserialize)]
struct Comment {
    body: Option<String>,
    author_association: String,
}

impl PullRequest<'_> {
    /// Reads the body, the head and every comment through GitHub's interface.
    pub(crate) fn read(&self, rule: &Rule) -> Result<Acceptance> {
        let pull_request: PullRequestBody = self
            .api
            .get(&format!("/repos/{}/pulls/{}", self.repository, self.number))
            .with_context(|| format!("reading pull request #{}", self.number))?;
        let comments: Vec<Comment> = self
            .api
            .get_all(
                &format!("/repos/{}/issues/{}/comments", self.repository, self.number),
                List::Array,
            )
            .with_context(|| format!("reading the comments of #{}", self.number))?;
        let accepted_on_head = comments
            .iter()
            .any(|comment| accepts(comment, rule, &pull_request.head.sha));
        Ok(Acceptance {
            section: section(
                pull_request.body.as_deref().unwrap_or_default(),
                rule.heading,
            ),
            accepted_on_head,
        })
    }
}

#[derive(Deserialize)]
struct CommitPull {
    number: u64,
    merge_commit_sha: Option<String>,
}

/// The pull request that merged `commit` into its branch, when one did: the
/// commit's pull request whose merge commit it is.
pub(crate) fn merged_by(api: &Api<'_>, repository: &str, commit: &str) -> Result<Option<u64>> {
    let pulls: Vec<CommitPull> = api
        .get(&format!("/repos/{repository}/commits/{commit}/pulls"))
        .with_context(|| format!("reading the pull requests of {commit}"))?;
    Ok(pulls
        .into_iter()
        .find(|pull| pull.merge_commit_sha.as_deref() == Some(commit))
        .map(|pull| pull.number))
}

impl Acceptance {
    /// What a change no pull request merged accepts: nothing.
    pub(crate) fn nothing() -> Self {
        Self {
            section: Vec::new(),
            accepted_on_head: false,
        }
    }

    /// Whether the item `id`, grown by `change`, is accepted: `listed`
    /// reads a change from the start of a text, and the first `+` after the
    /// identifier on a line of the section that gives one counts.
    pub(crate) fn accepts<T: PartialOrd>(
        &self,
        id: &str,
        change: &T,
        listed: impl Fn(&str) -> Option<T>,
    ) -> bool {
        self.accepted_on_head
            && self
                .section
                .iter()
                .filter_map(|line| after_identifier(line, id))
                .filter_map(|rest| {
                    rest.match_indices('+')
                        .find_map(|(plus, _)| listed(&rest[plus + 1..]))
                })
                .any(|delta| delta >= *change)
    }
}

fn accepts(comment: &Comment, rule: &Rule, head: &str) -> bool {
    let body = comment.body.as_deref().unwrap_or_default();
    WRITERS.contains(&comment.author_association.as_str())
        && body
            .to_ascii_lowercase()
            .contains(&rule.phrase.to_ascii_lowercase())
        && body
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|word| names_head(word, head))
}

fn names_head(word: &str, head: &str) -> bool {
    (SHORTEST_SHA..=FULL_SHA).contains(&word.len())
        && word.chars().all(|character| character.is_ascii_hexdigit())
        && head.starts_with(&word.to_ascii_lowercase())
}

/// The lines under the heading `wanted`.
fn section(body: &str, wanted: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut level = None;
    for line in body.lines() {
        if let Some((heading_level, text)) = heading(line) {
            match level {
                Some(open) if heading_level <= open => break,
                None if text.eq_ignore_ascii_case(wanted) => {
                    level = Some(heading_level);
                    continue;
                }
                _ => {}
            }
        }
        if level.is_some() {
            lines.push(line.to_owned());
        }
    }
    lines
}

/// A Markdown ATX heading's level and text.
fn heading(line: &str) -> Option<(usize, &str)> {
    let line = line.trim_start();
    let level = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    let rest = &line[level..];
    ((1..=6).contains(&level) && (rest.is_empty() || rest.starts_with(' ')))
        .then(|| (level, rest.trim().trim_end_matches('#').trim()))
}

/// The rest of `line` after the identifier `id`, when the line names it
/// whole: no identifier character touches it on either side.
fn after_identifier<'a>(line: &'a str, id: &str) -> Option<&'a str> {
    let lower = line.to_ascii_lowercase();
    let id = id.to_ascii_lowercase();
    let start = lower
        .match_indices(&id)
        .map(|(start, _)| start)
        .find(|start| {
            let before = lower[..*start].chars().next_back();
            let after = lower[start + id.len()..].chars().next();
            !before.is_some_and(is_identifier_character)
                && !after.is_some_and(is_identifier_character)
        })?;
    Some(&line[start + id.len()..])
}

fn is_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/')
}

/// A decimal number of at most `decimals` decimals at the start of `text`,
/// followed by `unit` (after optional spaces), in units of
/// `10^-decimals`: `1.10 MB` with 6 decimals is 1,100,000.
pub(crate) fn decimal(text: &str, unit: &str, decimals: usize) -> Option<u64> {
    let number_length = text
        .find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .unwrap_or(text.len());
    let (number, rest) = text.split_at(number_length);
    if !rest.trim_start().starts_with(unit) {
        return None;
    }
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() || fraction.len() > decimals {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<decimals$}").parse().ok()?
    };
    whole
        .checked_mul(10_u64.checked_pow(u32::try_from(decimals).ok()?)?)
        .and_then(|units| units.checked_add(fraction))
}
