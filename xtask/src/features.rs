//! `xtask features` (ADRs 0072 and 0075): the Feature inventory's types and
//! generators, which `xtask codegen` runs, the scanner that reads what each
//! test declares in its runner's idiom, and the Feature check: in the Quick
//! suite without the runners' listings, in the Pre-merge suite with them.

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
use std::path::PathBuf;
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
    let (Command::Scan { listings, .. } | Command::Check { listings }) = &command;
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
    match command {
        Command::Scan { packages, .. } => {
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
        Command::Check { .. } => {
            let verdict = check::check(root, &declarations, read)?;
            for note in &verdict.notes {
                output::line(&format!(
                    "note[{}] {}: {}; `just features-tests` also reads the Vitest, Playwright and pytest declarations",
                    note.kind, note.subject, note.message
                ));
            }
            Ok(crate::check::report(&verdict.findings))
        }
    }
}

/// The inventory as `xtask codegen` writes it: pretty JSON with a final
/// newline.
pub(crate) fn inventory_text(sources: &Sources<'_>) -> Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(&inventory::generate(sources)?)?;
    text.push('\n');
    Ok(text.into_bytes())
}
