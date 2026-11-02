//! Counted work (ADR 0140): work whose amount repeats exactly from run to
//! run, counted at the primitive that does it, beneath every owner module,
//! so that work added beside an owner is counted too.
//!
//! [`CountedWork`] is a value its owner is given, as owners receive their
//! other dependencies, never a global, so one process's Simulation hosts stay
//! apart. A count is one relaxed atomic increment, which is no counted work
//! itself and takes no lock, so it may sit inside the run-state writer's
//! closure (`STYLE.md`). No behaviour reads a count: the ops listener's
//! `goodfetch.counted_work` and `xtask counts` do.
//!
//! This crate depends on no Goodfetch crate, so a crate beneath the engine
//! takes it too. The labels of [`Row`] are `goodfetch-protocol`'s metric
//! declaration's values, which the engine's metric tests compare.

mod sync;

use std::sync::Arc;

use sync::Count;

/// A tree of an HTML document, parsed by [`CountedWork::parse_html`]: its
/// name for a signature, as `dom_query::Document` is refused outside this
/// crate (`clippy.toml`, ADR 0140).
#[expect(
    clippy::disallowed_types,
    reason = "the one name other crates' signatures hold a parsed tree under (ADR 0140)"
)]
pub type ParsedHtml = dom_query::Document;

/// A row of the Quality register that a counted function counts: one work of
/// one Shared path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Row {
    /// `F1.parses`: an HTML parse of a fetched page.
    PageParses,
    /// `F3.invariant_reads`: a read of the Scope, Payload rules or Fragments
    /// from the run file, admitting a links batch.
    BatchInvariantReads,
    /// `F3.compilations`: a glob compiled to admit a links batch.
    BatchCompilations,
    /// `F3.transactions`: a write of its own a links batch asks the writer for.
    BatchTransactions,
    /// `F3.invariant_reads`, of a redirect hop.
    HopInvariantReads,
    /// `F3.compilations`, of a redirect hop.
    HopCompilations,
    /// `F3.transactions`, of a redirect hop.
    HopTransactions,
    /// `F3.invariant_reads`, of a sitemap's read.
    SitemapInvariantReads,
    /// `F3.compilations`, of a sitemap's read.
    SitemapCompilations,
    /// `F3.transactions`, of a sitemap's read.
    SitemapTransactions,
    /// `F3.compilations`: a validator built for one URL's Payload.
    PayloadCompilations,
    /// `F3.payload_serializations`: one URL's Payload serialized.
    PayloadSerializations,
    /// `F3.validations`: one URL's Payload validated against its Label's schema.
    PayloadValidations,
}

impl Row {
    /// Every row, in the order of [`CountedWork`]'s counts.
    pub const ALL: [Self; 13] = [
        Self::PageParses,
        Self::BatchInvariantReads,
        Self::BatchCompilations,
        Self::BatchTransactions,
        Self::HopInvariantReads,
        Self::HopCompilations,
        Self::HopTransactions,
        Self::SitemapInvariantReads,
        Self::SitemapCompilations,
        Self::SitemapTransactions,
        Self::PayloadCompilations,
        Self::PayloadSerializations,
        Self::PayloadValidations,
    ];

    /// Its Shared path by name, the metric's `shared_path` label: a word, as every
    /// metric label's value is, so `page_analysis` for the register's F1.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::PageParses => "page_analysis",
            Self::BatchInvariantReads | Self::BatchCompilations | Self::BatchTransactions => {
                "frontier_admission"
            }
            Self::HopInvariantReads | Self::HopCompilations | Self::HopTransactions => {
                "frontier_hop"
            }
            Self::SitemapInvariantReads | Self::SitemapCompilations | Self::SitemapTransactions => {
                "frontier_sitemap"
            }
            Self::PayloadCompilations | Self::PayloadSerializations | Self::PayloadValidations => {
                "frontier_payload"
            }
        }
    }

    /// Its work, the metric's `work` label.
    #[must_use]
    pub const fn work(self) -> &'static str {
        match self {
            Self::PageParses => "parses",
            Self::BatchInvariantReads | Self::HopInvariantReads | Self::SitemapInvariantReads => {
                "invariant_reads"
            }
            Self::BatchCompilations
            | Self::HopCompilations
            | Self::SitemapCompilations
            | Self::PayloadCompilations => "compilations",
            Self::BatchTransactions | Self::HopTransactions | Self::SitemapTransactions => {
                "transactions"
            }
            Self::PayloadSerializations => "serializations",
            Self::PayloadValidations => "validations",
        }
    }

    /// Its name in what a benchmark prints: the Quality register's row, and
    /// for a hop's, a sitemap's or a Payload's own count of a row that has
    /// several outcomes, the row with its outcome (`F3.compilations.hop`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PageParses => "F1.parses",
            Self::BatchInvariantReads => "F3.invariant_reads",
            Self::BatchCompilations => "F3.compilations",
            Self::BatchTransactions => "F3.transactions",
            Self::HopInvariantReads => "F3.invariant_reads.hop",
            Self::HopCompilations => "F3.compilations.hop",
            Self::HopTransactions => "F3.transactions.hop",
            Self::SitemapInvariantReads => "F3.invariant_reads.sitemap",
            Self::SitemapCompilations => "F3.compilations.sitemap",
            Self::SitemapTransactions => "F3.transactions.sitemap",
            Self::PayloadCompilations => "F3.compilations.payload",
            Self::PayloadSerializations => "F3.payload_serializations",
            Self::PayloadValidations => "F3.validations",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// The counts of one engine's Counted work, shared by every clone. A thread
/// may count while another reads: a read sees some count that was true.
#[derive(Clone, Debug, Default)]
pub struct CountedWork(Arc<Counts>);

#[derive(Debug, Default)]
struct Counts([Count; Row::ALL.len()]);

impl CountedWork {
    /// Counts one unit of `row`'s work.
    pub fn count(&self, row: Row) {
        self.0.0[row.index()].add_one();
    }

    /// The units of `row`'s work counted so far.
    #[must_use]
    pub fn read(&self, row: Row) -> u64 {
        self.0.0[row.index()].read()
    }

    /// `html` parsed into a tree, counted on `row`: the one function an HTML
    /// parse goes through (ADR 0140).
    #[must_use]
    #[expect(
        clippy::disallowed_types,
        reason = "the one counted function of an HTML parse (ADR 0140)"
    )]
    pub fn parse_html(&self, row: Row, html: &str) -> ParsedHtml {
        self.count(row);
        dom_query::Document::from(html)
    }
}

#[cfg(test)]
mod tests;
