//! `xtask affected`, the pull-request tier's one change-detection job (ADRs
//! 0032 and 0061): from the paths a change touches, which jobs run and what
//! each test runner selects. CI gives it a base and a head; locally it diffs
//! the working tree, uncommitted and untracked files included, against the
//! merge base with `origin/main`, or with the branch `--since` names. Both
//! modes feed the same rules.

mod git;
mod imports;
mod parts;
mod readers;
mod selection;
mod workspace;

use std::ffi::OsStr;
use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;

use crate::environment::Environment;
use crate::output;

pub(crate) use git::Range;
pub(crate) use parts::ChangedParts;
pub(crate) use readers::{literal_readers, tests_read_elsewhere};
pub(crate) use selection::{Job, Selection, selectable_job_ids, tests_folder};
use workspace::{Ecosystem, Workspace};

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
    select_among(root, &WorkspaceMembers::read(cargo, root)?, range)
}

/// The selection for `range` among `members`, the repository's at `root`.
pub(crate) fn select_among(
    root: &Path,
    members: &WorkspaceMembers,
    range: &Range,
) -> Result<Selection> {
    let paths = changed_paths(root, range)?;
    let parts = changed_parts(root, range, members, &paths)?;
    select_with_readers(root, members, &paths, &parts)
}

/// The parts of each changed Markdown file no member owns, read from both
/// sides of `range`; a file missing from one side is left out, so it counts
/// as changed throughout.
fn changed_parts(
    root: &Path,
    range: &Range,
    members: &WorkspaceMembers,
    paths: &[RepoPath],
) -> Result<ChangedParts> {
    let mut parts = ChangedParts::default();
    let documents = paths.iter().filter(|path| {
        members.owner(path.as_str()).is_none()
            && Path::new(path.as_str())
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    });
    for path in documents {
        if let (Some(before), Some(after)) = git::versions(root, range, path)? {
            parts.insert(path.clone(), &before, &after);
        }
    }
    Ok(parts)
}

/// The selection for a change to `paths` in the repository at `root`, in
/// which a member whose own files name a changed path no member owns is
/// changed too: its test may read that file at run time (`TESTING.md`),
/// which no build or import shows, and the quick gate runs no such test
/// (ADR 0135). The gate's evidence widens its parts the same way. Of a
/// document whose `parts` were read, only the changed parts' readers are
/// (`literal_readers`). A crate whose changed `tests/` file a file outside
/// that folder names is changed throughout, as before #2238
/// (`tests_read_elsewhere`).
fn select_with_readers(
    root: &Path,
    members: &WorkspaceMembers,
    paths: &[RepoPath],
    parts: &ChangedParts,
) -> Result<Selection> {
    let mut widened = paths.to_vec();
    let readers = literal_readers(root, members, paths, parts)?;
    let shared = tests_read_elsewhere(root, members, paths)?;
    widened.extend(
        readers
            .keys()
            .chain(shared.keys())
            .filter_map(|member| members.folder(member))
            .map(|folder| RepoPath::new(format!("{folder}/.evidence"))),
    );
    Ok(selection::select(&members.workspace, &widened, parts))
}

/// The files under `folder` with one of `extensions`, outside installed
/// packages, build outputs and caches.
pub(crate) fn source_files(folder: &Path, extensions: &[&str]) -> Result<Vec<std::path::PathBuf>> {
    imports::files(folder, extensions)
}

/// The paths `range` changes in the repository at `root`.
pub(crate) fn changed_paths(root: &Path, range: &Range) -> Result<Vec<RepoPath>> {
    git::changed_paths(root, range)
}

/// The pseudo-member that names the tests no workspace member owns, such as
/// `console/screens.spec.ts`, where a member's name is taken.
pub(crate) const UNOWNED: &str = ":unowned";

/// Which runners test a member of the workspaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A crate, tested by nextest.
    Crate,
    /// A pnpm package, tested by Vitest and Playwright.
    TypeScript,
    /// A uv package, tested by pytest.
    Python,
}

/// The members of the repository's three workspaces, each named as `just
/// test` takes it and as the selection lists it: a crate by its name, a
/// pnpm or uv package by its folder.
pub(crate) struct WorkspaceMembers {
    workspace: Workspace,
}

impl WorkspaceMembers {
    pub(crate) fn read(cargo: &OsStr, root: &Path) -> Result<Self> {
        Ok(Self::from_workspace(workspace::read(cargo, root)?))
    }

    pub(crate) fn from_workspace(workspace: Workspace) -> Self {
        Self { workspace }
    }

    /// Whether `member` owns a visual test, which the visual job runs.
    pub(crate) fn owns_a_visual_test(&self, member: &str) -> bool {
        self.workspace
            .visual_tests
            .iter()
            .any(|test| self.owner(test.as_str()).as_deref() == Some(member))
    }

    /// Every member's name and kind.
    pub(crate) fn all(&self) -> impl Iterator<Item = (String, Kind)> + '_ {
        let crates = self
            .workspace
            .crates
            .iter()
            .map(|krate| (krate.name.clone(), Kind::Crate));
        let packages = self.workspace.packages.iter().map(|package| {
            let kind = match package.ecosystem {
                Ecosystem::TypeScript => Kind::TypeScript,
                Ecosystem::Python => Kind::Python,
            };
            (package.folder.as_str().to_owned(), kind)
        });
        crates.chain(packages)
    }

    /// The folder of the member named `name`, as a repository path.
    pub(crate) fn folder(&self, name: &str) -> Option<String> {
        self.workspace
            .crates
            .iter()
            .find(|krate| krate.name == name)
            .map(|krate| krate.folder.as_str().to_owned())
            .or_else(|| {
                self.workspace
                    .packages
                    .iter()
                    .find(|package| package.folder.as_str() == name)
                    .map(|package| package.folder.as_str().to_owned())
            })
    }

    /// The member that owns `path`, a repository path: the one whose folder
    /// holds it most closely.
    pub(crate) fn owner(&self, path: &str) -> Option<String> {
        selection::owner_name(&self.workspace, &RepoPath::new(path))
    }

    /// The crate whose `tests/` folder holds `path`, as its name and its
    /// folder.
    pub(crate) fn tests_owner(&self, path: &RepoPath) -> Option<(&str, &str)> {
        selection::tests_owner(&self.workspace, path)
            .map(|krate| (krate.name.as_str(), krate.folder.as_str()))
    }

    /// The selection for a change to `paths`, each changed throughout.
    pub(crate) fn select(&self, paths: &[RepoPath]) -> Selection {
        selection::select(&self.workspace, paths, &ChangedParts::default())
    }
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
    /// against the merge base with `origin/main`, or with `--since`.
    #[arg(long, conflicts_with_all = ["base", "head"])]
    local: bool,
    /// With `--local`, the revision whose merge base with `HEAD` the working
    /// tree is read against, in place of `origin/main`: the integration
    /// branch a ticket goes into, or what a push's remote ref holds.
    #[arg(long, requires = "local")]
    since: Option<String>,
}

pub(crate) fn run(arguments: Arguments, environment: &Environment) -> Result<ExitCode> {
    let range = match (arguments.base, arguments.head) {
        (Some(base), Some(head)) => Range::Commits { base, head },
        _ => Range::WorkingTree {
            since: arguments.since,
        },
    };
    let selection = select(&environment.cargo, &environment.root, &range)?;
    output::line(selection.to_string().trim_end());
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests;
