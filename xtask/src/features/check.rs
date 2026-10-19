//! The Feature check (ADR 0072's static check, plus ADR 0075's
//! `not_journey` rule), in the Quick suite: every declaration names an
//! active Feature of the inventory, every generated Feature has a declaring
//! test or an exception and exactly one owning area, every exception names a
//! Feature no test declares, and every story carries the reason its flag
//! needs. A file that breaks its schema fails it too.
//!
//! The Quick suite runs it without the runners' listings, which start
//! processes: it then reads Rust's, the Journeys' and the conformance
//! cases' declarations, and only notes a generated Feature none of them
//! declares, since a Vitest, Playwright or pytest test may. The Pre-merge
//! suite runs it with the listings (`just features-tests`), where such a
//! Feature fails it.
//!
//! The inventory's staleness is `xtask codegen --check`'s, which regenerates
//! it. A matrix row without a milestone is not checked yet: no ticket has
//! given the rows theirs (ADR 0072's note, pending owner review).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Result, bail};

use super::files::{
    self, AreasFile, ExceptionsFile, Feature, Inventory, Kind, Status, StoryFile, Unreadable,
};
use super::scan::Declaration;
use crate::check::Finding;

/// Whether the declarations hold every runner's: without the runners'
/// listings, a generated Feature only a Vitest, Playwright or pytest test
/// declares would look untested.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Declarations {
    /// Rust's, the Journeys' and the conformance cases' only.
    WithoutListings,
    /// Every idiom's, the runners' listings included.
    All,
}

/// What the check found: the findings fail it; the notes, each an untested
/// generated Feature the declarations it read may miss, do not.
pub(crate) struct Verdict {
    pub(crate) findings: Vec<Finding>,
    pub(crate) notes: Vec<Finding>,
}

pub(crate) fn check(
    root: &Path,
    declarations: &[Declaration],
    read: Declarations,
) -> Result<Verdict> {
    let mut findings = Vec::new();
    let inventory = readable(files::read_inventory(root), files::INVENTORY, &mut findings)?;
    let areas: Option<AreasFile> = readable(
        files::read_optional_toml(root, files::AREAS),
        files::AREAS,
        &mut findings,
    )?;
    let exceptions: Option<ExceptionsFile> = readable(
        files::read_optional_toml(root, files::EXCEPTIONS),
        files::EXCEPTIONS,
        &mut findings,
    )?;
    stories(root, &mut findings)?;
    let Some(inventory) = inventory else {
        return Ok(Verdict {
            findings,
            notes: Vec::new(),
        });
    };
    let features: BTreeMap<&str, &Feature> = inventory
        .features
        .iter()
        .map(|feature| (feature.id.as_str(), feature))
        .collect();
    declared(&features, declarations, &mut findings);
    let tested: BTreeSet<&str> = declarations
        .iter()
        .map(|declaration| declaration.feature.as_str())
        .collect();
    let excepted = exceptions
        .as_ref()
        .map(|file| excepted(file, &features, &tested, &mut findings))
        .unwrap_or_default();
    let mut untested = Vec::new();
    generated(
        &inventory,
        areas.as_ref(),
        &tested,
        &excepted,
        &mut untested,
        &mut findings,
    );
    let notes = match read {
        Declarations::All => {
            findings.append(&mut untested);
            Vec::new()
        }
        Declarations::WithoutListings => untested,
    };
    Ok(Verdict { findings, notes })
}

/// A file's value, or a `schema` finding when it breaks its schema.
fn readable<T>(
    read: Result<T, Unreadable>,
    path: &str,
    findings: &mut Vec<Finding>,
) -> Result<Option<T>> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(Unreadable::Schema(message)) => {
            findings.push(Finding::new("schema", path, message));
            Ok(None)
        }
        Err(Unreadable::Io(error)) => bail!(error),
    }
}

/// Each declaration names an active Feature of the inventory.
fn declared(
    features: &BTreeMap<&str, &Feature>,
    declarations: &[Declaration],
    findings: &mut Vec<Finding>,
) {
    let mut reported = BTreeSet::new();
    for declaration in declarations {
        let id = declaration.feature.as_str();
        let finding = match features.get(id) {
            None => Finding::new(
                "unknown-feature",
                id,
                format!(
                    "{} declares a Feature the inventory does not list",
                    declaration.test
                ),
            ),
            Some(feature) if feature.status == Status::Retired => Finding::new(
                "retired-feature",
                id,
                format!("{} declares a retired Feature", declaration.test),
            ),
            Some(_) => continue,
        };
        if reported.insert(id) {
            findings.push(finding);
        }
    }
}

/// The excepted Features; each exception names a known Feature no test
/// declares.
fn excepted<'a>(
    file: &'a ExceptionsFile,
    features: &BTreeMap<&str, &Feature>,
    tested: &BTreeSet<&str>,
    findings: &mut Vec<Finding>,
) -> BTreeSet<&'a str> {
    let mut excepted = BTreeSet::new();
    for exception in &file.exception {
        let id = exception.feature.as_str();
        if !features.contains_key(id) {
            findings.push(Finding::new(
                "exception-unknown",
                id,
                "the exception names a Feature the inventory does not list",
            ));
        } else if tested.contains(id) {
            findings.push(Finding::new(
                "exception-tested",
                id,
                "a test declares the excepted Feature; remove the exception",
            ));
        }
        excepted.insert(id);
    }
    excepted
}

/// Each generated Feature has a declaring test or an exception (else an
/// `untested` entry), and one owning area.
fn generated(
    inventory: &Inventory,
    areas: Option<&AreasFile>,
    tested: &BTreeSet<&str>,
    excepted: &BTreeSet<&str>,
    untested: &mut Vec<Finding>,
    findings: &mut Vec<Finding>,
) {
    for feature in &inventory.features {
        if feature.kind.is_declared() || feature.status == Status::Retired {
            continue;
        }
        let id = feature.id.as_str();
        if !tested.contains(id) && !excepted.contains(id) {
            untested.push(Finding::new(
                "untested",
                id,
                "no test declares the generated Feature and no exception names it",
            ));
        }
        let Some(areas) = areas else {
            continue;
        };
        let name = id
            .strip_prefix(feature.kind.as_str())
            .and_then(|rest| rest.strip_prefix(':'))
            .unwrap_or(id);
        match areas.owners(feature.kind, name).len() {
            1 => {}
            0 => findings.push(Finding::new(
                "no-area",
                id,
                format!("no entry of {} owns the generated Feature", files::AREAS),
            )),
            _ => findings.push(Finding::new(
                "two-areas",
                id,
                format!(
                    "several entries of {} own the generated Feature",
                    files::AREAS
                ),
            )),
        }
    }
}

/// Every stories file reads as its schema, and each story carries the
/// reasons its flags need (ADR 0075).
fn stories(root: &Path, findings: &mut Vec<Finding>) -> Result<()> {
    for (area, path) in files::story_files(root)? {
        let Some(area) = area else {
            findings.push(Finding::new(
                "schema",
                path,
                "a stories file is named by its area's number",
            ));
            continue;
        };
        let Some(file): Option<StoryFile> =
            readable(files::read_toml(root, &path), &path, findings)?
        else {
            continue;
        };
        let mut seen = BTreeSet::new();
        for story in &file.story {
            let id = format!("{}:{area}-{}", Kind::Story, story.id);
            if !seen.insert(story.id) {
                findings.push(Finding::new(
                    "duplicate-story",
                    &id,
                    "two stories share the number",
                ));
            }
            match (story.journey, story.not_journey) {
                (false, None) => findings.push(Finding::new(
                    "not-journey-missing",
                    &id,
                    "a story with `journey = false` names the test it fails in `not_journey`",
                )),
                (true, Some(_)) => findings.push(Finding::new(
                    "not-journey-extra",
                    &id,
                    "a story with `journey = true` has no `not_journey`",
                )),
                _ => {}
            }
            if story.status == Status::Retired && story.reason.is_none() {
                findings.push(Finding::new(
                    "retired-without-reason",
                    &id,
                    "a retired story says why in `reason`",
                ));
            }
        }
    }
    Ok(())
}
