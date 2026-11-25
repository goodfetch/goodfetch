//! `recipe digest` (issue 1664): the digest of the Recipe whose package's
//! root is a folder, computed in this process as every tool computes it,
//! so that a person, a build or a registry names a version of a Recipe the
//! way a Run records it. It prints the digest alone, or under `--json` the
//! digest and the files it covers; a package that is not valid, or has no
//! portable digest, fails with `recipe_invalid` naming the path, as every
//! command's failure prints.

use std::path::PathBuf;
use std::process::ExitCode;

use super::catalog_commands::{Joined, Joining};
use super::daemon::Machine;
use super::exit_codes;
use super::output::{self, Human};
use clap::Subcommand;
use goodfetch_engine::recipe::package;
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_protocol::recipe::RecipeDigestOutput;

// The `recipe` group's commands: the catalog's `recipe` commands this group
// does not write itself, listed first, then its own. No doc comment: clap
// would make it the group's help.
#[derive(Subcommand)]
pub(super) enum RecipeCommand {
    #[command(flatten)]
    Generated(Joining<OwnRecipeCommand>),
    #[command(flatten)]
    Own(OwnRecipeCommand),
}

// The `recipe` group's hand-written commands; no doc comment either.
#[derive(Subcommand)]
pub(super) enum OwnRecipeCommand {
    /// Prints the digest of the Recipe whose package's root is DIRECTORY,
    /// which names that version of the Recipe on every system: SHA-256
    /// over the files the package holds, by their paths in Unicode NFC and
    /// their bytes as they are. It starts no engine.
    Digest {
        /// The package's root, the folder holding `goodfetch-recipe.toml`.
        directory: PathBuf,
        /// Prints the digest and the files it covers as JSON.
        #[arg(long)]
        json: bool,
    },
}

impl Joined for OwnRecipeCommand {
    const NAMESPACE: &'static str = "recipe";
}

/// Runs the `recipe` command `command`.
pub(super) fn run(
    command: RecipeCommand,
    engine: &Engine,
    dependencies: Dependencies,
    machine: &Machine<'_>,
    human: &Human,
) -> ExitCode {
    match command {
        RecipeCommand::Generated(joining) => {
            joining.call().run(engine, dependencies, machine, human)
        }
        RecipeCommand::Own(OwnRecipeCommand::Digest { directory, json }) => {
            match package(&directory) {
                Ok(package) if json => {
                    output::json(&RecipeDigestOutput {
                        digest: package.digest.to_string(),
                        files: package.files,
                    });
                    ExitCode::from(exit_codes::SUCCESS)
                }
                Ok(package) => {
                    output::line(&package.digest.to_string());
                    ExitCode::from(exit_codes::SUCCESS)
                }
                Err(refused) => output::fail(refused.to_error_object(), None, json, human),
            }
        }
    }
}
