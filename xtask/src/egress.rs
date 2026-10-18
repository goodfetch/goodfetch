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

use std::path::Path;

use anyhow::Result;

pub(crate) use canary::{Record, Refusals, attempt, report, verdict};
pub(crate) use declarations::check;
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
