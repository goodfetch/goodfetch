//! The entry of both Products' executables (ADRs 0087 and 0090): one command
//! line, which Foxhound's executable keeps with its names, flags and exit
//! codes, and one start, from the composition and Product its `main` passes.
//! Today the command line holds `--version`, the hidden Attached-engine
//! entry, which an SDK starts, and `browsers install` and `browsers clean`;
//! the CLI's other commands join with area 11.
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
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::Network;
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

mod browsers;
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
    let (binary, about) = match start.product {
        Product::Collie => (
            "goodfetch-collie",
            "Collie, Goodfetch's Product for browser automation.",
        ),
        Product::Foxhound => (
            "goodfetch-foxhound",
            "Foxhound, Goodfetch's Product for crawling, which contains Collie.",
        ),
    };
    let command = CommandLine::command()
        .name(binary)
        .bin_name(binary)
        .version(start.version)
        .about(about);
    let parsed = command
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
    match log_file(binary, &start.environment, home.as_ref()) {
        Ok(file) => install_logs(filter, file),
        Err(problem) => {
            install_logs(filter, None);
            tracing::error!(%problem, "the logs cannot go where {LOG_OUTPUT_VARIABLE} says");
            return ExitCode::FAILURE;
        }
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
    match command_line.command {
        Command::AttachedEngine => run_attached(&engine, dependencies),
        Command::Browsers {
            command: BrowsersCommand::Install { json },
        } => browsers::install(pinned_by, &start.environment, json),
        Command::Browsers {
            command: BrowsersCommand::Clean { dry_run, json },
        } => browsers::clean(pinned_by, &start.environment, dry_run, json),
    }
}

/// The Goodfetch home: `GOODFETCH_HOME` when it is set, the `goodfetch`
/// folder of the OS's usual data folder otherwise.
fn home(environment: &BTreeMap<OsString, OsString>) -> Option<Home> {
    match environment.get(OsStr::new(HOME_VARIABLE)) {
        Some(home) if !home.is_empty() => Some(Home::new(home.into())),
        _ => dirs::data_dir().map(|data| Home::new(data.join("goodfetch"))),
    }
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
/// `GOODFETCH_LOG_OUTPUT` is `file`; none when it is `none` or unset.
fn log_file(
    binary: &str,
    environment: &BTreeMap<OsString, OsString>,
    home: Option<&Home>,
) -> Result<Option<File>, String> {
    let output = environment.get(OsStr::new(LOG_OUTPUT_VARIABLE));
    match output.map(|output| output.to_str()) {
        None | Some(Some("none")) => Ok(None),
        Some(Some("file")) => {
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
