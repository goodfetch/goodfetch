//! ADR 0043's outbound-traffic deny in CI: the one list of what each job may
//! reach (`ci/egress/endpoints.toml`), the check that every job is declared
//! under or outside the deny with the deny before its tests, the rules the
//! job's VM loads, and the Egress canary's verdict. `ci/egress/egress.sh`
//! runs them in each job.

mod canary;
mod declarations;
mod endpoints;
mod rules;

use std::path::Path;

use anyhow::Result;

pub(crate) use canary::{attempt, report, verdict};
pub(crate) use declarations::check;
pub(crate) use endpoints::Endpoint;
use endpoints::{Endpoints, JobKey};

/// What a job under the deny may reach (ADR 0043).
pub(crate) fn allowed(root: &Path, job: &str) -> Result<Vec<Endpoint>> {
    Endpoints::read(root)?.allowed(&JobKey::try_from(job.to_owned())?)
}

/// The nftables ruleset of a job's deny, for the user that runs its container.
pub(crate) fn rules(root: &Path, job: &str, resolved: &Path, uid: u32) -> Result<String> {
    rules::rules(
        root,
        &JobKey::try_from(job.to_owned())?,
        resolved,
        rules::Uid(uid),
    )
}
