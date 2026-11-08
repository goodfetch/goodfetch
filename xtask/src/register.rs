//! `xtask register check`, in `just lint`: keeps TESTING.md's Quality register
//! honest (ADR 0140). It reads the register's tables and ADR 0140's inventory,
//! and fails on an enforced row whose command does not exist, an expired
//! Exception, a path or a count of a Cost contract with no row, a row above
//! its contract, and a difference from `xtask perf`'s ceilings. What it cannot
//! read is a finding, so a reworded table never passes as an empty one.

#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod commands;
mod contract;
mod coverage;
pub(crate) mod document;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::Subcommand;
use jiff::civil::Date;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::perf;

use contract::SharedPath;
use document::Register;

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Fails on a register that drifts from ADR 0140 or `xtask perf`.
    Check {
        /// Today's date in UTC, `YYYY-MM-DD`; the system's when left out.
        #[arg(long)]
        today: Option<Date>,
    },
    /// Compares the coverage job's reports with the `coverage.project` row
    /// and keeps one issue on a fall (ADR 0072).
    Coverage(coverage::Arguments),
}

/// The folder holding ADR 0140, whose file name starts with `ADR_PREFIX`.
const ADR_FOLDER: &str = "docs/adr";
const ADR_PREFIX: &str = "0140-";
/// The subject of a finding on ADR 0140's inventory.
const ADR: &str = "ADR 0140";

const UNREADABLE: &str = "register-unreadable";

pub(crate) fn run(
    command: &Command,
    environment: &Environment,
    xtask: &clap::Command,
) -> Result<ExitCode> {
    let today = match command {
        Command::Check { today } => today,
        Command::Coverage(arguments) => return coverage::run(arguments, environment),
    };
    let today = today.unwrap_or_else(|| {
        jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .date()
    });
    Ok(check::report(&findings(&environment.root, today, xtask)?))
}

fn findings(root: &Path, today: Date, xtask: &clap::Command) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let register = Register::read(&read(&root.join(document::FILE))?);
    let inventory = contract::read(&read(&adr_file(root)?)?);
    if register.is_none() {
        findings.push(Finding::new(
            UNREADABLE,
            document::FILE,
            "the Quality register's Enforced, Measured or Exceptions table is missing or empty",
        ));
    }
    if inventory.is_none() {
        findings.push(Finding::new(
            UNREADABLE,
            ADR,
            "the table of \"The Shared paths\" is missing or empty",
        ));
    }
    let Some(register) = register else {
        return Ok(findings);
    };
    let justfile = read(&root.join("justfile"))?;
    findings.extend(enforced_commands(&register, root, &justfile, xtask));
    findings.extend(exceptions(&register, today));
    findings.extend(ceilings(&register));
    if let Some(paths) = inventory {
        findings.extend(contracts(&register, &paths));
    }
    Ok(findings)
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn adr_file(root: &Path) -> Result<PathBuf> {
    let folder = root.join(ADR_FOLDER);
    std::fs::read_dir(&folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(ADR_PREFIX))
        })
        .with_context(|| format!("{} holds no {ADR_PREFIX}*", folder.display()))
}

fn enforced_commands(
    register: &Register,
    root: &Path,
    justfile: &str,
    xtask: &clap::Command,
) -> Vec<Finding> {
    register
        .enforced
        .iter()
        .flat_map(|row| {
            document::code_spans(row.cell("Checked by"))
                .into_iter()
                .filter(|command| !commands::exists(command, root, justfile, xtask))
                .map(|command| {
                    Finding::new(
                        "command-missing",
                        row.name(),
                        format!(
                            "`{command}` is no recipe of the justfile, command of xtask, file or text a recipe runs"
                        ),
                    )
                })
        })
        .collect()
}

fn exceptions(register: &Register, today: Date) -> Vec<Finding> {
    register
        .exceptions
        .iter()
        .filter_map(|row| {
            let expires = row.cell("Expires");
            match expires.parse::<Date>() {
                Err(_) => Some(Finding::new(
                    UNREADABLE,
                    row.name(),
                    format!("its expiry \"{expires}\" is no date"),
                )),
                Ok(date) if date < today => Some(Finding::new(
                    "exception-expired",
                    row.name(),
                    format!("it expired on {date}: remove the gap, or ask the owner to extend it"),
                )),
                Ok(_) => None,
            }
        })
        .collect()
}

/// One row of a path, with what the contract check compares.
struct Held<'a> {
    row: &'a str,
    stems: BTreeSet<String>,
    cap: Option<u32>,
}

/// Each path's rows against its Cost contract. A count is held by the rows of
/// its path whose Metric holds every word of the count; it is met when one of
/// them allows no more than its cap, and a row allows no more than the largest
/// cap among the counts it holds.
fn contracts(register: &Register, paths: &[SharedPath]) -> Vec<Finding> {
    let rows = register.rows();
    let mut findings = Vec::new();
    let mut above = figures_above_contract(register);
    for path in paths {
        let prefix = format!("{}.", path.id);
        let own: Vec<Held<'_>> = rows
            .iter()
            .filter_map(|row| {
                let counter = row.counter()?;
                counter.starts_with(&prefix).then(|| Held {
                    row: counter,
                    stems: contract::stems(row.metric()),
                    cap: document::cap(row.limit()),
                })
            })
            .collect();
        if own.is_empty() {
            findings.push(Finding::new(
                "path-without-row",
                &path.id,
                "this path of ADR 0140 has no row in the register",
            ));
            continue;
        }
        let mut largest: BTreeMap<&str, u32> = BTreeMap::new();
        for count in &path.counts {
            let holders: Vec<&Held<'_>> = own
                .iter()
                .filter(|held| count.stems.is_subset(&held.stems))
                .collect();
            if holders.is_empty() {
                findings.push(Finding::new(
                    "count-without-row",
                    format!("{}/{}", path.id, count.label),
                    format!(
                        "no row's Metric names this count of the Cost contract (at most {})",
                        count.cap
                    ),
                ));
            }
            let capped: Vec<(&str, u32)> = holders
                .iter()
                .filter_map(|held| held.cap.map(|cap| (held.row, cap)))
                .collect();
            for (row, _) in &capped {
                let entry = largest.entry(row).or_insert(count.cap);
                *entry = (*entry).max(count.cap);
            }
            if capped.iter().all(|(_, cap)| *cap > count.cap) {
                for (row, cap) in capped {
                    above.entry(row).or_insert_with(|| {
                        format!(
                            "it allows {cap}, and ADR 0140 allows at most {} {}",
                            count.cap, count.label
                        )
                    });
                }
            }
        }
        for held in &own {
            if let (Some(cap), Some(allowed)) = (held.cap, largest.get(held.row))
                && cap > *allowed
            {
                above.entry(held.row).or_insert_with(|| {
                    format!("it allows {cap}, and ADR 0140's contract allows at most {allowed}")
                });
            }
        }
    }
    findings.extend(
        above
            .into_iter()
            .map(|(row, message)| Finding::new("row-above-contract", row, message)),
    );
    findings
}

/// Each counted row, measured or enforced, whose Today figure is over its own
/// Contract with no Exception naming it: a gap below the contract is an
/// Exception (ADR 0140).
fn figures_above_contract(register: &Register) -> BTreeMap<&str, String> {
    let waived: BTreeSet<&str> = register
        .exceptions
        .iter()
        .flat_map(|row| document::code_spans(row.cell("Rule")))
        .collect();
    register
        .rows()
        .into_iter()
        .filter_map(|row| {
            let counter = row.counter()?;
            let cap = document::cap(row.limit())?;
            let today = document::figure(row.cell("Today"))?;
            (today > f64::from(cap) && !waived.contains(counter)).then(|| {
                (
                    counter,
                    format!(
                        "its figure {today} is over its contract's {cap}, and no Exception names it"
                    ),
                )
            })
        })
        .collect()
}

/// Each of `xtask perf`'s ceilings against the row of its benchmark, both
/// ways: the register is the source of the numbers (ADR 0140).
fn ceilings(register: &Register) -> Vec<Finding> {
    const DIFFERENCE: &str = "ceiling-difference";
    let limits: Vec<(&str, u64)> = perf::ceiling_limits().collect();
    let mut findings = Vec::new();
    for (subject, max) in &limits {
        let row = register.measured.iter().find(|row| {
            document::code_spans(row.cell("Benchmark"))
                .iter()
                .any(|span| names(subject, span))
        });
        let Some(row) = row else {
            findings.push(Finding::new(
                DIFFERENCE,
                *subject,
                format!("`xtask perf` holds this ceiling at {max}, and no row names its benchmark"),
            ));
            continue;
        };
        if document::cap(row.limit()).map(u64::from) != Some(*max) {
            findings.push(Finding::new(
                DIFFERENCE,
                *subject,
                format!(
                    "`xtask perf` holds this ceiling at {max}, and {} says {}",
                    row.name(),
                    row.limit()
                ),
            ));
        }
    }
    for row in &register.measured {
        let unheld = document::code_spans(row.cell("Benchmark"))
            .into_iter()
            .any(|span| {
                (span.starts_with("heap::") || span.starts_with("size-limit/"))
                    && !limits.iter().any(|(subject, _)| names(subject, span))
            });
        if unheld {
            findings.push(Finding::new(
                DIFFERENCE,
                row.name(),
                "its benchmark has no ceiling in `xtask perf`",
            ));
        }
    }
    findings
}

/// Whether `span`, a row's Benchmark, names the ceiling's `subject`: whole, or
/// one of its `/`-separated parts.
fn names(subject: &str, span: &str) -> bool {
    subject == span || subject.split('/').any(|part| part == span)
}
