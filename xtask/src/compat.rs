//! `xtask compat` (ADR 0066): the Compatibility corpus under `compat/`, one
//! set of files per version of each format the registry in the committed
//! bundle lists.
//!
//! - `compat/<format>/head/` holds what the format's writer produces now;
//!   `write` regenerates it.
//! - `compat/<format>/<major>.<minor>/` holds a frozen set, with a
//!   `SHA256SUMS` file; `freeze --tag` copies it from a release tag's `head/`.
//! - `compat/releases.toml` records what each release shipped, and so which
//!   frozen sets must exist.
//!
//! Each set holds the writer's files and `format.json`, the format's name,
//! version, storage and schema as the registry gave them.
//!
//! `refusals` writes each registered format's generated refusal cases.
//!
//! `jobs` writes the Release suite's skew jobs, one set per released minor.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod coverage;
mod frozen;
pub(crate) mod jobs;
pub(crate) mod refusals;
mod registry;
mod sql;
mod tag;
mod tree;
mod writer;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use goodfetch_protocol::releases::{FormatName, FormatVersion, ReleasesFile};

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::output;
use registry::Format;
use tree::Tree;
use writer::Writers;

/// The corpus's folder.
const COMPAT: &str = "compat";

/// What each release shipped (ADR 0066).
const RELEASES: &str = "compat/releases.toml";

/// The folder of a format's current version.
const HEAD: &str = "head";

/// The formats the committed bundle registers.
fn formats(root: &Path) -> Result<Vec<Format>> {
    let path = root.join(registry::BUNDLE);
    let text = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    registry::parse(&text).with_context(|| format!("reading {}", path.display()))
}

/// The releases `compat/releases.toml` records; none before it exists.
fn releases(root: &Path) -> Result<ReleasesFile> {
    let path = root.join(RELEASES);
    if !path.exists() {
        return Ok(ReleasesFile::default());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// The sets of one format's folder under `compat/`.
#[derive(Default)]
struct Sets {
    head: bool,
    frozen: BTreeSet<FormatVersion>,
}

/// Every format folder under `compat/` and the sets it holds, by its name.
fn folders(root: &Path) -> Result<BTreeMap<String, Sets>> {
    let mut folders = BTreeMap::new();
    let compat = root.join(COMPAT);
    if !compat.is_dir() {
        return Ok(folders);
    }
    for entry in std::fs::read_dir(&compat).with_context(|| format!("reading {COMPAT}"))? {
        let entry = entry.with_context(|| format!("reading {COMPAT}"))?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let mut sets = Sets::default();
        for set in std::fs::read_dir(entry.path())? {
            let set = set?;
            if !set.file_type()?.is_dir() {
                continue;
            }
            let name = set.file_name().to_string_lossy().into_owned();
            if name == HEAD {
                sets.head = true;
            } else if let Ok(version) = FormatVersion::try_from(name.as_str()) {
                sets.frozen.insert(version);
            }
        }
        folders.insert(entry.file_name().to_string_lossy().into_owned(), sets);
    }
    Ok(folders)
}

fn set_folder(root: &Path, format: &str, set: &str) -> PathBuf {
    root.join(COMPAT).join(format).join(set)
}

/// A path under the root as findings name it.
fn subject(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// What `check` found.
pub(crate) struct Checked {
    /// What is wrong with the corpus.
    pub(crate) findings: Vec<Finding>,
    /// One `identical` note per writer whose two runs wrote the same files,
    /// with their digest, so a passing check's log is never empty and two
    /// checks' logs compare.
    pub(crate) notes: Vec<Finding>,
}

/// What `check` finds wrong with the corpus, and what each writer wrote.
pub(crate) fn check(cargo: &OsStr, root: &Path) -> Result<Checked> {
    let formats = formats(root)?;
    let folders = folders(root)?;
    let writers = Writers::find(cargo, root)?;
    // The writer of each format with a committed `head/` set, run together.
    let written: Vec<&Format> = formats
        .iter()
        .filter(|format| writers.has(format))
        .filter(|format| {
            tree::read(&set_folder(root, format.name.as_str(), HEAD))
                .is_ok_and(|committed| !committed.is_empty())
        })
        .collect();
    let first = writers.write(cargo, root, &written)?;
    // Every writer runs a second time, after the first run of all of them,
    // and must write the same bytes: a writer right most of the time would
    // otherwise pass the check by chance and fail a later one (#1924's run
    // state differed in 1 run of 19).
    let second = writers.write(cargo, root, &written)?;
    let mut findings = Vec::new();
    let mut notes = Vec::new();
    for ((format, first), second) in written.iter().zip(&first).zip(&second) {
        let folder = subject(root, &set_folder(root, format.name.as_str(), HEAD));
        if first == second {
            notes.push(Finding::new(
                "identical",
                folder,
                format!(
                    "both runs of its writer wrote {} file(s), sha256 {} over their SHA256SUMS",
                    first.len(),
                    frozen::digest(first)
                ),
            ));
        } else {
            nondeterministic(&folder, first, second, &mut findings);
        }
    }
    let mut produced: BTreeMap<&str, Tree> = written
        .iter()
        .map(|format| format.name.as_str())
        .zip(first)
        .collect();
    for format in &formats {
        let produced = produced.remove(format.name.as_str());
        check_format(root, format, &folders, &writers, produced, &mut findings)?;
    }
    check_folders(root, &formats, &folders, &mut findings)?;
    check_shipped(root, &folders, &mut findings)?;
    Ok(Checked { findings, notes })
}

/// A registered format: its writer, its version against its newest frozen
/// set, its `head/` set against what the writer produces, and the set's
/// schema coverage.
fn check_format(
    root: &Path,
    format: &Format,
    folders: &BTreeMap<String, Sets>,
    writers: &Writers,
    produced: Option<Tree>,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let has_writer = writers.has(format);
    if !has_writer {
        findings.push(Finding::new(
            "no-writer",
            format!("{COMPAT}/{}", format.name),
            format!(
                "a registered format needs a writer: a `{}` example in the package that owns it",
                writer::example(format)
            ),
        ));
    }
    let newest = folders
        .get(format.name.as_str())
        .and_then(|sets| sets.frozen.last());
    if newest == Some(&format.version) && schema_changed_since_frozen(root, format) {
        findings.push(Finding::new(
            "unbumped",
            format!("{COMPAT}/{}", format.name),
            format!(
                "its schema differs from the frozen {}'s; raise the format's version",
                format.version
            ),
        ));
    }
    let folder = set_folder(root, format.name.as_str(), HEAD);
    let committed = tree::read(&folder)?;
    if committed.is_empty() {
        findings.push(Finding::new(
            "missing-head",
            subject(root, &folder),
            "a registered format needs its head/ set; run `xtask compat write`",
        ));
        return Ok(());
    }
    if let Some(produced) = produced {
        drift(&subject(root, &folder), &committed, &produced, findings);
    }
    for path in coverage::uncovered(format, &committed)
        .with_context(|| format!("reading {}", subject(root, &folder)))?
    {
        findings.push(Finding::new(
            "coverage",
            format!("{}:{path}", subject(root, &folder)),
            "no document of the set gives it a value; extend the format's scenarios",
        ));
    }
    Ok(())
}

/// Every format folder: a `head/` set left by a format no longer registered,
/// and each frozen set against its `SHA256SUMS`.
fn check_folders(
    root: &Path,
    formats: &[Format],
    folders: &BTreeMap<String, Sets>,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let registered: BTreeSet<&str> = formats.iter().map(|format| format.name.as_str()).collect();
    for (name, sets) in folders {
        if sets.head && !registered.contains(name.as_str()) {
            let folder = set_folder(root, name, HEAD);
            drift(
                &subject(root, &folder),
                &tree::read(&folder)?,
                &Tree::new(),
                findings,
            );
        }
        for version in &sets.frozen {
            let folder = set_folder(root, name, &version.to_string());
            for path in frozen::broken(&tree::read(&folder)?) {
                findings.push(Finding::new(
                    "frozen",
                    format!("{}/{path}", subject(root, &folder)),
                    format!(
                        "breaks the set's {}; a frozen file never changes",
                        frozen::SUMS
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// A frozen set for each format version `compat/releases.toml` records as
/// shipped.
fn check_shipped(
    root: &Path,
    folders: &BTreeMap<String, Sets>,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let releases = releases(root)?;
    let shipped: BTreeSet<(&FormatName, &FormatVersion)> = releases
        .release
        .iter()
        .flat_map(|release| release.formats.iter())
        .collect();
    for (name, version) in shipped {
        let frozen = folders
            .get(name.as_str())
            .is_some_and(|sets| sets.frozen.contains(version));
        if !frozen {
            findings.push(Finding::new(
                "missing-frozen",
                format!("{COMPAT}/{name}/{version}"),
                format!(
                    "{RELEASES} records this version as shipped; run `xtask compat freeze --tag` with its release's tag"
                ),
            ));
        }
    }
    Ok(())
}

/// Whether the registry's schema of `format` accepts other files than the
/// one its frozen set of the same version recorded. Annotations such as
/// descriptions are no change; a frozen set without a readable schema is
/// left to its `SHA256SUMS` check.
fn schema_changed_since_frozen(root: &Path, format: &Format) -> bool {
    let path = set_folder(root, format.name.as_str(), &format.version.to_string())
        .join(writer::DESCRIPTION);
    let Ok(text) = std::fs::read(&path) else {
        return false;
    };
    let Ok(description) = serde_json::from_slice::<serde_json::Value>(&text) else {
        return false;
    };
    registry::without_annotations(&description["schema"])
        != registry::without_annotations(&format.schema)
}

/// A finding for each file that differs between a committed set and the one
/// its writer produces.
fn drift(folder: &str, committed: &Tree, produced: &Tree, findings: &mut Vec<Finding>) {
    let paths: BTreeSet<&String> = committed.keys().chain(produced.keys()).collect();
    for path in paths {
        if committed.get(path) != produced.get(path) {
            findings.push(Finding::new(
                "drift",
                format!("{folder}/{path}"),
                "differs from what the format's writer produces; run `xtask compat write`",
            ));
        }
    }
}

/// A finding for each file that differs between two runs of one writer.
fn nondeterministic(folder: &str, first: &Tree, second: &Tree, findings: &mut Vec<Finding>) {
    let paths: BTreeSet<&String> = first.keys().chain(second.keys()).collect();
    for path in paths {
        if first.get(path) != second.get(path) {
            findings.push(Finding::new(
                "nondeterministic",
                format!("{folder}/{path}"),
                "the format's writer wrote other bytes on its second run; give its scenario a fixed order, Entropy seed and Clock start (ADR 0059)",
            ));
        }
    }
}

/// What `freeze` did.
pub(crate) enum Freeze {
    /// The tag names a release candidate or a nightly, which freeze nothing.
    Prerelease,
    /// The frozen sets it wrote, as paths under the root; none when every
    /// shipped version was frozen already.
    Frozen(Vec<String>),
}

/// Copies each format's `head/` set from the release tag `tag` to the folder
/// of the version the tag's registry gives it, with its `SHA256SUMS`, for
/// every such version not frozen yet. Only a stable release or a patch
/// freezes (ADR 0066).
pub(crate) fn freeze(root: &Path, tag_name: &str) -> Result<Freeze> {
    if let tag::Release::Prerelease = tag::release(tag_name)? {
        return Ok(Freeze::Prerelease);
    }
    tag::verify(root, tag_name)?;
    let shipped = registry::parse(&tag::file(root, tag_name, registry::BUNDLE)?)
        .with_context(|| format!("reading {} at {tag_name}", registry::BUNDLE))?;
    let mut frozen = Vec::new();
    for format in &shipped {
        let folder = set_folder(root, format.name.as_str(), &format.version.to_string());
        if folder.exists() {
            continue;
        }
        let head = format!("{COMPAT}/{}/{HEAD}", format.name);
        let mut set = tag::folder(root, tag_name, &head)?;
        if set.is_empty() {
            anyhow::bail!(
                "{tag_name} registers `{}` but holds no {head}/ set",
                format.name
            );
        }
        set.insert(frozen::SUMS.to_owned(), frozen::sums(&set));
        tree::add(&folder, &set)?;
        frozen.push(subject(root, &folder));
    }
    Ok(Freeze::Frozen(frozen))
}

/// Regenerates every registered format's `head/` set, and removes the
/// `head/` set of a format no longer registered; frozen sets stay.
pub(crate) fn write(cargo: &OsStr, root: &Path) -> Result<()> {
    let formats = formats(root)?;
    let writers = Writers::find(cargo, root)?;
    let sets = writers.write(cargo, root, &formats.iter().collect::<Vec<_>>())?;
    for (format, set) in formats.iter().zip(&sets) {
        tree::replace(&set_folder(root, format.name.as_str(), HEAD), set)?;
    }
    for (name, sets) in folders(root)? {
        let registered = formats.iter().any(|format| format.name.as_str() == name);
        if sets.head && !registered {
            let folder = set_folder(root, &name, HEAD);
            std::fs::remove_dir_all(&folder)
                .with_context(|| format!("removing {}", folder.display()))?;
        }
    }
    Ok(())
}

/// Writes the refusal cases of every format the committed bundle registers,
/// removing those of a format no longer registered.
pub(crate) fn write_refusals(root: &Path) -> Result<()> {
    let path = root.join(registry::BUNDLE);
    let bundle = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let files = refusals::files(&bundle).with_context(|| format!("reading {}", path.display()))?;
    let folder = root.join(refusals::FOLDER);
    if folder.exists() {
        std::fs::remove_dir_all(&folder)
            .with_context(|| format!("removing {}", folder.display()))?;
    }
    for (relative, contents) in files {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// `xtask compat`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Regenerates each registered format's `head/` set by running its
    /// writer, the `compat-<format>` example of the package that owns it.
    Write,
    /// Fails on a `head/` set that differs from what `write` produces, a
    /// writer whose second run writes other bytes than its first, a changed
    /// frozen byte, a missing set, incomplete schema coverage, and a
    /// schema changed without a version bump. Notes the digest each writer
    /// wrote on both runs, so a passing check's log is never empty.
    Check,
    /// Copies each format's `head/` set from a stable release's or patch's
    /// tag, not from the working tree, to `compat/<format>/<major>.<minor>/`
    /// with its `SHA256SUMS`, for every shipped version not frozen yet. A
    /// release candidate's or nightly's tag freezes nothing.
    Freeze {
        /// The release tag, `vX.Y.Z`.
        #[arg(long)]
        tag: String,
    },
    /// Writes each registered format's refusal cases from its rule's table,
    /// one file per case under `crates/goodfetch-protocol/generated/refusals/`,
    /// which `xtask codegen --check` holds to the registry.
    Refusals,
    /// Writes the Release suite's skew jobs, `ci/release/skew-jobs.json`,
    /// which `xtask codegen --check` holds to `compat/releases.toml`,
    /// `compat/skew-checks.toml` and `compat/skew-exclusions.toml`. Notes a
    /// job whose estimate passes the job budget.
    Jobs,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let (cargo, root) = (environment.cargo.as_os_str(), environment.root.as_path());
    match command {
        Command::Write => {
            write(cargo, root)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Check => {
            let checked = check(cargo, root)?;
            check::note(&checked.notes);
            Ok(check::report(&checked.findings))
        }
        Command::Refusals => {
            write_refusals(root)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Jobs => {
            let jobs = jobs::jobs(root, &crate::product_version::read(root)?)?;
            let path = root.join(jobs::JOBS);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, jobs.text()?)
                .with_context(|| format!("writing {}", path.display()))?;
            check::note(&jobs.over_budget());
            Ok(ExitCode::SUCCESS)
        }
        Command::Freeze { tag } => {
            match freeze(root, &tag)? {
                Freeze::Prerelease => output::line(&format!(
                    "{tag} is a release candidate or a nightly: nothing to freeze"
                )),
                Freeze::Frozen(sets) if sets.is_empty() => {
                    output::line(&format!("every version {tag} shipped is frozen already"));
                }
                Freeze::Frozen(sets) => {
                    for set in sets {
                        output::line(&format!("froze {set} from {tag}"));
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
