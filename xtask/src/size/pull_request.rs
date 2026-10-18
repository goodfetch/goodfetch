//! A pull request's acceptance of a size increase (ADR 0057, as amended by
//! ADR 0133), through the rule `xtask perf` shares (`crate::acceptance`): a
//! file past its increase threshold passes only when
//!
//! - the body's "Size increase" section has a line naming the file's
//!   identifier, as the table's finding prints it, with a delta in MB at
//!   least the measured one: `- collie/x86_64-unknown-linux-gnu/<file>: +1.10 MB, <reason>`;
//! - and a comment by someone with write access says "Size increase
//!   accepted" and names the pull request's head SHA.

use anyhow::Result;

use crate::acceptance::{self, Acceptance, Rule};

pub(crate) use crate::acceptance::PullRequest;

use super::budget::Bytes;

const RULE: Rule = Rule {
    heading: "size increase",
    phrase: "size increase accepted",
};

/// The ADRs' MB has six decimals of bytes.
const MB_DECIMALS: usize = 6;

/// Reads what the pull request accepts.
pub(crate) fn read(pull_request: &PullRequest<'_>) -> Result<Acceptance> {
    pull_request.read(&RULE)
}

/// Whether the file `id`, grown by `change`, is accepted.
pub(crate) fn accepts(acceptance: &Acceptance, id: &str, change: Bytes) -> bool {
    acceptance.accepts(id, &change, |text| {
        acceptance::decimal(text, "MB", MB_DECIMALS).map(Bytes)
    })
}
