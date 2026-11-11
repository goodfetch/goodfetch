//! The incremental gate's choice (ADR 0135): for each step of `just
//! pre-merge all`, whether this run reruns it in full, reruns it over some
//! packages only, or reuses the green result that a gate run on an ancestor
//! head recorded. The choice reads what `xtask affected` selects for the
//! change since the head each green result was produced on, never a guess.

use std::collections::{BTreeMap, BTreeSet};

use crate::affected::{Job, RepoPath, Selection};
use crate::products;

/// A step of `just pre-merge all` that the gate records, declared in the
/// order `ci/pre-merge.sh` starts them (`steps`): the checks that judge the
/// whole change in seconds first, so the gate's log opens with their
/// verdicts, then those beside the host's, then the host's others. A red
/// step ends no gate: every step runs and records its result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    Dco,
    CommitMessages,
    FloorGuard,
    CommitLeaks,
    Audit,
    Notices,
    Perf,
    Linux,
    Visual,
    Lint,
    FeatureCombinations,
    FuzzCheck,
    Proofs,
    Quick,
    PreMerge,
    Stress,
    JourneysTasks,
    FeaturesTests,
    Tlc,
    Kennel,
    Msrv,
    Miri,
}

/// What a step's result depends on, besides the gate's own inputs.
#[derive(Clone, Copy)]
enum Reach {
    /// A check that judges the whole change and takes seconds or minutes:
    /// every run reruns it.
    Always,
    /// The step runs when the selection runs this job of the pull-request
    /// tier.
    Job(Job),
    /// The step builds every crate.
    Crates,
    /// The step runs the tests of each member separately: a crate's, a pnpm
    /// or a uv package's.
    Members,
}

impl Step {
    /// Every step, in the order `ci/pre-merge.sh` starts them.
    pub(crate) const ALL: [Self; 22] = [
        Self::Dco,
        Self::CommitMessages,
        Self::FloorGuard,
        Self::CommitLeaks,
        Self::Audit,
        Self::Notices,
        Self::Perf,
        Self::Linux,
        Self::Visual,
        Self::Lint,
        Self::FeatureCombinations,
        Self::FuzzCheck,
        Self::Proofs,
        Self::Quick,
        Self::PreMerge,
        Self::Stress,
        Self::JourneysTasks,
        Self::FeaturesTests,
        Self::Tlc,
        Self::Kennel,
        Self::Msrv,
        Self::Miri,
    ];

    /// The step's name in the plan, the record and `ci/pre-merge.sh`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Dco => "dco",
            Self::CommitMessages => "commit-messages",
            Self::FloorGuard => "floor-guard",
            Self::Lint => "lint",
            Self::FeatureCombinations => "feature-combinations",
            Self::Audit => "audit",
            Self::CommitLeaks => "commit-leaks",
            Self::FuzzCheck => "fuzz-check",
            Self::Proofs => "proofs",
            Self::JourneysTasks => "journeys-tasks",
            Self::FeaturesTests => "features-tests",
            Self::Notices => "notices",
            Self::Perf => "perf",
            Self::Linux => "linux",
            Self::Visual => "visual",
            Self::Quick => "quick",
            Self::PreMerge => "pre-merge",
            Self::Stress => "stress",
            Self::Tlc => "tlc",
            Self::Msrv => "msrv",
            Self::Miri => "miri",
            Self::Kennel => "kennel",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|step| step.name() == name)
    }

    const fn reach(self) -> Reach {
        match self {
            Self::Dco
            | Self::CommitMessages
            | Self::FloorGuard
            | Self::Lint
            | Self::FeatureCombinations
            | Self::Audit
            | Self::CommitLeaks
            | Self::FuzzCheck
            | Self::Proofs
            | Self::JourneysTasks
            | Self::FeaturesTests
            | Self::Notices => Reach::Always,
            Self::Perf => Reach::Job(Job::Perf),
            // On a Linux host the Pre-merge suite runs these tests itself;
            // elsewhere they are a slice of it, in a container.
            Self::Linux => Reach::Job(Job::PreMerge),
            Self::Visual => Reach::Job(Job::Visual),
            Self::Tlc => Reach::Job(Job::Tlc),
            Self::Kennel => Reach::Job(Job::Kennel),
            Self::Msrv | Self::Miri => Reach::Crates,
            Self::Quick | Self::PreMerge | Self::Stress => Reach::Members,
        }
    }

    /// Whether the step reads only the change's commits and the bar, which
    /// every form of the gate runs first, the docs form's included.
    pub(crate) const fn reads_the_commits(self) -> bool {
        matches!(
            self,
            Self::Dco | Self::CommitMessages | Self::FloorGuard | Self::CommitLeaks
        )
    }

    /// Whether the step judges the whole change in seconds: those that read
    /// its commits, the advisories and the notices, which the gate runs
    /// before any build and any step beside the host's.
    pub(crate) const fn judges_the_whole_change(self) -> bool {
        self.reads_the_commits() || matches!(self, Self::Audit | Self::Notices)
    }

    /// Whether the step needs Linux and runs beside the host's, in a
    /// container of its own.
    pub(crate) const fn beside(self) -> bool {
        matches!(self, Self::Perf | Self::Linux | Self::Visual)
    }

    /// Whether the step runs each member's tests separately.
    pub(crate) const fn per_member(self) -> bool {
        matches!(self.reach(), Reach::Members)
    }

    /// Whether the step runs Vitest, Playwright and pytest, which order
    /// their tests by the head's seed (TESTING.md, Before merge).
    const fn orders_by_the_head(self) -> bool {
        matches!(self, Self::Quick | Self::PreMerge | Self::Stress)
    }

    /// Whether the step asks each test for a count of clean passes, which a
    /// red run may leave part way ([`EarnedPasses`]).
    pub(crate) const fn counts_passes(self) -> bool {
        matches!(self, Self::Stress)
    }

    /// Whether the step compares the head with the change's base (the merge
    /// base with `origin/main`), so its result holds only for that base.
    const fn reads_the_base(self) -> bool {
        matches!(self, Self::Perf | Self::Stress)
    }
}

/// The paths whose change reruns every step in full, but those [`NARROWED`]
/// names: the gate's own commands and scripts, the tools' versions and
/// settings, the lockfiles and the build profiles. `xtask` is among them,
/// since it runs most steps and `xtask/src/` makes this very choice. So are
/// the files tests of several members read at run time, which no evidence
/// shows (ADR 0135): the Proving ground's pages and fixtures, the shared
/// fixtures and the browser pins.
const GATE_INPUTS: [&str; 18] = [
    "justfile",
    "ci",
    ".config",
    ".cargo",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
    "uv.lock",
    "pyproject.toml",
    "xtask",
    "proving-ground/pages",
    "proving-ground/fixtures",
    "test-support/fixtures",
    "chromium",
    concat!(products::collie_folder!(), "/src/pins.json"),
];

/// The workspace member whose tests are the gate's own: no crate depends on
/// it, and it holds no benchmark, so what reaches it reaches only its tests.
pub(crate) const XTASK: &str = "xtask";

/// A file or folder of the gate's inputs that reaches only what reads it
/// (#2090's F4; ADR 0135). `steps` are those that run or read it, of the
/// steps a run may reuse: one only a check every run reruns reads needs none,
/// and neither does one no gate step runs. The evidence of what each member
/// read still applies to it, so a test naming a `ci/` file reruns that test's
/// member.
pub(crate) struct Narrowed {
    path: &'static str,
    pub(crate) steps: &'static [Step],
    /// The member whose tests it is.
    member: Option<&'static str>,
}

const fn read_by(path: &'static str, steps: &'static [Step]) -> Narrowed {
    Narrowed {
        path,
        steps,
        member: None,
    }
}

/// The paths of the gate's inputs that reach less than every step; the first
/// that holds a path names it. Any other path of the inputs reruns every step.
/// So a new file directly in `ci/` reruns every step until it is named here,
/// while a new file in a folder named here (`ci/proofs`, `ci/release`,
/// `ci/visual`, `ci/test-health` and the like) reaches what that folder's entry
/// says from the start: the guard test in `gate/tests.rs` fails when a step's
/// recipe or script names such a file with a root-relative path. A step that
/// reads one any other way, by a sibling's relative path or from xtask's code,
/// is out of the guard's sight, so naming a folder here rests on what it holds.
const NARROWED: [Narrowed; 43] = [
    // xtask's own tests, with their fixtures. `just linux` runs some of them
    // on a host that is not Linux (ci/linux.sh).
    Narrowed {
        path: "xtask/tests",
        steps: &[Step::Linux],
        member: Some(XTASK),
    },
    // What one step of those a run may reuse runs or reads.
    read_by("ci/perf.sh", &[Step::Perf]),
    read_by("ci/perf-reference.toml", &[Step::Perf]),
    read_by("ci/linux.sh", &[Step::Linux]),
    // ci/vm/common.sh, which ci/perf.sh and ci/linux.sh source, reads the pin.
    read_by(
        "ci/visual/image.toml",
        &[Step::Perf, Step::Linux, Step::Visual],
    ),
    read_by("ci/visual", &[Step::Visual]),
    read_by("ci/vm/visual.sh", &[Step::Visual]),
    read_by("ci/container.sh", &[Step::Perf, Step::Linux, Step::Visual]),
    read_by("ci/msrv.sh", &[Step::Msrv]),
    read_by("ci/miri.toml", &[Step::Miri]),
    // What only the checks every run reruns read.
    read_by("ci/gitleaks.toml", &[]),
    read_by("ci/commit-leaks.sh", &[]),
    read_by("ci/commit-messages.toml", &[]),
    read_by("ci/audit.sh", &[]),
    read_by("ci/advisories.toml", &[]),
    read_by("ci/feature-combinations.sh", &[]),
    read_by("ci/fuzz-check.sh", &[]),
    read_by("ci/proofs", &[]),
    read_by("ci/deny", &[]),
    read_by("ci/canary-rule.sh", &[]),
    read_by("ci/ignored-sources.sh", &[]),
    read_by("ci/workspace-hack.sh", &[]),
    read_by("ci/repository-files.sh", &[]),
    read_by("ci/keep-going.sh", &[]),
    // What no gate step runs.
    read_by("ci/coverage.sh", &[]),
    read_by("ci/mutants.sh", &[]),
    read_by("ci/asan.sh", &[]),
    read_by("ci/gate-backstop.sh", &[]),
    read_by("ci/gate-backstop.crontab", &[]),
    read_by("ci/gate-backstop.state", &[]),
    read_by("ci/test-health.crontab", &[]),
    read_by("ci/test-health", &[]),
    read_by("ci/merge-cap.sh", &[]),
    read_by("ci/backstop-green.sh", &[]),
    read_by("ci/quick.sh", &[]),
    read_by("ci/results", &[]),
    read_by("ci/release", &[]),
    read_by("ci/runner/install.sh", &[]),
    read_by("ci/runner/pre-job.sh", &[]),
    read_by("ci/runner/post-job.sh", &[]),
    read_by("ci/stop-format.sh", &[]),
    read_by("ci/v8-corruption.sh", &[]),
    read_by("ci/junit-summary.sh", &[]),
];

/// What the map says of `path`.
pub(crate) fn narrowed(path: &RepoPath) -> Option<&'static Narrowed> {
    NARROWED.iter().find(|entry| path.is_in(entry.path))
}

/// The first changed path that is one of the gate's inputs and reruns every
/// step.
pub(crate) fn gate_input(paths: &[RepoPath]) -> Option<&RepoPath> {
    paths
        .iter()
        .find(|path| narrowed(path).is_none() && GATE_INPUTS.iter().any(|input| path.is_in(input)))
}

/// The changed paths `xtask affected`'s selection judges: it takes a path no
/// member owns for one that may change any test, which those [`NARROWED`]
/// names are not, and it would take an xtask test file for a change to a crate
/// the crate builds read.
pub(crate) fn selected(paths: &[RepoPath]) -> Vec<RepoPath> {
    paths
        .iter()
        .filter(|path| narrowed(path).is_none())
        .cloned()
        .collect()
}

/// The pseudo-member of the tests no workspace member owns: they rerun whenever a change reaches any
/// member, or when a runner's own listing since the head says so.
pub(crate) use crate::affected::UNOWNED;

/// How long a green result stays reusable, in seconds: a day, so a host's
/// own drift (packages, browsers, an expiring fixture) reaches the gate.
pub(crate) const MAX_AGE: u64 = 24 * 60 * 60;

/// Whether a result or earned pass of time `at` is past [`MAX_AGE`] at `now`.
pub(super) fn past_max_age(now: u64, at: u64) -> bool {
    now.saturating_sub(at) > MAX_AGE
}

/// What a change since a recorded head reaches, as the plan reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Touched {
    /// The first of the gate's inputs it changes, when it changes one.
    pub(crate) gate_input: Option<String>,
    /// The jobs of the pull-request tier its selection runs.
    pub(crate) jobs: BTreeSet<Job>,
    /// The steps that read a path of it [`NARROWED`] names, or the tests only
    /// Linux decides when the evidence reaches xtask's.
    pub(crate) steps: BTreeSet<Step>,
    /// Whether its selection reaches a crate.
    pub(crate) crates: bool,
    /// The members whose tests its selection runs, and those a path of it
    /// [`NARROWED`] names belongs to.
    pub(crate) members: BTreeSet<String>,
    /// The members the evidence of what each one read reaches, each with
    /// the evidence: Cargo's dep-info, the runners' own listings since the
    /// head, a string literal naming a changed file.
    pub(crate) evidence: BTreeMap<String, String>,
}

impl Touched {
    /// What `paths` reach, by `selection`, which judged their [`selected`]
    /// ones, and by the map.
    pub(crate) fn new(paths: &[RepoPath], selection: &Selection) -> Self {
        let named = || paths.iter().filter_map(narrowed);
        Self {
            gate_input: gate_input(paths).map(|path| path.as_str().to_owned()),
            jobs: [Job::PreMerge, Job::Perf, Job::Visual, Job::Tlc, Job::Kennel]
                .into_iter()
                .filter(|job| selection.selects(*job))
                .collect(),
            steps: named()
                .flat_map(|entry| entry.steps.iter().copied())
                .collect(),
            crates: selection.reaches_a_crate(),
            members: selection
                .members()
                .into_iter()
                .chain(named().filter_map(|entry| entry.member).map(str::to_owned))
                .collect(),
            evidence: BTreeMap::new(),
        }
    }
}

/// A recorded result of one unit: a whole step, or a step's tests of one
/// member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub(crate) green: bool,
    /// The head the unit ran on, and its tree.
    pub(crate) origin: String,
    pub(crate) tree: String,
    /// When it ran, in seconds since the Unix epoch; for a result that counted
    /// earned passes, when the first run that gave one began.
    pub(crate) at: u64,
    /// The head the earliest passes ran on, for a stress result that counted
    /// [`EarnedPasses`]. The result rests on a run this head never saw, so the
    /// backstop stresses it again.
    pub(crate) earned_on: Option<String>,
}

/// The clean passes a member's selected tests have toward their counts, from
/// the stress runs that stopped at a red iteration before they had them all
/// (#2233; ADRs 0064, 0135). Only a crate's: nextest runs each test in its own
/// process, where the other runners order them by the head's seed. A member
/// with a red test has none. They hold as a green result does, while nothing
/// that bears on the member changes since `origin`, and within `MAX_AGE` of
/// `at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EarnedPasses {
    /// The head the earliest of the passes ran on.
    pub(crate) origin: String,
    /// When the stress run that gave the earliest of them began, in seconds
    /// since the Unix epoch: no later than that pass, so never younger.
    pub(crate) at: u64,
    /// Each test's clean passes, by the ID the stress run's selection gives it.
    pub(crate) passes: BTreeMap<String, u32>,
}

/// What one gate run recorded: its head, the base its steps compared with,
/// and each unit's result, run there or carried from an earlier run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GateRun {
    pub(crate) head: String,
    pub(crate) base: String,
    /// The fingerprint of the host and its tools the run ran with.
    pub(crate) host: String,
    pub(crate) units: BTreeMap<String, Outcome>,
    /// The clean passes of the members its stress run left short, by member.
    pub(crate) earned: BTreeMap<String, EarnedPasses>,
}

/// A unit's key: the step's name, then `:<member>` for a step that runs
/// each member's tests separately.
pub(crate) fn unit(step: Step, member: Option<&str>) -> String {
    match member {
        Some(member) => format!("{}:{member}", step.name()),
        None => step.name().to_owned(),
    }
}

/// The head this run gates.
pub(crate) struct Head<'a> {
    /// The change's base: the merge base with `origin/main`.
    pub(crate) base: &'a str,
    /// Every member of the head's workspaces, and [`UNOWNED`] when some
    /// test belongs to none.
    pub(crate) members: &'a BTreeSet<String>,
    /// The members whose tests Vitest, Playwright or pytest run, which order
    /// them by the head's seed, and [`UNOWNED`].
    pub(crate) shuffled: &'a BTreeSet<String>,
    /// The fingerprint of this host and its tools.
    pub(crate) host: &'a str,
    /// Now, in seconds since the Unix epoch.
    pub(crate) now: u64,
}

/// What the run does with a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Runs it as the full gate does.
    Run { why: String },
    /// Runs its tests of these members only; the others' results are carried.
    Part {
        members: BTreeSet<String>,
        why: String,
    },
    /// Runs nothing: every unit's green result is carried, from the heads
    /// it ran on.
    Reuse { origins: BTreeSet<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Planned {
    pub(crate) step: Step,
    pub(crate) action: Action,
    /// The units whose green results this run carries into its record.
    pub(crate) carried: BTreeMap<String, Outcome>,
    /// The members it reruns that keep their clean passes, by member.
    pub(crate) earned: BTreeMap<String, EarnedPasses>,
}

/// Each step's action for `head`, from the nearest earlier run on an
/// ancestor (`previous`, `None` with the reason `no_previous` when there is
/// none or a full run is asked for) and what the change since each head a
/// carried result ran on reaches (`touched`, by that head).
pub(crate) fn plan(
    head: &Head<'_>,
    previous: Option<&GateRun>,
    no_previous: &str,
    touched: &BTreeMap<String, Touched>,
) -> Vec<Planned> {
    Step::ALL
        .into_iter()
        .map(|step| {
            let judge = |member: Option<&str>| match previous {
                None => Err(no_previous.to_owned()),
                Some(previous) => judge(step, member, head, previous, touched),
            };
            if !step.per_member() {
                return match judge(None) {
                    Ok(outcome) => Planned {
                        step,
                        action: Action::Reuse {
                            origins: BTreeSet::from([outcome.origin.clone()]),
                        },
                        carried: BTreeMap::from([(unit(step, None), outcome)]),
                        earned: BTreeMap::new(),
                    },
                    Err(why) => Planned {
                        step,
                        action: Action::Run { why },
                        carried: BTreeMap::new(),
                        earned: BTreeMap::new(),
                    },
                };
            }
            let mut rerun = BTreeSet::new();
            let mut reasons = Vec::new();
            let mut carried = BTreeMap::new();
            let mut earned = BTreeMap::new();
            for member in head.members {
                match judge(Some(member)) {
                    Ok(outcome) => {
                        carried.insert(unit(step, Some(member)), outcome);
                    }
                    Err(why) => {
                        rerun.insert(member.clone());
                        if !reasons.contains(&why) {
                            reasons.push(why);
                        }
                        if let Some(passes) = previous
                            .and_then(|previous| kept_passes(step, member, head, previous, touched))
                        {
                            earned.insert(member.clone(), passes);
                        }
                    }
                }
            }
            let why = reasons.join("; ");
            let action = if head.members.is_empty() {
                Action::Run {
                    why: "the workspaces have no member".to_owned(),
                }
            } else if rerun.is_empty() {
                Action::Reuse {
                    origins: carried
                        .values()
                        .map(|outcome| outcome.origin.clone())
                        .collect(),
                }
            } else if rerun.len() == head.members.len() && earned.is_empty() {
                Action::Run { why }
            } else {
                Action::Part {
                    members: rerun,
                    why,
                }
            };
            if matches!(action, Action::Run { .. }) {
                carried.clear();
            }
            Planned {
                step,
                action,
                carried,
                earned,
            }
        })
        .collect()
}

/// A unit's green result that this run may reuse, or why it reruns.
fn judge(
    step: Step,
    member: Option<&str>,
    head: &Head<'_>,
    previous: &GateRun,
    touched: &BTreeMap<String, Touched>,
) -> Result<Outcome, String> {
    if matches!(step.reach(), Reach::Always) {
        return Err("every run reruns it".to_owned());
    }
    // A full run on the head orders these tests by the head's seed: a
    // result from another tree's order would not be the same verdict.
    if step.orders_by_the_head() && member.is_some_and(|member| head.shuffled.contains(member)) {
        return Err(
            "Vitest, Playwright and pytest rerun every member in the head's test order".to_owned(),
        );
    }
    let Some(outcome) = previous.units.get(&unit(step, member)) else {
        return Err(format!("no result in the run on {}", short(&previous.head)));
    };
    let origin = short(&outcome.origin);
    if !outcome.green {
        return Err(format!("red on {origin}"));
    }
    still_holds(
        step,
        member,
        &outcome.origin,
        outcome.at,
        (head, previous, touched),
    )?;
    Ok(outcome.clone())
}

/// The clean passes of `member` this run may count, when `step` counts passes.
/// They hold by the rule a green result holds by, so they count only with the
/// member's inputs identical, on the same host and base, within the age limit.
fn kept_passes(
    step: Step,
    member: &str,
    head: &Head<'_>,
    previous: &GateRun,
    touched: &BTreeMap<String, Touched>,
) -> Option<EarnedPasses> {
    if !step.counts_passes() || head.shuffled.contains(member) {
        return None;
    }
    let passes = previous.earned.get(member)?;
    still_holds(
        step,
        Some(member),
        &passes.origin,
        passes.at,
        (head, previous, touched),
    )
    .ok()?;
    Some(passes.clone())
}

/// Whether what ran on `origin` at `at` still holds for this run: why not, if
/// not.
fn still_holds(
    step: Step,
    member: Option<&str>,
    origin_head: &str,
    at: u64,
    (head, previous, touched): (&Head<'_>, &GateRun, &BTreeMap<String, Touched>),
) -> Result<(), String> {
    let origin = short(origin_head);
    if previous.host != head.host {
        return Err(format!(
            "the host or its tools changed since the run on {}",
            short(&previous.head)
        ));
    }
    if past_max_age(head.now, at) {
        return Err(format!("its result on {origin} is older than a day"));
    }
    if step.reads_the_base() && previous.base != head.base {
        return Err(format!(
            "the base moved from {} to {}",
            short(&previous.base),
            short(head.base)
        ));
    }
    let Some(touched) = touched.get(origin_head) else {
        return Err(format!("the change since {origin} is unknown"));
    };
    if let Some(input) = &touched.gate_input {
        return Err(format!("{input} changed since {origin}"));
    }
    let reached = touched.steps.contains(&step)
        || match step.reach() {
            Reach::Always => true,
            Reach::Job(job) => touched.jobs.contains(&job),
            Reach::Crates => touched.crates,
            Reach::Members => member.is_some_and(|member| {
                touched.members.contains(member)
                    || touched.evidence.contains_key(member)
                    || (member == UNOWNED && !touched.members.is_empty())
            }),
        };
    if reached {
        return Err(
            match member.and_then(|member| touched.evidence.get(member)) {
                Some(evidence) => format!("the change since {origin} reaches it: {evidence}"),
                None => format!("the change since {origin} reaches it"),
            },
        );
    }
    Ok(())
}

/// A commit's short form for the log.
pub(crate) fn short(commit: &str) -> &str {
    commit.get(..10).unwrap_or(commit)
}
