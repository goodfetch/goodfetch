//! `xtask features` (ADRs 0072 and 0075): the Feature inventory's types and
//! generators, which `xtask codegen` runs, the scanner that reads what each
//! test declares in its runner's idiom, the Feature check: in the Quick
//! suite without the runners' listings, in the Pre-merge suite with them,
//! and the exercise audit of what each declaring test reached.

mod audit;
mod block_cause;
mod check;
mod command;
mod contract;
mod covers;
mod entry;
mod files;
mod format;
mod inventory;
mod listed;
mod row;
mod scan;
mod screen;
mod story;

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;

use crate::environment::Environment;
use crate::output;

pub(crate) use command::{CommandLines, CommandTree, SECTION as COMMANDS};
pub(crate) use files::{INVENTORY, schemas};
pub(crate) use inventory::{FoxhoundNames, Sources};

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Prints every declaration, one `<feature>\t<level>\t<source>\t<test>`
    /// line each: the source is `nextest`, `vitest`, `playwright`, `pytest`,
    /// `journey` or `conformance`, and the test is named as its runner names
    /// it.
    Scan {
        /// Only the declarations of these packages: crates, npm or Python
        /// packages.
        #[arg(long = "package")]
        packages: Vec<String>,
        #[command(flatten)]
        listings: Listings,
    },
    /// The Feature check: fails on a declaration of an unknown or retired
    /// Feature, a generated Feature with no declaring test, no exception or
    /// not one owning area, a wrong exception, a story's missing, unknown or
    /// extra `not_journey`, and a file that breaks its schema. Without
    /// `--listings`, a generated Feature no Rust test, Journey or conformance
    /// case declares is only noted.
    Check {
        #[command(flatten)]
        listings: Listings,
    },
    /// The exercise audit: fails a Rust test that declares an `operation:`,
    /// `error:`, `event:`, `command:` or `exit:` Feature its exercise record
    /// never shows it reached, or that left no record. With `--junit`, over
    /// the tests those files show ran (the Pre-merge suite); without it,
    /// over every declaring test (the Release suite).
    Audit {
        /// The folder of the exercise records `just test` has test support
        /// write, one file per test.
        #[arg(long, default_value = "target/test-records")]
        records: PathBuf,
        /// nextest's JUnit files of the run, whose tests that ran are the
        /// ones audited.
        #[arg(long)]
        junit: Vec<PathBuf>,
    },
}

#[derive(clap::Args)]
pub(crate) struct Listings {
    /// The folder of the runners' listings `xtask journeys listings` writes,
    /// whose Vitest, Playwright and pytest declarations are read too.
    #[arg(long = "listings")]
    folder: Option<PathBuf>,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let (cargo, root) = (environment.cargo.as_os_str(), environment.root.as_path());
    match command {
        Command::Scan { packages, listings } => {
            let (declarations, _) = declarations(cargo, root, &listings)?;
            let packages: BTreeSet<String> = packages.into_iter().collect();
            for declaration in declarations.iter().filter(|declaration| {
                packages.is_empty()
                    || declaration
                        .package
                        .as_ref()
                        .is_some_and(|package| packages.contains(package))
            }) {
                output::line(&format!(
                    "{}\t{}\t{}\t{}",
                    declaration.feature, declaration.level, declaration.source, declaration.test
                ));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Check { listings } => {
            let (declarations, read) = declarations(cargo, root, &listings)?;
            let verdict = check::check(root, &declarations, read)?;
            for note in &verdict.notes {
                output::line(&format!(
                    "note[{}] {}: {}; `just features-tests` also reads the Vitest, Playwright and pytest declarations",
                    note.kind, note.subject, note.message
                ));
            }
            Ok(crate::check::report(&verdict.findings))
        }
        Command::Audit { records, junit } => {
            let declarations = scan::rust(cargo, root)?;
            let ran = if junit.is_empty() {
                None
            } else {
                Some(audit::tests_that_ran(root, &junit)?)
            };
            let findings = audit::audit(root, &declarations, &root.join(records), ran.as_ref())?;
            Ok(crate::check::report(&findings))
        }
    }
}

/// Every declaration the scanner reads, the runners' listings' too when
/// they are given, sorted, with which of them the check reads.
fn declarations(
    cargo: &OsStr,
    root: &Path,
    listings: &Listings,
) -> Result<(Vec<scan::Declaration>, check::Declarations)> {
    let mut declarations = scan::rust(cargo, root)?;
    declarations.extend(covers::read(root)?);
    let read = match &listings.folder {
        Some(folder) => {
            declarations.extend(listed::read(root, &root.join(folder))?);
            check::Declarations::All
        }
        None => check::Declarations::WithoutListings,
    };
    declarations.sort();
    declarations.dedup();
    Ok((declarations, read))
}

/// The inventory as `xtask codegen` writes it: pretty JSON with a final
/// newline.
pub(crate) fn inventory_text(sources: &Sources<'_>) -> Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(&inventory::generate(sources)?)?;
    text.push('\n');
    Ok(text.into_bytes())
}
