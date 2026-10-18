//! `xtask affected`, the pull-request tier's one change-detection job (ADRs
//! 0032 and 0061): from the paths a change touches, which jobs run and what
//! each test runner selects. CI gives it a base and a head; locally it diffs
//! the working tree, uncommitted and untracked files included, against the
//! merge base with `origin/main`. Both modes feed the same rules.

mod git;
mod selection;
mod workspace;

use std::ffi::OsStr;
use std::path::Path;

use anyhow::Result;

pub(crate) use git::Range;
pub(crate) use selection::{Selection, selectable_job_ids};

/// A path relative to the repository root, with `/` separators and no
/// trailing one: a changed file or a package's folder.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RepoPath(String);

impl RepoPath {
    pub(crate) fn new(path: impl Into<String>) -> Self {
        let path: String = path.into();
        Self(path.trim_end_matches('/').to_owned())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this path is `folder` or lies inside it.
    pub(crate) fn is_in(&self, folder: &str) -> bool {
        let folder = folder.trim_end_matches('/');
        self.0 == folder
            || self
                .0
                .strip_prefix(folder)
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

/// The selection for `range` in the repository at `root`.
pub(crate) fn select(cargo: &OsStr, root: &Path, range: &Range) -> Result<Selection> {
    let paths = git::changed_paths(root, range)?;
    let workspace = workspace::read(cargo, root)?;
    Ok(selection::select(&workspace, &paths))
}

#[cfg(test)]
mod tests;
