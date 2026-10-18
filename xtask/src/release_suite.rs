//! The Release suite's private-period runs (ADRs 0061 and 0108): the
//! workflow's first job, which chooses the cells a manual dispatch starts
//! under the ceiling of included minutes, and a commit's verdict across its
//! dispatches. Native macOS Intel and Windows arm64, and the merge queue,
//! join once the repository is public.

mod cells;
mod history;
mod plan;
mod time;
mod verdict;

use anyhow::Result;

use crate::check::Finding;
use crate::github::Api;

pub(crate) use plan::{Dispatch, plan};

/// Whether every cell is green on `commit`, across its dispatches.
pub(crate) fn verdict(api: Api<'_>, repository: &str, commit: &str) -> Result<Vec<Finding>> {
    verdict::verdict(&mut history::History::new(api, repository), commit)
}
