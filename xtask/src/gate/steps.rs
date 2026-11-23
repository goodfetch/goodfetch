//! The gate's step list (ADR 0135): what each form of `just pre-merge`
//! runs, in its order, and the `just` command each step runs.
//! `ci/pre-merge.sh` reads it from `xtask gate steps` and follows it, so
//! the list, its order and its commands live here alone; the script keeps
//! how a step runs (on the host, beside it, from the plan) and how a part
//! of a step run per member runs.

use std::fmt::Write as _;

use super::plan::Step;

/// A form of `just pre-merge`, its argument (`changed` when it has none).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Form {
    /// The full gate, over the whole workspace, which the plan may narrow.
    All,
    /// The full gate's steps over what changed: `just pre-merge` alone.
    Changed,
    /// The gate of a change no test reads: the checks every change needs.
    Quick,
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
/// form.
pub(crate) const fn runs_in(step: Step, form: Form) -> bool {
    match form {
        Form::All | Form::Changed => step.covered_by().is_none(),
        Form::Quick => {
            step.judges_the_whole_change()
                || matches!(
                    step,
                    Step::Lint | Step::Quick | Step::JourneysTasks | Step::CodegenTests
                )
        }
        Form::Docs => step.reads_the_commits(),
        Form::Preflight => matches!(step, Step::Counts | Step::Stress),
    }
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
    if matches!(form, Form::All | Form::Quick) {
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
