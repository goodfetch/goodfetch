//! `xtask`: Goodfetch's release and CI tool, one deep module behind a small
//! command line (spec #130). The workflows and `just` call it; it is never a
//! test-setup tool (ADR 0062). Every command works on the workspace in the
//! current folder. A check exits with 0 when it passes, 1 with its findings,
//! and 2 when it could not reach a verdict.

mod affected;
mod all_green;
mod check;
mod dependency_direction;
mod fixture_size;
mod foxhound_side;
mod metadata;
mod mocking_lockfile;
mod modules;
mod output;
mod self_hosted;
mod workflow;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "Goodfetch's release and CI tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Prints which jobs of the pull-request tier a change needs and what
    /// each test runner selects, as `key=value` lines (ADRs 0032 and 0061).
    #[command(group(clap::ArgGroup::new("change").required(true).args(["base", "local"])))]
    Affected {
        /// The change's base, for CI: the change is what `--head` changed
        /// since its merge base with it.
        #[arg(long, requires = "head")]
        base: Option<String>,
        #[arg(long, requires = "base")]
        head: Option<String>,
        /// The working tree, uncommitted and untracked files included,
        /// against the merge base with `origin/main`.
        #[arg(long, conflicts_with_all = ["base", "head"])]
        local: bool,
    },
    /// The "all green" check: every needed job succeeded, and no earlier
    /// attempt of the run had a failed test step (ADR 0064).
    AllGreen {
        /// A file holding the workflow's `${{ toJSON(needs) }}`.
        #[arg(long)]
        needs: PathBuf,
        /// The repository, as `owner/name`.
        #[arg(long)]
        repository: String,
        #[arg(long)]
        run_id: u64,
        /// This run attempt's number.
        #[arg(long)]
        attempt: u32,
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
    },
    /// Fails on a dependency against the workspace's enforced directions.
    DependencyDirection,
    /// Fails on any file over 256 KiB under a `fixtures/` folder (ADR 0062).
    FixtureSize,
    /// Fails when `uv.lock` holds a mocking library such as pytest-mock (ADR 0062).
    MockingLockfile,
    /// Fails on an import between top-level modules that a crate's
    /// `modules.toml` does not allow, and on a long file that does not say
    /// why it is one concern (ADR 0126).
    Modules,
    /// The self-hosted allowlist (ADRs 0108 and 0135).
    SelfHosted {
        #[command(subcommand)]
        command: SelfHostedCommand,
    },
}

#[derive(Subcommand)]
enum SelfHostedCommand {
    /// Fails on any job that may reach a self-hosted runner off the allowlist.
    Check,
    /// Prints the flat copy of the allowlist that the pre-job script reads.
    Export,
}

/// What `main` reads from the environment, the only place that does (ADR 0059).
struct Environment {
    /// The Cargo that runs `xtask`, or the one on `PATH`.
    cargo: OsString,
    /// `GITHUB_TOKEN`, for GitHub's API; never taken on the command line.
    github_token: Option<String>,
}

fn run(command: Command, root: &Path, environment: &Environment) -> anyhow::Result<ExitCode> {
    match command {
        Command::Affected { base, head, local } => {
            // clap requires `--local` or both of `--base` and `--head`.
            let range = match (local, base, head) {
                (true, _, _) => affected::Range::WorkingTree,
                (false, Some(base), Some(head)) => affected::Range::Commits { base, head },
                (false, _, _) => anyhow::bail!("give --base and --head, or --local"),
            };
            let selection = affected::select(&environment.cargo, root, &range)?;
            output::line(selection.to_string().trim_end());
            Ok(ExitCode::SUCCESS)
        }
        Command::AllGreen {
            needs,
            repository,
            run_id,
            attempt,
            api_url,
        } => {
            let run = all_green::Run {
                api_url: &api_url,
                token: environment.github_token.as_deref(),
                repository: &repository,
                id: run_id,
                attempt,
            };
            Ok(check::report(&all_green::check(&root.join(needs), &run)?))
        }
        Command::DependencyDirection => Ok(check::report(&dependency_direction::check(
            &environment.cargo,
            root,
        )?)),
        Command::FixtureSize => Ok(check::report(&fixture_size::check(root)?)),
        Command::MockingLockfile => Ok(check::report(&mocking_lockfile::check(root)?)),
        Command::Modules => Ok(check::report(&modules::check(&environment.cargo, root)?)),
        Command::SelfHosted { command } => match command {
            SelfHostedCommand::Check => Ok(check::report(&self_hosted::check(root)?)),
            SelfHostedCommand::Export => {
                output::line(&self_hosted::export(root)?);
                Ok(ExitCode::SUCCESS)
            }
        },
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "main is the one place that reads the environment (ADR 0059)"
)]
fn read_environment() -> Environment {
    Environment {
        cargo: std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")),
        github_token: std::env::var("GITHUB_TOKEN")
            .ok()
            .filter(|token| !token.is_empty()),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command, Path::new("."), &read_environment()) {
        Ok(code) => code,
        Err(error) => {
            output::error(&format!("error: {error:#}"));
            ExitCode::from(2)
        }
    }
}
