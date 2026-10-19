//! `xtask features` (ADRs 0072 and 0075): the Feature inventory's types and
//! generators, which `xtask codegen` runs, the scanner that reads what each
//! test declares, and the Feature check of the Quick suite.

mod check;
mod contract;
mod entry;
mod files;
mod format;
mod inventory;
mod row;
mod scan;
mod story;

use std::collections::BTreeSet;
use std::process::ExitCode;

use anyhow::Result;

use crate::environment::Environment;
use crate::output;

pub(crate) use files::{INVENTORY, schemas};
pub(crate) use inventory::{FoxhoundNames, Sources};

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Prints every declaration, one `<feature>\t<level>\t<test>` line each,
    /// the test as nextest names it.
    Scan {
        /// Only the declarations of these packages.
        #[arg(long = "package")]
        packages: Vec<String>,
    },
    /// The Feature check: fails on a declaration of an unknown or retired
    /// Feature, a generated Feature with no declaring test, no exception or
    /// not one owning area, a wrong exception, a story's missing, unknown or
    /// extra `not_journey`, and a file that breaks its schema.
    Check,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let (cargo, root) = (environment.cargo.as_os_str(), environment.root.as_path());
    let declarations = scan::rust(cargo, root)?;
    match command {
        Command::Scan { packages } => {
            let packages: BTreeSet<String> = packages.into_iter().collect();
            for declaration in declarations.iter().filter(|declaration| {
                packages.is_empty() || packages.contains(&declaration.package)
            }) {
                output::line(&format!(
                    "{}\t{}\t{}",
                    declaration.feature, declaration.level, declaration.test
                ));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Check => Ok(crate::check::report(&check::check(root, &declarations)?)),
    }
}

/// The inventory as `xtask codegen` writes it: pretty JSON with a final
/// newline.
pub(crate) fn inventory_text(sources: &Sources<'_>) -> Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(&inventory::generate(sources)?)?;
    text.push('\n');
    Ok(text.into_bytes())
}
