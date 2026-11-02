//! `xtask counts` (ADR 0140): runs a family's benchmark or a path's rows,
//! reads the Counted work each cell did, divides each count by the units of
//! the outcome that did the work, reads each row's limit from `TESTING.md`'s
//! Quality register, prints every figure and fails over a limit, naming the
//! row, its figure and its limit. It is the "Checked by" of every counted
//! row, and runs at implementer done (`just counts`) and in the merge gate's
//! `perf` step.
//!
//! `--plant <row>` has the benchmark do one extra unit of the row's work, as
//! `--prove` does for every row and expects the failure: each counter proves
//! it can fail (the register's rules).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

mod register;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::check::{self, Finding};
use crate::console;
use crate::environment::Environment;
use crate::output;

use register::Limit;

/// A benchmark family: the one target whose run reads every row it counts.
struct Family {
    /// Its name in ADR 0140.
    name: &'static str,
    /// The package holding its target.
    package: &'static str,
    /// Its example target: `<target> [--plant <row>]` prints one JSON line
    /// per cell.
    target: &'static str,
}

const B1: Family = Family {
    name: "B1",
    package: "goodfetch-foxhound",
    target: "counts",
};

const FAMILIES: [Family; 1] = [B1];

/// One counted row of the register.
struct Row {
    /// Its name in the register, `<path>.<work>`, and the name its
    /// benchmark prints its count under.
    name: &'static str,
    /// The family that counts it.
    family: &'static str,
    /// The key of the units of the outcome that does the work, in its
    /// benchmark's line.
    unit: &'static str,
    /// Whether the default cell must do this work: a benchmark that did
    /// none of what its row counts proves nothing. A row whose contract is
    /// 0 says no; its plant still proves its counter.
    happens: bool,
}

/// Every row with a counter. Adding one here, with its counter and its
/// plant in the family's benchmark, is what gives a row its command.
const ROWS: [Row; 1] = [Row {
    name: "F1.parses",
    family: "B1",
    unit: "pages",
    happens: true,
}];

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// Only this family's rows (B1).
    #[arg(long)]
    family: Option<String>,
    /// Only this path's rows (F1).
    #[arg(long)]
    path: Option<String>,
    /// Has the benchmark do one extra unit of this row's work.
    #[arg(long, conflicts_with = "prove")]
    plant: Option<String>,
    /// Plants one extra unit of each row's work in turn and fails unless
    /// each is caught, naming its row.
    #[arg(long)]
    prove: bool,
    /// Runs this executable as the family `<family>=<path>` names, in place
    /// of building its target.
    #[arg(long, value_name = "FAMILY=PATH")]
    executable: Vec<String>,
}

/// The cell that runs with every default feature on.
const DEFAULT_CELL: &str = "default";

/// One line a benchmark prints: what one cell of it counted.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cell {
    /// `default`, or the feature it runs with off.
    #[serde(rename = "cell")]
    name: String,
    /// The units of each outcome.
    units: BTreeMap<String, u64>,
    /// Each row's whole count.
    counts: BTreeMap<String, u64>,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    let limits = register::limits(root)?;
    let rows: Vec<&Row> = ROWS
        .iter()
        .filter(|row| {
            arguments
                .family
                .as_deref()
                .is_none_or(|family| family == row.family)
        })
        .filter(|row| {
            arguments
                .path
                .as_deref()
                .is_none_or(|path| row.name.split('.').next() == Some(path))
        })
        .collect();
    if rows.is_empty() {
        bail!("no counted row matches the family and path asked for");
    }
    if let Some(planted) = &arguments.plant
        && !rows.iter().any(|row| row.name == planted)
    {
        bail!("{planted} is no counted row of what runs");
    }
    let executables = executables(arguments, environment, &rows)?;
    if arguments.prove {
        let mut findings = Vec::new();
        for row in &rows {
            let cells = cells(&executables[row.family], Some(row.name))?;
            let caught = verdict(&[row], &cells, &limits)?
                .iter()
                .any(|finding| finding.kind == "over-limit" && finding.subject == row.name);
            output::line(&format!(
                "{}: one planted unit {}",
                row.name,
                if caught { "fails" } else { "passes unseen" }
            ));
            if !caught {
                findings.push(Finding::new(
                    "counter-blind",
                    row.name,
                    "one planted unit of its work did not fail it",
                ));
            }
        }
        return Ok(check::report(&findings));
    }
    let mut findings = Vec::new();
    let mut families: Vec<&str> = rows.iter().map(|row| row.family).collect();
    families.dedup();
    for family in families {
        let cells = cells(&executables[family], arguments.plant.as_deref())?;
        let of_family: Vec<&Row> = rows
            .iter()
            .copied()
            .filter(|row| row.family == family)
            .collect();
        findings.extend(verdict(&of_family, &cells, &limits)?);
    }
    Ok(check::report(&findings))
}

/// Each family's executable: the one `--executable` names, else its built
/// target.
fn executables(
    arguments: &Arguments,
    environment: &Environment,
    rows: &[&Row],
) -> Result<BTreeMap<&'static str, PathBuf>> {
    let named: BTreeMap<&str, &str> = arguments
        .executable
        .iter()
        .filter_map(|named| named.split_once('='))
        .collect();
    let mut executables = BTreeMap::new();
    for family in FAMILIES {
        if !rows.iter().any(|row| row.family == family.name) {
            continue;
        }
        let executable = match named.get(family.name) {
            Some(path) => environment.path(Path::new(path)),
            None => build(&family, environment)?,
        };
        executables.insert(family.name, executable);
    }
    Ok(executables)
}

/// The one message of Cargo's JSON output this command reads.
#[derive(Deserialize)]
struct Message {
    executable: Option<PathBuf>,
}

/// Builds `family`'s target and answers its executable.
fn build(family: &Family, environment: &Environment) -> Result<PathBuf> {
    // The package's dev-dependency on itself turns its executable on, which
    // embeds its Console build (ADR 0057).
    if environment
        .root
        .join(console::PACKAGE)
        .join("package.json")
        .is_file()
    {
        console::build(&environment.root)?;
    }
    let output = Command::new(&environment.cargo)
        .args(["build", "--message-format", "json-render-diagnostics"])
        .args(["-p", family.package, "--example", family.target])
        .current_dir(&environment.root)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("building {}'s benchmark", family.name))?;
    if !output.status.success() {
        bail!("cargo build of {}'s benchmark failed", family.name);
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Message>(line).ok())
        .find_map(|message| message.executable)
        .with_context(|| format!("cargo build reported no executable for {}", family.name))
}

/// What `executable` counted, each extra unit of `plant` done.
fn cells(executable: &Path, plant: Option<&str>) -> Result<Vec<Cell>> {
    let mut command = Command::new(executable);
    if let Some(plant) = plant {
        command.args(["--plant", plant]);
    }
    let output = command
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("running {}", executable.display()))?;
    if !output.status.success() {
        bail!("the benchmark {} failed", executable.display());
    }
    let cells = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<Cell>, _>>()
        .with_context(|| format!("reading what {} printed", executable.display()))?;
    if cells.is_empty() {
        bail!("the benchmark {} printed no cell", executable.display());
    }
    Ok(cells)
}

/// Prints each row's figure in each of `cells` and answers what fails.
fn verdict(
    rows: &[&Row],
    cells: &[Cell],
    limits: &BTreeMap<String, Limit>,
) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for row in rows {
        let limit = limits
            .get(row.name)
            .with_context(|| format!("the Quality register has no row {}", row.name))?;
        for cell in cells {
            let (Some(count), Some(units)) = (cell.counts.get(row.name), cell.units.get(row.unit))
            else {
                findings.push(Finding::new(
                    "row-not-counted",
                    row.name,
                    format!(
                        "the cell `{}` printed no count or no {}",
                        cell.name, row.unit
                    ),
                ));
                continue;
            };
            if *units == 0 || (row.happens && *count == 0 && cell.name == DEFAULT_CELL) {
                findings.push(Finding::new(
                    "work-not-done",
                    row.name,
                    format!(
                        "the cell `{}` did none of the work it counts: {count} over {units} {}",
                        cell.name, row.unit
                    ),
                ));
                continue;
            }
            let figure = Limit::figure(*count, *units);
            output::line(&format!(
                "{} [{}]: {figure} per {}, limit {limit} ({count} over {units})",
                row.name,
                cell.name,
                row.unit.trim_end_matches('s'),
            ));
            if limit.exceeded_by(*count, *units) {
                findings.push(Finding::new(
                    "over-limit",
                    row.name,
                    format!(
                        "{figure} per {} in the cell `{}`, over its limit of {limit}",
                        row.unit.trim_end_matches('s'),
                        cell.name
                    ),
                ));
            }
        }
    }
    Ok(findings)
}
