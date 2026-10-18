//! `xtask`: Goodfetch's release and CI tool, one deep module behind a small
//! command line (spec #130). The workflows and `just` call it; it is never a
//! test-setup tool (ADR 0062). Every command works on the workspace in the
//! current folder. A check exits with 0 when it passes, 1 with its findings,
//! and 2 when it could not reach a verdict.

mod affected;
mod all_green;
mod check;
mod codegen;
mod containment;
mod dco;
mod dependency_direction;
mod egress;
mod fixture_size;
mod foxhound_side;
mod github;
mod metadata;
mod mocking_lockfile;
mod modules;
mod no_retry;
mod output;
mod package_dependencies;
mod quarantine;
mod screenshots;
mod self_hosted;
mod size;
mod tree;
mod wording;
mod workflow;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

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
    /// Regenerates every file generated from `goodfetch-protocol`'s types:
    /// the JSON Schema bundle and catalog file, each Product's projection,
    /// the names only Foxhound carries, the cargo-deny `wrappers` list and
    /// each SDK package's `generated/` layer (ADR 0038).
    Codegen(CodegenArguments),
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
    /// Fails on any setting, flag, variable or workflow pattern that retries a
    /// test (ADRs 0064 and 0074).
    NoRetry,
    /// Fails when an npm or PyPI package under `sdks/`, `adapters/` or
    /// `integrations/` depends on a package of the other Product, or on a
    /// Product's package when it is the Collie client package (ADR 0095).
    PackageDependencies,
    /// The test quarantine list, `quarantine.toml` (ADR 0064).
    Quarantine {
        #[command(subcommand)]
        command: QuarantineCommand,
    },
    /// Reference screenshots (ADR 0073).
    Screenshots {
        #[command(subcommand)]
        command: ScreenshotsCommand,
    },
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
    /// Fails on a term of `wording.toml` in the prose of any text published
    /// about either Product (ADRs 0035 and 0097).
    Wording,
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
enum ScreenshotsCommand {
    /// Applies the screenshots of a downloaded `update-screenshots` run, or
    /// fails when it was not made in the pinned image on the head.
    Fetch {
        /// The folder the run's artifact was downloaded into.
        run: PathBuf,
    },
    /// Fails on a Reference screenshot that no visual test read, after a
    /// run of every visual project.
    Orphans,
}

#[derive(Subcommand)]
enum SelfHostedCommand {
    /// Fails on any job that may reach a self-hosted runner off the allowlist.
    Check,
    /// Prints the flat copy of the allowlist that the pre-job script reads.
    Export,
}

#[derive(Subcommand)]
enum QuarantineCommand {
    /// Fails on an entry whose issue is not an open `flaky-test` issue, or
    /// whose test its runner no longer lists.
    Check {
        /// The repository whose issues the entries name, as `owner/name`.
        #[arg(long, default_value = "goodfetch/goodfetch")]
        repository: String,
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
        /// The Cargo profile nextest builds its listing with.
        #[arg(long)]
        cargo_profile: Option<String>,
    },
    /// Prints, as `<runner>\t<argument>` lines, the arguments that make each
    /// runner skip the listed tests.
    Exclude,
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
        Command::Codegen(arguments) => run_codegen(arguments, root, environment),
        Command::Containment => Ok(check::report(&containment::check(root)?)),
        Command::Dco { base, head } => Ok(check::report(&dco::check(root, &base, &head)?)),
        Command::DependencyDirection => Ok(check::report(&dependency_direction::check(
            &environment.cargo,
            root,
        )?)),
        Command::Egress { command } => run_egress(command, root),
        Command::FixtureSize => Ok(check::report(&fixture_size::check(root)?)),
        Command::MockingLockfile => Ok(check::report(&mocking_lockfile::check(root)?)),
        Command::Modules => Ok(check::report(&modules::check(&environment.cargo, root)?)),
        Command::NoRetry => Ok(check::report(&no_retry::check(root)?)),
        Command::PackageDependencies => Ok(check::report(&package_dependencies::check(root)?)),
        Command::Quarantine { command } => run_quarantine(command, root, environment),
        Command::Screenshots { command } => run_screenshots(command, root),
        Command::SelfHosted { command } => match command {
            SelfHostedCommand::Check => Ok(check::report(&self_hosted::check(root)?)),
            SelfHostedCommand::Export => {
                output::line(&self_hosted::export(root)?);
                Ok(ExitCode::SUCCESS)
            }
        },
        Command::Size { mode } => run_size(&mode, root, environment),
        Command::Wording => Ok(check::report(&wording::check(root)?)),
    }
}

/// The `size` modes (ADRs 0057 and 0087).
fn run_size(mode: &SizeMode, root: &Path, environment: &Environment) -> anyhow::Result<ExitCode> {
    match mode {
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
    }
}

#[derive(Args)]
struct CodegenArguments {
    /// Fails on any committed file that differs from what the types
    /// generate, without running the TypeScript and Python generators.
    #[arg(long)]
    check: bool,
    /// `json-schema-to-typescript`'s executable.
    #[arg(long, default_value = "node_modules/.bin/json2ts")]
    json2ts: PathBuf,
    /// `datamodel-code-generator`'s executable.
    #[arg(long, default_value = ".venv/bin/datamodel-codegen")]
    datamodel_codegen: PathBuf,
}

/// `codegen` and its `--check` (ADR 0038).
fn run_codegen(
    arguments: CodegenArguments,
    root: &Path,
    environment: &Environment,
) -> anyhow::Result<ExitCode> {
    if arguments.check {
        return Ok(check::report(&codegen::check(&environment.cargo, root)?));
    }
    let generators = codegen::Generators {
        json2ts: arguments.json2ts,
        datamodel_codegen: arguments.datamodel_codegen,
    };
    codegen::write(&environment.cargo, &generators, root)?;
    Ok(ExitCode::SUCCESS)
}

/// The `screenshots` commands (ADR 0073).
fn run_screenshots(command: ScreenshotsCommand, root: &Path) -> anyhow::Result<ExitCode> {
    match command {
        ScreenshotsCommand::Fetch { run } => {
            Ok(check::report(&screenshots::fetch(root, &root.join(run))?))
        }
        ScreenshotsCommand::Orphans => Ok(check::report(&screenshots::orphans(root)?)),
    }
}

/// The `quarantine` commands (ADR 0064).
fn run_quarantine(
    command: QuarantineCommand,
    root: &Path,
    environment: &Environment,
) -> anyhow::Result<ExitCode> {
    match command {
        QuarantineCommand::Check {
            repository,
            api_url,
            cargo_profile,
        } => {
            let sources = quarantine::Sources {
                api: github::Api {
                    url: &api_url,
                    token: environment.github_token.as_deref(),
                },
                repository: &repository,
                cargo: &environment.cargo,
                cargo_profile: cargo_profile.as_deref(),
            };
            Ok(check::report(&quarantine::check(root, &sources)?))
        }
        QuarantineCommand::Exclude => {
            for (runner, argument) in quarantine::exclusions(root)? {
                output::line(&format!("{runner}\t{argument}"));
            }
            Ok(ExitCode::SUCCESS)
        }
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
