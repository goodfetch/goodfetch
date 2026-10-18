//! A pull request's acceptance of tripped figures (ADR 0071, as amended by
//! ADR 0133), through the rule `xtask size` shares (`crate::acceptance`): a
//! figure past its increase threshold passes only when the body's
//! "Performance change" section lists it with its change at least the
//! measured one and its reason, and a comment by someone with write access
//! says "Performance change accepted" and names the head SHA:
//!
//! ```text
//! ## Performance change
//!
//! - goodfetch-frontier/instructions::frontier::push instructions: +5.10%, the new index
//! - size-limit/collie-client brotli: +6.20 kB, the extractor's tables
//! ```
//!
//! A job reads it before denying outbound traffic and saves it as JSON; the
//! comparison then reads that file. A ceiling is never accepted.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::acceptance::{self, Acceptance, PullRequest, Rule};
use crate::github::Api;

use super::compare::{Increase, Row, Verdict};

const RULE: Rule = Rule {
    heading: "performance change",
    phrase: "performance change accepted",
};

/// A listed change has two decimals: hundredths of a percent, or of a kB.
const DECIMALS: usize = 2;

/// Which pull request's acceptance to read.
pub(crate) enum Source<'a> {
    /// The pull-request tier's own pull request.
    Number(u64),
    /// The pull request that merged a commit of `main`, for the Release
    /// suite's run of it; a commit no pull request merged accepts nothing.
    MergedCommit(&'a str),
}

/// Reads the pull request's acceptance and saves it at `save`.
pub(crate) fn save(api: Api<'_>, repository: &str, source: &Source<'_>, save: &Path) -> Result<()> {
    let number = match source {
        Source::Number(number) => Some(*number),
        Source::MergedCommit(commit) => acceptance::merged_by(&api, repository, commit)?,
    };
    let acceptance = match number {
        Some(number) => PullRequest {
            api,
            repository,
            number,
        }
        .read(&RULE)?,
        None => Acceptance::nothing(),
    };
    if let Some(folder) = save.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::write(save, serde_json::to_vec_pretty(&acceptance)?)
        .with_context(|| format!("writing {}", save.display()))
}

pub(crate) fn read(path: &Path) -> Result<Acceptance> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

/// Marks each tripped figure the pull request accepts.
pub(crate) fn accept(rows: &mut [Row], acceptance: &Acceptance) {
    for row in rows
        .iter_mut()
        .filter(|row| row.verdict == Verdict::Tripped)
    {
        let Some(change) = row
            .listed_change()
            .and_then(|change| u64::try_from(change).ok())
        else {
            continue;
        };
        let unit = match row.limit.increase {
            Increase::Percent(_) => "%",
            Increase::Bytes(_) => "kB",
        };
        if acceptance.accepts(&row.measured.id(), &change, |text| {
            acceptance::decimal(text, unit, DECIMALS)
        }) {
            row.verdict = Verdict::Accepted;
        }
    }
}
