//! The evidence of what each workspace member actually read, checked apart
//! from `xtask affected`'s member graph before the incremental gate reuses
//! a member's result (ADR 0135): a member reruns when the evidence shows a
//! changed file among its inputs, and when its evidence is missing or
//! unreadable (fail closed).
//! - **Rust**: Cargo's dep-info (`target/<profile>/deps/*.d`), which lists
//!   every source file and `include_str!` or `include_bytes!` target of a
//!   crate's last build, and its build scripts' `rerun-if-changed` paths.
//! - **TypeScript**: Vitest's `--changed` and Playwright's `--only-changed`
//!   listings since the head, which follow each test's own module graph.
//! - **Every member**: a string literal naming a changed file that no member
//!   owns, such as a test reading `TESTING.md` at run time
//!   (`affected::literal_readers`, which the selection reads too).
//!
//! Python's imports are read into the selection itself (`xtask affected`).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and blocking-process bans target async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result};

use super::plan::{UNOWNED, short};
use crate::affected::{Kind, RepoPath, WorkspaceMembers};
use crate::metadata;
use crate::runners::Runner;
use crate::runners::listing::{self, Listed, Location};

/// What each crate's last build read, by crate, as repository paths.
pub(crate) struct RustInputs {
    by_crate: BTreeMap<String, BTreeSet<String>>,
    /// Each crate's targets that a test build compiles (its library, its
    /// executables and its `tests/` targets), by their dep-info name.
    test_targets: BTreeMap<String, BTreeSet<String>>,
    /// Each crate's targets that have dep-info.
    seen: BTreeMap<String, BTreeSet<String>>,
    /// Where the dep-info was read, for the log.
    target: PathBuf,
}

/// The target kinds a crate's test build compiles.
const TESTED_KINDS: [&str; 7] = [
    "lib",
    "rlib",
    "dylib",
    "cdylib",
    "proc-macro",
    "bin",
    "test",
];

/// Reads the dep-info and build-script outputs of every profile's build
/// under `target`, and the test targets `cargo metadata` lists.
pub(crate) fn rust_inputs(
    root: &Path,
    cargo: &OsStr,
    target: &Path,
    members: &WorkspaceMembers,
) -> Result<RustInputs> {
    let test_targets: BTreeMap<String, BTreeSet<String>> = metadata::packages(cargo, root)?
        .into_iter()
        .map(|package| {
            let names = package
                .targets
                .iter()
                // A target that needs a feature builds only with it, so
                // its absence shows nothing.
                .filter(|target| {
                    target.required_features.is_empty()
                        && target
                            .kind
                            .iter()
                            .any(|kind| TESTED_KINDS.contains(&kind.as_str()))
                })
                .map(|target| target.name.replace('-', "_"))
                .collect();
            (package.name, names)
        })
        .collect();
    let mut seen: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let root =
        std::fs::canonicalize(root).with_context(|| format!("resolving {}", root.display()))?;
    let crates: BTreeSet<String> = members
        .all()
        .filter(|(_, kind)| *kind == Kind::Crate)
        .map(|(name, _)| name)
        .collect();
    let mut by_crate: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for profile in folders(target)? {
        for file in files(&profile.join("deps"))? {
            if file.extension() != Some(OsStr::new("d")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let paths: Vec<String> = dep_info_paths(&text)
                .into_iter()
                .filter_map(|path| repository_path(&root, Path::new(&path)))
                .filter(|path| !RepoPath::new(path.as_str()).is_in("target"))
                .collect();
            let owner = paths
                .iter()
                .filter(|path| Path::new(path).extension() == Some(OsStr::new("rs")))
                .find_map(|path| members.owner(path).filter(|owner| crates.contains(owner)));
            if let Some(owner) = owner {
                let stem = file.file_stem().and_then(OsStr::to_str).unwrap_or_default();
                let name = stem.rsplit_once('-').map_or(stem, |(name, _hash)| name);
                seen.entry(owner.clone())
                    .or_default()
                    .insert(name.to_owned());
                by_crate.entry(owner).or_default().extend(paths);
            }
        }
        for folder in folders(&profile.join("build"))? {
            let Some(name) = folder.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            let Some((krate, _hash)) = name.rsplit_once('-') else {
                continue;
            };
            let Some(crate_folder) = members.folder(krate) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(folder.join("output")) else {
                continue;
            };
            for line in text.lines() {
                let value = line
                    .strip_prefix("cargo::rerun-if-changed=")
                    .or_else(|| line.strip_prefix("cargo:rerun-if-changed="));
                if let Some(value) = value {
                    let path = root.join(&crate_folder).join(value);
                    if let Some(path) = repository_path(&root, &path) {
                        by_crate.entry(krate.to_owned()).or_default().insert(path);
                    }
                }
            }
        }
    }
    Ok(RustInputs {
        by_crate,
        test_targets,
        seen,
        target: target.to_path_buf(),
    })
}

impl RustInputs {
    /// Each crate a changed path reaches through what its build read, and
    /// each crate with no dep-info, which reruns.
    pub(crate) fn reached(
        &self,
        members: &WorkspaceMembers,
        changed: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        let mut reached = BTreeMap::new();
        for (krate, _) in members.all().filter(|(_, kind)| *kind == Kind::Crate) {
            match self.by_crate.get(&krate) {
                None => {
                    reached.insert(
                        krate,
                        format!(
                            "no dep-info of its build under {}, so nothing shows what it read",
                            self.target.display()
                        ),
                    );
                }
                Some(inputs) => {
                    let seen = self.seen.get(&krate);
                    let missing = self.test_targets.get(&krate).and_then(|targets| {
                        targets
                            .iter()
                            .find(|target| seen.is_none_or(|seen| !seen.contains(*target)))
                    });
                    if let Some(missing) = missing {
                        reached.insert(
                            krate,
                            format!(
                                "no dep-info of its test target `{missing}` under {}, so nothing shows what it read",
                                self.target.display()
                            ),
                        );
                    } else if let Some(path) = inputs.iter().find(|input| changed.contains(*input))
                    {
                        reached.insert(krate, format!("its build read {path}"));
                    }
                }
            }
        }
        reached
    }
}

/// The paths `text`, a Makefile-style dep-info file, names: each target and
/// prerequisite, with `\ ` read as a space.
fn dep_info_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in text.lines() {
        let mut token = String::new();
        let mut characters = line.chars().peekable();
        while let Some(character) = characters.next() {
            match character {
                '\\' if characters.peek() == Some(&' ') => {
                    token.push(' ');
                    characters.next();
                }
                ' ' | '\t' => {
                    push(&mut paths, &mut token);
                }
                _ => token.push(character),
            }
        }
        push(&mut paths, &mut token);
    }
    paths
}

fn push(paths: &mut Vec<String>, token: &mut String) {
    let path = token.trim_end_matches(':');
    if !path.is_empty() {
        paths.push(path.to_owned());
    }
    token.clear();
}

/// `path`, absolute or relative to `root`, as a `/`-separated repository
/// path, with `.` and `..` resolved; `None` outside the repository.
fn repository_path(root: &Path, path: &Path) -> Option<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let mut parts: Vec<String> = Vec::new();
    for component in absolute.strip_prefix(root).ok()?.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?.to_owned()),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn folders(folder: &Path) -> Result<Vec<PathBuf>> {
    Ok(entries(folder)?
        .into_iter()
        .filter(|path| path.is_dir())
        .collect())
}

fn files(folder: &Path) -> Result<Vec<PathBuf>> {
    Ok(entries(folder)?
        .into_iter()
        .filter(|path| path.is_file())
        .collect())
}

fn entries(folder: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", folder.display())),
    };
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(
            entry
                .with_context(|| format!("reading {}", folder.display()))?
                .path(),
        );
    }
    paths.sort();
    Ok(paths)
}

/// The members whose tests Vitest's `--changed` and Playwright's
/// `--only-changed` list since `origin`, each with the test; when a listing
/// fails, every pnpm member and the unowned tests, with the failure.
pub(crate) fn listed_since(
    root: &Path,
    cargo: &OsStr,
    origin: &str,
    members: &WorkspaceMembers,
) -> BTreeMap<String, String> {
    match try_listed_since(root, cargo, origin, members) {
        Ok(reached) => reached,
        Err(error) => members
            .all()
            .filter(|(_, kind)| *kind == Kind::TypeScript)
            .map(|(name, _)| name)
            .chain([UNOWNED.to_owned()])
            .map(|member| {
                (
                    member,
                    format!(
                        "the runners' listing since {} failed, so nothing shows what its tests read ({error:#})",
                        short(origin)
                    ),
                )
            })
            .collect(),
    }
}

fn try_listed_since(
    root: &Path,
    cargo: &OsStr,
    origin: &str,
    members: &WorkspaceMembers,
) -> Result<BTreeMap<String, String>> {
    let canonical =
        std::fs::canonicalize(root).with_context(|| format!("resolving {}", root.display()))?;
    let mut reached = BTreeMap::new();
    for runner in [Runner::Vitest, Runner::Playwright] {
        let output = listing::output(runner, &canonical, cargo, None, None, Some(origin))?;
        for Listed { test, location } in listing::parse(runner, &canonical, &output)? {
            let member = match &location {
                Location::File(Some(file)) => {
                    members.owner(file).unwrap_or_else(|| UNOWNED.to_owned())
                }
                Location::File(None) | Location::Target { .. } => UNOWNED.to_owned(),
            };
            reached.entry(member).or_insert_with(|| {
                format!(
                    "{} lists its test `{test}` among those the change since {} reaches",
                    runner.name(),
                    short(origin)
                )
            });
        }
    }
    Ok(reached)
}

/// The test files no member owns, by the runners' file patterns: each
/// part reruns them, since no member's result covers them (the review's
/// F3, `console/screens.spec.ts`).
pub(crate) fn unowned_tests(root: &Path, members: &WorkspaceMembers) -> Result<Vec<String>> {
    let text = crate::git::text(root, &["ls-files", "-z"])?;
    Ok(text
        .split('\0')
        .filter(|path| is_test_file(path))
        .filter(|path| members.owner(path).is_none())
        .map(str::to_owned)
        .collect())
}

/// Whether the path is a test file of Vitest, Playwright or pytest, outside
/// the fixtures, the proofs and the R&D archive.
fn is_test_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or_default();
    let script = [
        ".spec.ts",
        ".spec.tsx",
        ".test.ts",
        ".test.tsx",
        ".test.mts",
        ".test.js",
        ".test.mjs",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix));
    let stem = Path::new(name)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    let python = Path::new(name).extension() == Some(OsStr::new("py"))
        && (stem.starts_with("test_") || stem.ends_with("_test"));
    let excluded = path.contains("/fixtures/")
        || path.contains("node_modules/")
        || ["rnd", "ci/proofs"]
            .iter()
            .any(|folder| RepoPath::new(path).is_in(folder));
    (script || python) && !excluded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dep_info_lists_each_target_and_prerequisite() {
        let text = "/r/target/debug/deps/it-1.d: crates/a/tests/it/main.rs /r/TESTING.md my\\ file.rs\n\ncrates/a/tests/it/main.rs:\n/r/TESTING.md:\n";

        assert_eq!(
            dep_info_paths(text),
            [
                "/r/target/debug/deps/it-1.d",
                "crates/a/tests/it/main.rs",
                "/r/TESTING.md",
                "my file.rs",
                "crates/a/tests/it/main.rs",
                "/r/TESTING.md",
            ]
        );
    }

    #[test]
    fn a_path_is_read_from_the_repository_root() {
        let root = Path::new("/r");

        assert_eq!(
            repository_path(root, Path::new("crates/a/../../TESTING.md")).as_deref(),
            Some("TESTING.md")
        );
        assert_eq!(repository_path(root, Path::new("/elsewhere/x.rs")), None);
        assert_eq!(repository_path(root, Path::new("../x.rs")), None);
    }

    #[test]
    fn test_files_are_known_by_the_runners_patterns_outside_fixtures() {
        assert!(is_test_file("console/screens.spec.ts"));
        assert!(is_test_file("sdks/python/x/tests/test_results.py"));
        assert!(!is_test_file(
            "xtask/tests/it/fixtures/stress/web/changed.test.ts"
        ));
        assert!(!is_test_file("ci/proofs/oxlint/a.test.ts"));
        assert!(!is_test_file("console/collie/src/app.tsx"));
    }
}
