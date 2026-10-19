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
//! - `retired-without-reason`: a retired Journey gives no reason;
//! - `missing-task`: a task a Journey names is not in the committed task
//!   list of its seed, or no list is committed for the seed;
//! - `task-digest`: the digest a Journey pins is not the one its seed's list
//!   holds, since the Proving ground changed the task's entry or expected
//!   result.
//!
//! Each finding's subject is the file that breaks the rule. The task rule
//! (ADR 0063, rule 3) reads the task lists committed under `journeys/tasks/`,
//! one per seed, so the check starts no process. `tasks` writes those lists
//! from the Proving ground's `tasks --seed N`, and `tasks --check`, in the
//! Pre-merge suite, fails with `task-list-drift` on a committed list that
//! differs from the Proving ground's, is missing, or is of a seed no Journey
//! uses. The rule on the tests of each variant joins later, since it reads
//! the runners' listings.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
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

/// The folder of the committed task lists, one per seed the catalog uses,
/// which `tasks` writes from the Proving ground's own.
const TASK_LISTS: &str = "journeys/tasks";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on a Journey file that breaks one of ADR 0063's rules that need
    /// no process: it does not parse or fit its schema, it is not named after
    /// its ID, its ID repeats or reaches the next free number, it names a
    /// surface ADR 0063 does not list or one due before it ships, or it is
    /// retired without a reason.
    Check,
    /// Writes the Proving ground's task list of each seed the catalog uses
    /// to `journeys/tasks/seed-<seed>.json`, from its `tasks --seed`, and
    /// removes the list of any seed no Journey uses; with `--check`, writes
    /// nothing and fails on each committed list that differs.
    Tasks(Tasks),
}

#[derive(clap::Args)]
pub(crate) struct Tasks {
    /// The Proving ground's executable, built from this commit.
    #[arg(long)]
    proving_ground: PathBuf,
    /// Writes nothing, and fails on a committed list that differs from the
    /// Proving ground's, is missing, or is of a seed no Journey uses.
    #[arg(long)]
    check: bool,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Check => Ok(crate::check::report(&check(&environment.root)?)),
        Command::Tasks(tasks) => {
            let proving_ground = environment.path(&tasks.proving_ground);
            let lists = proving_ground_lists(&environment.root, &proving_ground)?;
            if tasks.check {
                Ok(crate::check::report(&drift(&environment.root, &lists)?))
            } else {
                write_lists(&environment.root, &lists)?;
                Ok(ExitCode::SUCCESS)
            }
        }
    }
}

/// The task list the Proving ground prints for each seed the parsed
/// Journeys use, rendered as committed, by its path from the root.
fn proving_ground_lists(root: &Path, proving_ground: &Path) -> Result<BTreeMap<String, String>> {
    let mut journeys = Vec::new();
    for path in journey_files(root)? {
        let text =
            std::fs::read_to_string(root.join(&path)).with_context(|| format!("reading {path}"))?;
        // A file that does not parse is `check`'s finding; it uses no seed.
        if let Ok((journey, _)) = parse(&text) {
            journeys.push(journey);
        }
    }
    let mut lists = BTreeMap::new();
    for seed in seeds(journeys.iter()) {
        let output = std::process::Command::new(proving_ground)
            .args(["tasks", "--seed", &seed.to_string()])
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .output()
            .with_context(|| format!("running {}", proving_ground.display()))?;
        if !output.status.success() {
            bail!(
                "{} tasks --seed {seed} failed: {}",
                proving_ground.display(),
                output.status
            );
        }
        let printed: Value = serde_json::from_slice(&output.stdout)
            .with_context(|| format!("reading the task list of seed {seed}"))?;
        let tasks = printed
            .get("tasks")
            .filter(|tasks| tasks.is_array())
            .with_context(|| format!("the task list of seed {seed} holds no `tasks` array"))?;
        // The Proving ground's version names its commit, so it stays out of
        // the committed list, which changes only with the tasks.
        let list = serde_json::json!({ "seed": seed, "tasks": tasks });
        let rendered = serde_json::to_string_pretty(&list)? + "\n";
        lists.insert(task_list_path(seed), rendered);
    }
    Ok(lists)
}

/// Every committed task list, by its path from the root.
fn committed_lists(root: &Path) -> Result<BTreeSet<String>> {
    let folder = root.join(TASK_LISTS);
    let mut lists = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Ok(lists);
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_file()
            && Path::new(&name)
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            lists.insert(format!("{TASK_LISTS}/{name}"));
        }
    }
    Ok(lists)
}

/// Each committed list that is not the Proving ground's: a differing or
/// missing one, and one of a seed no Journey uses.
fn drift(root: &Path, lists: &BTreeMap<String, String>) -> Result<Vec<Finding>> {
    let fix = "run `just journeys-tasks`, check that each Journey whose tasks moved still fits, and commit the result";
    let mut findings = Vec::new();
    for (path, list) in lists {
        match std::fs::read_to_string(root.join(path)) {
            Ok(committed) if committed == *list => {}
            Ok(_) => findings.push(Finding::new(
                "task-list-drift",
                path.clone(),
                format!("it differs from the Proving ground's; {fix}"),
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                findings.push(Finding::new(
                    "task-list-drift",
                    path.clone(),
                    format!("a Journey uses its seed, but it is not committed; {fix}"),
                ));
            }
            Err(error) => return Err(error).with_context(|| format!("reading {path}")),
        }
    }
    for path in committed_lists(root)?.difference(&lists.keys().cloned().collect()) {
        findings.push(Finding::new(
            "task-list-drift",
            path.clone(),
            format!("no Journey uses its seed; {fix}"),
        ));
    }
    findings.sort_by(|left, right| left.subject.cmp(&right.subject));
    Ok(findings)
}

/// Writes each list, and removes the committed list of any seed no Journey
/// uses.
fn write_lists(root: &Path, lists: &BTreeMap<String, String>) -> Result<()> {
    for path in committed_lists(root)? {
        if !lists.contains_key(&path) {
            std::fs::remove_file(root.join(&path)).with_context(|| format!("removing {path}"))?;
        }
    }
    if !lists.is_empty() {
        std::fs::create_dir_all(root.join(TASK_LISTS))
            .with_context(|| format!("creating {TASK_LISTS}"))?;
    }
    for (path, list) in lists {
        std::fs::write(root.join(path), list).with_context(|| format!("writing {path}"))?;
    }
    Ok(())
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
    let lists = read_task_lists(root, &parsed, &mut findings);
    for file in &parsed {
        findings.extend(file_findings(file, schema.as_ref(), next, &numbers));
        findings.extend(task_findings(file, &lists));
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

/// A seed's committed task list, as the task rule reads it.
enum Committed {
    /// Each task's digest, by its ID.
    Listed(BTreeMap<String, String>),
    /// No list is committed for the seed.
    Absent,
    /// The list does not parse, which a `parse` finding reports.
    Unreadable,
}

/// A committed task list as the task rule reads it: its seed, and each
/// task's ID and digest among the fields the Proving ground lists.
#[derive(Deserialize)]
struct TaskList {
    seed: u64,
    tasks: Vec<ListedTask>,
}

#[derive(Deserialize)]
struct ListedTask {
    id: String,
    digest: String,
}

/// The committed task list of one seed, by its path from the root.
fn task_list_path(seed: u64) -> String {
    format!("{TASK_LISTS}/seed-{seed}.json")
}

/// The committed task list of each seed a parsed Journey uses; a list that
/// does not parse, or names another seed, is a `parse` finding.
fn read_task_lists(
    root: &Path,
    parsed: &[Parsed],
    findings: &mut Vec<Finding>,
) -> BTreeMap<u64, Committed> {
    let mut lists = BTreeMap::new();
    for seed in seeds(parsed.iter().map(|file| &file.journey)) {
        let path = task_list_path(seed);
        let read = match std::fs::read(root.join(&path)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
            Ok(bytes) => match serde_json::from_slice::<TaskList>(&bytes) {
                Ok(list) if list.seed == seed => Ok(Some(list)),
                Ok(list) => Err(format!("it lists seed {}, not {seed}", list.seed)),
                Err(error) => Err(error.to_string()),
            },
        };
        let committed = match read {
            Ok(Some(list)) => Committed::Listed(
                list.tasks
                    .into_iter()
                    .map(|task| (task.id, task.digest))
                    .collect(),
            ),
            Ok(None) => Committed::Absent,
            Err(error) => {
                findings.push(Finding::new(
                    "parse",
                    path,
                    format!("{error}; run `just journeys-tasks` and commit the result"),
                ));
                Committed::Unreadable
            }
        };
        lists.insert(seed, committed);
    }
    lists
}

/// Each seed the Journeys' `[proving_ground]` tables use, once.
fn seeds<'a>(journeys: impl Iterator<Item = &'a Journey>) -> BTreeSet<u64> {
    journeys
        .filter_map(|journey| journey.proving_ground.as_ref())
        .map(|proving_ground| proving_ground.seed)
        .collect()
}

/// The task rule (ADR 0063, rule 3): each task a Journey names is in its
/// seed's committed list, with the digest the Journey pins.
fn task_findings(file: &Parsed, lists: &BTreeMap<u64, Committed>) -> Vec<Finding> {
    let Some(proving_ground) = &file.journey.proving_ground else {
        return Vec::new();
    };
    let seed = proving_ground.seed;
    let path = task_list_path(seed);
    let mut findings = Vec::new();
    for pinned in &proving_ground.tasks {
        let (id, digest) = (pinned.id.as_str(), pinned.digest.as_str());
        match lists.get(&seed) {
            Some(Committed::Listed(listed)) => match listed.get(id) {
                None => findings.push(Finding::new(
                    "missing-task",
                    file.path.clone(),
                    format!(
                        "`{path}` lists no task `{id}`; pin the task the Journey now uses, after checking the Journey still fits"
                    ),
                )),
                Some(listed) if listed != digest => findings.push(Finding::new(
                    "task-digest",
                    file.path.clone(),
                    format!(
                        "`{path}` lists `{id}` with the digest {listed}, not the pinned {digest}; pin it after checking the Journey still fits"
                    ),
                )),
                Some(_) => {}
            },
            Some(Committed::Unreadable) => {}
            Some(Committed::Absent) | None => findings.push(Finding::new(
                "missing-task",
                file.path.clone(),
                format!(
                    "no task list is committed for seed {seed}, so none lists `{id}`; run `just journeys-tasks` and commit `{path}`"
                ),
            )),
        }
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
