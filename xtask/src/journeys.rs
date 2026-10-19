//! The Journey catalog under `journeys/` (ADR 0063). `check` holds it to the
//! rules that need no process, so it runs in the Quick suite:
//!
//! - `parse`: a Journey file, or `journeys/catalog.toml`, does not parse
//!   under `goodfetch-protocol`'s types, which refuse unknown fields;
//! - `schema`: a file that parses is refused by the committed
//!   `journeys/journey.schema.json`, which editors and the TypeScript and
//!   Python helpers read, or that schema is missing;
//! - `file-name`: a file is not named after its ID;
//! - `duplicate-id`: two Journeys share a number (`J-001` and `J-0001` do);
//! - `id-too-high`: an ID is not below the catalog's next free number;
//! - `unlisted-surface`: a surface, listed or pinned by a step's `on`, is none
//!   of ADR 0063's;
//! - `milestone-too-early`: a surface is due before the milestone that ships
//!   it;
//! - `retired-without-reason`: a retired Journey gives no reason.
//!
//! Each finding's subject is the file that breaks the rule. The task and
//! digest rule and the rule on the tests of each variant join later, since
//! they read the Proving ground's task lists and the runners' listings.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde_json::Value;

use goodfetch_protocol::journey::{Journey, JourneyCatalog, Status, Surface};

use crate::check::Finding;
use crate::environment::Environment;

/// The catalog's folder, from the root.
const FOLDER: &str = "journeys";

/// The file that holds the next free number.
const CATALOG: &str = "journeys/catalog.toml";

/// The Journey file's committed JSON Schema.
const SCHEMA: &str = "journeys/journey.schema.json";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on a Journey file that breaks one of ADR 0063's rules that need
    /// no process: it does not parse or fit its schema, it is not named after
    /// its ID, its ID repeats or reaches the next free number, it names a
    /// surface ADR 0063 does not list or one due before it ships, or it is
    /// retired without a reason.
    Check,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Check => Ok(crate::check::report(&check(&environment.root)?)),
    }
}

/// A Journey file that parsed, by its path from the root.
struct Parsed {
    path: String,
    stem: String,
    journey: Journey,
    json: Value,
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let next = read_catalog(root, &mut findings);
    let schema = read_schema(root, &mut findings);

    let mut parsed = Vec::new();
    for path in journey_files(root)? {
        let text =
            std::fs::read_to_string(root.join(&path)).with_context(|| format!("reading {path}"))?;
        match parse(&text) {
            Ok((journey, json)) => parsed.push(Parsed {
                stem: Path::new(&path)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path,
                journey,
                json,
            }),
            Err(error) => findings.push(Finding::new("parse", path, error)),
        }
    }

    let mut numbers: BTreeMap<u64, usize> = BTreeMap::new();
    for file in &parsed {
        *numbers.entry(file.journey.id.number()).or_default() += 1;
    }
    for file in &parsed {
        findings.extend(file_findings(file, schema.as_ref(), next, &numbers));
    }
    findings.sort_by(|left, right| left.subject.cmp(&right.subject));
    Ok(findings)
}

/// Every rule that judges one parsed file, in the order the module lists
/// them.
fn file_findings(
    file: &Parsed,
    schema: Option<&jsonschema::Validator>,
    next: Option<u64>,
    numbers: &BTreeMap<u64, usize>,
) -> Vec<Finding> {
    let journey = &file.journey;
    let subject = || file.path.clone();
    let mut findings = Vec::new();
    if let Some(error) = schema.and_then(|schema| schema.validate(&file.json).err()) {
        findings.push(Finding::new(
            "schema",
            subject(),
            format!("`{SCHEMA}` refuses it: {error}; run `xtask codegen` and commit the result"),
        ));
    }
    if file.stem != journey.id.as_str() {
        findings.push(Finding::new(
            "file-name",
            subject(),
            format!(
                "its ID is {}; name the file `{}.toml`",
                journey.id, journey.id
            ),
        ));
    }
    if numbers
        .get(&journey.id.number())
        .copied()
        .unwrap_or_default()
        > 1
    {
        findings.push(Finding::new(
            "duplicate-id",
            subject(),
            format!("another Journey has the number of {}", journey.id),
        ));
    }
    if let Some(next) = next.filter(|next| journey.id.number() >= *next) {
        findings.push(Finding::new(
            "id-too-high",
            subject(),
            format!(
                "{} is not below the next free number, {next}, in `{CATALOG}`",
                journey.id
            ),
        ));
    }
    let pinned = journey.steps.iter().filter_map(|step| step.on.as_deref());
    for name in journey.surfaces.keys().map(String::as_str).chain(pinned) {
        if Surface::from_name(name).is_none() {
            findings.push(Finding::new(
                "unlisted-surface",
                subject(),
                format!("`{name}` is no surface of ADR 0063"),
            ));
        }
    }
    for (name, milestone) in &journey.surfaces {
        if let Some(first) = Surface::from_name(name)
            .map(|surface| surface.first_milestone())
            .filter(|first| milestone < first)
        {
            findings.push(Finding::new(
                "milestone-too-early",
                subject(),
                format!("`{name}` is due at {milestone:?}, but ships at {first:?}"),
            ));
        }
    }
    let has_reason = journey
        .reason
        .as_deref()
        .is_some_and(|reason| !reason.trim().is_empty());
    if journey.status == Status::Retired && !has_reason {
        findings.push(Finding::new(
            "retired-without-reason",
            subject(),
            "a retired Journey keeps its file and gives its `reason`",
        ));
    }
    findings
}

/// A Journey file as the type reads it, and as JSON for its schema.
fn parse(text: &str) -> Result<(Journey, Value), String> {
    let journey: Journey = toml::from_str(text).map_err(|error| error.to_string())?;
    let table: toml::Table = toml::from_str(text).map_err(|error| error.to_string())?;
    let json = serde_json::to_value(table).map_err(|error| error.to_string())?;
    Ok((journey, json))
}

/// The next free number, or a finding when the catalog file does not parse.
fn read_catalog(root: &Path, findings: &mut Vec<Finding>) -> Option<u64> {
    let parsed = std::fs::read_to_string(root.join(CATALOG))
        .map_err(|error| error.to_string())
        .and_then(|text| {
            toml::from_str::<JourneyCatalog>(&text).map_err(|error| error.to_string())
        });
    match parsed {
        Ok(catalog) => Some(catalog.next),
        Err(error) => {
            findings.push(Finding::new("parse", CATALOG, error));
            None
        }
    }
}

/// The committed schema's validator, or a finding when it is missing or no
/// JSON Schema.
fn read_schema(root: &Path, findings: &mut Vec<Finding>) -> Option<jsonschema::Validator> {
    let validator = std::fs::read(root.join(SCHEMA))
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            serde_json::from_slice::<Value>(&bytes).map_err(|error| error.to_string())
        })
        .and_then(|schema| jsonschema::validator_for(&schema).map_err(|error| error.to_string()));
    match validator {
        Ok(validator) => Some(validator),
        Err(error) => {
            findings.push(Finding::new(
                "schema",
                SCHEMA,
                format!("{error}; run `xtask codegen` and commit the result"),
            ));
            None
        }
    }
}

/// Every `.toml` file directly in `journeys/` but the catalog file, by its
/// path from the root.
fn journey_files(root: &Path) -> Result<Vec<String>> {
    let folder = root.join(FOLDER);
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Ok(files);
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = format!("{FOLDER}/{name}");
        if entry.file_type()?.is_file()
            && Path::new(&name)
                .extension()
                .is_some_and(|extension| extension == "toml")
            && path != CATALOG
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}
