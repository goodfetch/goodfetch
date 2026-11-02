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
    /// `C1.encodes`: a CDP command written into the bytes its pipe carries.
    CdpEncodes,
    /// `C1.sends`: a CDP command's bytes written to its pipe.
    CdpSends,
    /// `C1.file_opens`: a directory created or a file opened for a CDP
    /// command, as the CDP command log's writer does.
    CdpFileOpens,
    /// `C1.log_sends_off`: a CDP command's method sent to the CDP command
    /// log's writer. The log has no off yet, so every send counts (X9).
    CdpLogSendsOff,
}

impl Row {
    /// Every row, in the order of [`CountedWork`]'s counts.
    pub const ALL: [Self; 5] = [
        Self::PageParses,
        Self::CdpEncodes,
        Self::CdpSends,
        Self::CdpFileOpens,
        Self::CdpLogSendsOff,
    ];

    /// Its Shared path by name, the metric's `shared_path` label: a word, as every
    /// metric label's value is, so `page_analysis` for the register's F1.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::PageParses => "page_analysis",
            Self::CdpEncodes | Self::CdpSends | Self::CdpFileOpens | Self::CdpLogSendsOff => {
                "cdp_command"
            }
        }
    }

    /// Its work, the metric's `work` label.
    #[must_use]
    pub const fn work(self) -> &'static str {
        match self {
            Self::PageParses => "parses",
            Self::CdpEncodes => "encodes",
            Self::CdpSends => "sends",
            Self::CdpFileOpens => "file_opens",
            Self::CdpLogSendsOff => "log_sends_off",
        }
    }

    /// Its name in the Quality register.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PageParses => "F1.parses",
            Self::CdpEncodes => "C1.encodes",
            Self::CdpSends => "C1.sends",
            Self::CdpFileOpens => "C1.file_opens",
            Self::CdpLogSendsOff => "C1.log_sends_off",
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
