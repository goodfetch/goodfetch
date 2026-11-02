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

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
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
    /// `F1.selector_compilations`: a selector, glob or regular expression
    /// compiled for a fetched page.
    PageSelectorCompilations,
    /// `F1.readability_runs`: a run of the main-content step on a fetched page.
    PageReadabilityRuns,
    /// `F1.file_opens`: a file opened or a folder created for a fetched page.
    PageFileOpens,
    /// `F1.locale_form_builds`: a Value transform's locale forms built.
    PageLocaleFormBuilds,
    /// `F1.plugin_serializations`: a page serialized for an Extractor Plugin.
    PagePluginSerializations,
    /// `F1.plugin_answer_parses`: a parse of an Extractor Plugin's answer.
    PagePluginAnswerParses,
    /// `E1.deep_copies`: a whole copy of what a Client's message carries.
    MessageDeepCopies,
    /// `E1.encodes`: an encode of what the engine sends, into bytes or into
    /// a `Value` that is encoded again.
    MessageEncodes,
    /// `F5.messages_no_hook`: a message or its formatting for a stage with no
    /// Hook on it.
    HookMessagesNoHook,
    /// `F5.calls`: a request out to a Hook called.
    HookCalls,
    /// `F5.builds`: a Hook chain's build or compilation at a stage.
    HookBuilds,
    /// `F6.opens`: a store open or statement preparation for a delivered batch.
    SinkOpens,
    /// `F6.transactions`: a transaction for a delivered batch.
    SinkTransactions,
    /// `F6.writer_reads`: a read through the run file's writer for a delivered
    /// batch.
    SinkWriterReads,
    /// `F4.declaration_reads`: a read of the Datasets' declarations to check a Record.
    RecordDeclarationReads,
    /// `F4.validator_builds`: a validator built to check a Record.
    RecordValidatorBuilds,
    /// `F4.validations`: a validation of a Record against its Dataset's schema.
    RecordValidations,
    /// `F4.serializations`: a Record's data written as the JSON text it is kept as.
    RecordSerializations,
    /// `F4.deep_copies`: a deep copy of a Record's data.
    RecordDeepCopies,
    /// `F4.constant_lookups`: a lookup of a value that stays constant for the Run, for a Record.
    RecordConstantLookups,
    /// `F4.transactions`: a run-file transaction of a Record's own, apart from its attempt's commit.
    RecordTransactions,
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
    pub const ALL: [Self; 38] = [
        Self::PageParses,
        Self::PageSelectorCompilations,
        Self::PageReadabilityRuns,
        Self::PageFileOpens,
        Self::PageLocaleFormBuilds,
        Self::PagePluginSerializations,
        Self::PagePluginAnswerParses,
        Self::MessageDeepCopies,
        Self::MessageEncodes,
        Self::HookMessagesNoHook,
        Self::HookCalls,
        Self::HookBuilds,
        Self::SinkOpens,
        Self::SinkTransactions,
        Self::SinkWriterReads,
        Self::RecordDeclarationReads,
        Self::RecordValidatorBuilds,
        Self::RecordValidations,
        Self::RecordSerializations,
        Self::RecordDeepCopies,
        Self::RecordConstantLookups,
        Self::RecordTransactions,
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
            Self::PageParses
            | Self::PageSelectorCompilations
            | Self::PageReadabilityRuns
            | Self::PageFileOpens
            | Self::PageLocaleFormBuilds
            | Self::PagePluginSerializations
            | Self::PagePluginAnswerParses => "page_analysis",
            Self::MessageDeepCopies | Self::MessageEncodes => "json_rpc_message",
            Self::HookMessagesNoHook | Self::HookCalls | Self::HookBuilds => "hook_stage",
            Self::SinkOpens | Self::SinkTransactions | Self::SinkWriterReads => "sink_delivery",
            Self::RecordDeclarationReads
            | Self::RecordValidatorBuilds
            | Self::RecordValidations
            | Self::RecordSerializations
            | Self::RecordDeepCopies
            | Self::RecordConstantLookups
            | Self::RecordTransactions => "record",
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
            Self::PageSelectorCompilations => "selector_compilations",
            Self::PageReadabilityRuns => "readability_runs",
            Self::PageFileOpens | Self::CdpFileOpens => "file_opens",
            Self::PageLocaleFormBuilds => "locale_form_builds",
            Self::PagePluginSerializations => "plugin_serializations",
            Self::PagePluginAnswerParses => "plugin_answer_parses",
            Self::MessageEncodes | Self::CdpEncodes => "encodes",
            Self::CdpSends => "sends",
            Self::CdpLogSendsOff => "log_sends_off",
            Self::HookMessagesNoHook => "messages_no_hook",
            Self::HookCalls => "calls",
            Self::HookBuilds => "builds",
            Self::SinkOpens => "opens",
            Self::SinkWriterReads => "writer_reads",
            Self::RecordDeclarationReads => "declaration_reads",
            Self::RecordValidatorBuilds => "validator_builds",
            Self::RecordValidations | Self::PayloadValidations => "validations",
            Self::RecordSerializations | Self::PayloadSerializations => "serializations",
            Self::MessageDeepCopies | Self::RecordDeepCopies => "deep_copies",
            Self::RecordConstantLookups => "constant_lookups",
            Self::SinkTransactions
            | Self::RecordTransactions
            | Self::BatchTransactions
            | Self::HopTransactions
            | Self::SitemapTransactions => "transactions",
            Self::BatchInvariantReads | Self::HopInvariantReads | Self::SitemapInvariantReads => {
                "invariant_reads"
            }
            Self::BatchCompilations
            | Self::HopCompilations
            | Self::SitemapCompilations
            | Self::PayloadCompilations => "compilations",
        }
    }

    /// Its name in what a benchmark prints: the Quality register's row, and
    /// for a hop's, a sitemap's or a Payload's own count of a row that has
    /// several outcomes, the row with its outcome (`F3.compilations.hop`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::PageParses => "F1.parses",
            Self::PageSelectorCompilations => "F1.selector_compilations",
            Self::PageReadabilityRuns => "F1.readability_runs",
            Self::PageFileOpens => "F1.file_opens",
            Self::PageLocaleFormBuilds => "F1.locale_form_builds",
            Self::PagePluginSerializations => "F1.plugin_serializations",
            Self::PagePluginAnswerParses => "F1.plugin_answer_parses",
            Self::MessageDeepCopies => "E1.deep_copies",
            Self::MessageEncodes => "E1.encodes",
            Self::HookMessagesNoHook => "F5.messages_no_hook",
            Self::HookCalls => "F5.calls",
            Self::HookBuilds => "F5.builds",
            Self::SinkOpens => "F6.opens",
            Self::SinkTransactions => "F6.transactions",
            Self::SinkWriterReads => "F6.writer_reads",
            Self::RecordDeclarationReads => "F4.declaration_reads",
            Self::RecordValidatorBuilds => "F4.validator_builds",
            Self::RecordValidations => "F4.validations",
            Self::RecordSerializations => "F4.serializations",
            Self::RecordDeepCopies => "F4.deep_copies",
            Self::RecordConstantLookups => "F4.constant_lookups",
            Self::RecordTransactions => "F4.transactions",
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

#[derive(Debug)]
struct Counts([Count; Row::ALL.len()]);

// By hand: the standard library derives `Default` for arrays of at most 32.
impl Default for Counts {
    fn default() -> Self {
        Self(std::array::from_fn(|_| Count::default()))
    }
}

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

    /// `selector` compiled, counted on `row`, none when it is no selector: the
    /// one function a CSS selector compiles through (ADR 0140), which a
    /// tree reader then matches with `select_matcher`.
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "the one counted function of a selector's compilation (ADR 0140)"
    )]
    pub fn compile_selector(&self, row: Row, selector: &str) -> Option<dom_query::Matcher> {
        self.count(row);
        dom_query::Matcher::new(selector).ok()
    }

    /// [`Self::compile_selector`] of a selector written in the code.
    ///
    /// # Panics
    ///
    /// When `selector` is no selector, which a test of its code shows.
    #[must_use]
    pub fn literal_selector(&self, row: Row, selector: &'static str) -> dom_query::Matcher {
        self.compile_selector(row, selector)
            .expect("invariant: a selector written in the code is CSS")
    }

    /// `pattern` compiled, counted on `row`: the one function a regular
    /// expression compiles through (ADR 0140).
    ///
    /// # Errors
    ///
    /// When `pattern` is no regular expression.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one counted function of a regular expression's compilation (ADR 0140)"
    )]
    pub fn compile_regex(&self, row: Row, pattern: &str) -> Result<regex::Regex, regex::Error> {
        self.count(row);
        regex::Regex::new(pattern)
    }

    /// `pattern` compiled, counted on `row`: the one function a glob compiles
    /// through (ADR 0140).
    ///
    /// # Errors
    ///
    /// When `pattern` is no glob.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one counted function of a glob's compilation (ADR 0140)"
    )]
    pub fn compile_glob(
        &self,
        row: Row,
        pattern: &str,
    ) -> Result<glob::Pattern, glob::PatternError> {
        self.count(row);
        glob::Pattern::new(pattern)
    }

    /// `path` opened as `options` say, counted on `row`: the one function a
    /// file opens through (ADR 0140).
    ///
    /// # Errors
    ///
    /// When the file cannot be opened.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one counted function of a file's opening (ADR 0140); its caller runs it under its blocking policy"
    )]
    pub fn open_file(&self, row: Row, options: &OpenOptions, path: &Path) -> io::Result<File> {
        self.count(row);
        options.open(path)
    }

    /// The folder `path` and those above it created, counted on `row`: the
    /// one function a folder is created through (ADR 0140).
    ///
    /// # Errors
    ///
    /// When a folder cannot be created.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one counted function of a folder's creation (ADR 0140); its caller runs it under its blocking policy"
    )]
    pub fn create_dir_all(&self, row: Row, path: &Path) -> io::Result<()> {
        self.count(row);
        std::fs::create_dir_all(path)
    }
}

#[cfg(test)]
mod tests;
