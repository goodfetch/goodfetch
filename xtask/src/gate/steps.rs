//! The gate's step list (ADR 0135): what each form of `just pre-merge`
//! runs, in its order, and the `just` command each step runs.
//! `ci/pre-merge.sh` reads it from `xtask gate steps` and follows it, so
//! the list, its order and its commands live here alone; the script keeps
//! how a step runs (on the host, beside it, from the plan) and how a part
//! of a step run per member runs.

use std::fmt::Write as _;

use crate::affected::Job;

/// A step of `just pre-merge all` that the gate records, declared in the
/// order `ci/pre-merge.sh` starts them (`steps`): the checks that judge the
/// whole change in seconds first, so the gate's log opens with their
/// verdicts, then those beside the host's, then the host's others: the
/// other checks of the whole change, then the MSRV build, before the suites
/// (issue 2040). A red step ends no gate: every step runs and records its
/// result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    Dco,
    CommitMessages,
    FloorGuard,
    CommitLeaks,
    Audit,
    Notices,
    Perf,
    Counts,
    Linux,
    Visual,
    Tlc,
    Miri,
    Loom,
    Windows,
    Lint,
    FeatureCombinations,
    FuzzCheck,
    Proofs,
    Msrv,
    Quick,
    PreMerge,
    Stress,
    JourneysTasks,
    FeaturesTests,
    Kennel,
    /// xtask's codegen tests, `tests/` tests the Quick suite never runs, when
    /// the change reaches what only they check (`xtask affected`'s
    /// `codegen-tests=true`): the quick form's last step, so a change to the
    /// emitter or to the protocol files their fixture copies cannot merge on
    /// the quick gate with them red (issue 2892). The full forms' Pre-merge
    /// suite runs them with every other test.
    CodegenTests,
}

/// What a step's result depends on, besides the gate's own inputs.
#[derive(Clone, Copy)]
pub(crate) enum Reach {
    /// A check that judges the whole change and takes seconds or minutes:
    /// every run reruns it.
    Always,
    /// A check whose result follows from the tree and the host: it reruns on
    /// another tree, whatever the change reaches. Lint's also follows from
    /// the day and GitHub, which the plan reads again
    /// ([`super::plan::LintReads`]).
    Tree,
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
    pub(crate) const ALL: [Self; 26] = [
        Self::Dco,
        Self::CommitMessages,
        Self::FloorGuard,
        Self::CommitLeaks,
        Self::Audit,
        Self::Notices,
        Self::Perf,
        Self::Counts,
        Self::Linux,
        Self::Visual,
        Self::Tlc,
        Self::Miri,
        Self::Loom,
        Self::Windows,
        Self::Lint,
        Self::FeatureCombinations,
        Self::FuzzCheck,
        Self::Proofs,
        Self::Msrv,
        Self::Quick,
        Self::PreMerge,
        Self::Stress,
        Self::JourneysTasks,
        Self::FeaturesTests,
        Self::Kennel,
        Self::CodegenTests,
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
            Self::Counts => "counts",
            Self::Linux => "linux",
            Self::Visual => "visual",
            Self::Quick => "quick",
            Self::PreMerge => "pre-merge",
            Self::Stress => "stress",
            Self::Tlc => "tlc",
            Self::Msrv => "msrv",
            Self::Miri => "miri",
            Self::Loom => "loom",
            Self::Windows => "windows",
            Self::Kennel => "kennel",
            Self::CodegenTests => "codegen-tests",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|step| step.name() == name)
    }

    pub(crate) const fn reach(self) -> Reach {
        match self {
            Self::Dco
            | Self::CommitMessages
            | Self::FloorGuard
            | Self::Audit
            | Self::CommitLeaks
            | Self::FeaturesTests
            | Self::Notices => Reach::Always,
            // The Journey task lists follow from the tree's Proving ground,
            // and the codegen tests from the tree and its base.
            Self::Lint
            | Self::FeatureCombinations
            | Self::FuzzCheck
            | Self::Proofs
            | Self::JourneysTasks
            | Self::CodegenTests => Reach::Tree,
            // The counted rows read the benchmarks perf reads, and judge them
            // against the register's limits, never against the base.
            Self::Perf | Self::Counts => Reach::Job(Job::Perf),
            // On a Linux host the Pre-merge suite runs these tests itself;
            // elsewhere they are a slice of it, in a container.
            Self::Linux => Reach::Job(Job::PreMerge),
            Self::Visual => Reach::Job(Job::Visual),
            Self::Tlc => Reach::Job(Job::Tlc),
            Self::Kennel => Reach::Job(Job::Kennel),
            Self::Msrv | Self::Miri | Self::Loom | Self::Windows => Reach::Crates,
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

    /// Whether the step runs beside the host's: those that need Linux, in
    /// a container of their own where the host is not Linux, TLC, Miri
    /// (#2090's F7) and loom (#2200), and the Windows build under Wine
    /// (#2767), in a container of its own. Each builds and writes in
    /// folders of its own (`target/tlc/`, `target/miri/`, `target/loom/`,
    /// `target/windows/`), and its verdict follows from the
    /// tree and its pinned tools, whatever runs beside it; what it shares
    /// with the host's steps, it only reads or waits for (ADR 0135's note
    /// of 2026-11-13). The Kennel's
    /// check stays on the host, after the suites: it builds the test tools
    /// into the build folder the suites' own are in, with the features of
    /// its own build (`kennel/scripts/check.sh`), so beside them each could
    /// replace an executable under the other's tests.
    pub(crate) const fn beside(self) -> bool {
        matches!(
            self,
            Self::Perf
                | Self::Counts
                | Self::Linux
                | Self::Visual
                | Self::Tlc
                | Self::Miri
                | Self::Loom
                | Self::Windows
        )
    }

    /// The step whose units hold this one's, member by member: the Pre-merge
    /// suite's selection of a member holds every Quick test of it, so where
    /// the full gate runs or reuses a member's Pre-merge unit, which it does
    /// for every member, its Quick unit is covered and never runs beside it
    /// (ADR 0135's amendment of 2026-11-22).
    /// The Pre-merge suite runs xtask's codegen tests with every other test,
    /// so they are covered alike.
    pub(crate) const fn covered_by(self) -> Option<Self> {
        match self {
            Self::Quick | Self::CodegenTests => Some(Self::PreMerge),
            _ => None,
        }
    }

    /// Whether the step runs each member's tests separately.
    pub(crate) const fn per_member(self) -> bool {
        matches!(self.reach(), Reach::Members)
    }

    /// Whether the step runs Vitest, Playwright and pytest, which order
    /// their tests by the head's seed (TESTING.md, Before merge).
    pub(crate) const fn orders_by_the_head(self) -> bool {
        matches!(self, Self::Quick | Self::PreMerge | Self::Stress)
    }

    /// Whether the step asks each test for a count of clean passes, which a
    /// red run may leave part way ([`super::plan::EarnedPasses`]).
    pub(crate) const fn counts_passes(self) -> bool {
        matches!(self, Self::Stress)
    }

    /// Whether the step compares the head with the change's base (the merge
    /// base with `origin/main`), so its result holds only for that base. The
    /// codegen tests run only when the change since the base reaches them.
    pub(crate) const fn reads_the_base(self) -> bool {
        matches!(self, Self::Perf | Self::Stress | Self::CodegenTests)
    }
}

/// A form of `just pre-merge`, its argument (`changed` when it has none).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Form {
    /// The full gate, over the whole workspace, which the plan may narrow.
    All,
    /// The full gate's steps over what changed: `just pre-merge` alone.
    Changed,
    /// The gate of a change no test reads: the checks every change needs.
    Quick,
    /// A batch's: the quick form's steps and the Pre-merge suite over the
    /// whole workspace (ADR 0135's note of 2026-11-21), which the hosted
    /// tier runs for a batch's pull request, each member's Pre-merge unit
    /// covering its Quick unit.
    Batch,
    /// The gate of a change of documents alone.
    Docs,
    /// `just preflight`: the full gate's units only the gate host runs, the
    /// counted rows and the stress run, planned and recorded as the full
    /// gate's are, so the full gate on the same tree reuses them (ADR 0135's
    /// amendment of 2026-11-22).
    Preflight,
}

/// How the gate runs an entry of its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A step on the host, whose result the gate records.
    Step,
    /// A step beside the host's, in a container or in folders of its own,
    /// whose result the gate records once it has ended.
    Beside,
    /// A command on the host that is no step of the record.
    Command,
}

impl Kind {
    const fn name(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Beside => "beside",
            Self::Command => "command",
        }
    }
}

/// The layer that owns an entry: the one that runs it on a tree, while every
/// other layer reuses its result there (ADR 0135's amendment of 2026-11-22,
/// spec #3373's table of owning layers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// `just merge` on its own host, in seconds: the checks that judge the
    /// whole change.
    Local,
    /// The pull-request tier on hosted runners, on a batch, spec or fork
    /// pull request: `just merge` reuses its units, and runs here those no
    /// hosted run it accepts holds (ADR 0135's amendment of 2026-11-22).
    Hosted,
    /// The quiet gate host, `just preflight` or `just merge`, whichever runs
    /// first: what a noisy runner would move, perf and the stress run.
    GateHost,
}

impl Owner {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Hosted => "hosted",
            Self::GateHost => "gate-host",
        }
    }

    /// The owner of `step`.
    pub(crate) const fn of(step: Step) -> Self {
        if step.judges_the_whole_change() {
            Self::Local
        } else if matches!(step, Step::Perf | Step::Counts | Step::Stress) {
            Self::GateHost
        } else {
            Self::Hosted
        }
    }
}

/// One entry of a form's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: &'static str,
    pub(crate) kind: Kind,
    pub(crate) owner: Owner,
    /// The `just` arguments it runs.
    pub(crate) just: Vec<&'static str>,
}

/// The checks of `just lint` that read documents: the docs form's only
/// lint.
const LINT_DOCUMENTS: &str = "lint-documents";

/// The entries `form` runs, in their order.
pub(crate) fn entries(form: Form) -> Vec<Entry> {
    // A command is part of a hosted step: the documents' lint is lint's.
    let command = |name| Entry {
        name,
        kind: Kind::Command,
        owner: Owner::Hosted,
        just: vec![name],
    };
    let mut entries = Vec::new();
    for step in Step::ALL.into_iter().filter(|step| runs_in(*step, form)) {
        entries.push(Entry {
            name: step.name(),
            kind: if step.beside() {
                Kind::Beside
            } else {
                Kind::Step
            },
            owner: Owner::of(step),
            just: just(step, form),
        });
    }
    if form == Form::Docs {
        entries.push(command(LINT_DOCUMENTS));
    }
    entries
}

/// Whether `form` runs `step`. The full forms run no step another one's
/// units cover ([`Step::covered_by`]): the Pre-merge suite holds every
/// Quick test and xtask's codegen tests, so those two run only in the quick
/// and batch forms, where the plan covers each member's that a Pre-merge
/// unit holds.
pub(crate) const fn runs_in(step: Step, form: Form) -> bool {
    match form {
        Form::All | Form::Changed => step.covered_by().is_none(),
        Form::Quick => in_the_quick_form(step),
        Form::Batch => in_the_quick_form(step) || matches!(step, Step::PreMerge),
        Form::Docs => step.reads_the_commits(),
        Form::Preflight => matches!(step, Step::Counts | Step::Stress),
    }
}

/// The quick form's steps: the checks that judge the whole change, lint,
/// the Quick suite, the Journey task lists and xtask's codegen tests.
const fn in_the_quick_form(step: Step) -> bool {
    step.judges_the_whole_change()
        || matches!(
            step,
            Step::Lint | Step::Quick | Step::JourneysTasks | Step::CodegenTests
        )
}

/// The `just` arguments `step` runs in `form`. A step whose recipe runs
/// over the whole workspace or over what changed takes `all` for the
/// first, nothing for the second.
fn just(step: Step, form: Form) -> Vec<&'static str> {
    let mut arguments = match step {
        Step::JourneysTasks => return vec!["journeys-tasks", "--check"],
        // It reads the change itself, and runs no test when the change
        // reaches nothing only these tests check.
        Step::CodegenTests => return vec!["codegen-tests"],
        // One invocation, so the listings `journeys-tests` writes serve both.
        Step::FeaturesTests => return vec!["features-tests", "clients-tests"],
        Step::Quick => vec!["_suite", "quick"],
        Step::PreMerge => vec!["_suite", "pre-merge"],
        Step::Tlc => vec!["_tlc"],
        Step::Kennel => vec!["_kennel"],
        // The counted rows are a recipe of their own, apart from `just
        // perf`, so `just preflight` runs them without the bench build.
        Step::Counts => return vec!["counted-rows"],
        other => return vec![other.name()],
    };
    if matches!(form, Form::All | Form::Quick | Form::Batch) {
        arguments.push("all");
    }
    arguments
}

/// The list as `ci/pre-merge.sh` reads it, through `ci/output-value.sh`:
/// a `steps=` line of the entries' names in order, then each entry's
/// `<name>.kind=` (`step`, `beside` or `command`), `<name>.owner=`
/// (`local`, `hosted` or `gate-host`) and `<name>.just=` lines.
pub(crate) fn render(form: Form) -> String {
    let entries = entries(form);
    let names: Vec<&str> = entries.iter().map(|entry| entry.name).collect();
    let mut text = format!("steps={}\n", names.join(" "));
    for entry in &entries {
        let _ = writeln!(text, "{}.kind={}", entry.name, entry.kind.name());
        let _ = writeln!(text, "{}.owner={}", entry.name, entry.owner.name());
        let _ = writeln!(text, "{}.just={}", entry.name, entry.just.join(" "));
    }
    text
}
