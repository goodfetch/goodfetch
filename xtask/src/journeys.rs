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
//! - `fake-model-setup`: an AI Journey, one covering an AI row of ADR
//!   0037's matrix or an area 9 story, does not start with an unpinned,
//!   automated step that points the Model provider setting at the Fake
//!   model, so a variant could reach a real provider (ADR 0065);
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
//! uses.
//!
//! With `--listings`, `check` also holds the tests of each variant to ADR
//! 0063's rule 4 (see [`variants`]), from the runners' listings that
//! `listings` writes, in the Pre-merge suite where the tests are built:
//! - `unknown-journey`: a test names a Journey no file is;
//! - `retired-journey`: a test names a retired Journey;
//! - `unlisted-variant`: a test's surface is not one its Journey has a
//!   variant on, `foxhound-cli` included;
//! - `duplicate-variant`: two tests carry one variant;
//! - `malformed-variant`: a Journey test carries no single Journey and
//!   surface.
//!
//! Each variant of an active Journey that no test carries is reported as a
//! `note[untested-variant]` line, which fails nothing. `select` prints the
//! runners' filters for chosen variants (see [`select`]). `report` prints
//! each variant's outcome on `main`'s latest Release run, and an owner
//! test's "Journeys to try" (see [`report`]).
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

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::journey::{
    Journey, JourneyCatalog, JourneyId, Milestone, Status, Surface, TaskId,
};

use crate::check::Finding;
use crate::environment::Environment;
use crate::output;

mod listings;
mod report;
mod select;
mod variants;

pub(crate) use report::{ManualException, VariantOutcome, VariantTests};

/// Each variant's outcome on `main`'s latest Release run in the store at
/// `store`, judged as the Journey report judges it, by the variant's ID.
pub(crate) fn variant_outcomes(
    root: &Path,
    store: &Path,
    variant_tests: Option<&VariantTests>,
) -> Result<BTreeMap<String, VariantOutcome>> {
    report::variant_outcomes(&working_journeys(root)?, store, variant_tests)
}

/// The catalog's folder, from the root.
const FOLDER: &str = "journeys";

/// The file that holds the next free number.
const CATALOG: &str = "journeys/catalog.toml";

/// The Journey file's committed JSON Schema.
const SCHEMA: &str = "journeys/journey.schema.json";

/// The folder of the committed task lists, one per seed the catalog uses,
/// which `tasks` writes from the Proving ground's own.
const TASK_LISTS: &str = "journeys/tasks";

/// Reads each `manual` exception whose Feature is due at a milestone, given
/// every active variant: the Feature inventory's reader, passed in by the
/// caller since `features` imports this module.
pub(crate) type ManualExceptions =
    fn(&Path, Milestone, &[CatalogVariant]) -> Result<Vec<ManualException>>;

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on a Journey file that breaks one of ADR 0063's rules that need
    /// no process: it does not parse or fit its schema, it is not named after
    /// its ID, its ID repeats or reaches the next free number, it names a
    /// surface ADR 0063 does not list or one due before it ships, or it is
    /// retired without a reason. With `--listings`, also fails on a test
    /// that breaks ADR 0063's rule on the tests of each variant, and
    /// reports each variant no test carries.
    Check(Check),
    /// Writes the four runners' listings of the workspace's tests, which
    /// `check --listings` reads: `nextest.json`, `vitest.json`,
    /// `playwright.json` and `pytest.json`.
    Listings(Listings),
    /// Prints each variant of an active Journey with its outcome on `main`'s
    /// latest Release run in the results store, by milestone, area and
    /// Product: `proved` or `failed` as its result documents give,
    /// `rejected` when the reader refuses one, `unproved` when none was
    /// kept. With `--listings`, a variant whose test ran in a cell that kept
    /// no document of it is `failed`. With `--out`, also writes
    /// `journeys.json` and `journeys.md` there. With `--owner-test`, prints
    /// instead that milestone's owner test's "Journeys to try".
    Report(Report),
    /// Prints, one `<runner>\t<argument>` line each, the runners' filters
    /// that select the variants of active Journeys the flags choose: values
    /// of one flag add variants, different flags intersect. Fails when no
    /// variant is chosen.
    Select(Select),
    /// Writes the Proving ground's task list of each seed the catalog uses
    /// to `journeys/tasks/seed-<seed>.json`, from its `tasks --seed`, and
    /// removes the list of any seed no Journey uses; with `--check`, writes
    /// nothing and fails on each committed list that differs.
    Tasks(Tasks),
}

#[derive(clap::Args)]
pub(crate) struct Check {
    /// The folder of the runners' listings that `listings` writes.
    #[arg(long)]
    listings: Option<PathBuf>,
}

#[derive(clap::Args)]
pub(crate) struct Listings {
    /// The folder to write them to.
    #[arg(long)]
    out: PathBuf,
    /// The Cargo profile nextest builds the tests with, the suite's own.
    #[arg(long)]
    cargo_profile: Option<String>,
}

#[derive(clap::Args)]
pub(crate) struct Report {
    /// The results store's checkout; without it, no variant is proved.
    #[arg(long)]
    store: Option<PathBuf>,
    /// The folder of the runners' listings that `listings` writes, which
    /// name the tests that carry each variant.
    #[arg(long)]
    listings: Option<PathBuf>,
    /// The folder to write `journeys.json` and `journeys.md` to.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Prints the "Journeys to try" of this milestone's owner test, `M1` to
    /// `M4`: both Products' Journeys with a variant due at it, their manual
    /// steps, and the `manual` exceptions of its Features.
    #[arg(long, value_parser = parse_milestone)]
    owner_test: Option<Milestone>,
}

#[derive(clap::Args)]
pub(crate) struct Select {
    /// A Journey's ID, such as `J-001`.
    #[arg(long = "id", value_parser = parse_id)]
    ids: Vec<JourneyId>,
    /// A surface of ADR 0063, or `foxhound-cli`.
    #[arg(long = "surface", value_parser = parse_surface)]
    surfaces: Vec<String>,
    /// The milestone a variant is due at, `M1` to `M4`.
    #[arg(long = "milestone", value_parser = parse_milestone)]
    milestones: Vec<Milestone>,
    /// The number of the ADR 0037 area whose spec owns the Journey.
    #[arg(long = "area")]
    areas: Vec<u8>,
    /// A Git tag whose catalog to read instead of the working tree's.
    #[arg(long)]
    tag: Option<String>,
    /// The folder of the runners' listings that `listings` writes: each
    /// chosen variant no test carries is noted, and a choice none of whose
    /// variants has a test fails.
    #[arg(long)]
    listings: Option<PathBuf>,
}

fn parse_id(text: &str) -> Result<JourneyId, String> {
    JourneyId::try_from(text.to_owned()).map_err(|error| error.to_string())
}

fn parse_surface(text: &str) -> Result<String, String> {
    if variants::is_surface(text) {
        Ok(text.to_owned())
    } else {
        Err(format!(
            "`{text}` is no surface of ADR 0063, nor `foxhound-cli`"
        ))
    }
}

fn parse_milestone(text: &str) -> Result<Milestone, String> {
    Milestone::deserialize(serde::de::value::StrDeserializer::<serde::de::value::Error>::new(text))
        .map_err(|_| format!("`{text}` is not M1, M2, M3 or M4"))
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

pub(crate) fn run(
    command: &Command,
    environment: &Environment,
    manual_exceptions: ManualExceptions,
) -> Result<ExitCode> {
    match command {
        Command::Check(arguments) => {
            let listings = arguments
                .listings
                .as_ref()
                .map(|folder| environment.path(folder));
            Ok(crate::check::report(&check(
                &environment.root,
                listings.as_deref(),
            )?))
        }
        Command::Listings(arguments) => {
            listings::write(
                &environment.root,
                &environment.path(&arguments.out),
                &environment.cargo,
                arguments.cargo_profile.as_deref(),
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Report(arguments) => {
            let journeys = working_journeys(&environment.root)?;
            if let Some(milestone) = arguments.owner_test {
                let exceptions = manual_exceptions(
                    &environment.root,
                    milestone,
                    &active_variants(&environment.root)?,
                )?;
                let text = report::journeys_to_try(&journeys, milestone, &exceptions);
                output::line(text.trim_end_matches('\n'));
                return Ok(ExitCode::SUCCESS);
            }
            let store = arguments
                .store
                .as_ref()
                .map(|store| environment.path(store));
            let variant_tests = arguments
                .listings
                .as_ref()
                .map(|folder| variant_tests(&environment.root, &environment.path(folder)))
                .transpose()?;
            let built = report::build(&journeys, store.as_deref(), variant_tests.as_ref())?;
            let markdown = report::markdown(&built);
            if let Some(out) = &arguments.out {
                write_report(&environment.path(out), &built, &markdown)?;
            }
            output::line(markdown.trim_end_matches('\n'));
            Ok(ExitCode::SUCCESS)
        }
        Command::Select(arguments) => select_command(arguments, environment),
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

/// Writes the report's `journeys.json` and `journeys.md` into `out`.
fn write_report(out: &Path, built: &report::Report, markdown: &str) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let mut json = serde_json::to_string_pretty(built)?;
    json.push('\n');
    for (name, contents) in [("journeys.json", json.as_str()), ("journeys.md", markdown)] {
        let path = out.join(name);
        std::fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// The task list the Proving ground prints for each seed the parsed
/// Journeys use, rendered as committed, by its path from the root.
fn proving_ground_lists(root: &Path, proving_ground: &Path) -> Result<BTreeMap<String, String>> {
    let mut journeys = Vec::new();
    for path in journey_files(root)? {
        let text =
            std::fs::read_to_string(root.join(&path)).with_context(|| format!("reading {path}"))?;
        // A file that does not parse names no seed, so its seed's list
        // would be removed as unused: refuse until it parses (`check`).
        let (journey, _) = parse(&text).map_err(|error| {
            anyhow::anyhow!("{path} does not parse ({error}); fix it before writing the task lists")
        })?;
        journeys.push(journey);
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

/// Every finding of the catalog's rules, and with `listings`, of the rule on
/// the tests of each variant, whose untested variants it writes as notes.
pub(crate) fn check(root: &Path, listings: Option<&Path>) -> Result<Vec<Finding>> {
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
    if let Some(folder) = listings {
        let tests = listings::read(root, folder)?;
        let journeys: Vec<&Journey> = parsed.iter().map(|file| &file.journey).collect();
        let (tested, untested) = variants::check(&journeys, &tests);
        findings.extend(tested);
        for variant in untested {
            output::line(&format!(
                "note[untested-variant] {variant}: no test carries it, so it stays unproved"
            ));
        }
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
    findings.extend(check_findings(journey, &file.path));
    findings.extend(fake_model_finding(journey, &file.path));
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

/// The rules on a Journey's checks: each surface a check is scoped to is
/// one of the Journey's variants, and each task a check names is pinned.
fn check_findings(journey: &Journey, subject: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let pinned: Vec<&TaskId> = journey
        .proving_ground
        .iter()
        .flat_map(|table| table.tasks.iter().map(|task| &task.id))
        .collect();
    for check in &journey.expect {
        // A check scoped to a surface the Journey has no variant on belongs
        // to no variant; `foxhound-cli` is a Collie Journey's `cli` variant.
        for surface in &check.surfaces {
            let variant = journey.surfaces.contains_key(surface)
                || (surface == variants::FOXHOUND_CLI
                    && journey.product == Product::Collie
                    && journey.surfaces.contains_key("cli"));
            if !variant {
                findings.push(Finding::new(
                    "check-surface",
                    subject.to_owned(),
                    format!(
                        "check `{}` is scoped to `{surface}`, on which the Journey has no variant",
                        check.text
                    ),
                ));
            }
        }
        if let Some(task) = check.task.as_ref().filter(|task| !pinned.contains(task)) {
            findings.push(Finding::new(
                "check-task-unpinned",
                subject.to_owned(),
                format!(
                    "check `{}` names task `{task}`, which the Journey's `[proving_ground]` does not pin",
                    check.text
                ),
            ));
        }
    }
    findings
}

/// The words a step names the Fake model by (the glossary's term).
const FAKE_MODEL: &str = "Fake model";

/// The rule that keeps an AI Journey off real providers (ADR 0065): a
/// Journey covering an AI row of ADR 0037's matrix (`row:AI-…`) or an area
/// 9 story (`story:9-…`) starts with a step that names the Fake model,
/// pinned to no surface and left to no person, so every variant points its
/// Model provider setting at the Fake model through its own surface before
/// any step can call a model.
fn fake_model_finding(journey: &Journey, subject: &str) -> Option<Finding> {
    let ai = journey.covers.iter().any(|feature| {
        let id = feature.as_str();
        id.starts_with("row:AI-") || id.starts_with("story:9-")
    });
    let setup = journey.steps.first().is_some_and(|step| {
        step.text.contains(FAKE_MODEL) && step.on.is_none() && step.manual.is_none()
    });
    (ai && !setup).then(|| {
        Finding::new(
            "fake-model-setup",
            subject.to_owned(),
            format!(
                "an AI Journey's first step points the Model provider setting at the {FAKE_MODEL} \
                 through every variant's own surface: name the {FAKE_MODEL}, with no `on` and no \
                 `manual`, so no variant can reach a real provider"
            ),
        )
    })
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

/// One variant of an active Journey, as the Feature report reads it
/// (ADR 0075): its ID, `J-NNN@<surface>`, the milestone it is due at, and
/// the Features its Journey covers.
pub(crate) struct CatalogVariant {
    pub(crate) id: String,
    pub(crate) milestone: Milestone,
    pub(crate) covers: Vec<String>,
}

/// Every variant of each active Journey of the working tree's catalog,
/// `foxhound-cli` included.
pub(crate) fn active_variants(root: &Path) -> Result<Vec<CatalogVariant>> {
    let journeys = working_journeys(root)?;
    Ok(journeys
        .iter()
        .filter(|journey| journey.status == Status::Active)
        .flat_map(variants::of)
        .map(|variant| CatalogVariant {
            id: variant.id(),
            milestone: variant.milestone,
            covers: variant
                .journey
                .covers
                .iter()
                .map(|feature| feature.as_str().to_owned())
                .collect(),
        })
        .collect())
}

/// `xtask journeys select`: each runner's filter for the chosen variants.
fn select_command(arguments: &Select, environment: &Environment) -> Result<ExitCode> {
    let journeys = match &arguments.tag {
        Some(tag) => tagged_journeys(&environment.root, tag)?,
        None => working_journeys(&environment.root)?,
    };
    let choice = select::Choice {
        ids: arguments.ids.clone(),
        surfaces: arguments.surfaces.clone(),
        milestones: arguments.milestones.clone(),
        areas: arguments.areas.clone(),
    };
    let active = journeys
        .iter()
        .filter(|journey| journey.status == Status::Active)
        .flat_map(variants::of)
        .collect();
    let chosen = select::chosen(active, &choice);
    if chosen.is_empty() {
        output::error("error: no variant of an active Journey matches the flags");
        return Ok(ExitCode::from(1));
    }
    if let Some(folder) = &arguments.listings {
        let carried = variant_tests(&environment.root, &environment.path(folder))?;
        let untested: Vec<String> = chosen
            .iter()
            .map(variants::Variant::id)
            .filter(|variant| !carried.contains_key(variant))
            .collect();
        for variant in &untested {
            output::error(&format!(
                "note[no-test] {variant}: no test carries it, so it runs nowhere"
            ));
        }
        if untested.len() == chosen.len() {
            output::error("error: no chosen variant has a test, so nothing would run");
            return Ok(ExitCode::from(1));
        }
    }
    let (filters, unrun) = select::filters(&chosen);
    for variant in unrun {
        output::error(&format!(
            "skipped: {variant}, since no runner runs its surface's variants yet"
        ));
    }
    for (runner, argument) in filters {
        output::line(&format!("{}\t{argument}", runner.name()));
    }
    Ok(ExitCode::SUCCESS)
}

/// The tests that carry each variant, by its ID, from the runners' listings
/// in `folder`: each as `(<runner>, <test ID>)`, the test named as its
/// runner and the results store name it. A test that carries no single
/// variant carries none here; the Journey check fails on it.
pub(crate) fn variant_tests(
    root: &Path,
    folder: &Path,
) -> Result<BTreeMap<String, Vec<(String, String)>>> {
    let mut carriers: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for test in listings::read(root, folder)? {
        let variants::Carried::Variant { journey, surface } = &test.carried else {
            continue;
        };
        let Some((runner, id)) = test.subject.split_once(' ') else {
            continue;
        };
        carriers
            .entry(format!("{journey}@{surface}"))
            .or_default()
            .push((runner.to_owned(), id.to_owned()));
    }
    Ok(carriers)
}

/// Every Journey of the working tree's catalog, which must parse.
fn working_journeys(root: &Path) -> Result<Vec<Journey>> {
    journey_files(root)?
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(root.join(&path))
                .with_context(|| format!("reading {path}"))?;
            parsed_journey(&path, &text)
        })
        .collect()
}

/// Every Journey of the catalog at a Git tag, which must parse.
fn tagged_journeys(root: &Path, tag: &str) -> Result<Vec<Journey>> {
    let git = |args: &[&str]| crate::git::text(root, args);
    let commit = format!("{tag}^{{commit}}");
    let listed = git(&["ls-tree", "--name-only", &commit, &format!("{FOLDER}/")])?;
    listed
        .lines()
        .filter(|path| {
            Path::new(path)
                .extension()
                .is_some_and(|extension| extension == "toml")
                && *path != CATALOG
        })
        .map(|path| parsed_journey(path, &git(&["show", &format!("{commit}:{path}")])?))
        .collect()
}

fn parsed_journey(path: &str, text: &str) -> Result<Journey> {
    parse(text).map(|(journey, _)| journey).map_err(|error| {
        anyhow::anyhow!("{path} does not parse ({error}); run `xtask journeys check`")
    })
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
