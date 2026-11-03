//! The rules that turn a change's paths into the pull-request tier's
//! selection, and the selection's `key=value` lines, which the workflow writes
//! to its job outputs and `just` reads.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use super::RepoPath;
use super::workspace::{Crate, Ecosystem, Model, Package, Workspace, is_visual_test};
use crate::{counts, foxhound_side, perf, screenshots, tlc};

/// A job of the pull-request tier that the selection may skip, by its job ID
/// in `.github/workflows/pull-request.yml`. Jobs that are not written yet
/// (Sinks, docs, the Evaluation) are selected already, so the tickets adding
/// them only read the selection. `rnd/`'s lab CI is no job of this tier: its
/// own workflow runs when `rnd/` changes, and it is never required (ADR
/// 0032).
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
    /// The visual projects in the pinned Playwright image (ADR 0073).
    Visual,
    /// The docs build, lychee offline, the docs Playwright pass and samples.
    Docs,
    /// The pull-request Evaluation (ADR 0065).
    Evaluation,
    /// TLC on `models`, with their small constants (ADR 0060).
    Tlc,
}

impl Job {
    const ALL: [Self; 7] = [
        Self::PreMerge,
        Self::Sinks,
        Self::Perf,
        Self::Visual,
        Self::Docs,
        Self::Evaluation,
        Self::Tlc,
    ];

    const fn id(self) -> &'static str {
        match self {
            Self::PreMerge => "pre-merge",
            Self::Sinks => "sinks",
            Self::Perf => "perf",
            Self::Visual => "visual",
            Self::Docs => "docs",
            Self::Evaluation => "evaluation",
            Self::Tlc => "tlc",
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
    /// The TLA+ models TLC checks, by name.
    models: BTreeSet<String>,
}

/// The folder of every sink process (ADR 0087).
const SINKS_FOLDER: &str = "crates/foxhound/sinks";

/// Folders and segments of prompt-affecting paths (ADR 0065): skills, the
/// protocol crate whose doc comments are the catalog's descriptions, prompts
/// and request snapshots. A ticket that adds prompts or request snapshots
/// keeps them in a folder named so.
const PROMPT_FOLDERS: [&str; 2] = ["skills", CATALOG_CRATE];

/// The crate whose types are the catalog, from which the docs' reference and
/// samples are generated too.
const CATALOG_CRATE: &str = "crates/goodfetch-protocol";
const PROMPT_SEGMENTS: [&str; 2] = ["prompts", "request-snapshots"];

/// Paths no test reads: the Quick suite, which always runs, checks them.
pub(super) fn needs_no_tests(path: &RepoPath) -> bool {
    let top_level_markdown = !path.as_str().contains('/')
        && Path::new(path.as_str())
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    top_level_markdown
        || ["rnd", "docs", "website"]
            .iter()
            .any(|folder| path.is_in(folder))
}

/// What the visual job reads besides its tests and the code they exercise:
/// the runner configs that define the `visual` projects, the pinned
/// Playwright image and the scripts that run it (ADR 0073).
const VISUAL_INPUTS: [&str; 4] = [
    "playwright.config.ts",
    "vitest.config.ts",
    "ci/visual",
    "ci/vm/visual.sh",
];

/// Whether the path is a visual test, a Reference screenshot or one of the
/// visual job's own inputs, which select the job whatever owns them, even
/// when the change deletes them.
fn is_visual_input(path: &RepoPath) -> bool {
    let name = path.as_str().rsplit('/').next().unwrap_or_default();
    is_visual_test(name)
        || screenshots::is_reference_screenshot(Path::new(path.as_str()))
        || VISUAL_INPUTS.iter().any(|input| path.is_in(input))
}

/// What TLC's job reads besides the models: the pinned tools and the
/// command that fetches them and runs TLC. A change to one selects every
/// model.
const TLC_INPUTS: [&str; 3] = [tlc::TOOLS_FILE, "xtask/src/tlc.rs", "xtask/src/tlc"];

/// Whether a change to the path reaches the model: a file of its folder, or
/// of a module it traces.
fn reaches_model(path: &RepoPath, model: &Model) -> bool {
    path.is_in(model.folder.as_str()) || model.traces.iter().any(|trace| path.is_in(trace.as_str()))
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

/// The name of the member that owns `path`, as the selection lists it: a
/// crate's name, a package's folder.
pub(crate) fn owner_name(workspace: &Workspace, path: &RepoPath) -> Option<String> {
    owner(workspace, path).map(|owner| match owner {
        Owner::Crate(krate) => krate.name.clone(),
        Owner::Package(package) => package.folder.as_str().to_owned(),
    })
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
        if path.is_in("docs") || path.is_in("website") || path.is_in(CATALOG_CRATE) {
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
            // The models are checked by TLC alone.
            None if needs_no_tests(path) || path.is_in(tlc::MODELS_FOLDER) => {}
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
    // A crate that embeds a package's build changes with that package and
    // with what its build and code depend on, not with what its tests alone
    // use (ADR 0057's Console in Collie).
    let all_packages: Vec<&Package> = workspace.packages.iter().collect();
    let roots = changed
        .packages
        .iter()
        .map(|package| package.name.clone())
        .collect();
    let built_packages: BTreeSet<&RepoPath> = with_dependants(
        &all_packages,
        &roots,
        |package| &package.name,
        |package| &package.runtime_dependencies,
    )
    .into_iter()
    .map(|package| &package.folder)
    .collect();
    changed.crates.extend(
        workspace
            .embedded
            .iter()
            .filter(|embedded| built_packages.contains(&embedded.package))
            .map(|embedded| embedded.krate.clone()),
    );
    // A crate's executable changes with it and with what its build depends
    // on, not with its dev-dependencies; a crate or package whose tests run
    // a changed executable through the test tool manifest is reached by it.
    let members: Vec<&Crate> = workspace.crates.iter().collect();
    let built = names(&with_dependants(
        &members,
        &changed.crates,
        |krate| &krate.name,
        |krate| &krate.built_from,
    ));
    let runs_a_built_executable =
        |runs: &BTreeSet<String>| runs.iter().any(|run| built.contains(run));
    changed.crates.extend(
        workspace
            .crates
            .iter()
            .filter(|krate| runs_a_built_executable(&krate.runs))
            .map(|krate| krate.name.clone()),
    );
    changed.packages.extend(
        workspace
            .packages
            .iter()
            .filter(|package| runs_a_built_executable(&package.runs)),
    );
    changed
}

/// The folders of the changed packages of one ecosystem and of their
/// dependants; every package of the ecosystem when they reach one that every
/// run of its runner loads.
fn package_folders(
    workspace: &Workspace,
    changed: &[&Package],
    ecosystem: Ecosystem,
) -> BTreeSet<RepoPath> {
    let reached = dependant_folders(workspace, changed, ecosystem);
    let loaded_by_every_run = workspace.packages.iter().any(|package| {
        package.ecosystem == ecosystem
            && package.loaded_by_every_run
            && reached.contains(&package.folder)
    });
    if loaded_by_every_run {
        workspace
            .packages
            .iter()
            .filter(|package| package.ecosystem == ecosystem)
            .map(|package| package.folder.clone())
            .collect()
    } else {
        reached
    }
}

/// The folders of the changed packages of one ecosystem and of the packages
/// that depend on them.
fn dependant_folders(
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
    // A visual test exercises its own package and the packages it depends on.
    let reaches_a_visual_test = workspace.visual_tests.iter().any(|test| {
        matches!(owner(workspace, test), Some(Owner::Package(package)) if vitest_packages.contains(&package.folder))
    });
    if reaches_a_visual_test || paths.iter().any(is_visual_input) {
        jobs.insert(Job::Visual);
    }

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
    // A changed ceiling is checked at once, on the crates it reads.
    let ceilinged: BTreeSet<&str> = if paths.iter().any(|path| path.as_str() == perf::BUDGET_FILE) {
        perf::ceilinged_crates()
    } else {
        BTreeSet::new()
    };
    // A newly pinned reference build is compared at once, on every benchmark.
    let reference_moved = paths
        .iter()
        .any(|path| path.as_str() == perf::REFERENCE_FILE);
    let bench_targets: BTreeSet<String> = reached
        .iter()
        .copied()
        .chain(
            members
                .iter()
                .copied()
                .filter(|krate| reference_moved || ceilinged.contains(krate.name.as_str())),
        )
        .flat_map(|krate| {
            krate
                .benches
                .iter()
                .map(|bench| format!("{}/{bench}", krate.name))
        })
        .collect();
    // `xtask counts`, in the perf job, holds the counted rows to the
    // register's limits, so a changed register is counted at once.
    let register_moved = paths
        .iter()
        .any(|path| path.as_str() == counts::REGISTER_FILE);
    if !bench_targets.is_empty() || register_moved {
        jobs.insert(Job::Perf);
    }
    let every_model = paths
        .iter()
        .any(|path| TLC_INPUTS.iter().any(|input| path.is_in(input)));
    let models: BTreeSet<String> = workspace
        .models
        .iter()
        .filter(|model| every_model || paths.iter().any(|path| reaches_model(path, model)))
        .map(|model| model.name.clone())
        .collect();
    if !models.is_empty() {
        jobs.insert(Job::Tlc);
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
        models,
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

    /// Whether the selection runs `job`.
    pub(crate) fn selects(&self, job: Job) -> bool {
        self.jobs.contains(&job)
    }

    /// Every member whose tests the selection runs, as `just test` takes
    /// them: the crates by name, the pnpm and uv packages by folder.
    pub(crate) fn members(&self) -> BTreeSet<String> {
        self.crates
            .iter()
            .cloned()
            .chain(
                self.vitest_packages
                    .iter()
                    .chain(&self.pytest_packages)
                    .map(|folder| folder.as_str().to_owned()),
            )
            .collect()
    }

    /// Whether the selection reaches any crate.
    pub(crate) fn reaches_a_crate(&self) -> bool {
        !self.crates.is_empty()
    }

    /// What the local gate runs (ADR 0135): `quick` when the change selects
    /// no job but the docs', so no test, benchmark, model, container or
    /// Evaluation reads it, else `full`.
    fn gate(&self) -> &'static str {
        if self.jobs.iter().all(|job| *job == Job::Docs) {
            "quick"
        } else {
            "full"
        }
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
        )?;
        writeln!(
            formatter,
            "models={}",
            joined(self.models.iter().map(String::as_str))
        )?;
        writeln!(formatter, "gate={}", self.gate())
    }
}
