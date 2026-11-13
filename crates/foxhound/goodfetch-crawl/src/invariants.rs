//! A crawl's invariants (#1896): the Frontier's, its Scope, Bounds and
//! sitemap rules, with the Payload rules and which Labels keep a URL's
//! fragment, built once as the Run starts or resumes and handed to every
//! admission, a seed added to the running crawl, a handler's enqueue, a
//! links batch, a redirect hop and a sitemap's read, so none reads them
//! from the run file again.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_engine::CallError;
use goodfetch_engine::settings::Resolved;
use goodfetch_frontier::payload::PayloadRules;
use goodfetch_frontier::{Bounds, Scope};
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
        Ok(Self::new(
            frontier,
            PayloadRules::of(settings, labels)?,
            fragments,
        ))
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
