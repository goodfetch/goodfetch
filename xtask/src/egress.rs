//! ADR 0043's outbound-traffic deny in CI: the one list of what each job may
//! reach (`ci/egress/endpoints.toml`), the check that every job is declared
//! under or outside the deny with the deny before its tests, the rules the
//! deny loads (nftables in a Linux job's VM or network namespace, firewall
//! rules on Windows), and the Egress canary's verdict. `ci/egress/egress.sh`
//! runs them in each job.

mod canary;
mod declarations;
mod endpoints;
mod rules;
mod windows;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;

use crate::environment::Environment;
use crate::output;

pub(crate) use canary::{Record, Refusals, attempt, report, verdict};
pub(crate) use declarations::{DENY, check};
pub(crate) use endpoints::Endpoint;
use endpoints::{Endpoints, JobKey};

/// What a job under the deny may reach (ADR 0043).
pub(crate) fn allowed(root: &Path, job: &str) -> Result<Vec<Endpoint>> {
    Endpoints::read(root)?.allowed(&JobKey::try_from(job.to_owned())?)
}

/// Where the deny is enforced, which decides the form of its rules.
pub(crate) enum Enforcer {
    /// nftables in a Linux VM or network namespace, refusing every user's
    /// traffic and counting and logging that of the user running the product.
    Nftables { uid: u32 },
    /// Windows Firewall block rules for the product's programs.
    WindowsFirewall,
}

/// The rules of a job's deny.
pub(crate) fn rules(
    root: &Path,
    job: &str,
    resolved: &Path,
    enforcer: &Enforcer,
) -> Result<String> {
    let job = JobKey::try_from(job.to_owned())?;
    match enforcer {
        Enforcer::Nftables { uid } => rules::rules(root, &job, resolved, rules::Uid(*uid)),
        Enforcer::WindowsFirewall => {
            let endpoints = Endpoints::read(root)?;
            Ok(windows::rules(&rules::resolved_addresses(
                &endpoints, &job, resolved,
            )?))
        }
    }
}

/// `xtask egress`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on a workflow job not declared in `ci/egress/endpoints.toml`,
    /// and on a job under the deny whose steps do not deny before its tests,
    /// run the Egress canary right after, and check the deny's log at the end.
    Check,
    /// Prints what a job under the deny may reach, one `<host> <port>` line
    /// each: the job's named endpoints, none for a job only under the deny.
    Endpoints {
        /// The job, as `<workflow file>#<job id>`.
        #[arg(long)]
        job: String,
    },
    /// Prints the nftables ruleset that denies a job's outbound traffic but
    /// to its allowed endpoints, at the addresses resolved before the deny.
    /// With `--windows`, prints the Windows Firewall block rules instead, one
    /// `<protocol> <remote addresses> <remote ports>` line each.
    Rules {
        /// The job, as `<workflow file>#<job id>`.
        #[arg(long)]
        job: String,
        /// A file of `<host> <port> <address>` lines, resolved before the deny.
        #[arg(long)]
        resolved: PathBuf,
        #[command(flatten)]
        enforcer: EnforcerArguments,
    },
    /// The Egress canary: a forbidden connection that must be refused.
    Canary {
        #[command(subcommand)]
        command: CanaryCommand,
    },
    /// Fails on every connection the deny refused but the canary's, read
    /// from the deny's counters and its kernel log at the job's end.
    Report {
        #[command(flatten)]
        refusals: RefusalArguments,
    },
}

/// Where the deny's rules are enforced: exactly one of the two.
#[derive(clap::Args)]
#[group(required = true, multiple = false)]
pub(crate) struct EnforcerArguments {
    /// The user that runs the product, in the job's VM or network namespace.
    #[arg(long)]
    uid: Option<u32>,
    /// The rules of the Windows cell's firewall.
    #[arg(long)]
    windows: bool,
}

/// Where the deny recorded its refusals: the counters with the kernel log,
/// or the Windows firewall's log.
#[derive(clap::Args)]
#[group(required = true, multiple = true)]
pub(crate) struct RefusalArguments {
    /// The output of `nft --json list counters table inet goodfetch_egress`.
    #[arg(long, requires = "log", conflicts_with = "firewall_log")]
    counters: Option<PathBuf>,
    /// The kernel log of the deny's VM or runner.
    #[arg(long, requires = "counters", conflicts_with = "firewall_log")]
    log: Option<PathBuf>,
    /// The Windows firewall's log of dropped packets.
    #[arg(long)]
    firewall_log: Option<PathBuf>,
}

#[derive(clap::Subcommand)]
pub(crate) enum CanaryCommand {
    /// Tries the canary's connection and writes how it ended.
    Attempt {
        #[arg(long)]
        outcome: PathBuf,
    },
    /// Passes when the attempt did not connect and the deny recorded it.
    Verdict {
        /// The file `attempt` wrote.
        #[arg(long)]
        outcome: PathBuf,
        #[command(flatten)]
        record: RecordArguments,
    },
}

/// How the deny recorded the canary's attempt: exactly one of its counters
/// before and after, or the Windows firewall's log.
#[derive(clap::Args)]
#[group(required = true, multiple = true)]
pub(crate) struct RecordArguments {
    /// The deny's counters before the attempt, as `nft --json` lists them.
    #[arg(long, requires = "after", conflicts_with = "firewall_log")]
    before: Option<PathBuf>,
    /// The deny's counters after the attempt.
    #[arg(long, requires = "before", conflicts_with = "firewall_log")]
    after: Option<PathBuf>,
    /// The Windows firewall's log of dropped packets.
    #[arg(long)]
    firewall_log: Option<PathBuf>,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    match command {
        Command::Check => Ok(crate::check::report(&check(root)?)),
        Command::Endpoints { job } => {
            for endpoint in allowed(root, &job)? {
                output::line(&format!("{} {}", endpoint.host, endpoint.port));
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Rules {
            job,
            resolved,
            enforcer,
        } => {
            let enforcer = match enforcer.uid {
                Some(uid) => Enforcer::Nftables { uid },
                None => Enforcer::WindowsFirewall,
            };
            output::line(rules(root, &job, &environment.path(&resolved), &enforcer)?.trim_end());
            Ok(ExitCode::SUCCESS)
        }
        Command::Canary {
            command: CanaryCommand::Attempt { outcome },
        } => {
            attempt(root, &environment.path(&outcome))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Canary {
            command: CanaryCommand::Verdict { outcome, record },
        } => {
            let paths = (
                record.before.map(|path| environment.path(&path)),
                record.after.map(|path| environment.path(&path)),
                record.firewall_log.map(|path| environment.path(&path)),
            );
            let record = match &paths {
                (Some(before), Some(after), _) => Record::Counters { before, after },
                (None, None, Some(log)) => Record::FirewallLog(log),
                _ => anyhow::bail!("give --before and --after, or --firewall-log"),
            };
            Ok(crate::check::report(&verdict(
                root,
                &environment.path(&outcome),
                &record,
            )?))
        }
        Command::Report { refusals } => {
            let paths = (
                refusals.counters.map(|path| environment.path(&path)),
                refusals.log.map(|path| environment.path(&path)),
                refusals.firewall_log.map(|path| environment.path(&path)),
            );
            let refusals = match &paths {
                (Some(counters), Some(log), _) => Refusals::Nftables { counters, log },
                (_, _, Some(log)) => Refusals::FirewallLog(log),
                _ => anyhow::bail!("give --counters and --log, or --firewall-log"),
            };
            Ok(crate::check::report(&report(root, &refusals)?))
        }
    }
}
