//! The entry of both Products' executables (ADRs 0087 and 0090): one command
//! line, which Foxhound's executable keeps with its names, flags and exit
//! codes, and one start, from the composition and Product its `main` passes.
//! Today the command line holds `--version`, the hidden Attached-engine
//! entry, which an SDK starts, `browsers install`, `browsers clean`,
//! `config explain` and `daemon start|stop|restart|status`, with the hidden
//! `daemon launched` a start runs; the CLI's other commands join with area
//! 11. Every command but `daemon launched` first records the executable in
//! the list of installed executables (ADR 0098).
//!
//! `main` reads the environment once and passes the map (ADR 0059); every
//! `GOODFETCH_` variable in it must be a deployment variable, or the
//! executable stops with `setting_unknown` or `setting_invalid` before any
//! command runs (ADR 0083).
//!
//! Logs go to stderr through the `tracing` subscriber, filtered by
//! `GOODFETCH_LOG` in `EnvFilter`'s syntax (`info` when unset); stdout is
//! the protocol's alone (STYLE.md). With `GOODFETCH_LOG_OUTPUT=file` they
//! also go, as JSON lines, to a file of its own in the Goodfetch home's log
//! folder, which test support turns on for ADR 0072's exercise audit (ADR
//! 0122). Both variables stand in for the `log.*` settings until the engine
//! reads its configuration.
//!
//! The engine runs on the system versions of its handles (ADR 0059): the
//! OS's clock, entropy seeded by the OS, blocking work on tracked threads,
//! and the Goodfetch home that `GOODFETCH_HOME` names, or else the
//! `goodfetch` folder of the OS's usual data folder.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{CommandFactory as _, FromArgMatches as _, Parser, Subcommand};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::settings::{check_environment, deployment_variable};
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::Network;
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

mod browsers;
mod config;
mod daemon;
mod output;

/// The variable that filters the logs, in `EnvFilter`'s syntax.
pub const LOG_VARIABLE: &str = "GOODFETCH_LOG";

/// Where the logs also go: `file`, the Goodfetch home's log folder, or
/// `none`, the default.
pub const LOG_OUTPUT_VARIABLE: &str = "GOODFETCH_LOG_OUTPUT";

/// The Goodfetch home's log folder (ADR 0122).
const LOG_FOLDER: &str = "logs";

/// The deployment variable that roots the Goodfetch home (ADRs 0059, 0083).
pub const HOME_VARIABLE: &str = "GOODFETCH_HOME";

/// The command line both executables share.
#[derive(Parser)]
#[command(disable_help_subcommand = true)]
struct CommandLine {
    #[command(subcommand)]
    command: Command,
}

// Each command's own flags and subcommands are built only when it is the
// one parsed or its help is shown, so `--version` and every other command
// skip building the rest of the tree (ADR 0071's startup figure).
#[derive(Subcommand)]
#[command(defer = true)]
enum Command {
    /// Runs as an Attached engine over stdio, which stops at the end of its
    /// input: what an SDK starts.
    #[command(hide = true)]
    AttachedEngine,
    /// Manages the browser builds this version pins in the Browser cache.
    Browsers {
        #[command(subcommand)]
        command: BrowsersCommand,
    },
    /// Shows the settings and where each value comes from.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Starts, stops and shows the Daemon of this executable's major, which
    /// both Products share.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
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
    /// Runs as the Daemon a start launched, from a versioned copy.
    #[command(hide = true)]
    Launched {
        /// The build of the versioned copy it runs from.
        #[arg(long)]
        copy: Option<String>,
        /// Never stops it after `daemon.idle_timeout` with no Client.
        #[arg(long)]
        keep_alive: bool,
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
        /// Sets a run value, as `key=value`; repeatable.
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
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
    /// Its composition.
    pub composition: Composition,
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
///     composition: goodfetch_collie::composition(),
///     arguments: vec!["goodfetch-collie".into(), "--version".into()],
///     environment: Default::default(),
/// });
/// ```
///
/// # Panics
///
/// If Tokio cannot build its runtime.
pub fn main(start: &Start) -> ExitCode {
    let command_line = command_line(start.product).version(start.version);
    let binary = command_line.get_name().to_owned();
    let parsed = command_line
        .try_get_matches_from(&start.arguments)
        .and_then(|matches| CommandLine::from_arg_matches(&matches));
    let command_line = match parsed {
        Ok(command_line) => command_line,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };
    let home = home(&start.environment);
    let filter = start.environment.get(OsStr::new(LOG_VARIABLE));
    // A Daemon a start launched has no terminal: it logs to its file
    // (ADR 0122).
    let launched = matches!(
        command_line.command,
        Command::Daemon {
            command: DaemonCommand::Launched { .. }
        }
    );
    match log_file(&binary, &start.environment, home.as_ref(), launched) {
        Ok(file) => install_logs(filter, file),
        Err(problem) => {
            install_logs(filter, None);
            tracing::error!(%problem, "the logs cannot go where {LOG_OUTPUT_VARIABLE} says");
            return ExitCode::FAILURE;
        }
    }
    if let Err(error) = check_environment(&start.environment) {
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
    let engine = match Engine::new(identity, &start.composition) {
        Ok(engine) => engine,
        Err(error) => {
            tracing::error!(%error, "the composition cannot start an engine");
            return ExitCode::FAILURE;
        }
    };
    let Some(home) = home else {
        tracing::error!("no Goodfetch home: set GOODFETCH_HOME, or a home folder the OS knows");
        return ExitCode::FAILURE;
    };
    let entropy = match Entropy::from_os() {
        Ok(entropy) => entropy,
        Err(error) => {
            tracing::error!(%error, "the engine has no seed for its random choices");
            return ExitCode::FAILURE;
        }
    };
    let dependencies = Dependencies {
        home,
        clock: Clock::system(),
        entropy,
        blocking: BlockingPolicy::Threads,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
    };
    run(
        command_line.command,
        &engine,
        dependencies,
        pinned_by,
        start,
    )
}

/// Runs `command` on `engine`, once the executable is set up.
fn run(
    command: Command,
    engine: &Engine,
    dependencies: Dependencies,
    pinned_by: PinnedBy,
    start: &Start,
) -> ExitCode {
    let home = dependencies.home.clone();
    let machine = daemon::Machine {
        identity: engine.identity(),
        home: &home,
        environment: &start.environment,
        executable: std::env::current_exe().ok(),
    };
    if !matches!(
        command,
        Command::Daemon {
            command: DaemonCommand::Launched { .. }
        }
    ) {
        machine.record();
    }
    match command {
        Command::AttachedEngine => run_attached(engine, dependencies),
        Command::Browsers {
            command: BrowsersCommand::Install { json },
        } => browsers::install(pinned_by, &start.environment, json),
        Command::Browsers {
            command: BrowsersCommand::Clean { dry_run, json },
        } => browsers::clean(pinned_by, &start.environment, dry_run, json),
        Command::Config {
            command:
                ConfigCommand::Explain {
                    keys,
                    set,
                    project,
                    json,
                    daemon,
                },
        } => config_explain(engine, &machine, daemon, keys, set, project, json),
        Command::Daemon { command } => match command {
            DaemonCommand::Start { keep_alive, json } => daemon::start(&machine, keep_alive, json),
            DaemonCommand::Stop { json } => daemon::stop(&machine, json),
            DaemonCommand::Restart { keep_alive, json } => {
                daemon::restart(&machine, keep_alive, json)
            }
            DaemonCommand::Status { json } => daemon::status(&machine, json),
            DaemonCommand::Launched { copy, keep_alive } => {
                daemon::launched(engine, dependencies, &machine, copy.as_deref(), keep_alive)
            }
        },
    }
}

/// `config explain`, from the working folder: in this process, or through
/// the Daemon's `config.explain` under `--daemon`.
fn config_explain(
    engine: &Engine,
    machine: &daemon::Machine<'_>,
    through_daemon: bool,
    keys: Vec<String>,
    set: Vec<String>,
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
        set,
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

/// The Goodfetch home: `GOODFETCH_HOME` when it is set, the `goodfetch`
/// folder of the OS's usual data folder otherwise.
fn home(environment: &BTreeMap<OsString, OsString>) -> Option<Home> {
    match deployment_variable(environment, HOME_VARIABLE) {
        Some(home) => Some(Home::new(home.into())),
        None => dirs::data_dir().map(|data| Home::new(data.join("goodfetch"))),
    }
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
    let mut command = command_line(product);
    command.build();
    command
}

/// The command line [`main`] parses, whose commands clap builds as it
/// reaches them.
fn command_line(product: Product) -> clap::Command {
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
    CommandLine::command()
        .name(binary)
        .bin_name(binary)
        .about(about)
}

fn run_attached(engine: &Engine, dependencies: Dependencies) -> ExitCode {
    #[expect(
        clippy::disallowed_methods,
        reason = "main builds the runtime (ADR 0060)"
    )]
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("invariant: the OS gives the runtime its threads");
    let served = runtime.block_on(engine.run_attached(Network::Os, dependencies));
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

/// The file of this process's logs in the home's log folder, when
/// `GOODFETCH_LOG_OUTPUT` is `file`, or it is unset and `by_default`; none
/// when it is `none` or unset.
fn log_file(
    binary: &str,
    environment: &BTreeMap<OsString, OsString>,
    home: Option<&Home>,
    by_default: bool,
) -> Result<Option<File>, String> {
    let output = environment.get(OsStr::new(LOG_OUTPUT_VARIABLE));
    match output.map(|output| output.to_str()) {
        None if !by_default => Ok(None),
        Some(Some("none")) => Ok(None),
        None | Some(Some("file")) => {
            let home = home.ok_or("no Goodfetch home holds a log folder")?;
            let folder = home.root().join(LOG_FOLDER);
            #[expect(
                clippy::disallowed_methods,
                reason = "main opens the log file before the runtime starts (ADR 0122)"
            )]
            let file = std::fs::create_dir_all(&folder).and_then(|()| {
                File::options()
                    .create(true)
                    .append(true)
                    .open(folder.join(format!("{binary}-{}.jsonl", std::process::id())))
            });
            file.map(Some)
                .map_err(|error| format!("{}: {error}", folder.display()))
        }
        Some(_) => Err("it is `file` or `none`".to_owned()),
    }
}

/// Installs the `tracing` subscriber: text on stderr, and JSON lines in
/// `file` when there is one, both filtered by `filter`.
fn install_logs(filter: Option<&OsString>, file: Option<File>) {
    let filter = filter
        .and_then(|filter| filter.to_str())
        .and_then(|filter| tracing_subscriber::EnvFilter::try_new(filter).ok())
        .unwrap_or_else(|| tracing_subscriber::EnvFilter::new("info"));
    #[expect(
        clippy::disallowed_methods,
        reason = "the tracing subscriber's writer is one of the three writers of stderr (STYLE.md)"
    )]
    let writer = std::io::stderr;
    let file = file.map(|file| {
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(Arc::new(file))
    });
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false),
        )
        .with(file)
        .init();
}
