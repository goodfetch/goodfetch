//! The entry of both Products' executables (ADRs 0087 and 0090): one command
//! line, which Foxhound's executable keeps with its names, flags and exit
//! codes, and one start, from the composition and Product its `main` passes.
//! Today the command line holds `--version`, the hidden Attached-engine
//! entry, which an SDK starts, `browsers install`, `browsers clean`,
//! `config explain`, `daemon start|stop|restart|status|install|uninstall|rotate-token|run`,
//! with the hidden `daemon launched` a start or the installed service
//! runs, `runs reindex` and `upgrade-files`, beside the commands `goodfetch-cli` generates
//! from the catalog for every other namespace, such as `run start` and
//! `engine info`, and `--color auto|always|never` on every command; the CLI's
//! other commands join with area 11. Every
//! command but `daemon launched` first records the executable in the list
//! of installed executables (ADR 0098).
//!
//! An executable more than three months past its release date prints the
//! age notice on stderr before its command, once a day, unless
//! `version.age_notice` is `false` (ADR 0030); a Daemon a start launched
//! has no terminal and prints none.
//!
//! `main` reads the environment once and passes the map (ADR 0059); every
//! `GOODFETCH_` variable in it must be a deployment variable, or the
//! executable stops with `setting_unknown` or `setting_invalid` before any
//! command runs (ADR 0083).
//!
//! Logs go to stderr through the `tracing` subscriber (the `logs` module),
//! filtered by `GOODFETCH_LOG` in `EnvFilter`'s syntax (`info` when unset),
//! as text unless `log.format` says `json`; stdout is the protocol's alone
//! (STYLE.md), but in `daemon run`, the foreground Daemon, whose logs are
//! JSON lines on stdout unless `log.format` says `text` (ADR 0049). With
//! `GOODFETCH_LOG_OUTPUT=file` they also go, as JSON lines, to a file of its
//! own in the Goodfetch home's log folder, which test support turns on for
//! ADR 0072's exercise audit (ADR 0122). Both variables stand in for the
//! `log.level` and `log.output` settings until #1566. Each Connection
//! receives the logs as `$/log` at its own level, and the spans go to the
//! OTLP collector `otlp.traces_endpoint` names, and nowhere without it. A
//! panic writes a crash report into the home's `crashes` folder, which
//! nothing sends (ADR 0043).
//!
//! The engine runs on the system versions of its handles (ADR 0059): the
//! OS's clock, entropy seeded by the OS, blocking work on tracked threads,
//! and the Goodfetch home that `GOODFETCH_HOME` names, or else the
//! `goodfetch` folder of the OS's usual data folder.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::process::ExitCode;

use clap::{CommandFactory as _, FromArgMatches as _, Parser, Subcommand};
use goodfetch_cli::generated::{Call, Generated, Joined, Joining};
use goodfetch_cli::output::{self, ColourChoice, ColourFlag, Human};
use goodfetch_cli::settings_flags::SettingsFlags;
use goodfetch_engine::age_notice::AgeNotice;
use goodfetch_engine::browser_cache::BROWSERS_PATH_VARIABLE;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::crash_report;
use goodfetch_engine::entropy::{Entropy, NoSeed};
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::log::LogEvents;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::settings::{check_environment, deployment_variable};
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;

mod browsers;
mod config;
mod daemon;
mod logs;
mod runs;
mod service;
mod upgrade;
mod usage;

/// The variable that filters the logs, in `EnvFilter`'s syntax.
pub const LOG_VARIABLE: &str = "GOODFETCH_LOG";

/// Where the logs also go: `file`, the Goodfetch home's log folder, or
/// `none`, the default.
pub const LOG_OUTPUT_VARIABLE: &str = "GOODFETCH_LOG_OUTPUT";

/// The Goodfetch home's log folder (ADR 0122).
const LOG_FOLDER: &str = "logs";

/// The Goodfetch home's folder of crash reports, which stay on the machine
/// (ADR 0043).
const CRASH_FOLDER: &str = "crashes";

/// The deployment variable that roots the Goodfetch home (ADRs 0059, 0083).
pub const HOME_VARIABLE: &str = "GOODFETCH_HOME";

/// The command line both executables share.
#[derive(Parser)]
#[command(disable_help_subcommand = true)]
struct CommandLine {
    #[command(subcommand)]
    command: Command,
    // Read from the parse by its id, so that a generated command's colour
    // reads the same way; each command group takes it too (`ColourFlag`).
    #[command(flatten)]
    _colour: ColourFlag,
}

/// The id of the `--color` flag in a parse.
const COLOR: &str = ColourFlag::ID;

// Each command's own flags and subcommands are built only when it is the
// one parsed or its help is shown, so `--version` and every other command
// skip building the rest of the tree (ADR 0071's startup figure).
#[derive(Subcommand)]
#[command(defer = true)]
enum Command {
    /// Runs as an Attached engine over stdio, which stops at the end of its
    /// input: what an SDK starts.
    #[command(hide = true)]
    AttachedEngine {
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Manages the browser builds this version pins in the Browser cache.
    Browsers {
        #[command(subcommand)]
        command: BrowsersCommand,
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Shows the settings and where each value comes from.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Starts, stops and shows the Daemon of this executable's major, which
    /// both Products share.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Works on the runs of this executable's major, of both Products.
    Runs {
        #[command(subcommand)]
        command: RunsCommand,
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Shows, sets and raises the Limits of the user's Usage ledger, which
    /// hold every engine of the user.
    Usage {
        #[command(subcommand)]
        command: UsageCommand,
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Gives this executable's major its own copy of an older major's
    /// settings, Operator policy, Run index and runs in Goodfetch home,
    /// never changing the older major's files.
    UpgradeFiles {
        /// The older major to upgrade from, such as `1` or `0.4`; the
        /// newest older major Goodfetch home holds when not given.
        #[arg(long, value_name = "MAJOR")]
        from: Option<String>,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        _colour: ColourFlag,
    },
}

// The `usage` group's commands: the catalog's `usage` commands, listed
// first, then its own. No doc comment: clap would make it the group's help.
#[derive(Subcommand)]
enum UsageCommand {
    #[command(flatten)]
    Generated(Joining<OwnUsageCommand>),
    #[command(flatten)]
    Own(OwnUsageCommand),
}

// The `usage` group's hand-written commands; no doc comment either.
#[derive(Subcommand)]
enum OwnUsageCommand {
    /// Raises a Limit, in place of the ones of its scope, Usage kind,
    /// currency and period but a managed Operator policy's: a person's
    /// answer, asked at the terminal, and refused with `person_required`
    /// when stdin is no terminal.
    Raise {
        /// The scope's kind.
        #[arg(long)]
        scope: String,
        /// Which one of its kind; the person's own when absent.
        #[arg(long)]
        scope_id: Option<String>,
        /// The Usage kind it caps.
        #[arg(long)]
        usage_kind: String,
        /// The currency it caps the price in, an ISO 4217 code.
        #[arg(long)]
        currency: Option<String>,
        /// The period it counts over, as JSON such as
        /// `{"every":"month","time_zone":"UTC"}`; none for a Limit that
        /// never starts again.
        #[arg(long, value_parser = usage::period)]
        period: Option<goodfetch_protocol::run::Period>,
        /// How much it allows per period from now on: a count of the
        /// Usage kind, or millionths of the currency.
        #[arg(long)]
        amount: u64,
    },
}

impl Joined for OwnUsageCommand {
    const NAMESPACE: &'static str = "usage";
}

#[derive(Subcommand)]
enum RunsCommand {
    /// Rebuilds the Run index from the run folders: the configured
    /// locations and the given paths. A run it does not find stays listed
    /// as missing; a run discarded on purpose stays erased.
    Reindex {
        /// Run folders, or folders of run folders, to look in as well.
        paths: Vec<std::path::PathBuf>,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Starts the Daemon, the major's pick, unless one runs.
    Start {
        /// Never stops it after `daemon.idle_timeout` with no Client.
        #[arg(long)]
        keep_alive: bool,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stops the Daemon; its live Runs stop with it.
    Stop {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Stops the Daemon, then starts the major's pick.
    Restart {
        /// Never stops it after `daemon.idle_timeout` with no Client.
        #[arg(long)]
        keep_alive: bool,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Shows whether the Daemon runs, as which Product, and its Clients.
    Status {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Registers the major's Daemon as a per-user service, which starts
    /// the major's pick: a launch agent on macOS and a systemd user unit on
    /// Linux, each started by the first connection to its socket, and a
    /// logon task on Windows.
    Install {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Removes the major's per-user service, stopping its Daemon.
    Uninstall {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Replaces the persistent token of the service's Daemon, which stops
    /// with every Connection that opened with the old one.
    RotateToken {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Runs the Daemon in the foreground, for Docker, Kubernetes and
    /// systemd: as this executable's Product, listening on
    /// `daemon.address`, with no runtime file and no idle stop, logging
    /// JSON lines to stdout; on `SIGTERM` it takes no run, checkpoints each
    /// run as resumable and exits within `daemon.drain_grace`.
    Run {
        /// Runs under the reaper of a `daemon run` on Linux, which started it.
        #[arg(long, hide = true)]
        reaped: bool,
    },
    /// Runs as the Daemon a start or the installed service launched, from a
    /// versioned copy.
    #[command(hide = true)]
    Launched {
        /// The build of the versioned copy it runs from.
        #[arg(long)]
        copy: Option<String>,
        /// Never stops it after `daemon.idle_timeout` with no Client.
        #[arg(long)]
        keep_alive: bool,
        /// Started by the major's service: starts the pick in its place
        /// when it is not the pick, and takes the socket its service
        /// manager hands over.
        #[arg(long)]
        service: bool,
        /// A deployment variable the service passes, as `NAME=VALUE`;
        /// repeatable.
        #[arg(long = "environment", value_name = "NAME=VALUE")]
        environment: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Shows each effective setting, the layer that set it and the layer
    /// that locked it: the built-in default, the Operator policy, user
    /// configuration, the project configuration or these flags.
    Explain {
        /// Keys, or prefixes of keys such as `timeout`, to show; every key
        /// when none.
        keys: Vec<String>,
        #[command(flatten)]
        settings: SettingsFlags,
        /// Reads this project configuration instead of the `goodfetch.toml`
        /// found from the working folder upward.
        #[arg(long)]
        project: Option<std::path::PathBuf>,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
        /// Asks the Daemon of the major, starting it if none runs, instead
        /// of resolving in this process.
        #[arg(long)]
        daemon: bool,
    },
}

#[derive(Subcommand)]
enum BrowsersCommand {
    /// Downloads every build this version pins into the Browser cache
    /// (`GOODFETCH_BROWSERS_PATH`, else Goodfetch home), for CI and images.
    Install {
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Deletes the builds nothing installed pins, unused for
    /// `browser_cache.max_unused_age` and held by no running browser.
    Clean {
        /// Lists what it would delete, and deletes nothing.
        #[arg(long)]
        dry_run: bool,
        /// Prints the result as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// What a Product's `main` passes to [`main`].
pub struct Start {
    /// The Product the executable is.
    pub product: Product,
    /// Its Product version, its crate's `CARGO_PKG_VERSION`.
    pub version: &'static str,
    /// Its release date, `YYYY-MM-DD`, which a release build records in
    /// `GOODFETCH_RELEASE_DATE`; none for any other build, which then never
    /// prints the age notice (ADR 0030).
    pub release_date: Option<&'static str>,
    /// Builds its composition, which [`main`] calls only once a command
    /// starts the engine, so `--version` and the help never build it.
    pub composition: fn() -> Composition,
    /// Its command line, as the OS gave it.
    pub arguments: Vec<OsString>,
    /// Its environment, which `main` reads once (ADR 0059).
    pub environment: BTreeMap<OsString, OsString>,
}

/// Runs the executable `start` describes, and returns its exit code.
///
/// # Examples
///
/// ```no_run
/// use goodfetch_collie::executable::{Start, main};
/// use goodfetch_protocol::catalog::Product;
///
/// let exit_code = main(&Start {
///     product: Product::Collie,
///     version: env!("CARGO_PKG_VERSION"),
///     release_date: None,
///     composition: goodfetch_collie::composition,
///     arguments: vec!["goodfetch-collie".into(), "--version".into()],
///     environment: Default::default(),
/// });
/// ```
///
/// # Panics
///
/// If Tokio cannot build its runtime.
pub fn main(start: &Start) -> ExitCode {
    let (command_line, generated) = command_line(start.product);
    let command_line = command_line.version(start.version);
    let binary = command_line.get_name().to_owned();
    let parsed = command_line
        .try_get_matches_from(&start.arguments)
        .and_then(|matches| {
            let colour = matches
                .get_one::<ColourChoice>(COLOR)
                .copied()
                .unwrap_or_default();
            let parsed = match generated.call(&matches) {
                Some(call) => Parsed::Generated(call),
                None => Parsed::HandWritten(CommandLine::from_arg_matches(&matches)?.command),
            };
            Ok((parsed, colour))
        });
    let (parsed, colour) = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };
    let environment = with_passed_variables(&start.environment, &parsed);
    let human = Human::new(colour, &environment);
    let home = home(&environment);
    let clock = Clock::system();
    let log_events = LogEvents::new();
    let (launched, _) = logs::logs_of(&parsed);
    let (_logs, entropy) = match observe(
        start,
        &binary,
        &environment,
        home.as_ref(),
        &parsed,
        clock,
        &log_events,
    ) {
        Ok(installed) => installed,
        Err(exit) => return exit,
    };
    if let Err(error) = check_environment(&environment) {
        tracing::error!(code = error.code(), "{error}");
        return ExitCode::FAILURE;
    }
    let Ok(product_version) = ProductVersion::parse(start.version) else {
        tracing::error!(
            version = start.version,
            "the executable's version is no Product version"
        );
        return ExitCode::FAILURE;
    };
    let pinned_by = PinnedBy {
        product: start.product,
        product_version: product_version.clone(),
    };
    let identity = Identity {
        product: start.product,
        product_version,
        // The packages of each channel say which they are with the release
        // pipeline (area 1); until then every executable is a source build.
        channel: Channel::CratesIo,
    };
    let engine = match Engine::new(identity, &(start.composition)()) {
        Ok(engine) => engine.with_log_events(log_events),
        Err(error) => {
            tracing::error!(%error, "the composition cannot start an engine");
            return ExitCode::FAILURE;
        }
    };
    let Some(home) = home else {
        tracing::error!("no Goodfetch home: set GOODFETCH_HOME, or a home folder the OS knows");
        return ExitCode::FAILURE;
    };
    if !launched {
        age_notice(start.release_date, &home, engine.identity());
    }
    let entropy = match entropy {
        Ok(entropy) => entropy,
        Err(error) => {
            tracing::error!(%error, "the engine has no seed for its random choices");
            return ExitCode::FAILURE;
        }
    };
    let dependencies = Dependencies {
        environment: environment.clone(),
        home,
        clock,
        entropy,
        blocking: BlockingPolicy::Threads,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    };
    run(
        parsed,
        &engine,
        dependencies,
        pinned_by,
        &environment,
        &human,
    )
}

/// Installs the `tracing` subscriber of `parsed`'s process, with
/// `log_events`, and its crash reports, and answers it with the process's
/// `Entropy` from the OS, a fork of which draws its export's trace and span
/// IDs; fails once it has logged why the logs cannot go where
/// `GOODFETCH_LOG_OUTPUT` says.
fn observe(
    start: &Start,
    binary: &str,
    environment: &BTreeMap<OsString, OsString>,
    home: Option<&Home>,
    parsed: &Parsed,
    clock: Clock,
    log_events: &LogEvents,
) -> Result<(logs::Installed, Result<Entropy, NoSeed>), ExitCode> {
    let mut entropy = Entropy::from_os();
    let handles = logs::Handles {
        clock,
        trace_ids: entropy.as_mut().ok().map(Entropy::fork),
    };
    let settings = home.and_then(|home| {
        let version = ProductVersion::parse(start.version).ok()?;
        browsers::machine_settings(home, start.product, &version).ok()
    });
    let (launched, to) = logs::logs_of(parsed);
    let file = logs::log_file(binary, environment, home, launched);
    let failed_file = file.as_ref().err().cloned();
    let installed = logs::install(logs::Logs {
        filter: environment.get(OsStr::new(LOG_VARIABLE)),
        file: file.ok().flatten(),
        to,
        settings: settings.as_ref(),
        events: log_events,
        product: start.product,
        handles,
    });
    if let Some(problem) = failed_file {
        tracing::error!(%problem, "the logs cannot go where {LOG_OUTPUT_VARIABLE} says");
        return Err(ExitCode::FAILURE);
    }
    if let Some(home) = home {
        crash_report::record_panics(
            home.root().join(CRASH_FOLDER),
            format!("{binary} {}", start.version),
            clock,
        );
    }
    Ok((installed, entropy))
}

/// What the command line asks for: a hand-written command, or one
/// generated from the catalog.
enum Parsed {
    HandWritten(Command),
    Generated(Call),
}

/// `start`'s environment with the deployment variables a service passes
/// to `daemon launched` as arguments, since a logon task sets none; only
/// those a service passes are taken.
fn with_passed_variables(
    environment: &BTreeMap<OsString, OsString>,
    parsed: &Parsed,
) -> BTreeMap<OsString, OsString> {
    let mut environment = environment.clone();
    if let Parsed::HandWritten(Command::Daemon {
        command: DaemonCommand::Launched {
            environment: passed,
            ..
        },
        ..
    }) = parsed
    {
        for pair in passed {
            if let Some((name, value)) = pair.split_once('=')
                && goodfetch_engine::daemon::service::PASSED_VARIABLES.contains(&name)
            {
                environment.insert(name.into(), value.into());
            }
        }
    }
    environment
}

/// Runs the command `parsed` asks for on `engine`, once the executable is
/// set up.
fn run(
    parsed: Parsed,
    engine: &Engine,
    dependencies: Dependencies,
    pinned_by: PinnedBy,
    environment: &BTreeMap<OsString, OsString>,
    human: &Human,
) -> ExitCode {
    let home = dependencies.home.clone();
    let machine = daemon::Machine {
        identity: engine.identity(),
        home: &home,
        environment,
        executable: std::env::current_exe().ok(),
    };
    if !matches!(
        parsed,
        Parsed::HandWritten(Command::Daemon {
            command: DaemonCommand::Launched { .. },
            ..
        })
    ) {
        machine.record();
    }
    let command = match parsed {
        Parsed::Generated(call) => return call.run(engine, dependencies, human),
        Parsed::HandWritten(command) => command,
    };
    match command {
        Command::AttachedEngine { .. } => run_attached(engine, dependencies, &machine),
        Command::Browsers {
            command: BrowsersCommand::Install { json },
            ..
        } => browsers::install(pinned_by, environment, json),
        Command::Browsers {
            command: BrowsersCommand::Clean { dry_run, json },
            ..
        } => browsers::clean(pinned_by, environment, dry_run, json),
        Command::Config {
            command:
                ConfigCommand::Explain {
                    keys,
                    settings,
                    project,
                    json,
                    daemon,
                },
            ..
        } => config_explain(
            engine,
            &machine,
            daemon,
            keys,
            settings.run_layer(),
            project,
            json,
        ),
        Command::Daemon { command, .. } => match command {
            DaemonCommand::Start { keep_alive, json } => daemon::start(&machine, keep_alive, json),
            DaemonCommand::Stop { json } => daemon::stop(&machine, json),
            DaemonCommand::Restart { keep_alive, json } => {
                daemon::restart(&machine, keep_alive, json)
            }
            DaemonCommand::Status { json } => daemon::status(&machine, json),
            DaemonCommand::Install { json } => service::install(&machine, json),
            DaemonCommand::Uninstall { json } => service::uninstall(&machine, json),
            DaemonCommand::RotateToken { json } => service::rotate_token(&machine, json),
            DaemonCommand::Run { reaped } => daemon::run(engine, dependencies, &machine, reaped),
            DaemonCommand::Launched {
                copy,
                keep_alive,
                service,
                environment: _,
            } => daemon::launched(
                engine,
                dependencies,
                &machine,
                copy.as_deref(),
                keep_alive,
                service,
            ),
        },
        Command::Runs {
            command: RunsCommand::Reindex { paths, json },
            ..
        } => {
            let Ok(folder) = std::env::current_dir() else {
                tracing::error!("the working folder cannot be read");
                return ExitCode::FAILURE;
            };
            runs::reindex(engine.identity(), &home, &folder, paths, json)
        }
        Command::Usage { command, .. } => {
            warn_of_a_temporary_ledger(&home, human);
            usage_command(command, engine, dependencies, human)
        }
        Command::UpgradeFiles { from, json, .. } => {
            upgrade::upgrade(engine.identity(), &home, from.as_deref(), json)
        }
    }
}

/// Runs the `usage` command `command`.
fn usage_command(
    command: UsageCommand,
    engine: &Engine,
    dependencies: Dependencies,
    human: &Human,
) -> ExitCode {
    let command = match command {
        UsageCommand::Generated(joining) => {
            return joining.call().run(engine, dependencies, human);
        }
        UsageCommand::Own(command) => command,
    };
    let OwnUsageCommand::Raise {
        scope,
        scope_id,
        usage_kind,
        currency,
        period,
        amount,
    } = command;
    let params = goodfetch_protocol::usage::UsageRaiseParams {
        scope,
        scope_id,
        usage_kind,
        currency,
        period,
        amount,
    };
    usage::raise(engine, dependencies, &params, human)
}

/// Warns when the Usage ledger sits on a temporary filesystem, where it is
/// lost on each restart and period Limits start again silently, as in a
/// container without a persistent volume (ADR 0041).
fn warn_of_a_temporary_ledger(home: &Home, human: &Human) {
    let ledger = home.usage_ledger();
    if let Some(kind) = goodfetch_engine::ledger::temporary_filesystem(&ledger) {
        human.warning(&format!(
            "the Usage ledger `{}` is on a temporary filesystem ({kind}): it is lost when the machine or container restarts, and period Limits start again; keep Goodfetch home on a persistent volume, or set GOODFETCH_HOME to one",
            ledger.display()
        ));
    }
}

/// `config explain`, from the working folder: in this process, or through
/// the Daemon's `config.explain` under `--daemon`.
fn config_explain(
    engine: &Engine,
    machine: &daemon::Machine<'_>,
    through_daemon: bool,
    keys: Vec<String>,
    settings: serde_json::Map<String, serde_json::Value>,
    project: Option<std::path::PathBuf>,
    json: bool,
) -> ExitCode {
    let Ok(folder) = std::env::current_dir() else {
        tracing::error!("the working folder cannot be read");
        return ExitCode::FAILURE;
    };
    let identity = engine.identity();
    let request = config::Explain {
        keys,
        settings,
        project,
        folder,
        json,
    };
    if through_daemon {
        return config::explain_through(machine, &request);
    }
    config::explain(
        identity.product,
        &identity.product_version,
        machine.home,
        &request,
    )
}

/// Prints the age notice on stderr when it is due for an executable
/// released on `release_date` (ADR 0030).
fn age_notice(release_date: Option<&str>, home: &Home, identity: &Identity) {
    let Some(released) = release_date.and_then(|date| date.parse::<jiff::civil::Date>().ok())
    else {
        return;
    };
    match AgeNotice::new(home, identity.clone()).due(released, Clock::system().wall()) {
        Ok(Some(line)) => output::note(&line),
        Ok(None) => {}
        Err(error) => tracing::debug!(%error, "the age notice was not recorded"),
    }
}

/// The Goodfetch home: `GOODFETCH_HOME` when it is set, the `goodfetch`
/// folder of the OS's usual data folder otherwise.
fn home(environment: &BTreeMap<OsString, OsString>) -> Option<Home> {
    let home = match deployment_variable(environment, HOME_VARIABLE) {
        Some(home) => Home::new(home.into()),
        None => Home::new(dirs::data_dir()?.join("goodfetch")),
    };
    Some(home.with_browsers_path(
        deployment_variable(environment, BROWSERS_PATH_VARIABLE).map(Into::into),
    ))
}

/// The command line of `product`'s executable, without its version: its
/// commands, flags and help, from which the Feature inventory generates its
/// `command:` Features (ADR 0072). Every command in it is built, so reading
/// its subcommands reads them all.
///
/// # Examples
///
/// ```
/// use goodfetch_collie::executable::command;
/// use goodfetch_protocol::catalog::Product;
///
/// assert_eq!(command(Product::Foxhound).get_name(), "goodfetch-foxhound");
/// ```
#[must_use]
pub fn command(product: Product) -> clap::Command {
    let (mut command, _) = command_line(product);
    command.build();
    command
}

/// The command line [`main`] parses, whose commands clap builds as it
/// reaches them: the hand-written commands, and those generated from the
/// catalog for every namespace no hand-written group takes.
fn command_line(product: Product) -> (clap::Command, Generated) {
    let (binary, about) = match product {
        Product::Collie => (
            "goodfetch-collie",
            "Collie, Goodfetch's Product for browser automation.",
        ),
        Product::Foxhound => (
            "goodfetch-foxhound",
            "Foxhound, Goodfetch's Product for crawling, which contains Collie.",
        ),
    };
    // `--color` stays the root's own: global at the root, clap would copy it
    // into every command group on each parse (see `ColourFlag`).
    let hand_written = CommandLine::command()
        .mut_arg(COLOR, |colour| colour.global(false))
        .name(binary)
        .bin_name(binary)
        .about(about);
    let generated = Generated::beside(&hand_written);
    (generated.add_to(hand_written), generated)
}

fn run_attached(
    engine: &Engine,
    dependencies: Dependencies,
    machine: &daemon::Machine<'_>,
) -> ExitCode {
    let identity = engine.identity();
    let settings = browsers::machine_settings(
        &dependencies.home,
        identity.product,
        &identity.product_version,
    )
    .ok();
    // In the per-install layout the Daemon alone writes run state: the
    // Attached engine hands its Runs to it (ADR 0002).
    let daemon = if daemon::per_install(settings.as_ref()) {
        match machine.reach_for_relay() {
            Ok(reach) => Some(reach),
            Err(failure) => return failure.report(),
        }
    } else {
        None
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "main builds the runtime (ADR 0060)"
    )]
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("invariant: the OS gives the runtime its threads");
    let served = runtime.block_on(async {
        match daemon {
            Some((endpoint, token)) => match transport::connect_daemon(&endpoint).await {
                Ok(connection) => Engine::relay_attached(Network::Os, connection, token)
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(format!("the Daemon at {endpoint} does not answer: {error}")),
            },
            None => engine
                .run_attached(Network::Os, dependencies)
                .await
                .map_err(|error| error.to_string()),
        }
    });
    // Tokio's stdin reads on a blocking thread that only new input wakes, so
    // the runtime is not waited for once the Connection has closed.
    runtime.shutdown_background();
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the Attached engine got no Connection");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
