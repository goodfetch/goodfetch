//! `goodfetch-test-env` (ADR 0062): a test's setup for TypeScript and
//! Python. It builds a `TestEnv`, prints its description as one JSON line on
//! stdout, and tears it down when its stdin closes, keeping the test's folder
//! when a line `failed` came first, as the packages write for a failed test.
//! It exits non-zero, with a
//! message on stderr, when a start fails. `--proving-ground <seed>` also
//! starts the commit's Proving ground on that seed, which the description
//! names and which stops before the folder is removed. `--daemon` also
//! starts the Daemon of the tool manifest's Collie executable in the test's
//! home, or of Foxhound's with `--daemon=foxhound` (ADR 0094); the
//! description names its socket and token, and at teardown it is stopped
//! with its executable's `daemon stop` and must leave no runtime file.
//! `--browser` prepares the home for an engine that launches a browser, as
//! an SDK's Attached engine does for a program that opens a page: it lays
//! out the Browser cache there, so nothing is downloaded, and, with
//! `--proving-ground`, its user configuration maps the Proving ground's
//! hosts and lists them in `egress.private_ranges`, as a Rust test's Run
//! settings do.
//! `--browser-download <served|not-found|read-only-cache>`, with
//! `--proving-ground`, prepares the home for an engine's first use of an
//! empty Browser cache instead (#3100): `GOODFETCH_BROWSERS_PATH` names
//! one in the test's folder, and the user configuration reaches the
//! Proving ground's hosts and downloads a missing build from its
//! `browser-builds.test`, which serves the pinned build from the tests'
//! copy `ci/chrome-for-testing-fetch.sh` downloads (`served`) or answers
//! `404` (`not-found`); with `read-only-cache`, the engine cannot write the
//! cache.
//! `--print-schemas` prints the JSON
//! Schemas of the crate's file types for `xtask codegen` and starts nothing;
//! `--print-canary-rule` prints the Secret canaries' `gitleaks` rule for it,
//! and `--print-secret-kinds` their kinds, which it writes for the
//! TypeScript and Python helpers.
//! On Linux, `dns-namespace` runs a program in a network and mount
//! namespace whose resolver is the DNS fixture (`DnsNamespace`), and exits
//! with its status; it exits 125 when the namespace cannot be made.

use std::io::{self, Read as _, Write as _};
use std::process::ExitCode;

use clap::Parser;
#[cfg(unix)]
use goodfetch_test_support::ReadOnlyBrowserCache;
use goodfetch_test_support::{
    DAEMON_DEADLINE, Daemon, Description, PROVING_GROUND_DEADLINE, Product, ProvingGround,
    SecretKind, TestEnv, Tool, Variables, browser_builds, browser_cache_in_home,
    empty_browser_cache, gitleaks_rule, schemas, tools,
};

#[derive(Parser)]
#[command(
    name = "goodfetch-test-env",
    about = "Prepares one test's setup and prints its description",
    args_conflicts_with_subcommands = true
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is a switch of the command line, which clap reads as a bool"
)]
struct Arguments {
    #[cfg(target_os = "linux")]
    #[command(subcommand)]
    command: Option<Subcommand>,
    /// The locale tag whose `LANG` the environment takes, such as `tr-TR`.
    #[arg(long)]
    locale: Option<String>,
    /// The time zone the environment's `TZ` names, such as `Asia/Kathmandu`.
    #[arg(long)]
    time_zone: Option<String>,
    /// The calling test's full name, from which its seed and folder name
    /// come.
    #[arg(long, default_value = "goodfetch-test-env")]
    test_name: String,
    /// Starts the commit's Proving ground on this seed, and describes it
    /// under `proving_ground`.
    #[arg(long, value_name = "SEED")]
    proving_ground: Option<u64>,
    /// Starts the Daemon of this Product's executable from the test tool
    /// manifest in the test's home, Collie's unless `--daemon=foxhound`,
    /// and describes its socket and token under `daemon`.
    #[arg(
        long,
        value_name = "PRODUCT",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "collie"
    )]
    daemon: Option<Product>,
    /// Lays out the Browser cache in the test's home and, with
    /// `--proving-ground`, lets the home's engines reach its hosts.
    #[arg(long)]
    browser: bool,
    /// Points the home's engines at an empty Browser cache and, from the
    /// Proving ground's `browser-builds.test`, downloads a missing build
    /// that host serves (`served`) or answers `404` for (`not-found`), or
    /// makes the cache one they cannot write (`read-only-cache`).
    #[arg(
        long,
        value_enum,
        value_name = "HOST",
        requires = "proving_ground",
        conflicts_with = "browser"
    )]
    browser_download: Option<BrowserDownload>,
    /// Prints the JSON Schemas of test support's file types, keyed by their
    /// paths under `test-support/fixtures/`, and starts nothing.
    #[arg(long, exclusive = true)]
    print_schemas: bool,
    /// Prints the Secret canaries' `gitleaks` rule, generated from their
    /// kinds, and starts nothing.
    #[arg(long, exclusive = true)]
    print_canary_rule: bool,
    /// Prints the Secret canaries' kinds as one JSON array of their names,
    /// in declaration order, and starts nothing.
    #[arg(long, exclusive = true)]
    print_secret_kinds: bool,
}

/// How an engine's first use of an empty Browser cache goes (#3100).
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum BrowserDownload {
    /// `browser-builds.test` serves the pinned build: the first page
    /// downloads it.
    Served,
    /// `browser-builds.test` answers `404`.
    NotFound,
    /// The engine cannot write the cache.
    ReadOnlyCache,
}

#[cfg(target_os = "linux")]
#[derive(clap::Subcommand)]
enum Subcommand {
    /// Runs a program in a network and mount namespace of its own, whose
    /// resolver relays to the DNS fixture.
    DnsNamespace(DnsNamespaceArguments),
}

#[cfg(target_os = "linux")]
#[derive(clap::Args)]
struct DnsNamespaceArguments {
    /// The DNS fixture's address.
    #[arg(long)]
    dns: std::net::SocketAddr,
    /// A folder whose files are each mounted over `/etc/<their name>`.
    #[arg(long)]
    etc: std::path::PathBuf,
    /// The `<uid>:<gid>` to start the program as, when run as root
    /// through sudo; without it, a user namespace is made too.
    #[arg(long)]
    user: Option<goodfetch_test_support::InsideUser>,
    /// One variable of the program's environment, as `NAME=VALUE`.
    #[arg(long = "env", value_name = "NAME=VALUE")]
    env: Vec<String>,
    /// The program and its arguments; with none, the namespace is made and
    /// left.
    #[arg(last = true)]
    child: Vec<std::ffi::OsString>,
}

/// The status a namespace that cannot be made exits with, as `env` and
/// `unshare` exit when they cannot run their command.
#[cfg(target_os = "linux")]
const NAMESPACE_FAILED: u8 = 125;

#[cfg(target_os = "linux")]
fn dns_namespace(arguments: DnsNamespaceArguments) -> ExitCode {
    let inside = goodfetch_test_support::Inside {
        dns: arguments.dns,
        etc: arguments.etc,
        user: arguments.user,
        env: arguments.env,
        child: arguments.child,
    };
    match inside.run() {
        Ok(status) => ExitCode::from(u8::try_from(status).unwrap_or(u8::MAX)),
        Err(error) => {
            #[expect(
                clippy::print_stderr,
                reason = "goodfetch-test-env reports a namespace it could not make on its own stderr (ADR 0067)"
            )]
            {
                eprintln!("goodfetch-test-env: dns-namespace: {error}");
            }
            ExitCode::from(NAMESPACE_FAILED)
        }
    }
}

fn main() -> ExitCode {
    #[cfg_attr(
        not(target_os = "linux"),
        expect(unused_mut, reason = "only Linux takes the subcommand out")
    )]
    let mut arguments = Arguments::parse();
    #[cfg(target_os = "linux")]
    if let Some(Subcommand::DnsNamespace(namespace)) = arguments.command.take() {
        return dns_namespace(namespace);
    }
    let result = if arguments.print_schemas {
        print_schemas()
    } else if arguments.print_canary_rule {
        write_text(&gitleaks_rule())
    } else if arguments.print_secret_kinds {
        serde_json::to_string(&SecretKind::ALL)
            .map_err(Failure::from)
            .and_then(|kinds| write_line(&kinds))
    } else {
        serve(&arguments)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            #[expect(
                clippy::print_stderr,
                reason = "goodfetch-test-env reports a failed start on its own stderr (ADR 0062)"
            )]
            {
                eprintln!("goodfetch-test-env: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

type Failure = Box<dyn std::error::Error>;

fn print_schemas() -> Result<(), Failure> {
    let schemas = serde_json::to_string(&schemas())?;
    write_line(&schemas)
}

fn serve(arguments: &Arguments) -> Result<(), Failure> {
    let mut env = TestEnv::create(&arguments.test_name, &Variables::from_process())?;
    if let Some(tag) = &arguments.locale {
        env = env.with_locale(tag)?;
    }
    if let Some(zone) = &arguments.time_zone {
        env = env.with_time_zone(zone)?;
    }
    // The variable naming the empty cache is in the environment the
    // description lists.
    let empty_cache = match arguments.browser_download {
        Some(_) => {
            let (with_cache, cache) = empty_browser_cache(env)?;
            env = with_cache;
            Some(cache)
        }
        None => None,
    };
    let tools = tools()?;
    let mut description = Description::new(&env, tools.clone());
    let proving_ground = match arguments.proving_ground {
        Some(seed) => {
            let program = tools.path(Tool::ProvingGround)?;
            // Its process joins the TestEnv's record, which stops it if it is
            // still alive at teardown (ADR 0062).
            let started = if arguments.browser_download == Some(BrowserDownload::Served) {
                ProvingGround::start_program_serving_builds(
                    &env,
                    program,
                    seed,
                    PROVING_GROUND_DEADLINE,
                    &browser_builds(&env)?,
                )?
            } else {
                ProvingGround::start_program(&env, program, seed, PROVING_GROUND_DEADLINE)?
            };
            description = description.with_proving_ground(started.description().clone());
            Some(started)
        }
        None => None,
    };
    if arguments.browser {
        browser_cache_in_home(&env)?;
        if let Some(started) = &proving_ground {
            env.user_configuration(&started.description().reaching());
        }
    }
    if let Some(started) = proving_ground.as_ref().filter(|_| empty_cache.is_some()) {
        let reached = started.description();
        env.user_configuration(&format!("{}{}", reached.reaching(), reached.mirroring()));
    }
    let read_only = match (&empty_cache, arguments.browser_download) {
        (Some(cache), Some(BrowserDownload::ReadOnlyCache)) => Some(read_only(cache)?),
        _ => None,
    };
    let daemon = match arguments.daemon {
        Some(product) => {
            // Its process joins the TestEnv's record too.
            let started =
                Daemon::start_program(&env, tools.path(product.tool())?, DAEMON_DEADLINE)?;
            description = description.with_daemon(started.description().clone());
            Some(started)
        }
        None => None,
    };
    write_line(&serde_json::to_string(&description)?)?;
    let mut input = Vec::new();
    #[expect(
        clippy::disallowed_methods,
        reason = "goodfetch-test-env tears down when its stdin closes (ADR 0062)"
    )]
    io::stdin().read_to_end(&mut input)?;
    // A package whose test failed writes `failed` before it closes stdin:
    // the folder stays for the failure's evidence, as a failed Rust test's.
    let mut env = env;
    if input
        .split(|byte| *byte == b'\n')
        .any(|line| line == b"failed")
    {
        env.keep();
    }
    if let Some(daemon) = daemon {
        daemon.stop(&env)?;
    }
    drop(proving_ground);
    // The cache is writable again before its folder is removed.
    drop(read_only);
    drop(env);
    Ok(())
}

/// The empty cache at `cache` made one the engine cannot write.
#[cfg(unix)]
fn read_only(cache: &std::path::Path) -> Result<ReadOnlyBrowserCache, Failure> {
    Ok(ReadOnlyBrowserCache::new(cache)?)
}

/// A read-only cache only Unix modes make here.
#[cfg(not(unix))]
fn read_only(_cache: &std::path::Path) -> Result<std::convert::Infallible, Failure> {
    Err("`--browser-download=read-only-cache` needs Unix file modes".into())
}

fn write_line(line: &str) -> Result<(), Failure> {
    write_text(&format!("{line}\n"))
}

fn write_text(text: &str) -> Result<(), Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "goodfetch-test-env writes its description to its own stdout (ADR 0062)"
    )]
    let mut stdout = io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()?;
    Ok(())
}
