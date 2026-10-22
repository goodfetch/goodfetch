//! The incremental gate's choice (ADR 0135): for each step of `just
//! pre-merge all`, whether this run reruns it in full, reruns it over some
//! packages only, or reuses the green result that a gate run on an ancestor
//! head recorded. The choice reads what `xtask affected` selects for the
//! change since the head each green result was produced on, never a guess.

use std::collections::{BTreeMap, BTreeSet};

use crate::affected::{Job, RepoPath, Selection};

/// A step of `just pre-merge all` that the gate records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    Dco,
    CommitMessages,
    Lint,
    FeatureCombinations,
    Audit,
    CommitLeaks,
    FuzzCheck,
    Proofs,
    JourneysTasks,
    FeaturesTests,
    Notices,
    Perf,
    Linux,
    Visual,
    Quick,
    PreMerge,
    Stress,
    Tlc,
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
    pub(crate) const ALL: [Self; 20] = [
        Self::Dco,
        Self::CommitMessages,
        Self::Lint,
        Self::FeatureCombinations,
        Self::Audit,
        Self::CommitLeaks,
        Self::FuzzCheck,
        Self::Proofs,
        Self::JourneysTasks,
        Self::FeaturesTests,
        Self::Notices,
        Self::Perf,
        Self::Linux,
        Self::Visual,
        Self::Quick,
        Self::PreMerge,
        Self::Stress,
        Self::Tlc,
        Self::Msrv,
        Self::Miri,
    ];

    /// The step's name in the plan, the record and `ci/pre-merge.sh`.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Dco => "dco",
            Self::CommitMessages => "commit-messages",
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
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|step| step.name() == name)
    }

    const fn reach(self) -> Reach {
        match self {
            Self::Dco
            | Self::CommitMessages
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
            Self::Msrv | Self::Miri => Reach::Crates,
            Self::Quick | Self::PreMerge | Self::Stress => Reach::Members,
        }
    }

    /// Whether the step runs each member's tests separately.
    pub(crate) const fn per_member(self) -> bool {
        matches!(self.reach(), Reach::Members)
    }

    /// Whether the step compares the head with the change's base (the merge
    /// base with `origin/main`), so its result holds only for that base.
    const fn reads_the_base(self) -> bool {
        matches!(self, Self::Perf | Self::Stress)
    }
}

/// The paths whose change reruns every step in full: the gate's own
/// commands and scripts, the tools' versions and settings, the lockfiles
/// and the build profiles. `xtask` is among them, since it runs most steps
/// and makes this very choice.
const GATE_INPUTS: [&str; 13] = [
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
];

/// The first changed path that is one of the gate's inputs.
pub(crate) fn gate_input(paths: &[RepoPath]) -> Option<&RepoPath> {
    paths
        .iter()
        .find(|path| GATE_INPUTS.iter().any(|input| path.is_in(input)))
}

/// What a change since a recorded head reaches, as the plan reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Touched {
    /// The first of the gate's inputs it changes, when it changes one.
    pub(crate) gate_input: Option<String>,
    /// The jobs of the pull-request tier its selection runs.
    pub(crate) jobs: BTreeSet<Job>,
    /// Whether its selection reaches a crate.
    pub(crate) crates: bool,
    /// The members whose tests its selection runs.
    pub(crate) members: BTreeSet<String>,
}

impl Touched {
    pub(crate) fn new(paths: &[RepoPath], selection: &Selection) -> Self {
        Self {
            gate_input: gate_input(paths).map(|path| path.as_str().to_owned()),
            jobs: [Job::PreMerge, Job::Perf, Job::Visual, Job::Tlc]
                .into_iter()
                .filter(|job| selection.selects(*job))
                .collect(),
            crates: selection.reaches_a_crate(),
            members: selection.members(),
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
}

/// What one gate run recorded: its head, the base its steps compared with,
/// and each unit's result, run there or carried from an earlier run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) head: String,
    pub(crate) base: String,
    pub(crate) units: BTreeMap<String, Outcome>,
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
    /// Every member of the head's workspaces.
    pub(crate) members: &'a BTreeSet<String>,
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
}

/// Each step's action for `head`, from the nearest earlier run on an
/// ancestor (`previous`, `None` with the reason `no_previous` when there is
/// none or a full run is asked for) and what the change since each head a
/// carried result ran on reaches (`touched`, by that head).
pub(crate) fn plan(
    head: &Head<'_>,
    previous: Option<&Record>,
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
                    },
                    Err(why) => Planned {
                        step,
                        action: Action::Run { why },
                        carried: BTreeMap::new(),
                    },
                };
            }
            let mut rerun = BTreeSet::new();
            let mut reasons = Vec::new();
            let mut carried = BTreeMap::new();
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
            } else if rerun.len() == head.members.len() {
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
            }
        })
        .collect()
}

/// A unit's green result that this run may reuse, or why it reruns.
fn judge(
    step: Step,
    member: Option<&str>,
    head: &Head<'_>,
    previous: &Record,
    touched: &BTreeMap<String, Touched>,
) -> Result<Outcome, String> {
    if matches!(step.reach(), Reach::Always) {
        return Err("every run reruns it".to_owned());
    }
    let Some(outcome) = previous.units.get(&unit(step, member)) else {
        return Err(format!("no result in the run on {}", short(&previous.head)));
    };
    let origin = short(&outcome.origin);
    if !outcome.green {
        return Err(format!("red on {origin}"));
    }
    if step.reads_the_base() && previous.base != head.base {
        return Err(format!(
            "the base moved from {} to {}",
            short(&previous.base),
            short(head.base)
        ));
    }
    let Some(touched) = touched.get(&outcome.origin) else {
        return Err(format!("the change since {origin} is unknown"));
    };
    if let Some(input) = &touched.gate_input {
        return Err(format!("{input} changed since {origin}"));
    }
    let reached = match step.reach() {
        Reach::Always => true,
        Reach::Job(job) => touched.jobs.contains(&job),
        Reach::Crates => touched.crates,
        Reach::Members => member.is_some_and(|member| touched.members.contains(member)),
    };
    if reached {
        return Err(format!("the change since {origin} reaches it"));
    }
    Ok(outcome.clone())
}

/// A commit's short form for the log.
pub(crate) fn short(commit: &str) -> &str {
    commit.get(..10).unwrap_or(commit)
}
