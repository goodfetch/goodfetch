//! The real OS services the tests meet (ADR 0059; TESTING.md, Storage,
//! network and OS services): the extra loopback addresses and the Secret
//! Service. `ci/os-services.sh` starts them in every test job of CI, then
//! checks them here, so a service that did not come up fails the job; a
//! local `just test` lists the ones its host lacks and skips the tests that
//! need them.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode, Stdio};

use anyhow::Result;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::{egress, output, workflow};

/// `xtask os-services`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails unless each extra loopback address binds on this host and, with
    /// `--secret-service`, the Secret Service stores and reads an item. With
    /// `--skipped`, for a local `just test`, it fails on nothing: it appends
    /// each missing service to that file and prints the test-name pattern
    /// that skips the tests needing it, one per line.
    Check {
        #[command(flatten)]
        services: Services,
        /// The local run's list of the parts it could not run.
        #[arg(long)]
        skipped: Option<PathBuf>,
    },
    /// Fails on a workflow job with a test step that does not run
    /// `ci/os-services.sh setup` before its first test step and before ADR
    /// 0043's deny (a Quick-suite check).
    Workflows,
}

/// The services a host must offer its tests.
#[derive(clap::Args)]
pub(crate) struct Services {
    /// The extra loopback addresses the tests use.
    #[arg(long, num_args = 1..)]
    loopback: Vec<Ipv4Addr>,
    /// Checks the Secret Service, the keychain on Linux, through libsecret's
    /// `secret-tool` on the session bus `DBUS_SESSION_BUS_ADDRESS` names.
    #[arg(long)]
    secret_service: bool,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Check { services, skipped } => {
            let missing = missing(&services);
            match skipped {
                None => Ok(check::report(
                    &missing.iter().map(Missing::finding).collect::<Vec<_>>(),
                )),
                Some(file) => {
                    skip(&missing, &environment.path(&file))?;
                    Ok(ExitCode::SUCCESS)
                }
            }
        }
        Command::Workflows => Ok(check::report(&workflows(&environment.root)?)),
    }
}

/// The setup step, run on a runner's own system or in the job's container.
const SETUP: [&str; 2] = [
    "ci/os-services.sh setup",
    "ci/vm/exec.sh ci/os-services.sh setup",
];

/// Every job with a test step sets the services up before its first one, and
/// before the deny, since the setup may install packages and builds `xtask`.
fn workflows(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for workflow in workflow::read_all(root)? {
        for job in &workflow.jobs {
            let Some(test) = job
                .steps
                .iter()
                .position(|step| step.name.as_deref().is_some_and(workflow::is_test_step))
            else {
                continue;
            };
            let subject = format!("{}#{}", workflow.file, job.id);
            let setup = job.steps.iter().position(|step| runs(step, &SETUP));
            let deny = job
                .steps
                .iter()
                .position(|step| runs(step, &[egress::DENY]));
            let problem = match setup {
                None => Some((
                    "missing-os-services-setup",
                    format!("a job with a test step runs `{}` first", SETUP[0]),
                )),
                Some(setup) if setup > test => Some((
                    "os-services-setup-after-test",
                    "the OS services are set up after a test step".to_owned(),
                )),
                Some(setup) if deny.is_some_and(|deny| setup > deny) => Some((
                    "os-services-setup-after-deny",
                    "the OS services are set up after the deny, which refuses their packages"
                        .to_owned(),
                )),
                Some(_) => None,
            };
            if let Some((kind, message)) = problem {
                findings.push(Finding::new(kind, subject, message));
            }
        }
    }
    Ok(findings)
}

/// Whether the step's script is one of `scripts`.
fn runs(step: &workflow::Step, scripts: &[&str]) -> bool {
    step.run
        .as_deref()
        .is_some_and(|run| scripts.contains(&run.trim()))
}

/// A service this host lacks, and why.
enum Missing {
    LoopbackAddress(Ipv4Addr, String),
    SecretService(String),
}

impl Missing {
    fn finding(&self) -> Finding {
        match self {
            Self::LoopbackAddress(address, problem) => Finding::new(
                "loopback-address-missing",
                address.to_string(),
                problem.clone(),
            ),
            Self::SecretService(problem) => {
                Finding::new("secret-service-missing", "Secret Service", problem.clone())
            }
        }
    }

    /// The module the tests needing the service sit in, as the test-name
    /// pattern nextest's `--skip` matches (TESTING.md, Storage, network and
    /// OS services).
    fn tests(&self) -> &'static str {
        match self {
            Self::LoopbackAddress(..) => "extra_loopback::",
            Self::SecretService(_) => "keychain::",
        }
    }
}

fn missing(services: &Services) -> Vec<Missing> {
    let mut missing: Vec<Missing> = services
        .loopback
        .iter()
        .filter_map(|address| {
            loopback(*address)
                .err()
                .map(|problem| Missing::LoopbackAddress(*address, problem))
        })
        .collect();
    if services.secret_service
        && let Err(problem) = secret_service()
    {
        missing.push(Missing::SecretService(problem));
    }
    missing
}

/// Appends one line per group of skipped tests to the local run's list, with
/// the services they miss, and prints the pattern that skips each group.
fn skip(missing: &[Missing], file: &Path) -> Result<()> {
    let mut groups: BTreeMap<&str, Vec<Finding>> = BTreeMap::new();
    for service in missing {
        groups
            .entry(service.tests())
            .or_default()
            .push(service.finding());
    }
    for (pattern, findings) in &groups {
        let subjects: Vec<&str> = findings
            .iter()
            .map(|finding| finding.subject.as_str())
            .collect();
        let first = findings
            .first()
            .map_or("", |finding| finding.message.as_str());
        let line = format!(
            "tests in `{pattern}` modules: skipped, this host lacks {} ({first})",
            subjects.join(", ")
        );
        output::append(file, &format!("{line}\n"))?;
        output::error(&format!("skipped: {line}"));
    }
    for pattern in groups.keys() {
        output::line(pattern);
    }
    Ok(())
}

/// An address answers for the tests when a socket binds to it: a host
/// without it refuses the bind, as macOS does for `127.0.0.2` without a `lo0`
/// alias. Binding is the whole check, since macOS's sandbox of ADR 0043's
/// deny refuses connections to any address but `127.0.0.1` itself.
#[expect(
    clippy::disallowed_types,
    reason = "the check of a loopback address binds a socket to it by definition"
)]
fn loopback(address: Ipv4Addr) -> Result<(), String> {
    std::net::TcpListener::bind((address, 0))
        .map(drop)
        .map_err(|error| format!("no socket binds to it ({error})"))
}

/// The attribute that names the check's own item.
const ATTRIBUTE: &str = "goodfetch-os-services-check";

/// Stores an item, reads it back and removes it, as the keychain provider's
/// tests do: the Secret Service is up, and its default collection exists and
/// is unlocked. The item is named by this process's ID, so checks never meet.
fn secret_service() -> Result<(), String> {
    let id = std::process::id().to_string();
    let value = format!("goodfetch os-services check {id}");
    secret_tool(
        "store",
        &["--label=Goodfetch OS services check", ATTRIBUTE, &id],
        Some(&value),
    )?;
    let read = secret_tool("lookup", &[ATTRIBUTE, &id], None)?;
    let cleared = secret_tool("clear", &[ATTRIBUTE, &id], None);
    if read != value {
        return Err("the item read back differs from the item stored".to_owned());
    }
    cleared.map(drop)
}

/// Runs `secret-tool <action> <args>`, writing `input` to it, and returns
/// what it printed.
fn secret_tool(action: &str, args: &[&str], input: Option<&str>) -> Result<String, String> {
    let failed = |error: std::io::Error| format!("secret-tool {action} did not run ({error})");
    let mut child = Process::new("secret-tool")
        .arg(action)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(failed)?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(text.as_bytes()).map_err(failed)?;
    }
    let output = child.wait_with_output().map_err(failed)?;
    if !output.status.success() {
        return Err(format!(
            "secret-tool {action} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
