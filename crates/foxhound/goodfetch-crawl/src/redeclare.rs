//! The Datasets, the Labels and the code-free crawl a start under the
//! ID of a Run declares when no crawl of this engine works that Run (ADRs
//! 0102 and 0103): the start checked the Datasets against those the Run
//! began with, and the Labels against its pending requests, and all are
//! kept until the Run resumes here, when they are recorded in its run file
//! in one write before any work, so the Records committed from then on are
//! read by the new extraction or declaration, checked against the schema
//! generated from it, and keep its hash, each enqueue is checked against
//! the new Labels,
//! a new one accepted, and each Label works under its new declaration,
//! here and in an engine that reopens the Run (#2825). Its Hooks in code
//! are kept beside them and handed to the resumed delivery, their names,
//! stages and scopes recorded in the same write, never their code (ADR
//! 0104 and its note of #2912). A start under the ID of a crawl this engine
//! works changes none of its declarations.
#![expect(
    clippy::disallowed_types,
    reason = "Claim and PendingDelivery release or restore their entries on Drop, which cannot wait, and no lock guard crosses an await: one Mutex guards the two in-memory maps alone, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use goodfetch_dataset::Datasets;
use goodfetch_engine::run_state::RunState;
use goodfetch_frontier::payload::PayloadRules;
use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use goodfetch_protocol::foxhound::hook::CrawlHookDeclaration;
use goodfetch_protocol::run::RunId;

use crate::code_free::CodeFreeCrawl;
use crate::serving::Crawls;
use crate::{code_free, hooks, labels};

/// What a start under the ID of a Run declared: its Datasets, its Labels,
/// as declared and as the rules its Payloads are checked by, the
/// extraction or declaration of a code-free crawl, none for one its
/// program handles, and its Hooks in code.
struct Declared {
    datasets: Datasets,
    labels: BTreeMap<String, LabelDeclaration>,
    payloads: PayloadRules,
    crawl: Option<CodeFreeCrawl>,
    hooks: Vec<CrawlHookDeclaration>,
}

/// The declarations a start under the ID of a Run checked, kept for its
/// resume: its Datasets, its Labels as declared, their Payload rules, and
/// the extraction or declaration of a code-free crawl.
pub(crate) type Kept = (
    Datasets,
    BTreeMap<String, LabelDeclaration>,
    PayloadRules,
    Option<CodeFreeCrawl>,
);

#[derive(Default)]
struct State {
    /// Kept by `crawl.start` on an ended crawl, available to be claimed by at
    /// most one `run.retry_failed` reopen.
    kept: BTreeMap<RunId, Declared>,
    /// Claimed by a reopen (or `crawl.start` resuming an interrupted crawl)
    /// and waiting for its spawned driver to record them.
    delivering: BTreeMap<RunId, (u64, Declared)>,
    /// Monotonic token identifying each `delivering` entry so cancelling a
    /// failed resume never removes a newer entry.
    next_token: u64,
}

/// The declarations kept for each Run until it resumes.
#[derive(Clone, Default)]
pub(crate) struct Redeclared(Arc<Mutex<State>>);

/// An atomic claim on a Run's kept declarations, held while `run.retry_failed`
/// attempts to reopen the Run (#3542).
///
/// Calling [`Self::deliver`] hands the declarations to the reopened Run's
/// driver; calling [`Self::restore`] (or dropping the claim if cancelled
/// before delivery) puts them back unless a newer `crawl.start` already
/// replaced them.
pub(crate) struct Claim {
    redeclared: Redeclared,
    run_id: RunId,
    declared: Option<Declared>,
}

impl Claim {
    /// Hands the claimed declarations to the driver about to be spawned for
    /// this Run.
    pub(crate) fn deliver(mut self) {
        if let Some(declared) = self.declared.take() {
            let mut state = self.redeclared.lock();
            state.next_token += 1;
            let token = state.next_token;
            state
                .delivering
                .insert(self.run_id.clone(), (token, declared));
        }
    }

    /// Returns the claimed declarations to `kept` after a reopen that did not
    /// start a driver, without overwriting declarations kept by a newer start.
    pub(crate) fn restore(mut self) {
        if let Some(declared) = self.declared.take() {
            self.redeclared
                .lock()
                .kept
                .entry(self.run_id.clone())
                .or_insert(declared);
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(declared) = self.declared.take() {
            self.redeclared
                .lock()
                .kept
                .entry(self.run_id.clone())
                .or_insert(declared);
        }
    }
}

/// Declarations staged in `delivering` by `crawl.start` while resuming an
/// interrupted Run. Calling [`Self::commit`] leaves them for the spawned
/// driver; [`Self::move_to_kept`] moves them to `kept` when the Run had
/// already ended; [`Self::cancel`] (or dropping without committing) removes
/// them if the resume failed before starting a driver.
pub(crate) struct PendingDelivery {
    redeclared: Redeclared,
    run_id: RunId,
    token: u64,
    active: bool,
}

impl PendingDelivery {
    /// Marks the resume as started so the spawned driver will consume the
    /// staged declarations.
    pub(crate) fn commit(mut self) {
        self.active = false;
    }

    /// Moves the staged declarations to `kept` when the resume saw the Run had
    /// already ended and spawned no driver, without overwriting a newer start.
    pub(crate) fn move_to_kept(mut self) {
        if let Some(declared) = self.take_staged() {
            self.redeclared
                .lock()
                .kept
                .entry(self.run_id.clone())
                .or_insert(declared);
        }
    }

    /// Removes the staged declarations when the resume failed before starting
    /// a driver, leaving any newer entry intact.
    pub(crate) fn cancel(mut self) {
        let _ = self.take_staged();
    }

    fn take_staged(&mut self) -> Option<Declared> {
        if !self.active {
            return None;
        }
        self.active = false;
        let mut state = self.redeclared.lock();
        if state
            .delivering
            .get(&self.run_id)
            .is_some_and(|(token, _)| *token == self.token)
        {
            state.delivering.remove(&self.run_id).map(|(_, d)| d)
        } else {
            None
        }
    }
}

impl Drop for PendingDelivery {
    fn drop(&mut self) {
        let _ = self.take_staged();
    }
}

impl Redeclared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `datasets`, the Labels `labels` and their Payload rules
    /// `payloads`, its code-free `crawl` and `hooks` for a later
    /// `run.retry_failed` reopen of the ended Run `run_id`, in place of any
    /// kept before, unless a crawl of `crawls` works it, and answers whether
    /// it kept them.
    pub(crate) async fn keep(
        &self,
        run_id: &RunId,
        (datasets, labels, payloads, crawl): Kept,
        hooks: Vec<CrawlHookDeclaration>,
        crawls: &Crawls,
    ) -> bool {
        let keeps = crawls.live(run_id).await.is_none();
        if keeps {
            self.lock().kept.insert(
                run_id.clone(),
                Declared {
                    datasets,
                    labels,
                    payloads,
                    crawl,
                    hooks,
                },
            );
        }
        keeps
    }

    /// Stages `datasets`, `labels`, `payloads`, `crawl` and `hooks` directly
    /// for the driver of an interrupted Run `run_id` that `crawl.start` is
    /// resuming right now, isolated from `run.retry_failed` claims. Answers
    /// `None` if a crawl of `crawls` already works `run_id`.
    pub(crate) async fn keep_for_resume(
        &self,
        run_id: &RunId,
        (datasets, labels, payloads, crawl): Kept,
        hooks: Vec<CrawlHookDeclaration>,
        crawls: &Crawls,
    ) -> Option<PendingDelivery> {
        if crawls.live(run_id).await.is_some() {
            return None;
        }
        let mut state = self.lock();
        state.next_token += 1;
        let token = state.next_token;
        state.delivering.insert(
            run_id.clone(),
            (
                token,
                Declared {
                    datasets,
                    labels,
                    payloads,
                    crawl,
                    hooks,
                },
            ),
        );
        Some(PendingDelivery {
            redeclared: self.clone(),
            run_id: run_id.clone(),
            token,
            active: true,
        })
    }

    /// Atomically claims the declarations kept for `run_id` so at most one
    /// concurrent `run.retry_failed` reopen can authorize them (#3542).
    pub(crate) fn claim(&self, run_id: &RunId) -> Option<Claim> {
        let declared = self.lock().kept.remove(run_id)?;
        Some(Claim {
            redeclared: self.clone(),
            run_id: run_id.clone(),
            declared: Some(declared),
        })
    }

    /// Moves any unrecorded declarations staged for `run_id` back to `kept`
    /// when a resumed driver exits before calling [`Self::record`] (e.g. its
    /// Sinks were refused), without overwriting declarations kept by a newer
    /// start.
    pub(crate) fn restore_delivering(&self, run_id: &RunId) {
        let mut state = self.lock();
        if let Some((_, declared)) = state.delivering.remove(run_id) {
            state.kept.entry(run_id.clone()).or_insert(declared);
        }
    }

    /// Records the declarations staged for the resuming `run_id`, if any, in
    /// its run file through `state`, and answers the Hooks in code kept with
    /// them, none when nothing was staged. A write that fails is logged: the
    /// Run goes on under the declarations it held.
    pub(crate) async fn record(
        &self,
        run_id: &RunId,
        state: &RunState,
    ) -> Vec<CrawlHookDeclaration> {
        let Some((
            _,
            Declared {
                datasets,
                labels,
                payloads,
                crawl,
                hooks,
            },
        )) = self.lock().delivering.remove(run_id)
        else {
            return Vec::new();
        };
        let recorded = hooks.clone();
        if let Err(error) = state
            .write(move |connection| {
                datasets.record_again(connection)?;
                labels::record_again(connection, &labels)?;
                payloads.record_labels_again(connection)?;
                code_free::record_again(connection, crawl.as_ref())?;
                hooks::record(connection, &recorded)
            })
            .await
        {
            tracing::warn!(%error, run_id = %run_id, "the resumed crawl's Datasets and Labels were not recorded again");
        }
        hooks
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use goodfetch_dataset::Datasets;
    use goodfetch_engine::run_state::{RunState, RunStateOptions};
    use goodfetch_engine::settings::{Layers, resolve};
    use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
    use goodfetch_frontier::payload::PayloadRules;
    use goodfetch_protocol::catalog::Product;
    use goodfetch_protocol::foxhound::hook::{CrawlHookDeclaration, CrawlHookStage};
    use goodfetch_protocol::run::RunId;
    use goodfetch_test_support::TestEnv;

    use super::{Kept, Redeclared};
    use crate::serving::Crawls;
    use crate::{code_free, hooks, labels};

    fn empty_kept() -> Kept {
        let labels = BTreeMap::new();
        let resolved = resolve(&Layers::default(), Product::Foxhound, Product::Foxhound).unwrap();
        let payloads = PayloadRules::of(&resolved, &labels).unwrap();
        (Datasets::default(), labels, payloads, None)
    }

    fn hook(name: &str) -> Vec<CrawlHookDeclaration> {
        vec![CrawlHookDeclaration {
            name: name.to_owned(),
            stage: CrawlHookStage::BeforeRecordCommit,
            datasets: None,
            labels: None,
        }]
    }

    fn hook_names(declared: &[CrawlHookDeclaration]) -> Vec<&str> {
        declared.iter().map(|h| h.name.as_str()).collect()
    }

    async fn open_state(env: &TestEnv, name: &str) -> (TaskOwner, RunState) {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let state = RunState::open(
            env.folder().join(format!("{name}.sqlite")),
            &RunStateOptions::default(),
            vec![
                goodfetch_dataset::TABLES,
                labels::TABLES,
                goodfetch_frontier::payload::TABLES,
                code_free::TABLES,
                hooks::TABLES,
            ],
            1,
            &owner,
        )
        .await
        .unwrap();
        (owner, state)
    }

    #[test]
    fn at_most_one_concurrent_claim_takes_a_starts_kept_declarations() {
        let env = TestEnv::new();
        goodfetch_test_support::runtime().block_on(async {
            let (_owner, state) = open_state(&env, "raced").await;
            let redeclared = Redeclared::default();
            let crawls = Crawls::default();
            let run_id = RunId::parse("raced").unwrap();

            assert!(
                redeclared
                    .keep(&run_id, empty_kept(), hook("stamp"), &crawls)
                    .await
            );
            let first = redeclared.claim(&run_id);
            let second = redeclared.claim(&run_id);

            assert!(first.is_some());
            assert!(second.is_none());

            first.unwrap().deliver();
            assert!(redeclared.claim(&run_id).is_none());
            let recorded = redeclared.record(&run_id, &state).await;
            assert_eq!(hook_names(&recorded), ["stamp"]);
            let again = redeclared.record(&run_id, &state).await;
            assert_eq!(hook_names(&again), Vec::<&str>::new());
        });
    }

    #[test]
    fn restoring_a_failed_reopen_preserves_declarations_without_clobbering_a_newer_start() {
        let env = TestEnv::new();
        goodfetch_test_support::runtime().block_on(async {
            let (_owner, state) = open_state(&env, "restored").await;
            let redeclared = Redeclared::default();
            let crawls = Crawls::default();
            let run_id = RunId::parse("restored").unwrap();

            redeclared
                .keep(&run_id, empty_kept(), hook("first"), &crawls)
                .await;
            let claim = redeclared.claim(&run_id).unwrap();
            claim.restore();

            // A subsequent retry can still claim the restored declarations.
            let again = redeclared.claim(&run_id).unwrap();
            // If a newer `crawl.start` arrived before a failed claim restored, the
            // newer start's declarations win.
            redeclared
                .keep(&run_id, empty_kept(), hook("second"), &crawls)
                .await;
            again.restore();

            redeclared.claim(&run_id).unwrap().deliver();
            let recorded = redeclared.record(&run_id, &state).await;
            assert_eq!(hook_names(&recorded), ["second"]);
        });
    }

    #[test]
    fn dropping_an_unconsumed_claim_restores_its_kept_declarations() {
        let env = TestEnv::new();
        goodfetch_test_support::runtime().block_on(async {
            let (_owner, state) = open_state(&env, "dropped-claim").await;
            let redeclared = Redeclared::default();
            let crawls = Crawls::default();
            let run_id = RunId::parse("dropped-claim").unwrap();

            redeclared
                .keep(&run_id, empty_kept(), hook("stamp"), &crawls)
                .await;
            let claim = redeclared.claim(&run_id).unwrap();
            drop(claim);

            redeclared.claim(&run_id).unwrap().deliver();
            let recorded = redeclared.record(&run_id, &state).await;
            assert_eq!(hook_names(&recorded), ["stamp"]);
        });
    }

    #[test]
    fn keep_for_resume_is_isolated_from_retry_claims_and_cleans_up_on_cancel_drop_or_end() {
        let env = TestEnv::new();
        goodfetch_test_support::runtime().block_on(async {
            let (_owner, state) = open_state(&env, "resumed").await;
            let redeclared = Redeclared::default();
            let crawls = Crawls::default();
            let run_id = RunId::parse("resumed").unwrap();

            let pending = redeclared
                .keep_for_resume(&run_id, empty_kept(), hook("cancelled"), &crawls)
                .await
                .unwrap();
            assert!(redeclared.claim(&run_id).is_none());
            pending.cancel();
            assert_eq!(
                hook_names(&redeclared.record(&run_id, &state).await),
                Vec::<&str>::new()
            );

            // Dropping an uncommitted delivery also cleans up `delivering`.
            let dropped = redeclared
                .keep_for_resume(&run_id, empty_kept(), hook("dropped"), &crawls)
                .await
                .unwrap();
            drop(dropped);
            assert_eq!(
                hook_names(&redeclared.record(&run_id, &state).await),
                Vec::<&str>::new()
            );

            // Moving to `kept` when a resume finds the Run already ended lets
            // a later retry claim the declarations without leaking `delivering`.
            let ended = redeclared
                .keep_for_resume(&run_id, empty_kept(), hook("ended"), &crawls)
                .await
                .unwrap();
            ended.move_to_kept();
            assert_eq!(
                hook_names(&redeclared.record(&run_id, &state).await),
                Vec::<&str>::new()
            );
            redeclared.claim(&run_id).unwrap().deliver();
            // A driver that fails before `record` returns the entry to `kept`.
            redeclared.restore_delivering(&run_id);
            assert_eq!(
                hook_names(&redeclared.record(&run_id, &state).await),
                Vec::<&str>::new()
            );
            redeclared.claim(&run_id).unwrap().deliver();
            let recorded = redeclared.record(&run_id, &state).await;
            assert_eq!(hook_names(&recorded), ["ended"]);
        });
    }
}
