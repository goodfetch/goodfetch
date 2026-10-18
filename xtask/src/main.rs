//! `xtask`: Goodfetch's release and CI tool, one deep module behind a small
//! command line (spec #130). The workflows and `just` call it; it is never a
//! test-setup tool (ADR 0062). Every command works on the workspace in the
//! current folder. A check exits with 0 when it passes, 1 with its findings,
//! and 2 when it could not reach a verdict.
//!
//! Size: one concern, the command line: every command's `clap` definition and
//! the plumbing that hands its arguments to the module that does the work.

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
    /// The Compatibility corpus under `compat/` (ADR 0066).
    Compat {
        #[command(subcommand)]
        command: CompatCommand,
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
    /// Builds the third-party notice inventory: every dependency of Cargo,
    /// npm and Python, and every input each `third-party.toml` declares,
    /// with its license and where its source is; fails on a license ADR
    /// 0018's allowlist refuses for its use (ADRs 0018, 0043 and 0047).
    Notices {
        /// A further file of inputs in `third-party.toml`'s format, such as
        /// the list a Goodfetch Chromium build writes.
        #[arg(long)]
        inputs: Vec<PathBuf>,
        /// Writes the inventory, as JSON, to this file.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Fails when an npm or PyPI package under `sdks/`, `adapters/` or
    /// `integrations/` depends on a package of the other Product, or on a
    /// Product's package when it is the Collie client package (ADR 0095).
    PackageDependencies,
    /// The test quarantine list, `quarantine.toml` (ADR 0064).
    Quarantine {
        #[command(subcommand)]
        command: QuarantineCommand,
    },
    /// The Release suite's manual-dispatch runs while the repository is
    /// private (ADR 0108).
    ReleaseSuite {
        #[command(subcommand)]
        command: ReleaseSuiteCommand,
    },
    /// The Performance budget of a change (ADR 0071): counted figures against
    /// the base, measured in one job.
    Perf {
        #[command(subcommand)]
        command: perf::Command,
    },
    /// Prints, as `<runner>\t<argument>` lines, the arguments that make each
    /// runner run the chosen tests `--count` times, whatever each run's
    /// result (ADR 0064): the diagnostic runs and the daily test-health job.
    #[command(group(clap::ArgGroup::new("tests").required(true).args(["test", "quarantined"])))]
    Repeat {
        #[arg(long)]
        count: std::num::NonZeroU32,
        /// A test, as `<runner>=<test ID>` (the quarantine list's IDs).
        #[arg(long)]
        test: Vec<String>,
        /// The quarantine list's oldest entries, at most this many.
        #[arg(long)]
        quarantined: Option<usize>,
    },
    /// The results store, `test-results` (ADR 0064).
    Results {
        #[command(subcommand)]
        command: ResultsCommand,
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
enum CompatCommand {
    /// Regenerates each registered format's `head/` set by running its
    /// writer, the `compat-<format>` example of the package that owns it.
    Write,
    /// Fails on a `head/` set that differs from what `write` produces, a
    /// changed frozen byte, a missing set, incomplete schema coverage, and a
    /// schema changed without a version bump.
    Check,
    /// Copies each format's `head/` set from a stable release's or patch's
    /// tag, not from the working tree, to `compat/<format>/<major>.<minor>/`
    /// with its `SHA256SUMS`, for every shipped version not frozen yet. A
    /// release candidate's or nightly's tag freezes nothing.
    Freeze {
        /// The release tag, `vX.Y.Z`.
        #[arg(long)]
        tag: String,
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
enum ReleaseSuiteCommand {
    /// The Release workflow's first job: checks the commit, chooses the
    /// cells to start and holds the hosted ones back when the run would pass
    /// the month's ceiling of included minutes; writes the job's outputs and
    /// summary.
    Plan {
        /// The repository, as `owner/name`.
        #[arg(long)]
        repository: String,
        /// This run's ID.
        #[arg(long)]
        run_id: u64,
        /// `GITHUB_REF`, the branch the workflow was dispatched on.
        #[arg(long = "ref")]
        git_ref: String,
        /// `all`, or cell names separated by commas.
        #[arg(long, default_value = "all")]
        cells: String,
        /// On `main`, a commit of `main`; its head when empty.
        #[arg(long)]
        commit: Option<String>,
        /// The file of the job's outputs (`GITHUB_OUTPUT`).
        #[arg(long)]
        outputs: PathBuf,
        /// The job's summary (`GITHUB_STEP_SUMMARY`).
        #[arg(long)]
        summary: PathBuf,
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
    },
    /// Fails unless every cell is green on the commit, from one or more
    /// dispatches; a failed test step in any attempt leaves it red.
    Verdict {
        /// The repository, as `owner/name`.
        #[arg(long)]
        repository: String,
        /// The commit of `main` the runs tested.
        #[arg(long)]
        commit: String,
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
    },
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
        /// The repository of the run, as `owner/name`.
        #[arg(long, default_value = "goodfetch/goodfetch")]
        repository: String,
        #[arg(long)]
        run_id: u64,
        #[arg(long)]
        attempt: u32,
        /// The folder the run's artifacts were downloaded into, one folder each.
        #[arg(long)]
        artifacts: PathBuf,
        /// Only the tests that failed on this OS (`linux`, `macos`, `windows`).
        #[arg(long)]
        os: Option<String>,
        /// Only the tests that failed on this architecture (`x64`, `arm64`).
        #[arg(long)]
        arch: Option<String>,
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
    },
    /// Writes one run attempt into the store: its JUnit files as written,
    /// one record per test run and one per job.
    Ingest {
        /// The repository of the run, as `owner/name`.
        #[arg(long, default_value = "goodfetch/goodfetch")]
        repository: String,
        #[arg(long)]
        run_id: u64,
        #[arg(long)]
        attempt: u32,
        /// The folder the run's artifacts were downloaded into, one folder each.
        #[arg(long)]
        artifacts: PathBuf,
        /// A checkout of the results store.
        #[arg(long)]
        store: PathBuf,
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
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
        /// The repository of the runs, as `owner/name`.
        #[arg(long, default_value = "goodfetch/goodfetch")]
        repository: String,
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
    },
    /// Opens or reopens one `flaky-test` issue per test that passed and
    /// failed on one tree hash and configuration, and opens or updates one
    /// `live-check` issue per Live check that failed or was inconclusive
    /// three times in a row.
    Scan {
        /// A checkout of the results store.
        #[arg(long)]
        store: PathBuf,
        /// The repository whose issues the scan keeps, as `owner/name`.
        #[arg(long, default_value = "goodfetch/goodfetch")]
        repository: String,
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
    },
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
    /// With `--windows`, prints the Windows Firewall block rules instead, one
    /// `<protocol> <remote addresses> <remote ports>` line each.
    #[command(group(clap::ArgGroup::new("enforcer").required(true).args(["uid", "windows"])))]
    Rules {
        /// The job, as `<workflow file>#<job id>`.
        #[arg(long)]
        job: String,
        /// A file of `<host> <port> <address>` lines, resolved before the deny.
        #[arg(long)]
        resolved: PathBuf,
        /// The user that runs the product, in the job's VM or network namespace.
        #[arg(long)]
        uid: Option<u32>,
        /// The rules of the Windows cell's firewall.
        #[arg(long)]
        windows: bool,
    },
    /// The Egress canary: a forbidden connection that must be refused.
    Canary {
        #[command(subcommand)]
        command: CanaryCommand,
    },
    /// Fails on every connection the deny refused but the canary's, read
    /// from the deny's counters and its kernel log at the job's end.
    #[command(group(clap::ArgGroup::new("refusals").required(true).args(["counters", "firewall_log"])))]
    Report {
        /// The output of `nft --json list counters table inet goodfetch_egress`.
        #[arg(long, requires = "log")]
        counters: Option<PathBuf>,
        /// The kernel log of the deny's VM or runner.
        #[arg(long, requires = "counters")]
        log: Option<PathBuf>,
        /// The Windows firewall's log of dropped packets.
        #[arg(long)]
        firewall_log: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum CanaryCommand {
    /// Tries the canary's connection and writes how it ended.
    Attempt {
        #[arg(long)]
        outcome: PathBuf,
    },
    /// Passes when the attempt did not connect and the deny recorded it.
    #[command(group(clap::ArgGroup::new("record").required(true).args(["before", "firewall_log", "sandbox"])))]
    Verdict {
        /// The file `attempt` wrote.
        #[arg(long)]
        outcome: PathBuf,
        /// The deny's counters before the attempt, as `nft --json` lists them.
        #[arg(long, requires = "after")]
        before: Option<PathBuf>,
        /// The deny's counters after the attempt.
        #[arg(long, requires = "before")]
        after: Option<PathBuf>,
        /// The Windows firewall's log of dropped packets.
        #[arg(long)]
        firewall_log: Option<PathBuf>,
        /// The attempt ran under macOS's sandbox, whose refusal is its record.
        #[arg(long)]
        sandbox: bool,
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
        Command::Compat { command } => match command {
            CompatCommand::Write => {
                compat::write(&environment.cargo, root)?;
                Ok(ExitCode::SUCCESS)
            }
            CompatCommand::Check => Ok(check::report(&compat::check(&environment.cargo, root)?)),
            CompatCommand::Freeze { tag } => {
                match compat::freeze(root, &tag)? {
                    compat::Freeze::Prerelease => output::line(&format!(
                        "{tag} is a release candidate or a nightly: nothing to freeze"
                    )),
                    compat::Freeze::Frozen(sets) if sets.is_empty() => {
                        output::line(&format!("every version {tag} shipped is frozen already"));
                    }
                    compat::Freeze::Frozen(sets) => {
                        for set in sets {
                            output::line(&format!("froze {set} from {tag}"));
                        }
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
        },
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
        Command::Notices { inputs, out } => {
            let inputs: Vec<PathBuf> = inputs.iter().map(|path| root.join(path)).collect();
            let sources = notices::Sources {
                cargo: &environment.cargo,
                inputs: &inputs,
            };
            let out = out.map(|path| root.join(path));
            Ok(check::report(&notices::check(
                root,
                &sources,
                out.as_deref(),
            )?))
        }
        Command::PackageDependencies => Ok(check::report(&package_dependencies::check(root)?)),
        Command::Perf { command } => run_perf(command, root, environment),
        Command::Quarantine { command } => run_quarantine(command, root, environment),
        Command::ReleaseSuite { command } => run_release_suite(command, root, environment),
        Command::Repeat {
            count,
            test,
            quarantined,
        } => run_repeat(root, count, &test, quarantined),
        Command::Results { command } => run_results(command, root, environment),
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

/// The `perf` commands (ADR 0071), whose arguments `perf` defines.
fn run_perf(
    command: perf::Command,
    root: &Path,
    environment: &Environment,
) -> anyhow::Result<ExitCode> {
    let token = environment.github_token.as_deref();
    perf::run(command, root, &environment.cargo, token)
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

/// The `repeat` command (ADR 0064).
fn run_repeat(
    root: &Path,
    count: std::num::NonZeroU32,
    tests: &[String],
    quarantined: Option<usize>,
) -> anyhow::Result<ExitCode> {
    let selection = match quarantined {
        Some(limit) => repeat::Selection::Quarantined(limit),
        None => repeat::Selection::Tests(
            tests
                .iter()
                .map(|value| repeat::parse_test(value))
                .collect::<anyhow::Result<_>>()?,
        ),
    };
    for (runner, argument) in repeat::arguments(root, selection, count)? {
        output::line(&format!("{runner}\t{argument}"));
    }
    Ok(ExitCode::SUCCESS)
}

/// The `results` commands (ADR 0064).
fn run_results(
    command: ResultsCommand,
    root: &Path,
    environment: &Environment,
) -> anyhow::Result<ExitCode> {
    match command {
        ResultsCommand::Allure { store, out } => {
            allure::write(&root.join(store), &root.join(out))?;
            Ok(ExitCode::SUCCESS)
        }
        ResultsCommand::Ceiling {
            store,
            run,
            today,
            summary,
        } => {
            let summary = summary.map(|summary| root.join(summary));
            ceiling::check(&root.join(store), run, today, summary.as_deref())?;
            Ok(ExitCode::SUCCESS)
        }
        ResultsCommand::Failed {
            repository,
            run_id,
            attempt,
            artifacts,
            os,
            arch,
            api_url,
        } => {
            let source = results::Source {
                api: github::Api {
                    url: &api_url,
                    token: environment.github_token.as_deref(),
                },
                repository: &repository,
                run: run_id,
                attempt,
                artifacts: &root.join(artifacts),
            };
            let platform = results::Platform {
                os: os.as_deref(),
                arch: arch.as_deref(),
            };
            print_failed(&source, &platform)
        }
        ResultsCommand::Ingest {
            repository,
            run_id,
            attempt,
            artifacts,
            store,
            api_url,
        } => {
            let source = results::Source {
                api: github::Api {
                    url: &api_url,
                    token: environment.github_token.as_deref(),
                },
                repository: &repository,
                run: run_id,
                attempt,
                artifacts: &root.join(artifacts),
            };
            let path = results::ingest(&source, &root.join(store))?;
            output::line(&format!("wrote {}", path.display()));
            Ok(ExitCode::SUCCESS)
        }
        ResultsCommand::Pending {
            store,
            since,
            repository,
            api_url,
        } => {
            let api = github::Api {
                url: &api_url,
                token: environment.github_token.as_deref(),
            };
            for (run, attempt) in results::pending(&api, &repository, &root.join(store), since)? {
                output::line(&format!("{run} {attempt}"));
            }
            Ok(ExitCode::SUCCESS)
        }
        ResultsCommand::Scan {
            store,
            repository,
            api_url,
        } => {
            let api = github::Api {
                url: &api_url,
                token: environment.github_token.as_deref(),
            };
            Ok(check::report(&scan::scan(
                root,
                &root.join(store),
                &api,
                &repository,
            )?))
        }
    }
}

/// Prints a failed attempt's tests as a diagnostic job matrix (ADR 0064).
fn print_failed(
    source: &results::Source<'_>,
    platform: &results::Platform<'_>,
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

/// The `release-suite` commands (ADR 0108).
fn run_release_suite(
    command: ReleaseSuiteCommand,
    root: &Path,
    environment: &Environment,
) -> anyhow::Result<ExitCode> {
    match command {
        ReleaseSuiteCommand::Plan {
            repository,
            run_id,
            git_ref,
            cells,
            commit,
            outputs,
            summary,
            api_url,
        } => {
            let api = github::Api {
                url: &api_url,
                token: environment.github_token.as_deref(),
            };
            let dispatch = release_suite::Dispatch {
                repository: &repository,
                run_id,
                git_ref: &git_ref,
                cells: &cells,
                commit: commit.as_deref(),
                outputs: &root.join(outputs),
                summary: &root.join(summary),
            };
            Ok(check::report(&release_suite::plan(api, &dispatch)?))
        }
        ReleaseSuiteCommand::Verdict {
            repository,
            commit,
            api_url,
        } => {
            let api = github::Api {
                url: &api_url,
                token: environment.github_token.as_deref(),
            };
            Ok(check::report(&release_suite::verdict(
                api,
                &repository,
                &commit,
            )?))
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
        EgressCommand::Rules {
            job,
            resolved,
            uid,
            windows,
        } => {
            // clap requires `--uid` or `--windows`.
            let enforcer = match (uid, windows) {
                (_, true) => egress::Enforcer::WindowsFirewall,
                (Some(uid), false) => egress::Enforcer::Nftables { uid },
                (None, false) => anyhow::bail!("give --uid or --windows"),
            };
            output::line(egress::rules(root, &job, &root.join(resolved), &enforcer)?.trim_end());
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
                firewall_log,
                sandbox,
            } => {
                let (before, after) = (
                    before.map(|path| root.join(path)),
                    after.map(|path| root.join(path)),
                );
                let firewall_log = firewall_log.map(|path| root.join(path));
                // clap requires one record, and `--before` with `--after`.
                let record = match (&before, &after, &firewall_log, sandbox) {
                    (Some(before), Some(after), _, _) => egress::Record::Counters { before, after },
                    (_, _, Some(log), _) => egress::Record::FirewallLog(log),
                    (_, _, _, true) => egress::Record::Sandbox,
                    _ => anyhow::bail!("give --before and --after, --firewall-log or --sandbox"),
                };
                Ok(check::report(&egress::verdict(
                    root,
                    &root.join(outcome),
                    &record,
                )?))
            }
        },
        EgressCommand::Report {
            counters,
            log,
            firewall_log,
        } => {
            let counters = counters.map(|path| root.join(path));
            let log = log.map(|path| root.join(path));
            let firewall_log = firewall_log.map(|path| root.join(path));
            // clap requires `--counters` with `--log`, or `--firewall-log`.
            let refusals = match (&counters, &log, &firewall_log) {
                (Some(counters), Some(log), _) => egress::Refusals::Nftables { counters, log },
                (_, _, Some(log)) => egress::Refusals::FirewallLog(log),
                _ => anyhow::bail!("give --counters and --log, or --firewall-log"),
            };
            Ok(check::report(&egress::report(root, &refusals)?))
        }
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
