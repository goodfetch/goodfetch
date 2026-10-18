//! The no-retry check (ADRs 0064 and 0074): no test runs again after it
//! failed. It refuses retries in runner configurations, per-test overrides
//! included, and the flags and variables that undo the runners' locks.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod nextest;

use std::path::Path;

use anyhow::Result;

use crate::check::Finding;

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    nextest::check(root)
}
