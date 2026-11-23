//! The Developer Certificate of Origin check (ADR 0018): every commit a pull
//! request adds, from its base to its head, carries a `Signed-off-by:`
//! trailer naming its author, as `git commit -s` writes it. Merge commits are
//! not judged, as the DCO app does not judge them: they add no line of their
//! own, and GitHub writes some (a branch updated from its base). A commit
//! judged green before is not judged again (`judged_commits`).

use std::path::Path;

use anyhow::{Result, bail};

use crate::check::Finding;
use crate::judged_commits::{self, JudgedCommits, Judgment};

/// Separates one commit's fields, and one commit from the next, in `git log`'s
/// output.
const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// One commit's identity and its sign-offs, as `git log` prints them.
struct Commit<'a> {
    listed: judged_commits::Commit<'a>,
    author: String,
    sign_offs: Vec<&'a str>,
}

/// The check's name in the gate's record of judged commits.
const CHECK: &str = "dco";

/// The rules a verdict follows from beside the commit: this file, which
/// holds them all, so a change to them judges every commit again.
const RULES: &str = include_str!("dco.rs");

/// Judges each commit from `base` to `head` that no layer judged green
/// before in the gate's records `records`, and records those it judges
/// green there; with no `records`, judges every commit and records none.
pub(crate) fn check(
    root: &Path,
    base: &str,
    head: &str,
    records: Option<&Path>,
) -> Result<Judgment> {
    let format =
        "%H%x1f%P%x1f%an%x1f%ae%x1f%(trailers:key=Signed-off-by,valueonly,separator=%x1f)%x1e";
    let log = crate::git::text(
        root,
        &[
            "log",
            &format!("--format={format}"),
            &format!("{base}..{head}"),
            "--",
        ],
    )?;
    let commits = log
        .split(RECORD)
        .map(str::trim)
        .filter(|record| !record.is_empty())
        .map(parse)
        .collect::<Result<Vec<_>>>()?;
    let listed: Vec<judged_commits::Commit<'_>> =
        commits.iter().map(|commit| commit.listed).collect();
    let judged = JudgedCommits::read(records, CHECK, &[RULES.as_bytes()])?;
    let (notes, left) = judged.split(&listed);
    let mut findings = Vec::new();
    for commit in commits.iter().filter(|commit| {
        !commit.listed.merge && left.iter().any(|left| left.id == commit.listed.id)
    }) {
        findings.extend(judge(commit));
    }
    judged.add(&left, &findings)?;
    Ok(Judgment { notes, findings })
}

/// The finding on a commit that is no merge, if any.
fn judge(commit: &Commit<'_>) -> Option<Finding> {
    let signed_by_author = commit
        .sign_offs
        .iter()
        .any(|sign_off| same_identity(sign_off, &commit.author));
    if commit.sign_offs.is_empty() {
        Some(Finding::new(
            "missing-sign-off",
            commit.listed.id,
            "the commit has no `Signed-off-by:` trailer; amend it with `git commit -s` (ADR 0018)",
        ))
    } else if signed_by_author {
        None
    } else {
        Some(Finding::new(
            "sign-off-not-author",
            commit.listed.id,
            format!(
                "no `Signed-off-by:` trailer names its author, {}; the author signs off (ADR 0018)",
                commit.author
            ),
        ))
    }
}

fn parse(record: &str) -> Result<Commit<'_>> {
    let mut fields = record.split(FIELD);
    let (Some(id), Some(parents), Some(name), Some(email)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        bail!("reading git log's output: a commit has too few fields");
    };
    Ok(Commit {
        listed: judged_commits::Commit::parse(id, parents),
        author: format!("{name} <{email}>"),
        sign_offs: fields
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect(),
    })
}

/// Whether two `Name <email>` identities are the same person: the same name,
/// and the same email address whatever its case.
fn same_identity(sign_off: &str, author: &str) -> bool {
    let split = |identity: &str| -> Option<(String, String)> {
        let (name, rest) = identity.split_once('<')?;
        let email = rest.strip_suffix('>')?;
        Some((name.trim().to_owned(), email.trim().to_lowercase()))
    };
    split(sign_off).is_some_and(|identity| Some(identity) == split(author))
}
