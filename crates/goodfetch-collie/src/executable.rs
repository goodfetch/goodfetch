//! The entry of both Products' executables (ADRs 0087 and 0090): one command
//! line, which Foxhound's executable keeps with its names, flags and exit
//! codes, and one start, from the composition and Product its `main` passes.
//! Today the command line holds `--version`, the hidden Attached-engine
//! entry, which an SDK starts, the hidden `command-line`, which prints the
//! command line for `xtask codegen` (ADR 0072), `browser install`, `browser clean`,
//! `console`, `config explain`,
//! `daemon start|stop|restart|status|health|install|uninstall|rotate-token|http-listener-token|run`,
//! with the hidden `daemon launched` a start or the installed service
//! runs, `runs reindex` and `upgrade-files`, beside the commands the `generated` module generates
//! from the catalog for every other namespace, such as `run start` and
//! `engine info`, and `--color auto|always|never` on every command; the CLI's
//! other commands join with area 11. Every
//! command but `daemon launched` and `command-line` first records the
//! executable in the list of installed executables (ADR 0098);
//! `command-line` runs before any of the executable's set-up.
//!
//! An executable more than three months past its release date prints the
//! age notice on stderr before its command, once a day, unless
//! `version.age_notice` is `false` (ADR 0030); a Daemon a start launched
//! has no terminal and prints none.
//!
//! `main` reads the environment once and passes the map (ADR 0059); every
//! `GOODFETCH_` variable in it must be a deployment variable, or the
//! executable stops with `setting_unknown` or `setting_invalid` before any
//! command runs (ADR 0083), its exit code its Error code's row (ADR 0069);
//! Collie's refuses what only Foxhound carries (the `identity` module).
//!
//! Logs go to stderr through the `tracing` subscriber (the `logs` module),
//! filtered by `log.level` (`info` when unset), as text unless `log.format`
//! says `json`; stdout is the protocol's alone (STYLE.md), but in `daemon
//! run`, the foreground Daemon, whose logs are JSON lines on stdout unless
//! `log.format` says `text` (ADR 0049). Where `log.output` is `file`, as it
//! is by default for a Daemon a start launched, they also go, as JSON
//! lines, to files of its own in the Goodfetch home's log folder, rotated
//! by size, which test support turns on for ADR 0072's exercise audit (ADR
//! 0122). Each Connection receives the logs as `$/log` at its own level,
//! and the spans go to the OTLP collector `otlp.traces_endpoint` names, and
//! nowhere without it. A panic writes a crash report into the home's
//! `crashes` folder, which nothing sends (ADR 0043).
//!
//! The engine runs on the system versions of its handles (ADR 0059): the
//! OS's clock, entropy seeded by the OS, blocking work on tracked threads,
//! and the Goodfetch home that `GOODFETCH_HOME` names, or else the
//! `goodfetch` folder of the OS's usual data folder.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::process::ExitCode;

use clap::{CommandFactory as _, FromArgMatches as _, Parser, Subcommand};
use daemon_command::{DaemonCommand, daemon_command};
use engine_choice::EngineFlags;
use generated::{Call, Generated, Joined, Joining};
pub use generated::{OwnCall, OwnCommand, OwnCommands};
use goodfetch_engine::age_notice::AgeNotice;
use goodfetch_engine::audit::AuditEvents;
use goodfetch_engine::browser_cache::BROWSERS_PATH_VARIABLE;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::crash_report;
use goodfetch_engine::daemon::DaemonConsole;
use goodfetch_engine::entropy::{Entropy, NoSeed};
use goodfetch_engine::governor::System;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::log::LogEvents;
use goodfetch_engine::mask::Masks;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::settings::deployment_variable;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use identity::{Refused, identity, refuse};
use output::{ColourChoice, ColourFlag, Human};
use settings_flags::SettingsFlags;

pub use command_line::printed_command_line;

mod allocator;
mod audit;
mod audit_events;
mod browsers;
mod certs;
mod command_line;
mod config;
mod console;
mod daemon;
mod daemon_command;
pub mod engine_choice;
pub mod exit_codes;
pub mod generated;
mod identity;
mod log_file;
mod logs;
mod metrics;
pub mod operation;
mod other_product;
mod otlp;
pub mod output;
mod runs;
mod service;
mod settings_flags;
mod upgrade;
mod usage;

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
        /// Serves the engine's own Console at this `host:port` while it
        /// runs, as a script that wants no Daemon asks (ADR 0031); its
        /// `hello` answers the Console's sign-in link. Port 0 takes a free
        /// port. None by default: an Attached engine serves no Console.
        #[arg(long, value_name = "ADDRESS")]
        console_address: Option<String>,
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Prints this executable's command line as JSON, hidden commands
    /// included, with the namespaces of the generated commands: what `xtask
    /// codegen` commits (ADR 0072). It starts no engine and reads no
    /// setting.
    #[command(hide = true, name = "command-line")]
    PrintedCommandLine {
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Manages the browser builds this version pins in the Browser cache.
    Browser {
        #[command(subcommand)]
        command: BrowserCommand,
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Exports and verifies the user's Audit log.
    Audit {
        #[command(subcommand)]
        command: audit::AuditCommand,
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Lists, accepts and forgets the certificate pins of the user's
    /// known-certificates store.
    Certs {
        #[command(subcommand)]
        command: certs::CertsCommand,
        #[command(flatten)]
        _colour: ColourFlag,
    },
    /// Opens the Console in the browser, signed in: starts the major's
    /// Daemon unless one runs.
    Console {
        /// Opens no browser: prints the sign-in link instead.
        #[arg(long)]
        no_browser: bool,
        /// Prints the sign-in link as JSON.
        #[arg(long)]
        json: bool,
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
        #[command(flatten)]
        engine: EngineFlags,
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
        /// Resolves in this process, as it does unless `--daemon` is given.
        #[arg(long, conflicts_with = "daemon")]
        attached: bool,
    },
}

#[derive(Subcommand)]
enum BrowserCommand {
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
    /// The commands it writes by hand among those generated from its
    /// Product's catalog: [`OwnCommands::COLLIE`] for Collie's, Foxhound's
    /// own from Foxhound's crate (ADR 0087).
    pub commands: OwnCommands,
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
/// use goodfetch_collie::executable::{OwnCommands, Start, main};
/// use goodfetch_protocol::catalog::Product;
///
/// let exit_code = main(&Start {
///     product: Product::Collie,
///     version: env!("CARGO_PKG_VERSION"),
///     release_date: None,
///     composition: goodfetch_collie::composition,
///     commands: OwnCommands::COLLIE,
///     arguments: vec!["goodfetch-collie".into(), "--version".into()],
///     environment: Default::default(),
/// });
/// ```
///
/// # Panics
///
/// If Tokio cannot build its runtime.
pub fn main(start: &Start) -> ExitCode {
    allocator::purge_at_once();
    let (binary, parsed, colour) = match parse(start) {
        Ok(parsed) => parsed,
        Err(exit) => return exit,
    };
    if let Parsed::HandWritten(Command::PrintedCommandLine { .. }) = parsed {
        return command_line::print(start.product, start.commands);
    }
    let environment = with_passed_variables(&start.environment, &parsed);
    let human = Human::new(colour, &environment);
    let home = home(&environment);
    let clock = Clock::system();
    let log_events = LogEvents::new();
    let masks = Masks::new();
    let launched = logs::Process::of(&parsed) == logs::Process::DaemonLaunched;
    let (logs, entropy) = match observe(
        start,
        &binary,
        home.as_ref(),
        &parsed,
        clock,
        &log_events,
        &masks,
    ) {
        Ok(installed) => installed,
        Err(exit) => return exit,
    };
    let identity = match identity(start, &environment) {
        Ok(identity) => identity,
        Err(Refused::Setting(error)) => return identity::refused_setting(start, &error, &human),
        Err(Refused::Version) => {
            tracing::error!(
                version = start.version,
                "the executable's version is no Product version"
            );
            return ExitCode::FAILURE;
        }
    };
    let engine = match Engine::new(identity, &(start.composition)()) {
        Ok(engine) => instrumented(engine, log_events, logs.audit_events()).with_masks(masks),
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
        age_notice(start.release_date, &home, engine.identity(), clock);
    }
    let Ok(entropy) = entropy.inspect_err(|error| {
        tracing::error!(%error, "the engine has no seed for its random choices");
    }) else {
        return ExitCode::FAILURE;
    };
    let dependencies = Dependencies {
        environment: environment.clone(),
        keychain: goodfetch_engine::secrets::Keychain::os(),
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
        (&engine, dependencies),
        start.commands,
        &environment,
        &human,
    )
}

/// The executable's name, the command `start`'s command line asks for and
/// its colour; or, when the command line cannot be read, its exit code: a
/// usage error's, or `not_in_product`'s when it names what only Foxhound
/// carries and the executable is not Foxhound's (ADR 0087).
fn parse(start: &Start) -> Result<(String, Parsed, ColourChoice), ExitCode> {
    let (command_line, generated) = command_line(start.product, start.commands);
    let mut command_line = command_line.version(start.version);
    let binary = command_line.get_name().to_owned();
    let parsed = command_line
        .try_get_matches_from_mut(&start.arguments)
        .and_then(|matches| {
            let colour = matches
                .get_one::<ColourChoice>(COLOR)
                .copied()
                .unwrap_or_default();
            let parsed = match generated.call(&matches) {
                Some(call) => Parsed::Generated(call),
                None => Parsed::HandWritten(CommandLine::from_arg_matches(&matches)?.command),
            };
            Ok((binary, parsed, colour))
        });
    parsed.map_err(|error| {
        refuse(start, &command_line, &error).unwrap_or_else(|| {
            let _ = error.print();
            ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2))
        })
    })
}

/// `engine`, its Connections receiving `log_events`, its Resource governor
/// reading the OS it runs on (ADR 0040) and its Audit log also sending each
/// new entry to `audit_events`, if anywhere.
fn instrumented(
    engine: Engine,
    log_events: LogEvents,
    audit_events: Option<AuditEvents>,
) -> Engine {
    let engine = engine
        .with_log_events(log_events)
        .with_governor(System::os());
    match audit_events {
        Some(events) => engine.with_audit_events(events),
        None => engine,
    }
}

/// Installs the `tracing` subscriber of `parsed`'s process, with
/// `log_events` and every writer masking with `masks`, and its crash reports, and answers it with the process's
/// `Entropy` from the OS, a fork of which draws its export's trace and span
/// IDs; fails once it has logged why the logs cannot go where the `log.*`
/// settings say, with the Error code (ADR 0122).
fn observe(
    start: &Start,
    binary: &str,
    home: Option<&Home>,
    parsed: &Parsed,
    clock: Clock,
    log_events: &LogEvents,
    masks: &Masks,
) -> Result<(logs::Installed, Result<Entropy, NoSeed>), ExitCode> {
    let mut entropy = Entropy::from_os();
    let handles = logs::Handles {
        clock,
        trace_ids: entropy.as_mut().ok().map(Entropy::fork),
    };
    let version = ProductVersion::parse(start.version).ok();
    let settings = home
        .zip(version.as_ref())
        .and_then(|(home, version)| browsers::machine_settings(home, start.product, version).ok());
    let process = logs::Process::of(parsed);
    let filter = logs::filter_of(settings.as_ref());
    let file = version.as_ref().map_or(Ok(None), |version| {
        logs::file_of(binary, process, version, home, settings.as_ref(), clock)
    });
    let refused = filter.as_ref().err().or(file.as_ref().err()).cloned();
    let installed = logs::install(logs::Logs {
        filter: filter.unwrap_or_else(|_| "info".to_owned()),
        file: file.ok().flatten(),
        to: process.to(),
        settings: settings.as_ref(),
        events: log_events,
        masks,
        product: start.product,
        handles,
    });
    if let Some(refused) = refused {
        tracing::error!(code = refused.code, "{}", refused.message);
        return Err(ExitCode::FAILURE);
    }
    if let Some(home) = home {
        crash_report::record_panics(
            home.root().join(CRASH_FOLDER),
            format!("{binary} {}", start.version),
            clock,
            masks.clone(),
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

/// Whether `parsed` records the executable in the list of installed
/// executables: every command but `daemon launched`.
fn records_the_executable(parsed: &Parsed) -> bool {
    !matches!(
        parsed,
        Parsed::HandWritten(Command::Daemon {
            command: DaemonCommand::Launched { .. },
            ..
        })
    )
}

/// Runs the command `parsed` asks for on `engine`, once the executable is
/// set up, among `own` commands.
fn run(
    parsed: Parsed,
    (engine, dependencies): (&Engine, Dependencies),
    own: OwnCommands,
    environment: &BTreeMap<OsString, OsString>,
    human: &Human,
) -> ExitCode {
    let home = dependencies.home.clone();
    let machine = daemon::Machine {
        identity: engine.identity(),
        home: &home,
        environment,
        executable: std::env::current_exe().ok(),
        clock: dependencies.clock,
    };
    if records_the_executable(&parsed) {
        machine.record();
    }
    let command = match parsed {
        Parsed::Generated(call) => return call.run(engine, dependencies, &machine, human),
        Parsed::HandWritten(command) => command,
    };
    match command {
        // Printed before the executable is set up (`main`).
        Command::PrintedCommandLine { .. } => command_line::print(engine.identity().product, own),
        Command::AttachedEngine {
            console_address, ..
        } => run_attached(engine, dependencies, &machine, console_address.as_deref()),
        Command::Browser {
            command: BrowserCommand::Install { json },
            ..
        } => browsers::install(engine.identity(), environment, machine.clock, json, human),
        Command::Browser {
            command: BrowserCommand::Clean { dry_run, json },
            ..
        } => browsers::clean(
            engine.identity(),
            environment,
            machine.clock,
            (dry_run, json),
            human,
        ),
        Command::Console {
            no_browser, json, ..
        } => console::open(&machine, no_browser, json, human),
        Command::Config {
            command:
                ConfigCommand::Explain {
                    keys,
                    settings,
                    project,
                    json,
                    daemon,
                    attached: _,
                },
            ..
        } => config_explain(
            (engine, dependencies),
            &machine,
            daemon,
            (keys, settings.run_layer(), project),
            json,
            human,
        ),
        Command::Daemon { command, .. } => {
            daemon_command(command, engine, dependencies, &machine, human)
        }
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
            usage_command(command, engine, dependencies, &machine, human)
        }
        Command::Audit { command, .. } => {
            audit::run(command, engine, dependencies, &machine, human)
        }
        Command::Certs { command, .. } => {
            certs::run(command, engine, dependencies, &machine, human)
        }
        Command::UpgradeFiles { from, json, .. } => {
            upgrade::upgrade(engine.identity(), &home, from.as_deref(), json, human)
        }
    }
}

/// Runs the `usage` command `command`.
fn usage_command(
    command: UsageCommand,
    engine: &Engine,
    dependencies: Dependencies,
    machine: &daemon::Machine<'_>,
    human: &Human,
) -> ExitCode {
    let command = match command {
        UsageCommand::Generated(joining) => {
            return joining.call().run(engine, dependencies, machine, human);
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
        engine: flags,
    } = command;
    let params = goodfetch_protocol::usage::UsageRaiseParams {
        scope,
        scope_id,
        usage_kind,
        currency,
        period,
        amount,
    };
    usage::raise(
        engine,
        dependencies,
        (machine, flags.choice()),
        &params,
        human,
    )
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
/// the Daemon's `config.explain` under `--daemon`, the remote Daemon
/// `GOODFETCH_URL` names when it is set.
fn config_explain(
    (engine, dependencies): (&Engine, Dependencies),
    machine: &daemon::Machine<'_>,
    through_daemon: bool,
    (keys, settings, project): (
        Vec<String>,
        serde_json::Map<String, serde_json::Value>,
        Option<std::path::PathBuf>,
    ),
    json: bool,
    human: &Human,
) -> ExitCode {
    let Ok(folder) = std::env::current_dir() else {
        tracing::error!("the working folder cannot be read");
        return ExitCode::FAILURE;
    };
    let request = config::Explain {
        keys,
        settings,
        project,
        folder,
        json,
    };
    if through_daemon {
        return config::explain_through(engine, dependencies, machine, &request, human);
    }
    config::explain(
        engine.identity(),
        machine.home,
        machine.environment,
        &request,
        human,
    )
}

/// Prints the age notice on stderr when it is due for an executable
/// released on `release_date` (ADR 0030), at the time `clock` reads.
fn age_notice(release_date: Option<&str>, home: &Home, identity: &Identity, clock: Clock) {
    let Some(released) = release_date.and_then(|date| date.parse::<jiff::civil::Date>().ok())
    else {
        return;
    };
    match AgeNotice::new(home, identity.clone()).due(released, clock.wall()) {
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
        None => Home::new(dirs::data_dir()?.join(goodfetch_protocol::home::HOME_FOLDER)),
    };
    Some(home.with_browsers_path(
        deployment_variable(environment, BROWSERS_PATH_VARIABLE).map(Into::into),
    ))
}

/// The command line of `product`'s executable, whose own commands are
/// `own`, without its version: its commands, flags and help, from which the
/// Feature inventory generates its `command:` Features (ADR 0072). Every
/// command in it is built, so reading its subcommands reads them all.
///
/// # Examples
///
/// ```
/// use goodfetch_collie::executable::{OwnCommands, command};
/// use goodfetch_protocol::catalog::Product;
///
/// assert_eq!(
///     command(Product::Collie, OwnCommands::COLLIE).get_name(),
///     "goodfetch-collie"
/// );
/// ```
#[must_use]
pub fn command(product: Product, own: OwnCommands) -> clap::Command {
    let (mut command, _) = command_line(product, own);
    command.build();
    command
}

/// The command line [`main`] parses, whose commands clap builds as it
/// reaches them: the hand-written commands, and those generated from the
/// catalog for every namespace no hand-written group takes, `own` among
/// them written by hand.
fn command_line(product: Product, own: OwnCommands) -> (clap::Command, Generated) {
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
    let generated = Generated::beside(product, own, &hand_written);
    (generated.add_to(hand_written), generated)
}

fn run_attached(
    engine: &Engine,
    dependencies: Dependencies,
    machine: &daemon::Machine<'_>,
    console_address: Option<&str>,
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
        if let Some((endpoint, token)) = daemon {
            return match transport::connect_daemon(&endpoint).await {
                Ok(connection) => {
                    Engine::relay_attached(Network::Os, connection, token.expose().to_owned())
                        .await
                        .map_err(|error| error.to_string())
                }
                Err(error) => Err(format!("the Daemon at {endpoint} does not answer: {error}")),
            };
        }
        // `console.address` has no value on an Attached engine but the one
        // its script gives for its run (ADR 0031).
        let console = match console_address {
            Some(address) if engine.serves_console() => {
                daemon::start_console(address, settings.as_ref()).await.0
            }
            Some(_) | None => DaemonConsole::Absent,
        };
        engine
            .run_attached_with_console(Network::Os, dependencies, console)
            .await
            .map_err(|error| error.to_string())
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
