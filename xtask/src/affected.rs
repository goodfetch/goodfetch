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
use std::process::ExitCode;

use anyhow::Result;

use crate::environment::Environment;
use crate::output;

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

/// `xtask affected`'s arguments: a base and a head, or `--local`.
#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("change").required(true).args(["base", "local"])))]
pub(crate) struct Arguments {
    /// The change's base, for CI: the change is what `--head` changed
    /// since its merge base with it.
    #[arg(long, requires = "head")]
    base: Option<String>,
    #[arg(long, requires = "base")]
    head: Option<String>,
    /// The working tree, uncommitted and untracked files included,
    /// against the merge base with `origin/main`.
    #[arg(long, conflicts_with_all = ["base", "head"])]
    local: bool,
}

pub(crate) fn run(arguments: Arguments, environment: &Environment) -> Result<ExitCode> {
    let range = match (arguments.base, arguments.head) {
        (Some(base), Some(head)) => Range::Commits { base, head },
        _ => Range::WorkingTree,
    };
    let selection = select(&environment.cargo, &environment.root, &range)?;
    output::line(selection.to_string().trim_end());
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests;
