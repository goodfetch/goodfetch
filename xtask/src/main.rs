//! `xtask`: Goodfetch's release and CI tool, one deep module behind a small
//! command line (spec #130). The workflows and `just` call it; it is never a
//! test-setup tool (ADR 0062). Every command works on the workspace in the
//! current folder. A check exits with 0 when it passes, 1 with its findings,
//! and 2 when it could not reach a verdict.
//!
//! Each command's arguments and dispatch live in its module; this file names
//! the commands and reads the environment. The `results` commands span the
//! store's readers (`allure`, `ceiling`, `scan`), so their dispatch is here.

mod acceptance;
mod affected;
mod all_green;
mod allure;
mod ceiling;
mod check;
mod codegen;
mod compat;
mod containment;
mod dco;
mod dependency_direction;
mod egress;
mod environment;
mod fixture_size;
mod foxhound_side;
mod github;
mod issues;
mod license_allowlist;
mod metadata;
mod mocking_lockfile;
mod modules;
mod no_retry;
mod notices;
mod output;
mod package_dependencies;
mod perf;
mod quarantine;
mod release_suite;
mod repeat;
mod results;
mod scan;
mod screenshots;
mod self_hosted;
mod size;
mod tree;
mod wording;
mod workflow;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use environment::Environment;

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
    Affected(affected::Arguments),
    /// The "all green" check: every needed job succeeded, and no earlier
    /// attempt of the run had a failed test step (ADR 0064).
    AllGreen(all_green::Arguments),
    /// Regenerates every file generated from `goodfetch-protocol`'s types:
    /// the JSON Schema bundle and catalog file, each Product's projection,
    /// the names only Foxhound carries, the cargo-deny `wrappers` list and
    /// each SDK package's `generated/` layer (ADR 0038).
    Codegen(codegen::Arguments),
    /// The Compatibility corpus under `compat/` (ADR 0066).
    Compat {
        #[command(subcommand)]
        command: compat::Command,
    },
    /// Fails unless every generated contract of Collie's is in Foxhound's,
    /// unchanged (ADR 0094).
    Containment,
    /// Fails on a commit from `--base` to `--head`, merges aside, without a
    /// Developer Certificate of Origin sign-off by its author (ADR 0018).
    Dco {
        /// The pull request's base commit.
        #[arg(long)]
        base: String,
        /// The pull request's head commit.
        #[arg(long)]
        head: String,
    },
    /// Fails on a dependency against the workspace's enforced directions.
    DependencyDirection,
    /// ADR 0043's outbound-traffic deny in CI and its Egress canary.
    Egress {
        #[command(subcommand)]
        command: egress::Command,
    },
    /// Fails on any file over 256 KiB under a `fixtures/` folder (ADR 0062).
    FixtureSize,
    /// Fails when `uv.lock` holds a mocking library such as pytest-mock (ADR 0062).
    MockingLockfile,
    /// Fails on an import between top-level modules that a crate's
    /// `modules.toml` does not allow, and on a long file that does not say
    /// why it is one concern (ADR 0126).
    Modules,
    /// Fails on any setting, flag, variable or workflow pattern that retries a
    /// test (ADRs 0064 and 0074).
    NoRetry,
    /// Builds the third-party notice inventory: every dependency of Cargo,
    /// npm and Python, and every input each `third-party.toml` declares,
    /// with its license and where its source is; fails on a license ADR
    /// 0018's allowlist refuses for its use (ADRs 0018, 0043 and 0047).
    Notices(notices::Arguments),
    /// Fails when an npm or PyPI package under `sdks/`, `adapters/` or
    /// `integrations/` depends on a package of the other Product, or on a
    /// Product's package when it is the Collie client package (ADR 0095).
    PackageDependencies,
    /// The Performance budget of a change (ADR 0071): counted figures against
    /// the base, measured in one job.
    Perf {
        #[command(subcommand)]
        command: perf::Command,
    },
    /// The test quarantine list, `quarantine.toml` (ADR 0064).
    Quarantine {
        #[command(subcommand)]
        command: quarantine::Command,
    },
    /// The Release suite's manual-dispatch runs while the repository is
    /// private (ADR 0108).
    ReleaseSuite {
        #[command(subcommand)]
        command: release_suite::Command,
    },
    /// Prints, as `<runner>\t<argument>` lines, the arguments that make each
    /// runner run the chosen tests `--count` times, whatever each run's
    /// result (ADR 0064): the diagnostic runs and the daily test-health job.
    Repeat(repeat::Arguments),
    /// The results store, `test-results` (ADR 0064).
    Results {
        #[command(subcommand)]
        command: ResultsCommand,
    },
    /// Reference screenshots (ADR 0073).
    Screenshots {
        #[command(subcommand)]
        command: screenshots::Command,
    },
    /// The self-hosted allowlist (ADRs 0108 and 0135).
    SelfHosted {
        #[command(subcommand)]
        command: self_hosted::Command,
    },
    /// Each Product's Size budget (ADRs 0057 and 0087): measures built files
    /// and writes one table of their sizes.
    Size {
        #[command(subcommand)]
        mode: size::Command,
    },
    /// Fails on a term of `wording.toml` in the prose of any text published
    /// about either Product (ADRs 0035 and 0097).
    Wording,
}

#[derive(Subcommand)]
enum ResultsCommand {
    /// Writes the inputs of the Allure Report 3 build from the store: each
    /// test's latest result, one history point per run attempt and the
    /// report's configuration (ADR 0064).
    Allure {
        /// A checkout of the results store.
        #[arg(long)]
        store: PathBuf,
        /// The folder to write, emptied first; run `allure generate results
        /// --config allurerc.json` in it.
        #[arg(long)]
        out: PathBuf,
    },
    /// Prints `minutes=<n>` (the month's Linux-equivalent minutes of the
    /// test-health ceiling) and `admitted=<true|false>` for a run about to
    /// start, and writes a skip to `--summary` (ADRs 0064, 0072 and 0108).
    Ceiling {
        /// A checkout of the results store.
        #[arg(long)]
        store: PathBuf,
        #[arg(long, value_enum)]
        run: ceiling::Run,
        /// Today's date in UTC, `YYYY-MM-DD`, whose month is counted.
        #[arg(long)]
        today: jiff::civil::Date,
        /// The job summary, `$GITHUB_STEP_SUMMARY`.
        #[arg(long)]
        summary: Option<PathBuf>,
    },
    /// Prints the tests that failed in a run attempt, for its diagnostic
    /// runs (ADR 0064): `matrix=<JSON>`, one entry each, at most 20, and
    /// `left_out=<n>`, the failed tests past those.
    Failed {
        #[command(flatten)]
        attempt: AttemptArguments,
        /// Only the tests that failed on this OS.
        #[arg(long, value_enum)]
        os: Option<results::Os>,
        /// Only the tests that failed on this architecture.
        #[arg(long, value_enum)]
        arch: Option<results::Arch>,
    },
    /// Writes one run attempt into the store: its JUnit files as written,
    /// one record per test run and one per job.
    Ingest {
        #[command(flatten)]
        attempt: AttemptArguments,
        /// A checkout of the results store.
        #[arg(long)]
        store: PathBuf,
    },
    /// Prints `<run> <attempt>` for every attempt of the runs that finished
    /// since `--since` and that the store lacks, for the daily ingest (ADR 0135).
    Pending {
        /// A checkout of the results store.
        #[arg(long)]
        store: PathBuf,
        /// The first day, UTC, `YYYY-MM-DD`.
        #[arg(long)]
        since: jiff::civil::Date,
        #[command(flatten)]
        github: github::Arguments,
    },
    /// Prints `commit=<sha>`, the one commit a downloaded run's JUnit
    /// artifacts name as tested; fails when they name none or several.
    TestedCommit {
        /// The folder the run's artifacts were downloaded into, one folder each.
        #[arg(long)]
        artifacts: PathBuf,
    },
    /// Opens or reopens one `flaky-test` issue per test that passed and
    /// failed on one tree hash and configuration, and opens or updates one
    /// `live-check` issue per Live check that failed or was inconclusive
    /// three times in a row.
    Scan {
        /// A checkout of the results store.
        #[arg(long)]
        store: PathBuf,
        /// The repository whose issues the scan keeps, and GitHub's API.
        #[command(flatten)]
        github: github::Arguments,
    },
}

/// One run attempt of the repository and the folder of its downloaded
/// artifacts.
#[derive(clap::Args)]
struct AttemptArguments {
    /// The repository of the run, and GitHub's API.
    #[command(flatten)]
    github: github::Arguments,
    #[arg(long)]
    run_id: u64,
    #[arg(long)]
    attempt: u32,
    /// The folder the run's artifacts were downloaded into, one folder each.
    #[arg(long)]
    artifacts: PathBuf,
}

impl AttemptArguments {
    fn source<'a>(
        &'a self,
        environment: &'a Environment,
        artifacts: &'a PathBuf,
    ) -> results::Source<'a> {
        results::Source {
            api: self.github.api(environment.token()),
            repository: &self.github.repository,
            run: self.run_id,
            attempt: self.attempt,
            artifacts,
        }
    }
}

fn run(command: Command, environment: &Environment) -> anyhow::Result<ExitCode> {
    let root = environment.root.as_path();
    let cargo = environment.cargo.as_os_str();
    match command {
        Command::Affected(arguments) => affected::run(arguments, environment),
        Command::AllGreen(arguments) => all_green::run(&arguments, environment),
        Command::Codegen(arguments) => codegen::run(arguments, environment),
        Command::Compat { command } => compat::run(command, environment),
        Command::Containment => Ok(check::report(&containment::check(root)?)),
        Command::Dco { base, head } => Ok(check::report(&dco::check(root, &base, &head)?)),
        Command::DependencyDirection => {
            Ok(check::report(&dependency_direction::check(cargo, root)?))
        }
        Command::Egress { command } => egress::run(command, environment),
        Command::FixtureSize => Ok(check::report(&fixture_size::check(root)?)),
        Command::MockingLockfile => Ok(check::report(&mocking_lockfile::check(root)?)),
        Command::Modules => Ok(check::report(&modules::check(cargo, root)?)),
        Command::NoRetry => Ok(check::report(&no_retry::check(root)?)),
        Command::Notices(arguments) => notices::run(&arguments, environment),
        Command::PackageDependencies => Ok(check::report(&package_dependencies::check(root)?)),
        Command::Perf { command } => perf::run(command, environment),
        Command::Quarantine { command } => quarantine::run(command, environment),
        Command::ReleaseSuite { command } => release_suite::run(command, environment),
        Command::Repeat(arguments) => repeat::run(arguments, environment),
        Command::Results { command } => run_results(command, environment),
        Command::Screenshots { command } => screenshots::run(command, environment),
        Command::SelfHosted { command } => self_hosted::run(command, environment),
        Command::Size { mode } => size::run(&mode, environment),
        Command::Wording => Ok(check::report(&wording::check(root)?)),
    }
}

/// The `results` commands (ADR 0064).
fn run_results(command: ResultsCommand, environment: &Environment) -> anyhow::Result<ExitCode> {
    let path = |path: &PathBuf| environment.path(path);
    match command {
        ResultsCommand::Allure { store, out } => {
            allure::write(&path(&store), &path(&out))?;
            Ok(ExitCode::SUCCESS)
        }
        ResultsCommand::Ceiling {
            store,
            run,
            today,
            summary,
        } => {
            let summary = summary.as_ref().map(path);
            ceiling::check(&path(&store), run, today, summary.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
        ResultsCommand::Failed { attempt, os, arch } => {
            let artifacts = path(&attempt.artifacts);
            let platform = results::Platform { os, arch };
            print_failed(&attempt.source(environment, &artifacts), &platform)
        }
        ResultsCommand::Ingest { attempt, store } => {
            let artifacts = path(&attempt.artifacts);
            let written = results::ingest(&attempt.source(environment, &artifacts), &path(&store))?;
            output::line(&format!("wrote {}", written.display()));
            Ok(ExitCode::SUCCESS)
        }
        ResultsCommand::Pending {
            store,
            since,
            github,
        } => {
            let api = github.api(environment.token());
            for (run, attempt) in results::pending(&api, &github.repository, &path(&store), since)?
            {
                output::line(&format!("{run} {attempt}"));
            }
            Ok(ExitCode::SUCCESS)
        }
        ResultsCommand::TestedCommit { artifacts } => {
            let commit = results::tested_commit(&path(&artifacts))?;
            output::line(&format!("commit={commit}"));
            Ok(ExitCode::SUCCESS)
        }
        ResultsCommand::Scan { store, github } => Ok(check::report(&scan::scan(
            &environment.root,
            &path(&store),
            &github.api(environment.token()),
            &github.repository,
        )?)),
    }
}

/// Prints a failed attempt's tests as a diagnostic job matrix (ADR 0064).
fn print_failed(
    source: &results::Source<'_>,
    platform: &results::Platform,
) -> anyhow::Result<ExitCode> {
    // A run with more failures than this is broken, not flaky; the rest are
    // only counted, for the job summary.
    const DIAGNOSED_AT_MOST: usize = 20;
    let mut failed = results::failed(source, platform)?;
    let left_out = failed.split_off(failed.len().min(DIAGNOSED_AT_MOST));
    output::line(&format!("count={}", failed.len()));
    output::line(&format!(
        "matrix={}",
        serde_json::json!({ "include": failed })
    ));
    output::line(&format!("left_out={}", left_out.len()));
    Ok(ExitCode::SUCCESS)
}

#[expect(
    clippy::disallowed_methods,
    reason = "main is the one place that reads the environment (ADR 0059)"
)]
fn read_environment() -> Environment {
    Environment {
        root: PathBuf::from("."),
        cargo: std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")),
        github_token: std::env::var("GITHUB_TOKEN")
            .ok()
            .filter(|token| !token.is_empty()),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command, &read_environment()) {
        Ok(code) => code,
        Err(error) => {
            output::error(&format!("error: {error:#}"));
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    #[test]
    fn every_command_definition_is_consistent() {
        super::Cli::command().debug_assert();
    }
}
