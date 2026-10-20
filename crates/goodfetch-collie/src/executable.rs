//! The entry of both Products' executables (ADRs 0087 and 0090): one command
//! line, which Foxhound's executable keeps with its names, flags and exit
//! codes, and one start, from the composition and Product its `main` passes.
//! Today the command line holds `--version`, the hidden Attached-engine
//! entry, which an SDK starts, `browsers install`, `browsers clean` and
//! `config explain`; the CLI's other commands join with area 11.
//!
//! `main` reads the environment once and passes the map (ADR 0059); every
//! `GOODFETCH_` variable in it must be a deployment variable, or the
//! executable stops with `setting_unknown` or `setting_invalid` before any
//! command runs (ADR 0083).
//!
//! Logs go to stderr through the `tracing` subscriber, filtered by
//! `GOODFETCH_LOG` in `EnvFilter`'s syntax (`info` when unset); stdout is
//! the protocol's alone (STYLE.md).
//!
//! The engine runs on the system versions of its handles (ADR 0059): the
//! OS's clock, entropy seeded by the OS, blocking work on tracked threads,
//! and the Goodfetch home that `GOODFETCH_HOME` names, or else the
//! `goodfetch` folder of the OS's usual data folder.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::process::ExitCode;

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

mod browsers;
mod config;
mod output;

/// The variable that filters the logs, in `EnvFilter`'s syntax.
pub const LOG_VARIABLE: &str = "GOODFETCH_LOG";

/// The deployment variable that roots the Goodfetch home (ADRs 0059, 0083).
pub const HOME_VARIABLE: &str = "GOODFETCH_HOME";

/// The command line both executables share.
#[derive(Parser)]
#[command(disable_help_subcommand = true)]
struct CommandLine {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
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
    let parsed = command(start.product)
        .version(start.version)
        .try_get_matches_from(&start.arguments)
        .and_then(|matches| CommandLine::from_arg_matches(&matches));
    let command_line = match parsed {
        Ok(command_line) => command_line,
        Err(error) => {
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };
    log_to_stderr(start.environment.get(OsStr::new(LOG_VARIABLE)));
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
    let Some(home) = home(&start.environment) else {
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
    match command_line.command {
        Command::AttachedEngine => run_attached(&engine, dependencies),
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
                },
        } => config_explain(&engine, &dependencies.home, keys, set, project, json),
    }
}

/// `config explain`, from the working folder.
fn config_explain(
    engine: &Engine,
    home: &Home,
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
    config::explain(
        identity.product,
        &identity.product_version,
        home,
        &config::Explain {
            keys,
            set,
            project,
            folder,
            json,
        },
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
/// `command:` Features (ADR 0072).
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

/// Installs the `tracing` subscriber, writing to stderr.
fn log_to_stderr(filter: Option<&OsString>) {
    let filter = filter
        .and_then(|filter| filter.to_str())
        .and_then(|filter| tracing_subscriber::EnvFilter::try_new(filter).ok())
        .unwrap_or_else(|| tracing_subscriber::EnvFilter::new("info"));
    #[expect(
        clippy::disallowed_methods,
        reason = "the tracing subscriber's writer is one of the three writers of stderr (STYLE.md)"
    )]
    let writer = std::io::stderr;
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .init();
}
