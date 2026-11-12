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
//! `--console-port <port>` has its Console serve on that port, which a test
//! that restarts the Daemon names so that no other program can take it in
//! between, instead of one the OS chooses.
//! `--browser` prepares the home for an engine that launches a browser, as
//! an SDK's Attached engine does for a program that opens a page: it lays
//! out the Browser cache there, so nothing is downloaded, and, with
//! `--proving-ground`, its user configuration maps the Proving ground's
//! hosts and lists them in `egress.private_ranges`, as a Rust test's Run
//! settings do.
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
use goodfetch_test_support::{
    DAEMON_DEADLINE, Daemon, Description, PROVING_GROUND_DEADLINE, Product, ProvingGround,
    ProvingGroundDescription, SecretKind, TestEnv, Tool, Variables, browser_cache_in_home,
    gitleaks_rule, schemas, tools,
};

#[derive(Parser)]
#[command(
    name = "goodfetch-test-env",
    about = "Prepares one test's setup and prints its description",
    args_conflicts_with_subcommands = true
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
    /// The port the Daemon's Console serves on, instead of one the OS chooses.
    #[arg(long, value_name = "PORT", requires = "daemon")]
    console_port: Option<u16>,
    /// Lays out the Browser cache in the test's home and, with
    /// `--proving-ground`, lets the home's engines reach its hosts.
    #[arg(long)]
    browser: bool,
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
    let tools = tools()?;
    let mut description = Description::new(&env, tools.clone());
    let proving_ground = match arguments.proving_ground {
        Some(seed) => {
            let program = tools.path(Tool::ProvingGround)?;
            // Its process joins the TestEnv's record, which stops it if it is
            // still alive at teardown (ADR 0062).
            let started =
                ProvingGround::start_program(&env, program, seed, PROVING_GROUND_DEADLINE)?;
            description = description.with_proving_ground(started.description().clone());
            Some(started)
        }
        None => None,
    };
    let mut settings = Vec::new();
    if arguments.browser {
        browser_cache_in_home(&env)?;
        if let Some(started) = &proving_ground {
            settings.push(reaching(started.description()));
        }
    }
    if let Some(port) = arguments.console_port {
        settings.push(format!("console.address = \"127.0.0.1:{port}\"\n"));
    }
    if !settings.is_empty() {
        env.user_configuration(&settings.concat());
    }
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
    drop(env);
    Ok(())
}

/// The settings, lines of a user configuration, with which an engine
/// reaches `proving_ground`'s hosts: each mapped to its address, and listed
/// in `egress.private_ranges`, since that address is loopback and listing
/// the names keeps `localhost` and literal loopback addresses refused.
fn reaching(proving_ground: &ProvingGroundDescription) -> String {
    let hosts: Vec<String> = proving_ground
        .resolve
        .keys()
        .map(|host| format!("{host:?}"))
        .collect();
    let mapping: Vec<String> = proving_ground
        .resolve
        .iter()
        .map(|(host, address)| format!("{host:?} = [{address:?}]"))
        .collect();
    format!(
        "egress.private_ranges = [{}]\nnetwork.resolve = {{ {} }}\n",
        hosts.join(", "),
        mapping.join(", ")
    )
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
