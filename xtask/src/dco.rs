//! The Developer Certificate of Origin check (ADR 0018): every commit a pull
//! request adds, from its base to its head, carries a `Signed-off-by:`
//! trailer naming its author, as `git commit -s` writes it. Merge commits are
//! not judged, as the DCO app does not judge them: they add no line of their
//! own, and GitHub writes some (a branch updated from its base).

use std::path::Path;

use anyhow::{Result, bail};

use crate::check::Finding;

/// Separates one commit's fields, and one commit from the next, in `git log`'s
/// output.
const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

/// One commit's identity and its sign-offs, as `git log` prints them.
struct Commit<'a> {
    id: &'a str,
    author: String,
    sign_offs: Vec<&'a str>,
}

pub(crate) fn check(root: &Path, base: &str, head: &str) -> Result<Vec<Finding>> {
    let format = "%H%x1f%an%x1f%ae%x1f%(trailers:key=Signed-off-by,valueonly,separator=%x1f)%x1e";
    let log = crate::git::text(
        root,
        &[
            "log",
            "--no-merges",
            &format!("--format={format}"),
            &format!("{base}..{head}"),
            "--",
        ],
    )?;
    let mut findings = Vec::new();
    for record in log
        .split(RECORD)
        .map(str::trim)
        .filter(|record| !record.is_empty())
    {
        let commit = parse(record)?;
        let signed_by_author = commit
            .sign_offs
            .iter()
            .any(|sign_off| same_identity(sign_off, &commit.author));
        if commit.sign_offs.is_empty() {
            findings.push(Finding::new(
                "missing-sign-off",
                commit.id,
                "the commit has no `Signed-off-by:` trailer; amend it with `git commit -s` (ADR 0018)",
            ));
        } else if !signed_by_author {
            findings.push(Finding::new(
                "sign-off-not-author",
                commit.id,
                format!(
                    "no `Signed-off-by:` trailer names its author, {}; the author signs off (ADR 0018)",
                    commit.author
                ),
            ));
        }
    }
    Ok(findings)
}

fn parse(record: &str) -> Result<Commit<'_>> {
    let mut fields = record.split(FIELD);
    let (Some(id), Some(name), Some(email)) = (fields.next(), fields.next(), fields.next()) else {
        bail!("reading git log's output: a commit has too few fields");
    };
    Ok(Commit {
        id,
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
