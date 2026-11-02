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
//!
//! `--valgrind` runs the rows read under Valgrind in place of the counted ones
//! (ADR 0140's "Runs at"): each family's Callgrind and DHAT targets, whose
//! figure per unit is the difference between a scenario and the run of its
//! benchmark that sends nothing, so no start-up cost hides a change.
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
use crate::perf::{self, Figure, Measured};

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
    /// Whether its target's build embeds a Console build, as Foxhound's
    /// executable does.
    embeds_console: bool,
    /// Its benchmark under Valgrind, if it has rows read there.
    valgrind: Option<ValgrindBenchmark>,
}

/// A family's benchmark in its Callgrind and DHAT targets.
struct ValgrindBenchmark {
    /// The `#[library_benchmark]` function, which filters the run.
    function: &'static str,
    /// Its `#[bench::…]` that sends nothing, which each scenario is measured
    /// against.
    empty: &'static str,
    /// The messages each of its scenarios dispatches beyond `empty`'s.
    units: u64,
    /// Each cell, by the suffix of its `#[bench::…]`s.
    cells: &'static [(&'static str, &'static str)],
}

const B1: Family = Family {
    name: "B1",
    package: "goodfetch-foxhound",
    target: "counts",
    embeds_console: true,
    valgrind: None,
};

const B3: Family = Family {
    name: "B3",
    package: "goodfetch-engine",
    target: "counts",
    embeds_console: false,
    valgrind: Some(ValgrindBenchmark {
        function: "dispatch_messages",
        empty: "no_messages",
        units: 100,
        cells: &[(DEFAULT_CELL, ""), ("log_off", "_log_off")],
    }),
};

const B2: Family = Family {
    name: "B2",
    package: "goodfetch-page",
    target: "counts",
    embeds_console: false,
    valgrind: None,
};

const FAMILIES: [Family; 3] = [B1, B2, B3];

/// The variable a Valgrind target reads its plant from: gungraun takes no
/// argument for it.
const PLANT_VARIABLE: &str = "GOODFETCH_COUNTS_PLANT";

/// The folder under the workspace's `target/` of the Valgrind rows' reports.
const VALGRIND_OUT: &str = "target/counts";

/// The folder under the workspace's `target/` of each family's built
/// benchmark, under its family's name.
const EXECUTABLES_OUT: &str = "target/counts/executables";

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
    /// Where a row read under Valgrind gets its figure.
    valgrind: Option<ValgrindFigure>,
}

/// A figure read under Valgrind, of one scenario of its family's benchmark.
#[derive(Clone, Copy)]
struct ValgrindFigure {
    /// The bench target that measures it: `instructions` under Callgrind,
    /// `heap` under DHAT.
    target: &'static str,
    figure: Figure,
    /// The `#[bench::…]`, its cell's suffix left out.
    scenario: &'static str,
}

const fn message_row(
    name: &'static str,
    target: &'static str,
    figure: Figure,
    scenario: &'static str,
) -> Row {
    Row {
        name,
        family: "B3",
        unit: "messages",
        happens: true,
        valgrind: Some(ValgrindFigure {
            target,
            figure,
            scenario,
        }),
    }
}

/// The two scenarios of [`message_row`]s.
const SMALL: &str = "small_params";
const LARGE: &str = "large_params";

/// Every row with a counter. Adding one here, with its counter and its
/// plant in the family's benchmark, is what gives a row its command.
const ROWS: [Row; 26] = [
    Row {
        name: "F1.parses",
        family: "B1",
        unit: "pages",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F1.selector_compilations",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.readability_runs",
        family: "B1",
        unit: "pages",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F1.file_opens",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.locale_form_builds",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.plugin_serializations",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.plugin_answer_parses",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E1.deep_copies",
        family: "B3",
        unit: "incoming_messages",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E1.encodes",
        family: "B3",
        unit: "outgoing_messages",
        happens: true,
        valgrind: None,
    },
    message_row(
        "E1.instructions",
        "instructions",
        Figure::Instructions,
        SMALL,
    ),
    message_row(
        "E1.instructions_large",
        "instructions",
        Figure::Instructions,
        LARGE,
    ),
    message_row("E1.allocations", "heap", Figure::TotalAllocations, SMALL),
    message_row(
        "E1.allocations_large",
        "heap",
        Figure::TotalAllocations,
        LARGE,
    ),
    message_row("E1.bytes_allocated", "heap", Figure::TotalBytes, SMALL),
    message_row(
        "E1.bytes_allocated_large",
        "heap",
        Figure::TotalBytes,
        LARGE,
    ),
    Row {
        name: "F4.declaration_reads",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F4.validator_builds",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F4.validations",
        family: "B1",
        unit: "records",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F4.serializations",
        family: "B1",
        unit: "records",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F4.deep_copies",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F4.constant_lookups",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F4.transactions",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C1.encodes",
        family: "B2",
        unit: "commands",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C1.sends",
        family: "B2",
        unit: "commands",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C1.file_opens",
        family: "B2",
        unit: "commands",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C1.log_sends_off",
        family: "B2",
        unit: "commands",
        happens: false,
        valgrind: None,
    },
];

/// A row of a Shared path whose code does not exist yet: it holds its limit
/// and says it has not run, until the ticket that builds the path counts its
/// work and moves it to [`ROWS`].
struct NotRunYet {
    /// Its name in the register, and the name its benchmark prints it under.
    name: &'static str,
    /// The family that will count it.
    family: &'static str,
    /// The ticket that builds its path.
    builder: &'static str,
}

const NOT_RUN_YET: [NotRunYet; 11] = [
    NotRunYet {
        name: "F5.messages_no_hook",
        family: "B1",
        builder: "#1481",
    },
    NotRunYet {
        name: "F5.calls",
        family: "B1",
        builder: "#1481",
    },
    NotRunYet {
        name: "F5.builds",
        family: "B1",
        builder: "#1481",
    },
    NotRunYet {
        name: "F6.opens",
        family: "B1",
        builder: "#1121",
    },
    NotRunYet {
        name: "F6.transactions",
        family: "B1",
        builder: "#1121",
    },
    NotRunYet {
        name: "F6.writer_reads",
        family: "B1",
        builder: "#1121",
    },
    NotRunYet {
        name: "C6.file_opens",
        family: "B2",
        builder: "#959",
    },
    NotRunYet {
        name: "C6.compilations",
        family: "B2",
        builder: "#959",
    },
    NotRunYet {
        name: "C6.spawns",
        family: "B2",
        builder: "#959",
    },
    NotRunYet {
        name: "C6.acknowledgements",
        family: "B2",
        builder: "#959",
    },
    NotRunYet {
        name: "C6.seals",
        family: "B2",
        builder: "#959",
    },
];

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// Only this family's rows (B1, B2, B3).
    #[arg(long)]
    family: Option<String>,
    /// Only this path's rows (F1, C1, C6).
    #[arg(long)]
    path: Option<String>,
    /// Only this row, by its name in the register (F1.parses).
    #[arg(long)]
    row: Option<String>,
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
    /// Runs the rows read under Valgrind, in place of the counted ones.
    #[arg(long, conflicts_with = "executable")]
    valgrind: bool,
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
    let named: Vec<&str> = arguments
        .executable
        .iter()
        .filter_map(|named| named.split_once('=').map(|(family, _)| family))
        .collect();
    let rows: Vec<&Row> = ROWS
        .iter()
        .filter(|row| row.valgrind.is_some() == arguments.valgrind)
        // A family named by its executable is what this run asks for.
        .filter(|row| named.is_empty() || named.contains(&row.family))
        .filter(|row| arguments.asks_for(row.family, row.name))
        .collect();
    // A row not run yet is a counted row: no Valgrind run holds it.
    let not_run_yet: Vec<&NotRunYet> = NOT_RUN_YET
        .iter()
        .filter(|_| !arguments.valgrind)
        .filter(|row| arguments.asks_for(row.family, row.name))
        .collect();
    if rows.is_empty() && not_run_yet.is_empty() {
        bail!("no counted row matches the family, path and row asked for");
    }
    if let Some(planted) = &arguments.plant
        && !rows.iter().any(|row| row.name == planted)
    {
        if let Some(row) = not_run_yet.iter().find(|row| row.name == planted) {
            bail!("{planted} has not run yet: {} builds its path", row.builder);
        }
        bail!("{planted} is no counted row of what runs");
    }
    for row in &not_run_yet {
        let limit = limits
            .get(row.name)
            .with_context(|| format!("the Quality register has no row {}", row.name))?;
        output::line(&format!(
            "{}: not run yet, limit {limit}, until {} builds its path",
            row.name, row.builder
        ));
    }
    let executables = if arguments.valgrind {
        BTreeMap::new()
    } else {
        executables(arguments, environment, &rows)?
    };
    if arguments.prove {
        return prove(&rows, &executables, &limits, arguments, environment);
    }
    let mut findings = Vec::new();
    let mut families: Vec<&str> = rows.iter().map(|row| row.family).collect();
    families.sort_unstable();
    families.dedup();
    for family in families {
        let of_family: Vec<&Row> = rows
            .iter()
            .copied()
            .filter(|row| row.family == family)
            .collect();
        // A family's benchmark plants only its own rows.
        let plant = arguments
            .plant
            .as_deref()
            .filter(|planted| of_family.iter().any(|row| row.name == *planted));
        let cells = family_cells(family, &rows, &executables, plant, arguments, environment)?;
        findings.extend(verdict(&of_family, &cells, &limits)?);
        findings.extend(
            not_run_yet
                .iter()
                .filter(|row| row.family == family)
                .filter_map(|row| done_though_not_run(row, &cells)),
        );
    }
    Ok(check::report(&findings))
}

/// Plants one extra unit of each of `rows`' work in turn and fails each
/// counter that does not see it.
fn prove(
    rows: &[&Row],
    executables: &BTreeMap<&'static str, PathBuf>,
    limits: &BTreeMap<String, Limit>,
    arguments: &Arguments,
    environment: &Environment,
) -> Result<ExitCode> {
    let mut findings = Vec::new();
    // A Valgrind plant does the same work whichever row it names, so one
    // run of a family's targets proves all its rows.
    let mut planted: BTreeMap<&str, Vec<Cell>> = BTreeMap::new();
    for row in rows {
        if !planted.contains_key(row.family) || !arguments.valgrind {
            let cells = family_cells(
                row.family,
                rows,
                executables,
                Some(row.name),
                arguments,
                environment,
            )?;
            planted.insert(row.family, cells);
        }
        let caught = verdict(&[row], &planted[row.family], limits)?
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
    Ok(check::report(&findings))
}

/// What `family`'s rows counted, each extra unit of `plant` done.
fn family_cells(
    family: &str,
    rows: &[&Row],
    executables: &BTreeMap<&'static str, PathBuf>,
    plant: Option<&str>,
    arguments: &Arguments,
    environment: &Environment,
) -> Result<Vec<Cell>> {
    if arguments.valgrind {
        valgrind_cells(family, rows, plant, environment)
    } else {
        cells(&executables[family], plant)
    }
}

/// Runs `family`'s Valgrind targets for `rows` and answers each cell's
/// figures, as a counted benchmark prints its own.
fn valgrind_cells(
    family: &str,
    rows: &[&Row],
    plant: Option<&str>,
    environment: &Environment,
) -> Result<Vec<Cell>> {
    let Some(of_family) = FAMILIES.iter().find(|of| of.name == family) else {
        bail!("no family {family}");
    };
    let Some(benchmark) = &of_family.valgrind else {
        bail!("the family {family} has no benchmark under Valgrind");
    };
    let figures: Vec<(&Row, ValgrindFigure)> = rows
        .iter()
        .filter(|row| row.family == family)
        .filter_map(|row| row.valgrind.map(|figure| (*row, figure)))
        .collect();
    let mut targets: Vec<&str> = figures.iter().map(|(_, figure)| figure.target).collect();
    targets.sort_unstable();
    targets.dedup();
    let mut measured = Vec::new();
    for target in targets {
        measured.extend(run_valgrind_target(
            of_family,
            benchmark,
            target,
            plant,
            environment,
        )?);
    }
    let read = |target: &str, id: String, figure: Figure| {
        let end = format!("{}/{id}", benchmark.function);
        measured
            .iter()
            .find(|one: &&Measured| {
                one.figure == figure
                    && one.subject.ends_with(&end)
                    && one.subject.contains(&format!("/{target}::"))
            })
            .map(|one| one.head)
    };
    Ok(benchmark
        .cells
        .iter()
        .map(|(name, suffix)| Cell {
            name: (*name).to_owned(),
            units: BTreeMap::from([("messages".to_owned(), benchmark.units)]),
            counts: figures
                .iter()
                .filter_map(|(row, figure)| {
                    let scenario = read(
                        figure.target,
                        format!("{}{suffix}", figure.scenario),
                        figure.figure,
                    )?;
                    let empty = read(
                        figure.target,
                        format!("{}{suffix}", benchmark.empty),
                        figure.figure,
                    )?;
                    Some((row.name.to_owned(), scenario.saturating_sub(empty)))
                })
                .collect(),
        })
        .collect())
}

/// Runs one Valgrind target's benchmark, each extra unit of `plant` done, and
/// answers its figures.
fn run_valgrind_target(
    family: &Family,
    benchmark: &ValgrindBenchmark,
    target: &str,
    plant: Option<&str>,
    environment: &Environment,
) -> Result<Vec<Measured>> {
    let out = environment.root.join(VALGRIND_OUT);
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let report = out.join(format!("{}-{target}.jsonl", family.name));
    std::fs::File::create(&report).with_context(|| format!("creating {}", report.display()))?;
    let mut command = Command::new(&environment.cargo);
    command
        .args(["bench", "--locked", "--package", family.package])
        .args(["--bench", target, "--"])
        .arg(format!("*::{}::*", benchmark.function))
        .current_dir(&environment.root)
        .env("GUNGRAUN_HOME", out.join("gungraun"))
        .stderr(Stdio::inherit());
    if let Some(plant) = plant {
        command.env(PLANT_VARIABLE, plant);
    }
    let status = perf::gungraun_output(&mut command, &report)?
        .status()
        .with_context(|| format!("running {}'s {target} benchmark", family.name))?;
    if !status.success() {
        bail!("{}'s {target} benchmark failed", family.name);
    }
    perf::read_gungraun_summaries(&[&report])
}

impl Arguments {
    /// Whether the family, path and row asked for hold the row `name` of
    /// `family`.
    fn asks_for(&self, family: &str, name: &str) -> bool {
        self.family.as_deref().is_none_or(|asked| asked == family)
            && self
                .path
                .as_deref()
                .is_none_or(|path| name.split('.').next() == Some(path))
            && self.row.as_deref().is_none_or(|asked| asked == name)
    }
}

/// A finding when a cell did the work of `row`, which says it has not run: the
/// ticket that builds its path gives it its limit's verdict by moving it.
fn done_though_not_run(row: &NotRunYet, cells: &[Cell]) -> Option<Finding> {
    let (cell, count) = cells.iter().find_map(|cell| {
        let count = *cell.counts.get(row.name)?;
        (count > 0).then_some((cell, count))
    })?;
    Some(Finding::new(
        "row-listed-as-not-run",
        row.name,
        format!(
            "the cell `{}` counted {count} of its work: move it to `ROWS` in `xtask counts`",
            cell.name
        ),
    ))
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
    if family.embeds_console
        && environment
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
    let built = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Message>(line).ok())
        .find_map(|message| message.executable)
        .with_context(|| format!("cargo build reported no executable for {}", family.name))?;
    // Every family's example is `counts`, which Cargo leaves at one path, so
    // the next family's build would replace this one before it runs.
    let own = environment
        .root
        .join(EXECUTABLES_OUT)
        .join(format!("{}-{}", family.name, family.target));
    if let Some(folder) = own.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::copy(&built, &own)
        .with_context(|| format!("copying {} to {}", built.display(), own.display()))?;
    Ok(own)
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
            let limit_held = if row.valgrind.is_some() {
                limit.with_valgrind_tolerance()
            } else {
                *limit
            };
            if limit_held.exceeded_by(*count, *units) {
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
