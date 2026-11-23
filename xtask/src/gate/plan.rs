//! The incremental gate's choice (ADR 0135): for each step of `just
//! pre-merge all`, whether this run reruns it in full, reruns it over some
//! packages only, or reuses the green result that a gate run on an ancestor
//! head recorded. The choice reads what `xtask affected` selects for the
//! change since the head each green result was produced on, never a guess.
//! Size: one concern, the choice for each step: the table of what each step
//! reads (`Step`, `GATE_INPUTS`, `NARROWED`), the rules that judge a recorded
//! result against the change (`judge`, `still_holds`) and the hosted pin
//! (`Pin`); the tables are about a third of its length.

use std::collections::{BTreeMap, BTreeSet};

use super::steps::{self, Form, Owner, Reach, Step};
use crate::affected::{Job, RepoPath, Selection};
use crate::products;

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
/// steps a run on another tree may reuse: one only a check every such run
/// reruns reads needs none, and neither does one no gate step runs. The evidence of what each member
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
const NARROWED: [Narrowed; 47] = [
    // xtask's own tests, with their fixtures. `just linux` runs some of them
    // on a host that is not Linux (ci/linux.sh).
    Narrowed {
        path: "xtask/tests",
        steps: &[Step::Linux],
        member: Some(XTASK),
    },
    // What one step of those a run on another tree may reuse runs or reads.
    read_by("ci/perf.sh", &[Step::Perf, Step::Counts]),
    read_by("ci/perf-reference.toml", &[Step::Perf]),
    read_by("ci/linux.sh", &[Step::Linux]),
    // ci/vm/common.sh, which ci/perf.sh and ci/linux.sh source, reads the pin.
    read_by(
        "ci/visual/image.toml",
        &[Step::Perf, Step::Counts, Step::Linux, Step::Visual],
    ),
    read_by("ci/visual", &[Step::Visual]),
    read_by("ci/vm/visual.sh", &[Step::Visual]),
    read_by(
        "ci/container.sh",
        &[
            Step::Perf,
            Step::Counts,
            Step::Linux,
            Step::Visual,
            Step::Windows,
        ],
    ),
    read_by("ci/msrv.sh", &[Step::Msrv]),
    read_by("ci/miri.toml", &[Step::Miri]),
    read_by("ci/loom.toml", &[Step::Loom]),
    read_by("ci/windows.sh", &[Step::Windows]),
    read_by("ci/windows", &[Step::Windows]),
    // What only the checks every run on another tree reruns read.
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
    read_by("ci/mutants-typescript.sh", &[]),
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

/// The local layer that ran a unit, which every result of the shared record
/// names (ADR 0135's amendment of 2026-11-22): every layer plans with the
/// one planner over the one record, so a unit any of them ran green on a
/// tree runs on it no more.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Layer {
    /// `just merge`'s gate, or `just pre-merge` run by hand.
    Merge,
    /// `just preflight`: the gate host's units of the full gate, alone.
    Preflight,
    /// The backstop's full run on `main`'s head.
    Backstop,
    /// The implementer's `just quick` (`quick_runs`).
    Quick,
    /// The pre-push hook's `just quick`.
    PrePush,
    /// A job of the hosted tier, the pull-request workflow on a batch, spec
    /// or fork pull request (ADR 0135's amendment of 2026-11-22). It runs
    /// only the hosted-owned units and leaves the others to their layers.
    Hosted,
}

impl Layer {
    /// The layer's name in the record and on `gate record --by`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Preflight => "preflight",
            Self::Backstop => "backstop",
            Self::Quick => "quick",
            Self::PrePush => "pre-push",
            Self::Hosted => "hosted",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        [
            Self::Merge,
            Self::Preflight,
            Self::Backstop,
            Self::Quick,
            Self::PrePush,
            Self::Hosted,
        ]
        .into_iter()
        .find(|layer| layer.name() == name)
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
    /// The layer that ran it; a result carried into a later record keeps it.
    pub(crate) by: Layer,
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
    /// The head's tree.
    pub(crate) tree: &'a str,
    /// The fingerprint of this host and its tools.
    pub(crate) host: &'a str,
    /// Now, in seconds since the Unix epoch.
    pub(crate) now: u64,
    /// What lint reads besides the tree and the host, read again now. Asked
    /// only for a green lint this run would otherwise reuse.
    pub(crate) lint: LintReads<'a>,
    /// The form of `just pre-merge` this run plans: a step another one's
    /// units cover is covered only where the form runs that one.
    pub(crate) form: Form,
    /// The layer that plans: the hosted tier's runs only the hosted-owned
    /// units and leaves every other to its owner.
    pub(crate) layer: Layer,
    /// What a hosted run's record must name to count for this head.
    pub(crate) pin: Pin<'a>,
}

/// The CI image the head's tree pins, and the repository whose pull
/// requests' hosted runs count for it: a hosted record counts as an earlier
/// run only when its fingerprint names both ([`hosted_fingerprint`]). The
/// repository is the pull request's head repository, so a fork's run, which
/// names the fork, never counts for this repository's head.
#[derive(Clone, Copy)]
pub(crate) struct Pin<'a> {
    /// The SHA-256 of the tree's `ci/image/Containerfile`, which pins its
    /// base image by digest and each tool by version and checksum: the
    /// image's identity, by which `ci/vm/common.sh` tags it too.
    pub(crate) image: &'a str,
    /// `<owner>/<name>`; empty where no hosted run is read.
    pub(crate) repository: &'a str,
}

/// The host line of a hosted job's record: the image its steps ran in, the
/// pull request's head repository, the runner image's version and the digest
/// of the variables its steps read, each after `hosted:`. Only the first two
/// decide whether it counts ([`Pin`]); the others say where it ran.
pub(crate) fn hosted_fingerprint(
    image: &str,
    repository: &str,
    runner: &str,
    variables: &str,
) -> String {
    format!("{HOSTED}{image}:{repository}:{runner}:{variables}")
}

const HOSTED: &str = "hosted:";

/// The image and the repository a hosted fingerprint names; none for a
/// local host's.
fn hosted_pin(host: &str) -> Option<(&str, &str)> {
    let mut fields = host.strip_prefix(HOSTED)?.splitn(3, ':');
    Some((fields.next()?, fields.next()?))
}

/// What lint reads besides the tree and the host, its register check the
/// day and its quarantine check GitHub, read again now: `Ok` when a lint run
/// now would read what a green one did, or why not.
pub(crate) struct LintReads<'a>(&'a dyn Fn() -> Result<(), String>);

impl<'a> LintReads<'a> {
    pub(crate) fn new(read: &'a impl Fn() -> Result<(), String>) -> Self {
        Self(read)
    }

    fn hold(&self) -> Result<(), String> {
        (self.0)()
    }
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
    /// Runs nothing and records nothing: the units of `by`, which the run
    /// runs or reuses for every member, hold this step's tests
    /// ([`Step::covered_by`]).
    Covered { by: Step },
    /// Runs nothing and records nothing: the hosted tier leaves the step to
    /// the layer that owns it (spec #3373's table of owning layers).
    Left { owner: Owner },
    /// Runs nothing: a hosted run went red on a unit of it on this tree, so
    /// the gate is red (ADR 0064), and the unit is never run again beside it.
    Red { why: String },
}

/// The plan of a step another step's units cover where `form` runs those,
/// whatever the records say.
pub(crate) fn covered(step: Step, form: Form) -> Option<Planned> {
    step.covered_by()
        .filter(|by| steps::runs_in(*by, form) || form == Form::Preflight)
        .map(|by| Planned {
            step,
            action: Action::Covered { by },
            carried: BTreeMap::new(),
            earned: BTreeMap::new(),
        })
}

/// Why a step that `by` covers runs nothing.
pub(crate) fn covered_why(by: Step) -> String {
    format!(
        "covered by {}, which runs or reuses each member's unit, and whose selection of a member holds every test of this step's",
        by.name()
    )
}

/// Why the hosted tier runs nothing of a step `owner` owns.
pub(crate) fn left_why(owner: Owner) -> String {
    format!(
        "the {} layer owns it, and the hosted tier leaves it to that layer",
        owner.name()
    )
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

/// The earlier runs a plan reads: the nearest local run on an ancestor
/// (`previous`, `None` with the reason `no_previous` when there is none or a
/// full run is asked for), then the accepted hosted runs on the head or its
/// ancestors, nearest first (`hosted`), whose units count for the hosted-owned
/// steps alone, under the head's [`Pin`].
pub(crate) struct Earlier<'a> {
    pub(crate) previous: Option<&'a GateRun>,
    pub(crate) no_previous: &'a str,
    pub(crate) hosted: &'a [GateRun],
}

/// Why a unit's earlier result is not reused.
enum Unreused {
    /// It runs here.
    Rerun(String),
    /// A hosted run went red on it on this tree.
    Red(String),
}

/// Each step's action for `head`, from the `earlier` runs and what the change
/// since each head a carried result ran on reaches (`touched`, by that head).
/// Each unit takes the first earlier result that holds: the local run's, then
/// the hosted runs' in their order.
pub(crate) fn plan(
    head: &Head<'_>,
    earlier: &Earlier<'_>,
    touched: &BTreeMap<String, Touched>,
) -> Vec<Planned> {
    // The hosted tier runs no Pre-merge suite of a member whose tests launch
    // a browser or use an OS service: `just merge` runs those.
    let unhosted: BTreeSet<String> = if head.layer == Layer::Hosted {
        head.members
            .iter()
            .filter(|member| !launches_no_browser(head, member))
            .cloned()
            .collect()
    } else {
        BTreeSet::new()
    };
    let mut pre_merge = plan_step(Step::PreMerge, head, earlier, touched, &unhosted);
    if let Action::Part { why, .. } = &mut pre_merge.action
        && !unhosted.is_empty()
    {
        why.push_str(
            "; the members whose tests launch a browser or use an OS service run on the gate host",
        );
    }
    let in_hand = held_by(&pre_merge, head);
    Step::ALL
        .into_iter()
        .map(|step| {
            if step == Step::PreMerge {
                pre_merge.clone()
            } else if step.covered_by() == Some(Step::PreMerge) {
                plan_covered(step, (head, earlier, touched), &pre_merge, &in_hand)
            } else {
                plan_step(step, head, earlier, touched, &BTreeSet::new())
            }
        })
        .collect()
}

/// The crates whose tests launch Chrome for Testing or use an OS service
/// (ADR 0059's extra loopback addresses, the Secret Service, `tc netem`, the
/// user's service manager): the hosted tier's Pre-merge shards run none of
/// their tests until its browser shards do (spec #3373). A crate whose
/// tests come to launch the browser joins it: `.config/nextest.toml` names
/// each such test for the cells without the pinned build, and a unit test
/// holds every crate it names here.
pub(crate) const BROWSER_OR_SERVICE_CRATES: [&str; 10] = [
    "goodfetch-collie",
    "goodfetch-collie-client",
    "goodfetch-conformance",
    "goodfetch-engine",
    "goodfetch-fetch-tier",
    "goodfetch-foxhound",
    "goodfetch-page",
    "goodfetch-proving-ground",
    "goodfetch-test-support",
    XTASK,
];

/// Whether `member`'s tests launch no browser and use no OS service: a
/// crate outside [`BROWSER_OR_SERVICE_CRATES`]. Vitest's, Playwright's and
/// pytest's members drive the Products, so none is.
fn launches_no_browser(head: &Head<'_>, member: &str) -> bool {
    !head.shuffled.contains(member)
        && member != UNOWNED
        && !BROWSER_OR_SERVICE_CRATES.contains(&member)
}

/// The members whose Pre-merge unit this run holds: those it carries, and
/// those it runs where the form runs the Pre-merge suite.
fn held_by(pre_merge: &Planned, head: &Head<'_>) -> BTreeSet<String> {
    let runs_here = steps::runs_in(Step::PreMerge, head.form);
    head.members
        .iter()
        .filter(|member| {
            pre_merge
                .carried
                .contains_key(&unit(Step::PreMerge, Some(member)))
                || (runs_here
                    && match &pre_merge.action {
                        Action::Run { .. } => {
                            head.layer != Layer::Hosted || launches_no_browser(head, member)
                        }
                        Action::Part { members, .. } => members.contains(*member),
                        _ => false,
                    })
        })
        .cloned()
        .collect()
}

/// The plan of a step the Pre-merge suite's units cover ([`Step::covered_by`]):
/// covered in full where the form runs that suite for every member, else
/// member by member, by the members whose Pre-merge unit the run holds
/// (`in_hand`). Red when the Pre-merge suite's plan is: a hosted unit red on
/// the head's tree is the gate's verdict (ADR 0064).
fn plan_covered(
    step: Step,
    (head, earlier, touched): (&Head<'_>, &Earlier<'_>, &BTreeMap<String, Touched>),
    pre_merge: &Planned,
    in_hand: &BTreeSet<String>,
) -> Planned {
    if head.layer != Layer::Hosted
        && let Some(planned) = covered(step, head.form)
    {
        return planned;
    }
    if let Action::Red { why } = &pre_merge.action {
        return carrying_nothing(
            step,
            Action::Red {
                why: format!("{} is red: {why}", Step::PreMerge.name()),
            },
        );
    }
    let all_held = !head.members.is_empty() && head.members.iter().all(|m| in_hand.contains(m));
    let covered_here = if step.per_member() {
        all_held
    } else {
        // xtask's Pre-merge unit runs its codegen tests.
        in_hand.contains(XTASK)
    };
    if covered_here {
        return carrying_nothing(step, Action::Covered { by: Step::PreMerge });
    }
    let skipped = if step.per_member() {
        in_hand.clone()
    } else {
        BTreeSet::new()
    };
    let mut planned = plan_step(step, head, earlier, touched, &skipped);
    if let Action::Part { why, .. } = &mut planned.action
        && !skipped.is_empty()
    {
        why.push_str("; the others' tests are covered by ");
        why.push_str(Step::PreMerge.name());
        why.push_str(", whose unit of each the run holds");
    }
    planned
}

/// One step's action for `head`, as [`plan`] says, judging every member but
/// those of `skipped`, which it neither runs nor carries.
fn plan_step(
    step: Step,
    head: &Head<'_>,
    earlier: &Earlier<'_>,
    touched: &BTreeMap<String, Touched>,
    skipped: &BTreeSet<String>,
) -> Planned {
    let owner = Owner::of(step);
    if head.layer == Layer::Hosted && owner != Owner::Hosted {
        return carrying_nothing(step, Action::Left { owner });
    }
    let judge = |member: Option<&str>| judge_unit(step, member, owner, head, earlier, touched);
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
            Err(Unreused::Red(why)) => carrying_nothing(step, Action::Red { why }),
            Err(Unreused::Rerun(why)) => carrying_nothing(step, Action::Run { why }),
        };
    }
    let previous = earlier.previous;
    let mut rerun = BTreeSet::new();
    let mut reasons = Vec::new();
    let mut red = Vec::new();
    let mut carried = BTreeMap::new();
    let mut earned = BTreeMap::new();
    for member in head
        .members
        .iter()
        .filter(|member| !skipped.contains(*member))
    {
        match judge(Some(member)) {
            Ok(outcome) => {
                carried.insert(unit(step, Some(member)), outcome);
            }
            Err(Unreused::Red(why)) => red.push(format!("{member}: {why}")),
            Err(Unreused::Rerun(why)) => {
                rerun.insert(member.clone());
                if !reasons.contains(&why) {
                    reasons.push(why);
                }
                if let Some(passes) =
                    previous.and_then(|previous| kept_passes(step, member, head, previous, touched))
                {
                    earned.insert(member.clone(), passes);
                }
            }
        }
    }
    if !red.is_empty() {
        return carrying_nothing(
            step,
            Action::Red {
                why: red.join("; "),
            },
        );
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
}

/// A step's plan that carries no earlier unit.
fn carrying_nothing(step: Step, action: Action) -> Planned {
    Planned {
        step,
        action,
        carried: BTreeMap::new(),
        earned: BTreeMap::new(),
    }
}

/// The earlier result one unit takes: the local run's, then, for a
/// hosted-owned step, the hosted runs' in their order, a red one ending the
/// search.
fn judge_unit(
    step: Step,
    member: Option<&str>,
    owner: Owner,
    head: &Head<'_>,
    earlier: &Earlier<'_>,
    touched: &BTreeMap<String, Touched>,
) -> Result<Outcome, Unreused> {
    let mut reasons = vec![match earlier.previous {
        None => earlier.no_previous.to_owned(),
        Some(previous) => match judge(step, member, head, (previous, Rule::SameHost), touched) {
            Ok(outcome) => return Ok(outcome),
            Err(Unreused::Rerun(why) | Unreused::Red(why)) => why,
        },
    }];
    if owner == Owner::Hosted {
        for run in earlier.hosted {
            match judge(step, member, head, (run, Rule::Pinned), touched) {
                Ok(outcome) => return Ok(outcome),
                Err(Unreused::Red(why)) => return Err(Unreused::Red(why)),
                Err(Unreused::Rerun(why)) => reasons.push(why),
            }
        }
    }
    reasons.dedup();
    Err(Unreused::Rerun(reasons.join("; ")))
}

/// What an earlier run's host must match for its results to count.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// A local run's: this host and its tools.
    SameHost,
    /// A hosted run's: the head's [`Pin`], its image and repository.
    Pinned,
}

/// A unit's green result that this run may reuse, or why not.
fn judge(
    step: Step,
    member: Option<&str>,
    head: &Head<'_>,
    (previous, rule): (&GateRun, Rule),
    touched: &BTreeMap<String, Touched>,
) -> Result<Outcome, Unreused> {
    let rerun = |why: String| Err(Unreused::Rerun(why));
    if matches!(step.reach(), Reach::Always) {
        return rerun("every run reruns it".to_owned());
    }
    if rule == Rule::Pinned {
        let on = short(&previous.head);
        match hosted_pin(&previous.host) {
            None => return rerun(format!("the hosted run on {on} names no CI image")),
            Some((_, repository)) if repository != head.pin.repository => {
                return rerun(format!(
                    "the hosted run on {on} is another repository's pull request"
                ));
            }
            Some((image, _)) if image != head.pin.image => {
                return rerun(format!("the hosted run on {on} ran in another CI image"));
            }
            Some(_) => {}
        }
    }
    let Some(outcome) = previous.units.get(&unit(step, member)) else {
        return rerun(format!("no result in the run on {}", short(&previous.head)));
    };
    let origin = short(&outcome.origin);
    if !outcome.green {
        // A hosted unit red on this very tree is the gate's verdict: running
        // it again here could only hide a flaky test (ADR 0064).
        if rule == Rule::Pinned && outcome.tree == head.tree {
            return Err(Unreused::Red(format!("red in the hosted run on {origin}")));
        }
        return rerun(format!("red on {origin}"));
    }
    // These runners order a member's tests by the seed the tree under test
    // gives (`ci/order-seed.sh`): a result on another tree ran in another
    // order, so it is not this tree's verdict.
    if step.orders_by_the_head()
        && member.is_some_and(|member| head.shuffled.contains(member))
        && outcome.tree != head.tree
    {
        return rerun(
            "Vitest, Playwright and pytest rerun every member in the head's test order".to_owned(),
        );
    }
    if matches!(step.reach(), Reach::Tree) && outcome.tree != head.tree {
        return rerun(format!("the tree changed since {origin}"));
    }
    still_holds(
        step,
        member,
        (&outcome.origin, outcome.at),
        (head, previous, rule),
        touched,
    )
    .map_err(Unreused::Rerun)?;
    // Lint alone reads more than the tree and the host.
    if matches!(step, Step::Lint) {
        head.lint.hold().map_err(Unreused::Rerun)?;
    }
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
        (&passes.origin, passes.at),
        (head, previous, Rule::SameHost),
        touched,
    )
    .ok()?;
    Some(passes.clone())
}

/// Whether what ran on `origin` at `at` still holds for this run: why not, if
/// not. A hosted run's host was judged by its pin, in `judge`.
fn still_holds(
    step: Step,
    member: Option<&str>,
    (origin_head, at): (&str, u64),
    (head, previous, rule): (&Head<'_>, &GateRun, Rule),
    touched: &BTreeMap<String, Touched>,
) -> Result<(), String> {
    let origin = short(origin_head);
    if rule == Rule::SameHost && previous.host != head.host {
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
            // Judged by its tree, in `judge`.
            Reach::Tree => false,
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
