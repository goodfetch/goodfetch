//! The rules that turn a change's paths into the pull-request tier's
//! selection, and the selection's `key=value` lines, which the workflow writes
//! to its job outputs and `just` reads.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use super::RepoPath;
use super::workspace::{Crate, Ecosystem, Package, Workspace};
use crate::foxhound_side;

/// A job of the pull-request tier that the selection may skip, by its job ID
/// in `.github/workflows/pull-request.yml`. Jobs that are not written yet
/// (Sinks, perf, docs, the Evaluation) are selected already, so the tickets
/// adding them only read the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Job {
    /// The rest of the Pre-merge suite: the Rust tests of `nextest-filter`,
    /// and the touched TypeScript and Python packages, SDKs and adapters
    /// among them, on one runtime each.
    PreMerge,
    /// The sink suites against their emulators and containers.
    Sinks,
    /// `xtask perf` over `bench-targets` (ADR 0071).
    Perf,
    /// The docs build, lychee offline, the docs Playwright pass and samples.
    Docs,
    /// The pull-request Evaluation (ADR 0065).
    Evaluation,
    /// `rnd/`'s lab CI, run only when `rnd/` changes and never required.
    Lab,
}

impl Job {
    const ALL: [Self; 6] = [
        Self::PreMerge,
        Self::Sinks,
        Self::Perf,
        Self::Docs,
        Self::Evaluation,
        Self::Lab,
    ];

    const fn id(self) -> &'static str {
        match self {
            Self::PreMerge => "pre-merge",
            Self::Sinks => "sinks",
            Self::Perf => "perf",
            Self::Docs => "docs",
            Self::Evaluation => "evaluation",
            Self::Lab => "lab",
        }
    }
}

/// Whose tests the selected jobs run (ADR 0094).
#[derive(Clone, Copy)]
enum Products {
    /// The change is confined to Foxhound's side.
    Foxhound,
    Both,
}

/// What nextest runs.
enum RustTests {
    None,
    All,
    /// The changed crates; nextest adds the crates that depend on them.
    DependantsOf(BTreeSet<String>),
}

pub(crate) struct Selection {
    products: Products,
    jobs: BTreeSet<Job>,
    rust: RustTests,
    /// The changed crates and their dependants, for runs that select crates
    /// by name (`cargo -p`) rather than through nextest's `rdeps(…)`.
    crates: BTreeSet<String>,
    vitest_packages: BTreeSet<RepoPath>,
    pytest_packages: BTreeSet<RepoPath>,
    /// The Sinks whose suites run: every Sink until the merge queue exists
    /// (ADR 0032), whenever the Pre-merge suite runs.
    sinks: BTreeSet<String>,
    /// The Sinks the change reaches, the ones that run once the merge queue
    /// exists: a change to the sink protocol or shared sink code reaches
    /// every Sink through the crates' dependencies.
    touched_sinks: BTreeSet<String>,
    /// `<crate>/<bench target>`.
    bench_targets: BTreeSet<String>,
}

/// The folder of every sink process (ADR 0087).
const SINKS_FOLDER: &str = "crates/foxhound/sinks";

/// Folders and segments of prompt-affecting paths (ADR 0065): skills, the
/// protocol crate whose doc comments are the catalog's descriptions, prompts
/// and request snapshots. A ticket that adds prompts or request snapshots
/// keeps them in a folder named so.
const PROMPT_FOLDERS: [&str; 2] = ["skills", "crates/goodfetch-protocol"];
const PROMPT_SEGMENTS: [&str; 2] = ["prompts", "request-snapshots"];

/// Paths no test reads: the Quick suite, which always runs, checks them.
fn needs_no_tests(path: &RepoPath) -> bool {
    let top_level_markdown = !path.as_str().contains('/')
        && Path::new(path.as_str())
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    top_level_markdown
        || ["rnd", "docs", "website"]
            .iter()
            .any(|folder| path.is_in(folder))
}

fn is_prompt_affecting(path: &RepoPath) -> bool {
    PROMPT_FOLDERS.iter().any(|folder| path.is_in(folder))
        || path
            .as_str()
            .split('/')
            .any(|segment| PROMPT_SEGMENTS.contains(&segment))
}

fn is_sink(krate: &Crate) -> bool {
    krate.executable && krate.folder.is_in(SINKS_FOLDER)
}

/// What owns a changed path: the member of any workspace whose folder holds
/// it most closely.
enum Owner<'a> {
    Crate(&'a Crate),
    Package(&'a Package),
}

fn owner<'a>(workspace: &'a Workspace, path: &RepoPath) -> Option<Owner<'a>> {
    let crates = workspace
        .crates
        .iter()
        .map(|krate| (krate.folder.as_str(), Owner::Crate(krate)));
    let packages = workspace
        .packages
        .iter()
        .map(|package| (package.folder.as_str(), Owner::Package(package)));
    crates
        .chain(packages)
        .filter(|(folder, _)| path.is_in(folder))
        .max_by_key(|(folder, _)| folder.len())
        .map(|(_, owner)| owner)
}

/// `roots` and every member that depends on one of them, directly or not.
fn with_dependants<'a, T>(
    members: &[&'a T],
    roots: &BTreeSet<String>,
    name: impl Fn(&T) -> &str,
    dependencies: impl Fn(&T) -> &BTreeSet<String>,
) -> Vec<&'a T> {
    let mut reached = roots.clone();
    loop {
        let before = reached.len();
        for member in members {
            if dependencies(member)
                .iter()
                .any(|dependency| reached.contains(dependency))
            {
                reached.insert(name(member).to_owned());
            }
        }
        if reached.len() == before {
            break;
        }
    }
    members
        .iter()
        .copied()
        .filter(|member| reached.contains(name(member)))
        .collect()
}

fn products(paths: &[RepoPath]) -> Products {
    let confined = !paths.is_empty()
        && paths
            .iter()
            .all(|path| foxhound_side::contains(path.as_str()));
    if confined {
        Products::Foxhound
    } else {
        Products::Both
    }
}

/// What the paths change directly, before dependants are added.
struct Changed<'a> {
    jobs: BTreeSet<Job>,
    crates: BTreeSet<String>,
    packages: Vec<&'a Package>,
}

fn changed<'a>(workspace: &'a Workspace, paths: &[RepoPath], products: Products) -> Changed<'a> {
    let mut changed = Changed {
        jobs: BTreeSet::new(),
        crates: BTreeSet::new(),
        packages: Vec::new(),
    };
    let mut everything = false;
    for path in paths {
        if path.is_in("rnd") {
            changed.jobs.insert(Job::Lab);
        }
        if path.is_in("docs") || path.is_in("website") {
            changed.jobs.insert(Job::Docs);
        }
        if is_prompt_affecting(path) {
            changed.jobs.insert(Job::Evaluation);
        }
        match owner(workspace, path) {
            Some(Owner::Crate(krate)) => {
                changed.crates.insert(krate.name.clone());
            }
            Some(Owner::Package(package)) => changed.packages.push(package),
            None if needs_no_tests(path) => {}
            None => everything = true,
        }
    }
    if everything {
        // A path no member owns, such as a lockfile, a tool's settings or a
        // workflow, may change any test: every one of the Products' runs.
        let in_scope = |folder: &RepoPath| match products {
            Products::Foxhound => foxhound_side::contains(&format!("{}/", folder.as_str())),
            Products::Both => true,
        };
        changed.jobs.insert(Job::Docs);
        changed.crates.extend(
            workspace
                .crates
                .iter()
                .filter(|krate| in_scope(&krate.folder))
                .map(|krate| krate.name.clone()),
        );
        changed.packages = workspace
            .packages
            .iter()
            .filter(|package| in_scope(&package.folder))
            .collect();
    }
    changed
}

/// The folders of the changed packages of one ecosystem and of their
/// dependants.
fn package_folders(
    workspace: &Workspace,
    changed: &[&Package],
    ecosystem: Ecosystem,
) -> BTreeSet<RepoPath> {
    let members: Vec<&Package> = workspace
        .packages
        .iter()
        .filter(|package| package.ecosystem == ecosystem)
        .collect();
    let roots = changed
        .iter()
        .filter(|package| package.ecosystem == ecosystem)
        .map(|package| package.name.clone())
        .collect();
    with_dependants(
        &members,
        &roots,
        |package| &package.name,
        |package| &package.dependencies,
    )
    .into_iter()
    .map(|package| package.folder.clone())
    .collect()
}

fn names<'a>(crates: impl IntoIterator<Item = &'a &'a Crate>) -> BTreeSet<String> {
    crates.into_iter().map(|krate| krate.name.clone()).collect()
}

pub(crate) fn select(workspace: &Workspace, paths: &[RepoPath]) -> Selection {
    let products = products(paths);
    let Changed {
        mut jobs,
        crates: changed_crates,
        packages: changed_packages,
    } = changed(workspace, paths, products);

    let members: Vec<&Crate> = workspace.crates.iter().collect();
    let reached = with_dependants(
        &members,
        &changed_crates,
        |krate| &krate.name,
        |krate| &krate.dependencies,
    );
    let rust = if changed_crates.is_empty() {
        RustTests::None
    } else if changed_crates.len() == workspace.crates.len() {
        RustTests::All
    } else {
        RustTests::DependantsOf(changed_crates)
    };
    let vitest_packages = package_folders(workspace, &changed_packages, Ecosystem::TypeScript);
    let pytest_packages = package_folders(workspace, &changed_packages, Ecosystem::Python);

    let pre_merge = !matches!(rust, RustTests::None)
        || !vitest_packages.is_empty()
        || !pytest_packages.is_empty();
    let sinks = if pre_merge {
        jobs.insert(Job::PreMerge);
        names(members.iter().filter(|krate| is_sink(krate)))
    } else {
        BTreeSet::new()
    };
    if !sinks.is_empty() {
        jobs.insert(Job::Sinks);
    }
    let bench_targets: BTreeSet<String> = reached
        .iter()
        .flat_map(|krate| {
            krate
                .benches
                .iter()
                .map(|bench| format!("{}/{bench}", krate.name))
        })
        .collect();
    if !bench_targets.is_empty() {
        jobs.insert(Job::Perf);
    }

    Selection {
        products,
        jobs,
        rust,
        crates: names(&reached),
        vitest_packages,
        pytest_packages,
        sinks,
        touched_sinks: names(reached.iter().filter(|krate| is_sink(krate))),
        bench_targets,
    }
}

fn joined<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    items.into_iter().collect::<Vec<_>>().join(" ")
}

/// The ID of every job the selection may skip; any other job always runs.
pub(crate) fn selectable_job_ids() -> impl Iterator<Item = &'static str> {
    Job::ALL.into_iter().map(Job::id)
}

impl Selection {
    /// Each selected job's ID, as the workflow's `if:` reads them.
    pub(crate) fn job_ids(&self) -> impl Iterator<Item = &'static str> {
        self.jobs.iter().map(|job| job.id())
    }
}

impl fmt::Display for Selection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let products = match self.products {
            Products::Foxhound => "foxhound",
            Products::Both => "collie foxhound",
        };
        let jobs: Vec<String> = self.job_ids().map(|id| format!("\"{id}\"")).collect();
        let nextest_filter = match &self.rust {
            RustTests::None => "none()".to_owned(),
            RustTests::All => "all()".to_owned(),
            RustTests::DependantsOf(crates) => crates
                .iter()
                .map(|name| format!("rdeps(={name})"))
                .collect::<Vec<_>>()
                .join(" | "),
        };
        writeln!(formatter, "products={products}")?;
        writeln!(formatter, "jobs=[{}]", jobs.join(","))?;
        writeln!(formatter, "nextest-filter={nextest_filter}")?;
        writeln!(
            formatter,
            "crates={}",
            joined(self.crates.iter().map(String::as_str))
        )?;
        writeln!(
            formatter,
            "vitest-packages={}",
            joined(self.vitest_packages.iter().map(RepoPath::as_str))
        )?;
        writeln!(
            formatter,
            "pytest-packages={}",
            joined(self.pytest_packages.iter().map(RepoPath::as_str))
        )?;
        writeln!(
            formatter,
            "sinks={}",
            joined(self.sinks.iter().map(String::as_str))
        )?;
        writeln!(
            formatter,
            "sinks-touched={}",
            joined(self.touched_sinks.iter().map(String::as_str))
        )?;
        writeln!(
            formatter,
            "bench-targets={}",
            joined(self.bench_targets.iter().map(String::as_str))
        )
    }
}
