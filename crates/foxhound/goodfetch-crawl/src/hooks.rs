//! A crawl's Hooks in code (ADR 0104): the chain of each stage, built once
//! as the Run opens, in the order the crawl declared them, each Hook scoped
//! by its Labels and, at the Record stage, by its Datasets; and the deadline
//! each call waits under, `timeout.hook`, a Label's own value before the
//! Run's. Hooks are never kept in the Run's state: a start that resumes the
//! Run declares them again.
//!
//! The request and page stages run in the delivery's attempt, around its
//! fetch and before its handler (`hook_call`), the Record stage after its
//! handler answered (`record_stage`). A stage none of a request's Hooks is
//! scoped to sends nothing and builds nothing. A Label a navigation Hook
//! reaches never races its tiers ([`Hooks::navigates`]).

use std::collections::BTreeMap;

use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::LabelSettings;
use goodfetch_protocol::foxhound::hook::{CrawlHookDeclaration, CrawlHookStage};
use goodfetch_protocol::settings::{RunOrigin, TIMEOUT_HOOK};

/// The number of stages a Hook in code runs at.
const STAGES: usize = CrawlHookStage::ALL.len();

/// The Hooks in code of one Run, by stage, and the deadlines their calls
/// wait under.
#[derive(Debug)]
pub(crate) struct Hooks {
    /// Each stage's Hooks, in the order declared, at the stage's place in
    /// [`CrawlHookStage::ALL`].
    chains: [Vec<CrawlHookDeclaration>; STAGES],
    /// The Run's `timeout.hook`.
    run_deadline: Deadline,
    /// Each Label's own `timeout.hook`, where it sets one.
    label_deadlines: BTreeMap<String, Deadline>,
}

impl Hooks {
    /// The chains of `declared`, under the Run's `resolved` settings and
    /// its Labels' `labels`.
    pub(crate) fn of(
        declared: Vec<CrawlHookDeclaration>,
        resolved: &Resolved,
        labels: &LabelSettings,
    ) -> Self {
        let mut chains: [Vec<CrawlHookDeclaration>; STAGES] = Default::default();
        for hook in declared {
            // A declaration reads only the stages this engine knows.
            if let Some(place) = place(&hook.stage) {
                chains[place].push(hook);
            }
        }
        Self {
            chains,
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

    /// The Hooks of `stage`, in the order declared.
    fn chain(&self, stage: &CrawlHookStage) -> &[CrawlHookDeclaration] {
        place(stage).map_or(&[], |place| &self.chains[place])
    }

    /// Whether any Hook runs at the Record stage.
    pub(crate) fn at_record_stage(&self) -> bool {
        !self.chain(&CrawlHookStage::BeforeRecordCommit).is_empty()
    }

    /// The Hooks of `stage` a request of `label` goes through, in the
    /// order declared.
    pub(crate) fn on_request<'a>(
        &'a self,
        stage: &CrawlHookStage,
        label: &'a str,
    ) -> impl Iterator<Item = &'a CrawlHookDeclaration> {
        self.chain(stage)
            .iter()
            .filter(move |hook| in_scope(hook.labels.as_deref(), label))
    }

    /// Whether a request of `label` has a navigation Hook, which runs
    /// around a navigation the attempt itself starts.
    pub(crate) fn navigates(&self, label: &str) -> bool {
        [
            CrawlHookStage::BeforeNavigation,
            CrawlHookStage::AfterNavigation,
        ]
        .iter()
        .any(|stage| self.on_request(stage, label).next().is_some())
    }

    /// The Record stage's Hooks a Record of `dataset`, pushed for a request
    /// of `label`, goes through, in the order declared.
    pub(crate) fn on_record<'a>(
        &'a self,
        label: &'a str,
        dataset: &'a str,
    ) -> impl Iterator<Item = &'a CrawlHookDeclaration> {
        self.on_request(&CrawlHookStage::BeforeRecordCommit, label)
            .filter(move |hook| in_scope(hook.datasets.as_deref(), dataset))
    }

    /// The deadline each call of a Hook on a request of `label` waits
    /// under: its Label's `timeout.hook`, else the Run's.
    pub(crate) fn deadline(&self, label: &str) -> &Deadline {
        self.label_deadlines
            .get(label)
            .unwrap_or(&self.run_deadline)
    }
}

/// The place of `stage` in [`CrawlHookStage::ALL`]; `None` for a stage
/// this engine does not know.
fn place(stage: &CrawlHookStage) -> Option<usize> {
    CrawlHookStage::ALL.iter().position(|known| known == stage)
}

/// Whether a scope of `names`, none for every one, holds `name`.
fn in_scope(names: Option<&[String]>, name: &str) -> bool {
    names.is_none_or(|names| names.iter().any(|scoped| scoped == name))
}

#[cfg(test)]
mod tests;
