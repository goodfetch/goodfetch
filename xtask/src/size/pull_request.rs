//! A pull request's acceptance of a size increase (ADR 0057, as amended by
//! ADR 0133; [owner's answer](https://github.com/goodfetch/goodfetch/issues/130#issuecomment-6143249424)):
//! a file past its increase threshold passes only when
//!
//! - the body's "Size increase" section (a heading of that name, to the next
//!   heading of the same or a higher level) has a line naming the file's
//!   identifier, as the table's finding prints it, with a delta in MB at
//!   least the measured one: `- collie/x86_64-unknown-linux-gnu/<file>: +1.10 MB, <reason>`;
//! - and a comment by someone with write access (owner, member or
//!   collaborator) says "Size increase accepted" and names the pull
//!   request's head SHA, in full or abbreviated to at least 7 characters.

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::github::Api;

use super::budget::Bytes;

pub(crate) struct PullRequest<'a> {
    pub(crate) api: Api<'a>,
    /// `owner/name`.
    pub(crate) repository: &'a str,
    pub(crate) number: u64,
}

/// What the pull request accepts.
pub(crate) struct Acceptance {
    /// The "Size increase" section's lines.
    section: Vec<String>,
    /// Whether a comment accepts the increase on the pull request's head.
    accepted_on_head: bool,
}

const SECTION_HEADING: &str = "size increase";
const ACCEPTANCE: &str = "size increase accepted";
/// The comment authors GitHub reports as having write access.
const WRITERS: [&str; 3] = ["OWNER", "MEMBER", "COLLABORATOR"];
/// Git's shortest abbreviation of an object name.
const SHORTEST_SHA: usize = 7;
const FULL_SHA: usize = 40;
/// GitHub's largest page of comments.
const PAGE_SIZE: usize = 100;
const MB: u64 = 1_000_000;

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
    pub(crate) fn read(&self) -> Result<Acceptance> {
        let pull_request: PullRequestBody = self
            .api
            .get(&format!("/repos/{}/pulls/{}", self.repository, self.number))
            .with_context(|| format!("reading pull request #{}", self.number))?;
        let mut accepted_on_head = false;
        for page in 1.. {
            let comments: Vec<Comment> = self
                .api
                .get(&format!(
                    "/repos/{}/issues/{}/comments?per_page={PAGE_SIZE}&page={page}",
                    self.repository, self.number
                ))
                .with_context(|| format!("reading the comments of #{}", self.number))?;
            accepted_on_head |= comments
                .iter()
                .any(|comment| accepts(comment, &pull_request.head.sha));
            if comments.len() < PAGE_SIZE {
                break;
            }
        }
        Ok(Acceptance {
            section: section(pull_request.body.as_deref().unwrap_or_default()),
            accepted_on_head,
        })
    }
}

impl Acceptance {
    /// Whether the file `id`, grown by `change`, is accepted.
    pub(crate) fn accepts(&self, id: &str, change: Bytes) -> bool {
        self.accepted_on_head
            && self
                .section
                .iter()
                .filter_map(|line| listed_delta(line, id))
                .any(|delta| delta >= change)
    }
}

fn accepts(comment: &Comment, head: &str) -> bool {
    let body = comment.body.as_deref().unwrap_or_default();
    WRITERS.contains(&comment.author_association.as_str())
        && body.to_ascii_lowercase().contains(ACCEPTANCE)
        && body
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|word| names_head(word, head))
}

fn names_head(word: &str, head: &str) -> bool {
    (SHORTEST_SHA..=FULL_SHA).contains(&word.len())
        && word.chars().all(|character| character.is_ascii_hexdigit())
        && head.starts_with(&word.to_ascii_lowercase())
}

/// The lines under the "Size increase" heading.
fn section(body: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut level = None;
    for line in body.lines() {
        if let Some((heading_level, text)) = heading(line) {
            match level {
                Some(open) if heading_level <= open => break,
                None if text.eq_ignore_ascii_case(SECTION_HEADING) => {
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

/// The delta a section line gives for the file `id`: the first `+<n> MB`
/// after the identifier, when the line names it.
fn listed_delta(line: &str, id: &str) -> Option<Bytes> {
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
    let rest = &line[start + id.len()..];
    rest.match_indices('+')
        .find_map(|(plus, _)| megabytes(&rest[plus + 1..]))
}

fn is_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/')
}

/// `1.10 MB` (at most six decimals) at the start of `text`, in bytes.
fn megabytes(text: &str) -> Option<Bytes> {
    let number_length = text
        .find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(number_length);
    if !unit.trim_start().starts_with("MB") {
        return None;
    }
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() || fraction.len() > 6 {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<6}").parse().ok()?
    };
    whole
        .checked_mul(MB)
        .and_then(|bytes| bytes.checked_add(fraction))
        .map(Bytes)
}
