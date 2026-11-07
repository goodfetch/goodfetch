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
mod advisories;
mod affected;
mod all_green;
mod allure;
mod axe_exceptions;
mod budgets;
mod ceiling;
mod check;
mod child_cargo;
mod clients;
mod codegen;
mod commit_messages;
mod compare;
mod compat;
mod console;
mod containment;
mod counts;
mod dco;
mod dependency_direction;
mod egress;
mod environment;
mod evict_caches;
mod features;
mod fixture_size;
mod floor_guard;
mod foxhound_side;
mod gate;
mod git;
mod github;
mod image_pins;
mod issues;
mod journeys;
mod junit;
mod junit_summary;
mod lazyfs;
mod license_allowlist;
mod metadata;
mod mocking_lockfile;
mod modules;
mod no_retry;
mod notices;
mod order_seed;
mod os_services;
mod output;
mod output_scans;
mod package_dependencies;
mod perf;
mod pinned;
mod quarantine;
mod rate;
mod received_requests;
mod register;
mod release_suite;
mod repeat;
mod results;
mod rss;
mod scan;
mod screenshots;
mod self_hosted;
mod size;
mod stress;
mod sweep_target;
mod targets;
mod telemetry;
mod test_tools;
mod tlc;
mod tree;
mod watchlist;
mod wording;
mod workflow;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory as _, Parser, Subcommand};

use environment::Environment;

#[derive(Parser)]
#[command(name = "xtask", about = "Goodfetch's release and CI tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The pinned advisory snapshot the gate's `just audit` reads offline
    /// (ADR 0064).
    Advisories {
        #[command(subcommand)]
        command: advisories::Command,
    },
    /// Prints which jobs of the pull-request tier a change needs and what
    /// each test runner selects, as `key=value` lines (ADRs 0032 and 0061).
    Affected(affected::Arguments),
    /// The "all green" check: every needed job succeeded, and no earlier
    /// attempt of the run had a failed test step (ADR 0064).
    AllGreen(all_green::Arguments),
    /// The axe exception file, `console/axe-exceptions.toml` (ADR 0073).
    AxeExceptions {
        #[command(subcommand)]
        command: axe_exceptions::Command,
    },
    /// The budget check (ADR 0061): opens or updates one `needs-triage`
    /// issue per suite or job whose median of its last 20 runs in its own
    /// tier, read from the results store, is past its budget in TESTING.md;
    /// never fails.
    Budgets(budgets::Arguments),
    /// The Client conformance case list under `conformance/clients/` (ADR
    /// 0069).
    Clients {
        #[command(subcommand)]
        command: clients::Command,
    },
    /// Regenerates every file generated from `goodfetch-protocol`'s types:
    /// the JSON Schema bundle and catalog file, each Product's projection,
    /// the names only Foxhound carries, the namespaces of the commands
    /// `goodfetch-cli` generates, the cargo-deny `wrappers` list and each
    /// SDK package's `generated/` layer (ADR 0038).
    Codegen(codegen::Arguments),
    /// The floor guard (ADR 0140): fails on what the change from the merge base
    /// of `--base` and `--head` to `--head` does to the Quality register's Floor,
    /// unless a `Quality-change:` trailer names it.
    FloorGuard {
        /// The branch the change merges into.
        #[arg(long)]
        base: String,
        /// The change's head commit.
        #[arg(long)]
        head: String,
    },
    /// Fails on a commit from `--base` to `--head`, merges and the ancestors
    /// of `--since` aside, whose subject is not a Conventional Commit
    /// (CONTRIBUTING.md).
    CommitMessages {
        /// The pull request's base commit.
        #[arg(long)]
        base: String,
        /// The pull request's head commit.
        #[arg(long)]
        head: String,
        /// The recorded starting point: it and its ancestors are not judged.
        #[arg(long)]
        since: String,
    },
    /// The comparison matrix (ADRs 0134 and 0138): each scenario's cells,
    /// Foxhound against Spider, Crawlee and Scrapy, crawling the Proving
    /// ground's 1,000-page site on one machine, every metric reported only
    /// once every trial did the same work.
    Compare {
        #[command(subcommand)]
        command: compare::Command,
    },
    /// The Compatibility corpus under `compat/` (ADR 0066).
    Compat {
        #[command(subcommand)]
        command: compat::Command,
    },
    /// The Console's build, which each executable embeds (ADR 0057).
    Console {
        #[command(subcommand)]
        command: console::Command,
    },
    /// Fails unless every generated contract of Collie's is in Foxhound's,
    /// unchanged (ADR 0094).
    Containment,
    /// The Counted work of each row of the Quality register, on its
    /// benchmark, against its limit (ADR 0140).
    Counts(counts::Arguments),
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
    /// Once the repository's caches pass 9 GB, deletes the least recently
    /// used ones that are not the pull-request tier's Linux caches until they
    /// fit (ADR 0061).
    EvictCaches(evict_caches::Arguments),
    /// The incremental local gate (ADR 0135): which steps of `just
    /// pre-merge all` a run on a descendant of a gated head reruns, and
    /// each step's recorded result.
    Gate {
        #[command(subcommand)]
        command: gate::Command,
    },
    /// The Feature inventory's declarations and its Feature check (ADRs 0072
    /// and 0075).
    Features {
        #[command(subcommand)]
        command: features::Command,
    },
    /// Fails on any file over 256 KiB under a `fixtures/` folder (ADR 0062).
    FixtureSize,
    /// Fails on a container image a workflow runs that is not pinned by
    /// digest: in a workflow, an action, a Containerfile, a pin file or a
    /// script (ADR 0067).
    ImagePins,
    /// The Journey catalog under `journeys/` (ADR 0063).
    Journeys {
        #[command(subcommand)]
        command: journeys::Command,
    },
    /// Appends the failed tests of a job's JUnit files to its summary page,
    /// each with its failure's first lines and its seeds; fails on a JUnit
    /// file it cannot read (ADR 0061).
    JunitSummary(junit_summary::Arguments),
    /// LazyFS, built from pinned source archives, which power-cuts a real
    /// executable on Linux (ADR 0060).
    Lazyfs {
        #[command(subcommand)]
        command: lazyfs::Command,
    },
    /// Fails when `uv.lock` holds a mocking library such as pytest-mock (ADR 0062).
    MockingLockfile,
    /// Fails on an import between top-level modules that a crate's
    /// `modules.toml` does not allow, and on a long file that does not say
    /// why it is one concern (ADR 0126).
    Modules {
        /// The Cargo workspace to check, from the repository's root: the
        /// root's own by default, `kennel` for the Kennel's (ADR 0141).
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Fails on any setting, flag, variable or workflow pattern that retries a
    /// test (ADRs 0064 and 0074).
    NoRetry,
    /// Builds the third-party notice inventory: every dependency of Cargo,
    /// npm and Python, and every input each `third-party.toml` declares,
    /// with its license and where its source is; fails on a license ADR
    /// 0018's allowlist refuses for its use (ADRs 0018, 0043 and 0047).
    Notices(notices::Arguments),
    /// Prints the order seed of a tree, which every suite orders its tests
    /// by: the first 8 bytes of its hash, read as an unsigned integer (ADR
    /// 0064).
    OrderSeed(order_seed::Arguments),
    /// Fails on an upload of a job's outputs that does not follow a passing
    /// `gitleaks` scan of what it uploads (ADR 0067).
    OutputScans,
    /// The real OS services the tests meet: the extra loopback addresses and
    /// the Secret Service (ADR 0059).
    OsServices {
        #[command(subcommand)]
        command: os_services::Command,
    },
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
    /// The Quality register of `TESTING.md` (ADR 0140).
    Register {
        #[command(subcommand)]
        command: register::Command,
    },
    /// Fails on a test that reads the Proving ground's requests other than
    /// through `ReceivedRequests`, which says whether a count keeps the
    /// engine's robots.txt reads and the browser's icons (ADRs 0064 and 0086).
    ReceivedRequests,
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
    /// The resident-memory check (ADRs 0071, 0094 and 0134): each
    /// scenario's peak on each Product's executable, held to its ceiling.
    Rss {
        #[command(subcommand)]
        command: rss::Command,
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
    /// The stress run before merge (ADR 0064): each new
    /// or changed test runs 20 times, and a test taken off the quarantine
    /// list 200 times.
    Stress {
        #[command(subcommand)]
        command: stress::Command,
    },
    /// Removes the dead build units of a Cargo target folder, those whose
    /// fingerprint no build has read for `--unused-hours` or since the day
    /// before `--read-since`, and keeps whole every unit a build read, which
    /// the next build reuses.
    SweepTarget(sweep_target::Arguments),
    /// The Performance targets report (ADR 0134): each target's figure on
    /// this machine, met or missed, never published and gating nothing.
    Targets {
        #[command(subcommand)]
        command: targets::Command,
    },
    /// Fails on a setting that could turn telemetry on and on an analytics
    /// client in `Cargo.lock`, `uv.lock` or `pnpm-lock.yaml` (ADR 0043).
    Telemetry,
    /// Builds the commit's executables that tests run with the tests' own
    /// Cargo profile and writes the test tool manifest of their paths, which
    /// `just test` puts in `GOODFETCH_TEST_TOOLS` (ADRs 0062 and 0094).
    TestTools(test_tools::Arguments),
    /// The TLA+ models' model checking with TLC, on the pinned pair of TLA+
    /// tools (ADR 0060).
    Tlc {
        #[command(subcommand)]
        command: tlc::Command,
    },
    /// The watchlist job (story 61): opens one `watchlist` issue for each
    /// vendor webappanalyzer or Scrapfly's Antibot-Detector lists that no
    /// block-detection rule covers, reading their names only (ADR 0035).
    Watchlist(watchlist::Arguments),
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
        /// The hosted jobs the run would start, each held to its timeout.
        #[arg(long, default_value_t = 1)]
        jobs: u64,
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
    /// Prints `<run> <attempt>` for every attempt of the finished runs the
    /// store lacks, for the daily ingest (ADR 0135).
    Pending {
        /// A checkout of the results store.
        #[arg(long)]
        store: PathBuf,
        /// The first day, UTC, `YYYY-MM-DD`; by default the day before the
        /// first of the store's newest month.
        #[arg(long)]
        since: Option<jiff::civil::Date>,
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
        Command::Advisories { command } => advisories::run(&command, environment),
        Command::Affected(arguments) => affected::run(arguments, environment),
        Command::AllGreen(arguments) => all_green::run(&arguments, environment),
        Command::AxeExceptions { command } => axe_exceptions::run(command, environment),
        Command::Budgets(arguments) => budgets::run(&arguments, environment),
        Command::Clients { command } => clients::run(&command, environment),
        Command::Codegen(arguments) => codegen::run(arguments, environment),
        Command::CommitMessages { base, head, since } => Ok(check::report(
            &commit_messages::check(root, &base, &head, &since)?,
        )),
        Command::FloorGuard { base, head } => floor_guard::run(root, &base, &head),
        Command::Compat { command } => compat::run(command, environment),
        Command::Console { command } => console::run(&command, environment),
        Command::Containment => Ok(check::report(&containment::check(root)?)),
        Command::Counts(arguments) => counts::run(&arguments, environment),
        Command::Dco { base, head } => Ok(check::report(&dco::check(root, &base, &head)?)),
        Command::DependencyDirection => {
            Ok(check::report(&dependency_direction::check(cargo, root)?))
        }
        Command::Egress { command } => egress::run(command, environment),
        Command::EvictCaches(arguments) => evict_caches::run(&arguments, environment),
        Command::Features { command } => features::run(command, environment),
        Command::Gate { command } => gate::run(command, environment),
        Command::FixtureSize => Ok(check::report(&fixture_size::check(root)?)),
        Command::ImagePins => Ok(check::report(&image_pins::check(root)?)),
        Command::Journeys { command } => {
            journeys::run(&command, environment, features::manual_exceptions)
        }
        Command::JunitSummary(arguments) => junit_summary::run(&arguments, environment),
        Command::Lazyfs { command } => lazyfs::run(&command, environment),
        Command::MockingLockfile => Ok(check::report(&mocking_lockfile::check(root)?)),
        Command::Modules { workspace } => {
            let workspace =
                workspace.map_or_else(|| root.to_path_buf(), |folder| root.join(folder));
            Ok(check::report(&modules::check(cargo, &workspace)?))
        }
        Command::NoRetry => Ok(check::report(&no_retry::check(root)?)),
        Command::Notices(arguments) => notices::run(&arguments, environment),
        Command::OsServices { command } => os_services::run(command, environment),
        Command::OutputScans => Ok(check::report(&output_scans::check(root)?)),
        Command::PackageDependencies => Ok(check::report(&package_dependencies::check(root)?)),
        Command::Perf { command } => perf::run(command, environment),
        Command::Quarantine { command } => quarantine::run(command, environment),
        Command::ReceivedRequests => Ok(check::report(&received_requests::check(root)?)),
        Command::Register { command } => register::run(&command, environment, &Cli::command()),
        Command::ReleaseSuite { command } => release_suite::run(command, environment),
        Command::Repeat(arguments) => repeat::run(arguments, environment),
        Command::OrderSeed(arguments) => order_seed::run(&arguments, environment),
        Command::Results { command } => run_results(command, environment),
        Command::Compare { command } => compare::run(command, environment),
        Command::Rss { command } => rss::run(command, environment),
        Command::Screenshots { command } => screenshots::run(command, environment),
        Command::SelfHosted { command } => self_hosted::run(command, environment),
        Command::Size { mode } => size::run(&mode, environment),
        Command::Stress { command } => stress::run(command, environment),
        Command::SweepTarget(arguments) => sweep_target::run(&arguments, environment),
        Command::Targets { command } => targets::run(command, environment),
        Command::Telemetry => Ok(check::report(&telemetry::check(root)?)),
        Command::TestTools(arguments) => test_tools::run(&arguments, environment),
        Command::Tlc { command } => tlc::run(command, environment),
        Command::Watchlist(arguments) => watchlist::run(&arguments, environment),
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
            jobs,
            today,
            summary,
        } => {
            let summary = summary.as_ref().map(path);
            ceiling::check(&path(&store), run, jobs, today, summary.as_deref())?;
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
    let variables: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    Environment::of(&variables)
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
