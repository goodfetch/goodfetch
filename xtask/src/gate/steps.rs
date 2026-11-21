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

/// One entry of a form's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) name: &'static str,
    pub(crate) kind: Kind,
    /// The `just` arguments it runs.
    pub(crate) just: Vec<&'static str>,
}

/// The checks of `just lint` that read documents: the docs form's only
/// lint.
const LINT_DOCUMENTS: &str = "lint-documents";

/// xtask's codegen tests, `tests/` tests the Quick suite never runs, when
/// the change reaches what only they check (`xtask affected`'s
/// `codegen-tests=true`): the quick form's last entry, so a change to the
/// emitter or to the protocol files their fixture copies cannot merge on the
/// quick gate with them red (issue 2892). The full form's Pre-merge suite
/// runs them with every other test.
const CODEGEN_TESTS: &str = "codegen-tests";

/// The entries `form` runs, in their order.
pub(crate) fn entries(form: Form) -> Vec<Entry> {
    let command = |name| Entry {
        name,
        kind: Kind::Command,
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
            just: just(step, form),
        });
    }
    match form {
        Form::Docs => entries.push(command(LINT_DOCUMENTS)),
        Form::Quick => entries.push(command(CODEGEN_TESTS)),
        Form::All | Form::Changed => {}
    }
    entries
}

/// Whether `form` runs `step`.
const fn runs_in(step: Step, form: Form) -> bool {
    match form {
        Form::All | Form::Changed => true,
        Form::Quick => {
            step.judges_the_whole_change()
                || matches!(step, Step::Lint | Step::Quick | Step::JourneysTasks)
        }
        Form::Docs => step.reads_the_commits(),
    }
}

/// The `just` arguments `step` runs in `form`. A step whose recipe runs
/// over the whole workspace or over what changed takes `all` for the
/// first, nothing for the second.
fn just(step: Step, form: Form) -> Vec<&'static str> {
    let mut arguments = match step {
        Step::JourneysTasks => return vec!["journeys-tasks", "--check"],
        // One invocation, so the listings `journeys-tests` writes serve both.
        Step::FeaturesTests => return vec!["features-tests", "clients-tests"],
        Step::Quick => vec!["_suite", "quick"],
        Step::PreMerge => vec!["_suite", "pre-merge"],
        Step::Tlc => vec!["_tlc"],
        Step::Kennel => vec!["_kennel"],
        other => return vec![other.name()],
    };
    if matches!(form, Form::All | Form::Quick) {
        arguments.push("all");
    }
    arguments
}

/// The list as `ci/pre-merge.sh` reads it, through `ci/output-value.sh`:
/// a `steps=` line of the entries' names in order, then each entry's
/// `<name>.kind=` (`step`, `beside` or `command`) and `<name>.just=` lines.
pub(crate) fn render(form: Form) -> String {
    let entries = entries(form);
    let names: Vec<&str> = entries.iter().map(|entry| entry.name).collect();
    let mut text = format!("steps={}\n", names.join(" "));
    for entry in &entries {
        let _ = writeln!(text, "{}.kind={}", entry.name, entry.kind.name());
        let _ = writeln!(text, "{}.just={}", entry.name, entry.just.join(" "));
    }
    text
}
