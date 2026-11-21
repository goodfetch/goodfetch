//! A crawl's Hooks in code (ADR 0104): the chain of each stage, built once
//! as the Run opens, in the order the crawl declared them, each Hook scoped
//! by its Labels and, at the Record stage, by its Datasets; and the deadline
//! each call waits under, `timeout.hook`, a Label's own value before the
//! Run's. Hooks are never kept in the Run's state: a start that resumes the
//! Run declares them again.
//!
//! The Record stage, `before_record_commit`, runs here (the delivery's
//! `record_stage`). The request and page stages are issue 2905's, and a
//! start that declares a Hook at one of them is refused until they run.

use std::collections::BTreeMap;

use goodfetch_engine::CallError;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::LabelSettings;
use goodfetch_protocol::foxhound::hook::{CrawlHookDeclaration, CrawlHookStage};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::settings::{RunOrigin, TIMEOUT_HOOK};

/// The Hooks in code of one Run, by stage, and the deadlines their calls
/// wait under.
#[derive(Debug)]
pub(crate) struct Hooks {
    /// The Record stage's Hooks, in the order declared.
    record: Vec<CrawlHookDeclaration>,
    /// The Run's `timeout.hook`.
    run_deadline: Deadline,
    /// Each Label's own `timeout.hook`, where it sets one.
    label_deadlines: BTreeMap<String, Deadline>,
}

impl Hooks {
    /// The chains of `declared`, which [`check`] passed, under the Run's
    /// `resolved` settings and its Labels' `labels`.
    pub(crate) fn of(
        declared: Vec<CrawlHookDeclaration>,
        resolved: &Resolved,
        labels: &LabelSettings,
    ) -> Self {
        Self {
            record: declared
                .into_iter()
                .filter(|hook| hook.stage == CrawlHookStage::BeforeRecordCommit)
                .collect(),
            run_deadline: Deadline::of_resolved(Some(resolved), &TIMEOUT_HOOK),
            label_deadlines: labels
                .hook_timeouts()
                .map(|(label, millis)| {
                    (
                        label.to_owned(),
                        Deadline::set(&TIMEOUT_HOOK, millis, RunOrigin::Settings),
                    )
                })
                .collect(),
        }
    }

    /// Whether any Hook runs at the Record stage.
    pub(crate) fn at_record_stage(&self) -> bool {
        !self.record.is_empty()
    }

    /// The Record stage's Hooks a Record of `dataset`, pushed for a request
    /// of `label`, goes through, in the order declared.
    pub(crate) fn on_record<'a>(
        &'a self,
        label: &'a str,
        dataset: &'a str,
    ) -> impl Iterator<Item = &'a CrawlHookDeclaration> {
        self.record.iter().filter(move |hook| {
            in_scope(hook.labels.as_deref(), label) && in_scope(hook.datasets.as_deref(), dataset)
        })
    }

    /// The deadline each call of a Hook on a request of `label` waits
    /// under: its Label's `timeout.hook`, else the Run's.
    pub(crate) fn deadline(&self, label: &str) -> &Deadline {
        self.label_deadlines
            .get(label)
            .unwrap_or(&self.run_deadline)
    }
}

/// Whether a scope of `names`, none for every one, holds `name`.
fn in_scope(names: Option<&[String]>, name: &str) -> bool {
    names.is_none_or(|names| names.iter().any(|scoped| scoped == name))
}

/// Checks the stages of the Hooks a start declares.
///
/// # Errors
///
/// `invalid_params` for a Hook at a stage this engine does not run yet:
/// every stage but the Record stage, until issue 2905 lands.
pub(crate) fn check(declared: &[CrawlHookDeclaration]) -> Result<(), CallError> {
    match declared
        .iter()
        .find(|hook| hook.stage != CrawlHookStage::BeforeRecordCommit)
    {
        None => Ok(()),
        Some(hook) => Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "The Hook `{}` runs at `{}`, a stage this engine does not run Hooks in code at yet: only `before_record_commit` runs; declare it there, or leave it out.",
                hook.name,
                hook.stage.as_str()
            ),
        )),
    }
}

#[cfg(test)]
mod tests;
