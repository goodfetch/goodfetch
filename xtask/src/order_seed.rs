//! The order seed every suite runs its tests in (ADR 0064; TESTING.md,
//! Randomness): the first 8 bytes of the tree under test, read as an
//! unsigned integer, so an order repeats with its tree. `ci/order-seed.sh`
//! gives the same value where no Cargo runs.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};

use crate::environment::Environment;
use crate::output;

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The tree, or a commit of it.
    #[arg(long, default_value = "HEAD")]
    tree: String,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    output::line(&seed(&environment.root, &arguments.tree)?.to_string());
    Ok(ExitCode::SUCCESS)
}

/// The order seed of `tree`, a tree or anything that names one (a commit).
fn seed(root: &Path, tree: &str) -> Result<u64> {
    let hash = crate::git::text(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{tree}^{{tree}}"),
        ],
    )?;
    let prefix = hash
        .trim()
        .get(..16)
        .with_context(|| format!("`{}` is no tree hash", hash.trim()))?;
    u64::from_str_radix(prefix, 16).with_context(|| format!("`{prefix}` is not hexadecimal"))
}
