//! `xtask`: Goodfetch's release and CI tool, one deep module behind a small
//! command line (spec #130). The workflows and `just` call it; it is never a
//! test-setup tool (ADR 0062). Every command works on the workspace in the
//! current folder. A check exits with 0 when it passes, 1 with its findings,
//! and 2 when it could not reach a verdict.

mod all_green;
mod check;
mod dependency_direction;
mod egress;
mod fixture_size;
mod github;
mod metadata;
mod mocking_lockfile;
mod modules;
mod output;
mod self_hosted;
mod size;
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
    /// ADR 0043's outbound-traffic deny in CI and its Egress canary.
    Egress {
        #[command(subcommand)]
        command: EgressCommand,
    },
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
    /// Each Product's Size budget (ADRs 0057 and 0087): measures built files
    /// and writes one table of their sizes.
    Size {
        #[command(subcommand)]
        mode: SizeMode,
    },
}

#[derive(Subcommand)]
enum SizeMode {
    /// The pull request's report, compared with `main`'s; never fails.
    Estimate {
        #[command(flatten)]
        files: SizeFiles,
        /// `main`'s report, when there is one to compare with.
        #[arg(long)]
        main_report: Option<PathBuf>,
        #[arg(long, default_value = "origin/main")]
        main: String,
    },
    /// Fails on a file grown past its increase threshold over `main`'s
    /// report, unless the pull request accepts it, and on a crate tarball over
    /// its ceiling.
    Increase {
        #[command(flatten)]
        files: SizeFiles,
        /// `main`'s report, saved for the tree of `--main`.
        #[arg(long)]
        main_report: PathBuf,
        /// The reference to `main`'s head, whose tree the report must have.
        #[arg(long, default_value = "origin/main")]
        main: String,
        /// The pull request whose "Size increase" section and acceptance
        /// comment a tripped file needs.
        #[arg(long, requires = "repository")]
        pull_request: Option<u64>,
        /// The repository, as `owner/name`.
        #[arg(long, requires = "pull_request")]
        repository: Option<String>,
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
    },
    /// Fails on a release-profile file over its Product's ceiling.
    Ceilings {
        #[command(flatten)]
        files: SizeFiles,
    },
}

#[derive(clap::Args)]
struct SizeFiles {
    /// The TOML manifest naming each built file's Product, target, kind and
    /// path.
    #[arg(long)]
    files: PathBuf,
    /// Saves the measured sizes as a report keyed by the tree of `HEAD`,
    /// which a later run reads as `main`'s (ADR 0061).
    #[arg(long)]
    save: Option<PathBuf>,
}

#[derive(Subcommand)]
enum SelfHostedCommand {
    /// Fails on any job that may reach a self-hosted runner off the allowlist.
    Check,
    /// Prints the flat copy of the allowlist that the pre-job script reads.
    Export,
}

#[derive(Subcommand)]
enum EgressCommand {
    /// Fails on a workflow job not declared in `ci/egress/endpoints.toml`,
    /// and on a job under the deny whose steps do not deny before its tests,
    /// run the Egress canary right after, and check the deny's log at the end.
    Check,
    /// Prints what a job under the deny may reach, one `<host> <port>` line
    /// each: the pinned artifact mirror and the job's named endpoints.
    Endpoints {
        /// The job, as `<workflow file>#<job id>`.
        #[arg(long)]
        job: String,
    },
    /// Prints the nftables ruleset that denies a job's outbound traffic but
    /// to its allowed endpoints, at the addresses resolved before the deny.
    Rules {
        /// The job, as `<workflow file>#<job id>`.
        #[arg(long)]
        job: String,
        /// A file of `<host> <port> <address>` lines, resolved in the VM.
        #[arg(long)]
        resolved: PathBuf,
        /// The VM user that runs the job's container, so the product.
        #[arg(long)]
        uid: u32,
    },
    /// The Egress canary: a forbidden connection that must be refused.
    Canary {
        #[command(subcommand)]
        command: CanaryCommand,
    },
    /// Fails on every connection the deny refused but the canary's, read
    /// from the deny's counters and its kernel log at the job's end.
    Report {
        /// The output of `nft --json list counters table inet goodfetch_egress`.
        #[arg(long)]
        counters: PathBuf,
        /// The VM's kernel log.
        #[arg(long)]
        log: PathBuf,
    },
}

#[derive(Subcommand)]
enum CanaryCommand {
    /// Tries the canary's connection and writes how it ended.
    Attempt {
        #[arg(long)]
        outcome: PathBuf,
    },
    /// Passes when the attempt did not connect and the deny counted it.
    Verdict {
        /// The file `attempt` wrote.
        #[arg(long)]
        outcome: PathBuf,
        /// The deny's counters before the attempt, as `nft --json` lists them.
        #[arg(long)]
        before: PathBuf,
        /// The deny's counters after the attempt.
        #[arg(long)]
        after: PathBuf,
    },
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
        Command::Egress { command } => run_egress(command, root),
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
        Command::Size { mode } => match &mode {
            SizeMode::Estimate {
                files,
                main_report,
                main,
            } => {
                let main = main_report.as_deref().map(|report| size::Main {
                    report,
                    reference: main,
                });
                size::run(
                    &size::Mode::Estimate { main },
                    root,
                    &files.files,
                    files.save.as_deref(),
                )
            }
            SizeMode::Increase {
                files,
                main_report,
                main,
                pull_request,
                repository,
                api_url,
            } => {
                let main = size::Main {
                    report: main_report,
                    reference: main,
                };
                let pull_request =
                    pull_request
                        .zip(repository.as_deref())
                        .map(|(number, repository)| size::PullRequest {
                            api: github::Api {
                                url: api_url,
                                token: environment.github_token.as_deref(),
                            },
                            repository,
                            number,
                        });
                let mode = size::Mode::Increase { main, pull_request };
                size::run(&mode, root, &files.files, files.save.as_deref())
            }
            SizeMode::Ceilings { files } => size::run(
                &size::Mode::Ceilings,
                root,
                &files.files,
                files.save.as_deref(),
            ),
        },
    }
}

/// The `egress` commands (ADR 0043).
fn run_egress(command: EgressCommand, root: &Path) -> anyhow::Result<ExitCode> {
    match command {
        EgressCommand::Check => Ok(check::report(&egress::check(root)?)),
        EgressCommand::Endpoints { job } => {
            for endpoint in egress::allowed(root, &job)? {
                output::line(&format!("{} {}", endpoint.host, endpoint.port));
            }
            Ok(ExitCode::SUCCESS)
        }
        EgressCommand::Rules { job, resolved, uid } => {
            output::line(egress::rules(root, &job, &root.join(resolved), uid)?.trim_end());
            Ok(ExitCode::SUCCESS)
        }
        EgressCommand::Canary { command } => match command {
            CanaryCommand::Attempt { outcome } => {
                egress::attempt(root, &root.join(outcome))?;
                Ok(ExitCode::SUCCESS)
            }
            CanaryCommand::Verdict {
                outcome,
                before,
                after,
            } => Ok(check::report(&egress::verdict(
                root,
                &root.join(outcome),
                &root.join(before),
                &root.join(after),
            )?)),
        },
        EgressCommand::Report { counters, log } => Ok(check::report(&egress::report(
            root,
            &root.join(counters),
            &root.join(log),
        )?)),
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
