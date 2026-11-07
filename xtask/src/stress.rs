//! The stress run before merge (ADR 0064; TESTING.md, Before merge).
//! - **select**: each runner's tests at the head that are new (absent from
//!   the base's listing, so a renamed test too) or changed (their file
//!   changed, plus what Vitest's `--changed` and Playwright's
//!   `--only-changed` reach) run 20 times; a test the change takes off the
//!   quarantine list runs 200 times. A test still on the list runs never, as
//!   in every suite. The base's listings are read from those the
//!   cache-warming job saved for its commit, else listed from a checkout of
//!   the base in the same job.
//!   The incremental gate (ADR 0135) narrows it `--within` the members the
//!   change since an earlier gate run reaches, and adds every test whose
//!   file that change (`--since` the earlier run's head) changed.
//!   A test the base lists and the head does not fails the selection,
//!   unless a `Quality-change` trailer of the change names its ID (the
//!   Floor; `deleted`).
//! - **listings**: saves the head's four listings for later changes' bases.
//!
//! A count is never lowered and nothing skips the run.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and blocking-process bans target async crates (STYLE.md)"
)]

mod deleted;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result, bail};

use crate::affected::{Kind, UNOWNED, WorkspaceMembers};
use crate::check::{self, Finding};
use crate::console;
use crate::environment::Environment;
use crate::git;
use crate::metadata::{self, Package};
use crate::output;
use crate::quarantine;
use crate::runners::listing::{self, Listed, Location};
use crate::runners::{Runner, TestId};

/// How many times a new or changed test runs (ADR 0064).
const CHANGED_RUNS: u32 = 20;

/// How many times a test taken off the quarantine list runs (ADR 0064).
const UNQUARANTINED_RUNS: u32 = 200;

/// The file a runner's saved listing is written to, in its commit's folder.
fn saved_file(runner: Runner) -> &'static str {
    match runner {
        Runner::Nextest => "nextest.json",
        Runner::Vitest => "vitest.json",
        Runner::Playwright => "playwright.json",
        Runner::Pytest => "pytest.txt",
    }
}

/// A test to run, and how many times.
struct Selected {
    count: u32,
    runner: Runner,
    test: TestId,
}

/// What `select` chose, and what the change deleted.
struct Selection {
    stressed: Vec<Selected>,
    /// A `quality-change` note for each deleted test a trailer names.
    accepted: Vec<Finding>,
    /// A `deleted-test` finding for each one no trailer names.
    deleted: Vec<Finding>,
}

/// Where the runners run, and how nextest builds.
struct Runners<'a> {
    cargo: &'a OsStr,
    cargo_profile: Option<&'a str>,
}

impl Runners<'_> {
    fn output(&self, runner: Runner, root: &Path, changed_since: Option<&str>) -> Result<Vec<u8>> {
        listing::output(
            runner,
            root,
            self.cargo,
            self.cargo_profile,
            None,
            changed_since,
        )
    }

    /// A checkout's listing. nextest builds it into its own target folder:
    /// beside the head's, a crate of the same name and relative path would
    /// share its artifacts, and Cargo would take one build for the other.
    fn checkout_output(
        &self,
        runner: Runner,
        checkout: &Path,
        target_directory: &Path,
    ) -> Result<Vec<u8>> {
        listing::output(
            runner,
            checkout,
            self.cargo,
            self.cargo_profile,
            Some(target_directory),
            None,
        )
    }
}

/// The base's target folder, within the head's, as `xtask perf` keeps its own.
const BASE_TARGET: &str = "stress-base";

/// The incremental gate's narrowing of a stress run (ADR 0135).
struct Narrowing {
    /// The members whose new and changed tests run: crates by name, pnpm
    /// and uv packages by folder; a runner none of them is tested by lists
    /// nothing.
    within: BTreeSet<String>,
    /// The earlier gate run's head: each test whose file changed since then
    /// runs too. Such a file lies in a member the change reaches, so its
    /// runner lists.
    since: Option<String>,
    members: WorkspaceMembers,
}

impl Narrowing {
    /// Whether some member within is tested by `runner`.
    fn lists(&self, runner: Runner) -> bool {
        let kind = match runner {
            Runner::Nextest => Kind::Crate,
            Runner::Vitest | Runner::Playwright => Kind::TypeScript,
            Runner::Pytest => Kind::Python,
        };
        let unowned = kind != Kind::Crate && self.within.contains(UNOWNED);
        unowned
            || self
                .members
                .all()
                .any(|(name, member)| member == kind && self.within.contains(&name))
    }

    /// Whether the test lies within: its member is, or it lies in no member
    /// and [`UNOWNED`] is within.
    fn holds(&self, location: &Location) -> bool {
        let member = match location {
            Location::Target { package, .. } => Some(package.clone()),
            Location::File(Some(file)) => self.members.owner(file),
            Location::File(None) => None,
        };
        self.within.contains(member.as_deref().unwrap_or(UNOWNED))
    }
}

/// The paths the working tree at `root` changes since `commit`.
fn changed_since(root: &Path, commit: &str) -> Result<BTreeSet<String>> {
    Ok(git::text(
        root,
        &["diff", "--name-only", "--no-renames", "-z", commit, "--"],
    )?
    .split('\0')
    .filter(|path| !path.is_empty())
    .map(str::to_owned)
    .collect())
}

/// The tests the change from `base` to the workspace at `root` stresses,
/// and those it deletes.
fn select(
    root: &Path,
    runners: &Runners<'_>,
    base: &str,
    saved: Option<&Path>,
    narrowing: Option<&Narrowing>,
) -> Result<Selection> {
    let base = commit(root, base)?;
    let changed = changed_since(root, &base)?;
    let since = match narrowing.and_then(|narrowing| narrowing.since.as_deref()) {
        Some(since) => changed_since(root, &commit(root, since)?)?,
        None => BTreeSet::new(),
    };
    let lists = |runner: Runner| narrowing.is_none_or(|narrowing| narrowing.lists(runner));
    let quarantined = |entries: Vec<quarantine::Entry>| -> BTreeSet<(Runner, TestId)> {
        entries
            .into_iter()
            .map(|entry| (entry.runner, entry.test))
            .collect()
    };
    let head_list = if root.join(quarantine::LIST).is_file() {
        quarantined(quarantine::read(root)?)
    } else {
        BTreeSet::new()
    };
    let base_list = quarantined(base_quarantine(root, &base)?);
    let base_listings = base_listings(root, runners, &base, saved, &lists)?;
    let packages = metadata::packages(runners.cargo, root)?;
    let root = canonical(root)?;

    let mut selection = Vec::new();
    let mut head_listings = BTreeMap::new();
    for runner in Runner::ALL.into_iter().filter(|runner| lists(*runner)) {
        let head = listing::parse(runner, &root, &runners.output(runner, &root, None)?)?;
        head_listings.insert(
            runner,
            head.iter()
                .map(|listed| listed.test.clone())
                .collect::<BTreeSet<TestId>>(),
        );
        let reached: BTreeSet<TestId> = match runner {
            Runner::Vitest | Runner::Playwright => {
                listing::parse(runner, &root, &runners.output(runner, &root, Some(&base))?)?
                    .into_iter()
                    .map(|listed| listed.test)
                    .collect()
            }
            Runner::Nextest | Runner::Pytest => BTreeSet::new(),
        };
        let listed_at_base = &base_listings[&runner];
        for Listed { test, location } in head {
            if head_list.contains(&(runner, test.clone())) {
                continue;
            }
            let test_file = file(&root, &packages, &location, &test);
            let pull_request = if base_list.contains(&(runner, test.clone())) {
                Some(UNQUARANTINED_RUNS)
            } else if !listed_at_base.contains(&test)
                || reached.contains(&test)
                || test_file
                    .as_ref()
                    .is_some_and(|file| changed.contains(file))
            {
                Some(CHANGED_RUNS)
            } else {
                None
            }
            .filter(|_| narrowing.is_none_or(|narrowing| narrowing.holds(&location)));
            let changed_since_earlier = test_file
                .as_ref()
                .is_some_and(|file| since.contains(file))
                .then_some(CHANGED_RUNS);
            let Some(count) = pull_request.max(changed_since_earlier) else {
                continue;
            };
            selection.push(Selected {
                count,
                runner,
                test,
            });
        }
    }
    let deleted = deleted::judge(&root, &base, &base_listings, &head_listings)?;
    Ok(Selection {
        stressed: selection,
        accepted: deleted.accepted,
        deleted: deleted.refused,
    })
}

/// The base commit's quarantine list; none when it has no list.
fn base_quarantine(root: &Path, base: &str) -> Result<Vec<quarantine::Entry>> {
    let object = format!("{base}:{}", quarantine::LIST);
    if !git::succeeds(root, &["cat-file", "-e", "--end-of-options", &object])? {
        return Ok(Vec::new());
    }
    quarantine::parse(&git::text(root, &["show", "--end-of-options", &object])?)
        .with_context(|| format!("reading {object}"))
}

/// Each runner's tests at `base`: from the listings saved for it when they
/// exist, else listed from a checkout of it.
fn base_listings(
    root: &Path,
    runners: &Runners<'_>,
    base: &str,
    saved: Option<&Path>,
    lists: &dyn Fn(Runner) -> bool,
) -> Result<BTreeMap<Runner, BTreeSet<TestId>>> {
    let mut listings = BTreeMap::new();
    let mut missing = Vec::new();
    for runner in Runner::ALL.into_iter().filter(|runner| lists(*runner)) {
        let file = saved.map(|folder| folder.join(base).join(saved_file(runner)));
        match file {
            Some(file) if file.is_file() => {
                let output =
                    std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
                output::error(&format!(
                    "{}: the base's listing is the one saved in {}",
                    runner.name(),
                    file.display()
                ));
                listings.insert(runner, ids(runner, root, &output)?);
            }
            _ => missing.push(runner),
        }
    }
    if !missing.is_empty() {
        let target_directory = metadata::target_directory(runners.cargo, root)?.join(BASE_TARGET);
        let checkout = Checkout::new(root, base)?;
        let listed = missing
            .iter()
            .map(|&runner| {
                output::error(&format!(
                    "{}: no listing is saved for the base, so it is listed from a checkout of {base}",
                    runner.name()
                ));
                let output = runners.checkout_output(runner, checkout.path(), &target_directory)?;
                Ok((runner, ids(runner, checkout.path(), &output)?))
            })
            .collect::<Result<Vec<_>>>();
        checkout.remove()?;
        listings.extend(listed?);
    }
    Ok(listings)
}

fn ids(runner: Runner, root: &Path, output: &[u8]) -> Result<BTreeSet<TestId>> {
    Ok(listing::parse(runner, root, output)?
        .into_iter()
        .map(|listed| listed.test)
        .collect())
}

/// A test's file, `/`-separated from the workspace's canonical folder.
fn file(root: &Path, packages: &[Package], location: &Location, test: &TestId) -> Option<String> {
    match location {
        Location::File(file) => file.clone(),
        Location::Target {
            package,
            kind,
            name,
        } => {
            let source = target_source(packages, package, kind, name)?;
            let modules: Vec<&str> = test.nextest_name().split("::").collect();
            let file = module_file(&source, &modules[..modules.len().saturating_sub(1)]);
            listing::relative(root, &file)
        }
    }
}

/// The root source file of a package's target, as nextest names the target:
/// its kind and its name, with `-` read as `_`.
fn target_source(packages: &[Package], package: &str, kind: &str, name: &str) -> Option<PathBuf> {
    const LIBRARIES: [&str; 6] = ["lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"];
    let normal = |name: &str| name.replace('-', "_");
    packages
        .iter()
        .find(|candidate| candidate.name == package)?
        .targets
        .iter()
        .find(|target| {
            normal(&target.name) == normal(name)
                && target.kind.iter().any(|target_kind| {
                    target_kind == kind
                        || (kind == "lib" && LIBRARIES.contains(&target_kind.as_str()))
                })
        })
        .map(|target| target.src_path.clone())
}

/// The file that defines the module path `modules` in the crate whose root
/// is `root_file`: each module in `<name>.rs` or `<name>/mod.rs` beside its
/// parent's folder, the deepest that exists; a module with no file of its own
/// is inline in its parent's.
fn module_file(root_file: &Path, modules: &[&str]) -> PathBuf {
    let mut file = root_file.to_path_buf();
    let Some(mut folder) = root_file.parent().map(Path::to_path_buf) else {
        return file;
    };
    for module in modules {
        let flat = folder.join(format!("{module}.rs"));
        let nested = folder.join(module).join("mod.rs");
        if flat.is_file() {
            file = flat;
        } else if nested.is_file() {
            file = nested;
        } else {
            break;
        }
        folder = folder.join(module);
    }
    file
}

/// A checkout of a commit in a temporary folder, with its own npm packages
/// and Python environment installed, so its runners list its tests.
struct Checkout<'a> {
    root: &'a Path,
    _folder: tempfile::TempDir,
    path: PathBuf,
}

impl<'a> Checkout<'a> {
    fn new(root: &'a Path, commit: &str) -> Result<Self> {
        let folder = tempfile::tempdir().context("creating a folder for the base's checkout")?;
        let path = folder.path().join("base");
        git::text(
            root,
            &[
                "worktree",
                "add",
                "--detach",
                "--quiet",
                "--end-of-options",
                &path.to_string_lossy(),
                commit,
            ],
        )?;
        let checkout = Self {
            root,
            _folder: folder,
            path,
        };
        match checkout.install() {
            Ok(()) => Ok(checkout),
            Err(error) => {
                checkout.remove()?;
                Err(error)
            }
        }
    }

    /// Installs the commit's npm packages and Python environment from its
    /// own lockfiles, as `just fetch` does, from the package store and uv's
    /// cache where it can, then builds its Consoles, which its Products'
    /// executables embed (ADR 0057): nextest builds the commit's tests to
    /// list them, and Collie's fail to compile without its Console build.
    /// The head's would not do: pnpm links each workspace package to the
    /// head's sources, and uv installs each workspace member editable from
    /// them, so the commit's Vitest and Playwright configs and its pytest
    /// tests would import the head's test support and SDKs, and fail to
    /// load where they lack what is imported.
    fn install(&self) -> Result<()> {
        self.run_install(
            Process::new("pnpm").args(["install", "--frozen-lockfile", "--prefer-offline"]),
            "pnpm install",
        )?;
        console::build(&self.path).context("building the Consoles in the checkout of the base")?;
        // A project environment set for the head would be synced to the
        // base's lockfile; the checkout's runners use its own `.venv`.
        self.run_install(
            Process::new("uv")
                .args(["sync", "--frozen", "--all-extras"])
                .env_remove("UV_PROJECT_ENVIRONMENT"),
            "uv sync",
        )
    }

    fn run_install(&self, command: &mut Process, name: &str) -> Result<()> {
        let output = command
            .current_dir(&self.path)
            .output()
            .with_context(|| format!("running {name}"))?;
        if !output.status.success() {
            bail!(
                "{name} in the checkout of the base failed: {}{}",
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn remove(self) -> Result<()> {
        git::text(
            self.root,
            &[
                "worktree",
                "remove",
                "--force",
                "--end-of-options",
                &self.path.to_string_lossy(),
            ],
        )?;
        Ok(())
    }
}

/// Writes the four listings of the workspace at `root` into
/// `<folder>/<HEAD's commit>/`, for later changes whose base it is.
fn save(root: &Path, runners: &Runners<'_>, folder: &Path) -> Result<PathBuf> {
    let commit = commit(root, "HEAD")?;
    let out = folder.join(&commit);
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    for runner in Runner::ALL {
        let output = runners.output(runner, root, None)?;
        let file = out.join(saved_file(runner));
        std::fs::write(&file, output).with_context(|| format!("writing {}", file.display()))?;
    }
    Ok(out)
}

/// The commit `revision` names, in full.
fn commit(root: &Path, revision: &str) -> Result<String> {
    Ok(git::text(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )?
    .trim()
    .to_owned())
}

fn canonical(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).with_context(|| format!("resolving {}", path.display()))
}

/// `xtask stress`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Prints the tests a change stresses, as `<count>\t<runner>\t<test ID>`
    /// lines: each new or changed test 20 times, each test the change takes
    /// off the quarantine list 200 times.
    Select {
        /// The change's base commit.
        #[arg(long)]
        base: String,
        /// The folder of the listings the cache-warming job saved, one
        /// folder per commit; a base with none is listed from a checkout.
        #[arg(long)]
        saved: Option<PathBuf>,
        /// The Cargo profile nextest builds its listings with.
        #[arg(long)]
        cargo_profile: Option<String>,
        /// Only the new and changed tests of these members, crates by name
        /// and pnpm or uv packages by folder, `:unowned` for the tests no
        /// member owns (the incremental gate, ADR 0135); repeatable.
        #[arg(long)]
        within: Vec<String>,
        /// The earlier gate run's head, with `--within`: each test whose
        /// file changed since then runs too.
        #[arg(long, requires = "within")]
        since: Option<String>,
    },
    /// Saves each runner's listing of the workspace into `<out>/<HEAD's
    /// commit>/`, for the changes whose base it is.
    Listings {
        #[arg(long)]
        out: PathBuf,
        /// The Cargo profile nextest builds its listing with.
        #[arg(long)]
        cargo_profile: Option<String>,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    match command {
        Command::Select {
            base,
            saved,
            cargo_profile,
            within,
            since,
        } => {
            let runners = Runners {
                cargo: &environment.cargo,
                cargo_profile: cargo_profile.as_deref(),
            };
            let saved = saved.map(|folder| environment.path(&folder));
            let narrowing = if within.is_empty() {
                None
            } else {
                Some(Narrowing {
                    within: within.into_iter().collect(),
                    since,
                    members: WorkspaceMembers::read(&environment.cargo, root)?,
                })
            };
            let selection = select(root, &runners, &base, saved.as_deref(), narrowing.as_ref())?;
            // Stdout is the selection, which `just stress` reads.
            check::note_to(&selection.accepted, output::error);
            if !selection.deleted.is_empty() {
                return Ok(check::report_to(&selection.deleted, output::error));
            }
            for selected in selection.stressed {
                output::line(&format!(
                    "{}\t{}\t{}",
                    selected.count,
                    selected.runner.name(),
                    selected.test
                ));
            }
        }
        Command::Listings { out, cargo_profile } => {
            let runners = Runners {
                cargo: &environment.cargo,
                cargo_profile: cargo_profile.as_deref(),
            };
            let written = save(root, &runners, &environment.path(&out))?;
            output::line(&format!("wrote {}", written.display()));
        }
    }
    Ok(ExitCode::SUCCESS)
}
