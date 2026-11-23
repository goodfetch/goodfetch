//! A crawl's Hooks in code (ADR 0104): the chain of each stage, built once
//! as the Run opens, in the order the crawl declared them, each Hook scoped
//! by its Labels and, at the Record stage, by its Datasets; and the deadline
//! each call waits under, `timeout.hook`, a Label's own value before the
//! Run's. A Hook's code is never kept in the Run's state: a start that
//! resumes the Run declares them again. Its run file keeps their names,
//! stages and scopes ([`record`]), so a reopen no start declared them for,
//! which could not call them, is refused ([`undeclared`], ADR 0104's note
//! of #2912).
//!
//! The request and page stages run in the delivery's attempt, around its
//! fetch and before its handler (`hook_call`), the Record stage after its
//! handler answered (`record_stage`). A stage none of a request's Hooks is
//! scoped to sends nothing and builds nothing. A Label a navigation or
//! after-response Hook reaches never races its tiers ([`Hooks::lets_race`]).

use std::collections::BTreeMap;

use goodfetch_engine::CallError;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_engine::settings::Resolved;
use goodfetch_fetch_tier::LabelSettings;
use goodfetch_protocol::foxhound::hook::{
    CrawlHookDeclaration, CrawlHookStage, HOOK_UNDECLARED, HookUndeclaredDetails,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{RunOrigin, TIMEOUT_HOOK};
use rusqlite::{Connection, params};

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

    /// Whether a request of `label` may race its tiers: no navigation
    /// Hook reaches it, as a race's first request may win with no
    /// navigation, and no after-response Hook, as that request is judged
    /// before it wins (ADR 0104: after response runs before anything
    /// judges the response).
    pub(crate) fn lets_race(&self, label: &str) -> bool {
        [
            CrawlHookStage::AfterResponse,
            CrawlHookStage::BeforeNavigation,
            CrawlHookStage::AfterNavigation,
        ]
        .iter()
        .all(|stage| self.on_request(stage, label).next().is_none())
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

/// The table that keeps in a crawl's run file its Hooks in code as its
/// latest start declared them, in that order: their names, stages and
/// scopes, never their code, which the program holds.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-crawl-hooks",
    statements: &["CREATE TABLE IF NOT EXISTS declared_hooks (
            position INTEGER NOT NULL PRIMARY KEY,
            declaration TEXT NOT NULL
        ) STRICT"],
};

/// Records `declared`, a start's Hooks in code, in place of those the run
/// file holds: none clears them.
///
/// # Errors
///
/// The run file's, when the rows cannot be written.
pub(crate) fn record(
    connection: &Connection,
    declared: &[CrawlHookDeclaration],
) -> rusqlite::Result<()> {
    connection
        .prepare_cached("DELETE FROM declared_hooks")
        .and_then(|mut cached| cached.execute([]))?;
    let mut insert = connection
        .prepare_cached("INSERT INTO declared_hooks (position, declaration) VALUES (?1, ?2)")?;
    for (position, hook) in (0_i64..).zip(declared) {
        let declaration =
            serde_json::to_string(hook).expect("invariant: a Hook's declaration encodes as JSON");
        insert.execute(params![position, declaration])?;
    }
    Ok(())
}

/// The engine's check of a reopen of the crawl `run_id` that no start
/// under its ID declared again (`run.resume`'s, `run.retry_failed`'s):
/// `hook_undeclared` when its run file `connection` records a Hook in
/// code, which such a reopen could not call (ADR 0104's note of #2912).
///
/// # Errors
///
/// The run file's, when the rows cannot be read.
pub(crate) fn undeclared(
    connection: &Connection,
    run_id: &RunId,
) -> rusqlite::Result<Result<(), CallError>> {
    let kept: Vec<String> = connection
        .prepare_cached("SELECT declaration FROM declared_hooks ORDER BY position")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if kept.is_empty() {
        return Ok(Ok(()));
    }
    // One a later engine declared at a stage this one does not know still
    // refuses the reopen, unnamed.
    let hooks: Vec<CrawlHookDeclaration> = kept
        .iter()
        .filter_map(|declaration| serde_json::from_str(declaration).ok())
        .collect();
    let names = hooks
        .iter()
        .map(|hook| format!("`{}`", hook.name))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(Err(CallError::new(
        &HOOK_UNDECLARED,
        format!(
            "The Run `{run_id}` was not reopened: its crawl declared Hooks in code ({names}), which run in its program, and no start under its ID declared them again. Resume it with `crawl.start` and its Run ID from that program; to retry an ended Run's failures, call `crawl.start` with its Run ID first, then `run.retry_failed`. `crawl.start` takes no Checkpoint yet, so a Checkpoint of this Run cannot be resumed."
        ),
        &HookUndeclaredDetails {
            run_id: run_id.clone(),
            hooks,
        },
    )))
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
