//! The Journey report (ADRs 0058, 0063 and 0094): each variant of an active
//! Journey with its outcome on `main`'s latest Release run, by milestone,
//! area and Product, so the owner sees whether an owner test is ready; and
//! an owner test's "Journeys to try", generated from the catalog so it never
//! drifts from the Journeys.
//!
//! A variant's outcome comes from the result documents, `<id>@<surface>.json`
//! under `result-documents/`, that the Release suite's cells kept in their
//! JUnit artifacts for that run's commit. Each is loaded through
//! `goodfetch-protocol`'s reader, which recomputes its outcome and compares
//! it with its Journey:
//! - `proved` or `failed`, as every accepted document of the variant gives,
//!   `failed` when any does;
//! - `rejected` when the reader refuses any of them, or one names another
//!   variant than its file: never `proved`;
//! - `unproved` when the run kept none;
//! - with the runners' listings, `failed` when a cell of that run ran a test
//!   that carries the variant and kept no document of it, as when the test
//!   panicked before `finish`: an owner test is ready only when every
//!   automated variant is `proved` (ADR 0063).
//!
//! The variant's Product is its Journey's, but Foxhound's for
//! `foxhound-cli`, which runs on Foxhound's executable.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::journey::{Journey, Milestone, Status};
use goodfetch_protocol::result_document::{self, JourneyOutcome, LoadedDocument};
use serde::Serialize;

use super::variants::{self, FOXHOUND_CLI, Variant};
use crate::results;

/// The folder of a JUnit artifact that holds its result documents.
const DOCUMENTS: &str = "result-documents/";

/// A variant's outcome on `main`'s latest Release run, from the best to the
/// worst.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VariantOutcome {
    Proved,
    Unproved,
    Failed,
    Rejected,
}

impl VariantOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Proved => "proved",
            Self::Unproved => "unproved",
            Self::Failed => "failed",
            Self::Rejected => "rejected",
        }
    }
}

/// One variant of the report.
#[derive(Serialize)]
pub(super) struct Entry {
    /// `J-NNN@<surface>`.
    pub(super) id: String,
    pub(super) title: String,
    pub(super) outcome: VariantOutcome,
    /// Why the reader refused a document, for a `rejected` variant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) detail: Option<String>,
}

/// The variants of one milestone, area and Product.
#[derive(Serialize)]
pub(super) struct Group {
    pub(super) milestone: Milestone,
    pub(super) area: u8,
    pub(super) product: Product,
    pub(super) variants: Vec<Entry>,
}

/// The whole report, as `journeys.json` holds it.
#[derive(Serialize)]
pub(super) struct Report {
    /// The commit of `main`'s latest Release run; none when the report read
    /// no store, or the store holds no Release run of `main`.
    pub(super) release_commit: Option<String>,
    pub(super) groups: Vec<Group>,
}

/// One exception with evidence `manual` in `features/exceptions.toml`: a
/// step the owner test runs by hand.
pub(crate) struct ManualException {
    pub(crate) feature: String,
    pub(crate) reason: String,
    pub(crate) link: String,
}

/// What a variant's documents gave, before the absence of any is `unproved`.
struct Judged {
    outcome: VariantOutcome,
    detail: Option<String>,
}

/// The tests that carry each variant, by its ID, as `(<runner>, <test>)`.
pub(crate) type VariantTests = BTreeMap<String, Vec<(String, String)>>;

/// The report of the active Journeys, from the store at `store` if any,
/// cross-checked against the cells' test runs when `variant_tests` is given.
pub(super) fn build(
    journeys: &[Journey],
    store: Option<&Path>,
    variant_tests: Option<&VariantTests>,
) -> Result<Report> {
    let (release_commit, judged) = match store {
        Some(store) => judge_documents(journeys, store, variant_tests)?,
        None => (None, BTreeMap::new()),
    };
    let mut groups: BTreeMap<(Milestone, u8, Product), Vec<Entry>> = BTreeMap::new();
    for variant in active(journeys) {
        let id = variant.id();
        let judged = judged.get(&id);
        groups
            .entry((
                variant.milestone,
                variant.journey.area,
                product_of(&variant),
            ))
            .or_default()
            .push(Entry {
                title: variant.journey.title.clone(),
                outcome: judged.map_or(VariantOutcome::Unproved, |judged| judged.outcome),
                detail: judged.and_then(|judged| judged.detail.clone()),
                id,
            });
    }
    Ok(Report {
        release_commit,
        groups: groups
            .into_iter()
            .map(|((milestone, area, product), variants)| Group {
                milestone,
                area,
                product,
                variants,
            })
            .collect(),
    })
}

/// Every variant of each active Journey, in the catalog's order.
fn active(journeys: &[Journey]) -> impl Iterator<Item = Variant<'_>> {
    journeys
        .iter()
        .filter(|journey| journey.status == Status::Active)
        .flat_map(variants::of)
}

/// The Product whose executable a variant drives.
fn product_of(variant: &Variant<'_>) -> Product {
    if variant.surface == FOXHOUND_CLI {
        Product::Foxhound
    } else {
        variant.journey.product
    }
}

/// A Release cell: its OS, architecture and runtime.
type Cell = (String, String, Option<String>);

fn cell_name(cell: &Cell) -> String {
    match &cell.2 {
        Some(runtime) => format!("{} {} {runtime}", cell.0, cell.1),
        None => format!("{} {}", cell.0, cell.1),
    }
}

/// Each variant's outcome on `main`'s latest Release run, by its ID, as the
/// Journey report gives it: the Feature report reads its variants through
/// this, never from JUnit alone (ADR 0072's rules 3 and 4).
pub(crate) fn variant_outcomes(
    journeys: &[Journey],
    store: &Path,
    variant_tests: Option<&VariantTests>,
) -> Result<BTreeMap<String, VariantOutcome>> {
    let (_, judged) = judge_documents(journeys, store, variant_tests)?;
    Ok(judged
        .into_iter()
        .map(|(variant, judged)| (variant, judged.outcome))
        .collect())
}

/// The commit of `main`'s latest Release run, and what the documents its
/// cells kept give for each variant they name, a variant whose test a cell
/// ran without keeping its document `failed`.
fn judge_documents(
    journeys: &[Journey],
    store: &Path,
    variant_tests: Option<&VariantTests>,
) -> Result<(Option<String>, BTreeMap<String, Judged>)> {
    let attempts = results::read_all(store)
        .with_context(|| format!("reading the results store at {}", store.display()))?;
    let Some((latest, runs)) = results::latest_on_main(&attempts) else {
        return Ok((None, BTreeMap::new()));
    };
    let mut kept_by: BTreeMap<String, BTreeSet<Cell>> = BTreeMap::new();
    let by_id: BTreeMap<&str, &Journey> = journeys
        .iter()
        .map(|journey| (journey.id.as_str(), journey))
        .collect();
    let kept = results::read_kept(store, |file| {
        file.strip_prefix(DOCUMENTS)
            .is_some_and(|name| !name.contains('/'))
    })?;
    let mut judged: BTreeMap<String, Judged> = BTreeMap::new();
    for kept in kept.iter().filter(|kept| {
        kept.run.branch.as_deref() == Some(results::MAIN)
            && kept.configuration.suite == results::RELEASE_SUITE
            && kept.commit == latest
    }) {
        let Some(variant) = kept
            .file
            .strip_prefix(DOCUMENTS)
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let Some(journey) = variant
            .split_once('@')
            .and_then(|(journey, _)| by_id.get(journey))
        else {
            continue;
        };
        kept_by.entry(variant.to_owned()).or_default().insert((
            kept.configuration.os.clone(),
            kept.configuration.arch.clone(),
            kept.configuration.runtime.clone(),
        ));
        let read = read_document(&kept.contents, variant, journey);
        let entry = judged.entry(variant.to_owned()).or_insert(Judged {
            outcome: read.outcome,
            detail: None,
        });
        if read.outcome >= entry.outcome {
            entry.outcome = read.outcome;
            entry.detail = entry.detail.take().or(read.detail);
        }
    }
    for (variant, carriers) in variant_tests.into_iter().flatten() {
        let ran: BTreeSet<Cell> = runs
            .iter()
            .filter(|run| {
                carriers
                    .iter()
                    .any(|(runner, test)| *runner == run.runner && *test == run.test)
            })
            .map(|run| (run.os.clone(), run.arch.clone(), run.runtime.clone()))
            .collect();
        let kept = kept_by.get(variant);
        let Some(missing) = ran
            .iter()
            .find(|cell| kept.is_none_or(|kept| !kept.contains(*cell)))
        else {
            continue;
        };
        let detail = format!(
            "the {} cell ran its test and kept no document of it",
            cell_name(missing)
        );
        let entry = judged.entry(variant.clone()).or_insert(Judged {
            outcome: VariantOutcome::Failed,
            detail: None,
        });
        if VariantOutcome::Failed >= entry.outcome {
            entry.outcome = VariantOutcome::Failed;
            entry.detail = Some(detail);
        }
    }
    Ok((Some(latest), judged))
}

/// What one kept document gives for `variant`.
fn read_document(bytes: &[u8], variant: &str, journey: &Journey) -> Judged {
    let rejected = |detail: String| Judged {
        outcome: VariantOutcome::Rejected,
        detail: Some(detail),
    };
    let document = match result_document::read(bytes) {
        Ok(LoadedDocument::Journey(document)) => document,
        Ok(LoadedDocument::Conformance(_)) => {
            return rejected("it is a `conformance` document".to_owned());
        }
        Err(error) => return rejected(error.to_string()),
    };
    let document = match result_document::read_journey_document(document, journey) {
        Ok(document) => document,
        Err(error) => return rejected(error.to_string()),
    };
    let named = format!(
        "{}@{}",
        document.facts().journey_id,
        document.facts().surface
    );
    if named != variant {
        return rejected(format!("it is `{named}`'s document, kept as `{variant}`'s"));
    }
    Judged {
        outcome: match document.outcome() {
            JourneyOutcome::Proved => VariantOutcome::Proved,
            JourneyOutcome::Failed => VariantOutcome::Failed,
        },
        detail: None,
    }
}

/// The report as Markdown, as the job summary and `journeys.md` show it.
pub(super) fn markdown(report: &Report) -> String {
    let mut text = String::from("# Journey report\n\n");
    match &report.release_commit {
        Some(commit) => {
            let _ = writeln!(
                text,
                "From `main`'s latest Release run, of commit `{commit}`."
            );
        }
        None => text.push_str("No Release run of `main` was read, so no variant is proved.\n"),
    }
    for group in &report.groups {
        let _ = write!(
            text,
            "\n## {:?}, area {}, {}\n\n",
            group.milestone,
            group.area,
            group.product.name()
        );
        for variant in &group.variants {
            let _ = write!(
                text,
                "- `{}` {}: {}",
                variant.id,
                variant.outcome.as_str(),
                variant.title
            );
            if let Some(detail) = &variant.detail {
                let _ = write!(text, " ({detail})");
            }
            text.push('\n');
        }
    }
    text
}

/// The owner test's "Journeys to try" at `milestone` (ADRs 0058 and 0094):
/// each active Journey of either Product with a variant due at it, by ID,
/// with its Product, title, those variants and its manual steps, then each
/// `manual` exception of a Feature due at it.
pub(super) fn journeys_to_try(
    journeys: &[Journey],
    milestone: Milestone,
    exceptions: &[ManualException],
) -> String {
    let mut text = String::new();
    let mut active: Vec<&Journey> = journeys
        .iter()
        .filter(|journey| journey.status == Status::Active)
        .collect();
    active.sort_by_key(|journey| journey.id.number());
    for journey in active {
        let due: Vec<String> = variants::of(journey)
            .iter()
            .filter(|variant| variant.milestone == milestone)
            .map(|variant| format!("`{}`", variant.id()))
            .collect();
        if due.is_empty() {
            continue;
        }
        let _ = write!(
            text,
            "- [ ] **{}** ({}) {} ({})",
            journey.id,
            journey.product.name(),
            journey.title,
            due.join(", ")
        );
        let manual: Vec<String> = journey
            .steps
            .iter()
            .filter_map(|step| {
                let why = step.manual.as_deref()?;
                Some(format!("{} ({why})", step.text))
            })
            .collect();
        match manual.as_slice() {
            [] => {}
            [step] => {
                let _ = write!(text, ". Manual step: {step}.");
            }
            steps => {
                let _ = write!(text, ". Manual steps: {}.", steps.join("; "));
            }
        }
        text.push('\n');
    }
    text.push_str("\n**Manual exceptions** (`features/exceptions.toml`, evidence `manual`):\n\n");
    if exceptions.is_empty() {
        text.push_str("- none\n");
    }
    for exception in exceptions {
        let _ = writeln!(
            text,
            "- [ ] `{}`: {} ({})",
            exception.feature, exception.reason, exception.link
        );
    }
    text
}
