//! A crawl's invariants (#1896): the Frontier's, its Scope, Bounds and
//! sitemap rules, with the Payload rules and which Labels keep a URL's
//! fragment, built once as the Run starts or resumes and handed to every
//! admission, a seed added to the running crawl, a handler's enqueue, a
//! links batch, a redirect hop and a sitemap's read, so none reads them
//! from the run file again.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::run_state::RunState;
use goodfetch_engine::settings::Resolved;
use goodfetch_frontier::payload::PayloadRules;
use goodfetch_frontier::{Bounds, Scope};
use goodfetch_protocol::foxhound::continuous::{
    CONTINUOUS_DEFAULT_MERGED_KEPT, CRAWL_CONTINUOUS, CRAWL_CONTINUOUS_FORGET_AFTER,
    CRAWL_CONTINUOUS_MERGED_KEPT,
};
use goodfetch_protocol::foxhound::crawl::LabelDeclaration;
use rusqlite::Connection;

use crate::labels::Fragments;

#[cfg(test)]
pub(crate) mod fixture;

/// What every admission of a crawl reads, built once.
#[derive(Clone, Debug)]
pub(crate) struct Invariants {
    frontier: Arc<goodfetch_frontier::Invariants>,
    payloads: Arc<PayloadRules>,
    fragments: Arc<Fragments>,
}

impl Invariants {
    /// The invariants of the Frontier's `frontier`, the Payload rules
    /// `payloads` and the Labels' `fragments`.
    pub(crate) fn new(
        frontier: goodfetch_frontier::Invariants,
        payloads: PayloadRules,
        fragments: Fragments,
    ) -> Self {
        Self {
            frontier: Arc::new(frontier),
            payloads: Arc::new(payloads),
            fragments: Arc::new(fragments),
        }
    }

    /// The invariants of a crawl started under `settings` with its
    /// declared `labels` and their `fragments`.
    ///
    /// # Errors
    ///
    /// `pattern_unsupported` for a Scope's or a path budget's pattern the
    /// matcher cannot read, and the Payload rules' errors for a Label's
    /// schema.
    pub(crate) fn of_settings(
        settings: &Resolved,
        labels: &BTreeMap<String, LabelDeclaration>,
        fragments: Fragments,
    ) -> Result<Self, CallError> {
        let frontier = goodfetch_frontier::Invariants::new(
            Scope::of_settings(settings)?,
            Bounds::of_settings(settings),
        )?;
        let invariants = Self::new(frontier, PayloadRules::of(settings, labels)?, fragments);
        invariants.configure_continuous(settings);
        Ok(invariants)
    }

    /// Applies Continuous-crawl growth settings (`crawl.continuous.merged_kept`
    /// and `crawl.continuous.forget_after`, ADR 0139) to the Frontier's
    /// invariants when `crawl.continuous.enabled` is on.
    pub(crate) fn configure_continuous(&self, settings: &Resolved) {
        if !settings.get::<bool>(&CRAWL_CONTINUOUS) {
            return;
        }
        let merged_kept = settings
            .value(&CRAWL_CONTINUOUS_MERGED_KEPT)
            .as_u64()
            .unwrap_or(CONTINUOUS_DEFAULT_MERGED_KEPT);
        self.frontier
            .cap_merged(usize::try_from(merged_kept).unwrap_or(usize::MAX));
        if let Some(forget_after) = settings.get::<Option<Duration>>(&CRAWL_CONTINUOUS_FORGET_AFTER)
        {
            let ms = u64::try_from(forget_after.as_millis()).unwrap_or(u64::MAX);
            self.frontier.set_forget_after_ms(ms);
        }
    }

    /// The invariants the run file `connection` holds, as the Run resumes
    /// with its `fragments` read.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub(crate) fn read(connection: &Connection, fragments: Fragments) -> rusqlite::Result<Self> {
        Ok(Self::new(
            goodfetch_frontier::Invariants::read(connection)?,
            PayloadRules::read(connection)?,
            fragments,
        ))
    }

    /// Prepares the statements an admission runs on each connection of
    /// `run`'s run-state writer and of its readers, once, as its crawl
    /// starts or resumes, so that no page's admission, redirect hop (which
    /// a reader judges at `batch`) or sitemap read prepares one
    /// (ADR 0140, `F3.prepares`), with `beside`, the work items' own
    /// statements a sitemap read shares with a page's end. One that is not
    /// prepared here is at its first use.
    pub(crate) async fn prepare(
        &self,
        (run, state): (&RunIdentity, &RunState),
        beside: &[&'static str],
    ) {
        let mut statements = goodfetch_frontier::admission_statements(&self.frontier);
        statements.extend_from_slice(beside);
        if let Err(error) = state.prepare(statements).await {
            tracing::warn!(%error, run_id = %run.id(), "the crawl's admission statements were not prepared as it started");
        }
    }

    /// The Frontier's invariants.
    pub(crate) fn frontier(&self) -> &goodfetch_frontier::Invariants {
        &self.frontier
    }

    /// The Frontier's invariants, shared.
    pub(crate) fn shared_frontier(&self) -> Arc<goodfetch_frontier::Invariants> {
        Arc::clone(&self.frontier)
    }

    /// The Payload rules.
    pub(crate) fn payloads(&self) -> &PayloadRules {
        &self.payloads
    }

    /// Which Labels keep a URL's fragment.
    pub(crate) fn fragments(&self) -> &Fragments {
        &self.fragments
    }

    /// Records, in the new Run's run file `connection`, its Scope, Bounds,
    /// Payload rules and which Labels keep a URL's fragment, which a resume
    /// reads them from.
    ///
    /// # Errors
    ///
    /// When the run file cannot be written.
    pub(crate) fn record(&self, connection: &Connection) -> rusqlite::Result<()> {
        self.frontier.scope().record(connection)?;
        self.frontier.bounds().record(connection)?;
        self.payloads.record(connection)?;
        self.fragments.record(connection)
    }

    /// Whether a request of `label` keeps its URL's fragment.
    pub(crate) fn keeps_fragment(&self, label: &str) -> bool {
        self.fragments.keeps(label)
    }
}
