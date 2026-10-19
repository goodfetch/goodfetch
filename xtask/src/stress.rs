//! The stress run before merge (ADR 0064; TESTING.md, Before merge), and the
//! order seed every suite runs its tests in.
//! - **seed**: the first 8 bytes of the tree under test, read as an unsigned
//!   integer. `ci/order-seed.sh` gives the same value where no Cargo runs.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::path::Path;
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result, bail};

use crate::environment::Environment;
use crate::output;

/// The order seed of `tree`, a tree or anything that names one (a commit).
pub(crate) fn seed(root: &Path, tree: &str) -> Result<u64> {
    let hash = git(
        root,
        &["rev-parse", "--verify", "--end-of-options"],
        &[&format!("{tree}^{{tree}}")],
    )?;
    let prefix = hash
        .trim()
        .get(..16)
        .with_context(|| format!("`{}` is no tree hash", hash.trim()))?;
    u64::from_str_radix(prefix, 16).with_context(|| format!("`{prefix}` is not hexadecimal"))
}

/// Runs git with `args` then `operands` in `root`, and returns its output.
fn git(root: &Path, args: &[&str], operands: &[&str]) -> Result<String> {
    let output = Process::new("git")
        .args(args)
        .args(operands)
        .current_dir(root)
        .output()
        .context("running git")?;
    if !output.status.success() {
        bail!(
            "git {} {} failed: {}",
            args.join(" "),
            operands.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("reading git's output")
}

/// `xtask stress`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Prints the order seed of a tree: the first 8 bytes of its hash, read
    /// as an unsigned integer.
    Seed {
        /// The tree, or a commit of it.
        #[arg(long, default_value = "HEAD")]
        tree: String,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Seed { tree } => {
            output::line(&seed(&environment.root, &tree)?.to_string());
            Ok(ExitCode::SUCCESS)
        }
    }
}
