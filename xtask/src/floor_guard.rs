//! The floor guard (ADR 0140; `TESTING.md`, Quality register): what a change
//! does to the bar, from the merge base to its head. It fails on a new
//! suppression outside the Floor's forms, a stub or an empty catch, a skipped or
//! deleted test, a weakened register row and an edit to the gate's own files,
//! unless a `Quality-change: <file or row>: <reason>` trailer of the change
//! names it. Adapted from `constraint-driven-development`'s floor guard, reading
//! `TESTING.md`: tightening is silent, loosening is loud, and a guard that could
//! not run exits 2, never 0.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask compiles each of its patterns once, in a `LazyLock`; `CountedWork` counts the engine's compilations (ADR 0140)"
)]

mod diff;
mod moves;
mod register;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result, anyhow};

use crate::check::{self, Finding};
use crate::register::document::FILE as REGISTER_FILE;

/// Runs `git <args>` in `root` and answers its standard output, a byte that
/// is no UTF-8 replaced: a diff may show a file of any encoding.
fn git(root: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&crate::git::bytes(root, args)?).into_owned())
}

/// A file at a commit, or `None` when the commit has no such file.
fn file_at(root: &Path, commit: &str, path: &str) -> Result<Option<String>> {
    let object = format!("{commit}:{path}");
    if !crate::git::succeeds(root, &["cat-file", "-e", &object])? {
        return Ok(None);
    }
    git(root, &["show", &object]).map(Some)
}

/// What the `Quality-change` trailers of a change name, and a finding for
/// each trailer that names nothing. `xtask stress select` judges the tests
/// its listings show deleted by them too.
pub(crate) struct Trailers {
    /// What each trailer names when its subject holds no `: `: its text
    /// before the first one.
    named: BTreeSet<String>,
    /// Each trailer whole, `<subject>: <reason>`: a subject may itself hold
    /// `: `, as a Vitest or Playwright test's ID does, so a trailer also
    /// names the subject its text starts with, whole, before a `: ` and a
    /// reason.
    texts: Vec<String>,
    invalid: Vec<Finding>,
}

impl Trailers {
    /// The trailers of every commit in `range` (`<base>..<head>`).
    pub(crate) fn read(root: &Path, range: &str) -> Result<Self> {
        let log = git(
            root,
            &[
                "log",
                "--format=%(trailers:key=Quality-change,valueonly,unfold)",
                range,
                "--",
            ],
        )?;
        let mut named = BTreeSet::new();
        let mut texts = Vec::new();
        let mut invalid = Vec::new();
        for trailer in log.lines().map(str::trim).filter(|line| !line.is_empty()) {
            match trailer.split_once(": ") {
                Some((name, reason)) if !name.is_empty() && !reason.trim().is_empty() => {
                    named.insert(name.trim().to_owned());
                    texts.push(trailer.to_owned());
                }
                _ => invalid.push(Finding::new(
                    "invalid-trailer",
                    trailer,
                    "the trailer is `Quality-change: <file or row>: <reason>`",
                )),
            }
        }
        Ok(Self {
            named,
            texts,
            invalid,
        })
    }

    /// Whether a trailer names `subject`.
    fn names(&self, subject: &str) -> bool {
        self.named.contains(subject)
            || self.texts.iter().any(|text| {
                text.strip_prefix(subject)
                    .and_then(|rest| rest.strip_prefix(": "))
                    .is_some_and(|reason| !reason.trim().is_empty())
            })
    }

    /// Splits `flagged` into a `quality-change` note for each finding whose
    /// subject a trailer names, and the findings none names.
    pub(crate) fn judge(&self, flagged: Vec<Finding>) -> (Vec<Finding>, Vec<Finding>) {
        let (accepted, refused): (Vec<Finding>, Vec<Finding>) = flagged
            .into_iter()
            .partition(|finding| self.names(&finding.subject));
        let accepted = accepted
            .into_iter()
            .map(|finding| {
                Finding::new(
                    "quality-change",
                    finding.subject,
                    format!("a trailer accepts: {}", finding.kind),
                )
            })
            .collect();
        (accepted, refused)
    }
}

pub(crate) fn run(root: &Path, base: &str, head: &str) -> Result<ExitCode> {
    let merge_base = git(root, &["merge-base", base, head])?.trim().to_owned();
    let head_testing = file_at(root, head, REGISTER_FILE)?
        .ok_or_else(|| anyhow!("{head} has no {REGISTER_FILE}"))?;
    let head_register = register::read(&head_testing)
        .with_context(|| format!("reading {head}'s register"))?
        .ok_or_else(|| anyhow!("{head}'s {REGISTER_FILE} has no Quality register"))?;
    let base_register = match file_at(root, &merge_base, REGISTER_FILE)? {
        Some(testing) => {
            register::read(&testing).with_context(|| format!("reading {merge_base}'s register"))?
        }
        None => None,
    };

    let unified = git(
        root,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--unified=0",
            "--no-renames",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            &merge_base,
            head,
            "--",
        ],
    )?;
    let files = diff::parse(&unified);

    let mut lowered = moves::findings(&files, |path, at_head| {
        file_at(root, if at_head { head } else { &merge_base }, path)
    })?;
    if let Some(base_register) = &base_register {
        lowered.extend(
            files
                .iter()
                .filter(|file| file.existed && base_register.is_gate_file(&file.path))
                .map(|file| {
                    Finding::new(
                        "gate-file-edited",
                        file.path.as_str(),
                        "this is one of the gate's own files; a `Quality-change` trailer naming it records why",
                    )
                }),
        );
        lowered.extend(register::findings(base_register, &head_register));
    }

    let trailers = Trailers::read(root, &format!("{merge_base}..{head}"))?;
    let (accepted, refused) = trailers.judge(lowered);
    let mut findings = trailers.invalid;
    findings.extend(refused);
    check::note(&accepted);
    Ok(check::report(&findings))
}
