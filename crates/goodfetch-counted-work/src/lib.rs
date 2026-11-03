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
    /// `C3.script_sends`: a send of `check.js`'s source for a guard's check.
    ActionScriptSends,
    /// `C3.check_commands`: a CDP command a guard's check sent.
    ActionCheckCommands,
    /// `C3.commands`, of a click: a CDP command a click's attempt sent.
    ActionClickCommands,
    /// `C3.commands`, of a fill: a CDP command a fill's attempt sent.
    ActionFillCommands,
    /// `C3.commands`, of a press: a CDP command a key press's attempt sent.
    ActionPressCommands,
    /// `F2.transactions`: a transaction on a run file: a commit its commit
    /// hook saw, and each job the writer folded into a commit's group, so the
    /// count does not depend on how commits group (ADR 0138's note).
    RunTransactions,
    /// `F2.writer_reads`: a read through a run file's writer.
    WriterReads,
    /// `F2.configuration_reads`: a read of the machine's settings files.
    ConfigurationReads,
    /// `F2.prunes`: a prune pass.
    Prunes,
    /// `F2.effect_syncs`: a synced commit of the Effect journal, which no code
    /// does until the journal lands (#1550).
    EffectSyncs,
    /// `E3.statements`: a SQL statement a Usage charge reads its Limits with.
    ChargeStatements,
    /// `E3.slice_transactions`: a transaction committed on the ledger's
    /// connections a charge goes through.
    ChargeTransactions,
    /// `E3.opens`: a connection opened to the ledger for a charge.
    ChargeOpens,
    /// `C4.detection_reads`: a detector's read of a navigation's document,
    /// or the Consent prompt runner's start in it.
    NavigationDetectionReads,
    /// `C4.compilations`: a glob or a regular expression compiled to judge a
    /// navigation.
    NavigationCompilations,
    /// `C4.script_compilations`: a script compiled for a navigation.
    NavigationScriptCompilations,
    /// `C4.provenance_entries`: a Provenance entry a navigation's judgement
    /// writes.
    NavigationProvenanceEntries,
    /// `E7.merge_round_trips`: a round trip to the Merge host for a
    /// Working-copy update.
    WorkingCopyMergeRoundTrips,
    /// `E7.document_loads`: a whole document loaded for a Working-copy update.
    WorkingCopyDocumentLoads,
    /// `E7.document_encodes`: a whole document encoded for a Working-copy
    /// update.
    WorkingCopyDocumentEncodes,
    /// `E7.opens`: a store opened for a Working-copy update.
    WorkingCopyOpens,
    /// `E7.prepares`: a statement prepared for a Working-copy update.
    WorkingCopyPrepares,
    /// `E7.transactions`: a transaction for a Working-copy update.
    WorkingCopyTransactions,
    /// `E7.syncs`: a sync for a Working-copy update.
    WorkingCopySyncs,
    /// `E7.encodes`: an encode of a Working-copy update for the Connections
    /// that follow it.
    WorkingCopyEncodes,
    /// `E2.configuration_reads`: a read of the machine's settings files for
    /// an Audit entry.
    AuditConfigurationReads,
    /// `E2.opens`: an open of the Audit log's store, with its schema set-up.
    AuditOpens,
    /// `E2.prunes`: a prune pass over the Audit log.
    AuditPrunes,
    /// `E2.transactions`: a transaction the Audit log's store committed.
    AuditTransactions,
    /// `E2.syncs`: a write of the Audit log's head file; the store's own
    /// syncs are seen by a counting VFS, which no production store has.
    AuditSyncs,
    /// `C6.file_opens`: a file opened or a folder created for a Live view
    /// frame.
    LiveViewFileOpens,
    /// `C6.compilations`: a selector, glob or regular expression compiled for
    /// a Live view frame.
    LiveViewCompilations,
    /// `C6.spawns`: a task spawned for a Live view frame.
    LiveViewSpawns,
    /// `C6.acknowledgements`: a Live view frame's acknowledgement command.
    LiveViewAcknowledgements,
    /// `C6.seals`: a seal of a fragment of a relayed viewer's frame.
    LiveViewSeals,
    /// `E5.file_opens`: a file opened or a folder created for a Run-trace
    /// entry.
    RunTraceFileOpens,
    /// `E5.spawns`: a task spawned for a Run-trace entry.
    RunTraceSpawns,
    /// `E5.formatting_off`: a Run-trace entry formatted though its kind is
    /// switched off.
    RunTraceFormattingOff,
    /// `E6.transactions`: a transaction on a Politeness file, as its commit
    /// hook sees it.
    PolitenessTransactions,
    /// `E6.opens`: a Politeness file opened. Its statement preparations are
    /// the benchmark's, read from Callgrind, as no wrapper sees them.
    PolitenessOpens,
    /// `E6.script_sends`: a script sent whole to Valkey or Redis, which no
    /// code does until a Politeness store lands.
    PolitenessScriptSends,
    /// `E6.robots_parses`: a robots.txt parsed for the copy a user's runs
    /// share.
    RobotsParses,
    /// `E6.robots_reads`: a read of that copy from the Politeness file.
    RobotsReads,
    /// `C2.commands`: a CDP command an engine read in a frame sends.
    PageReadCommands,
    /// `C2.world_creations`: a `Page.createIsolatedWorld` an engine read
    /// sends past its document's first. The driver keeps no registry of each
    /// frame's document yet, so every read's creation counts (X10).
    PageReadWorldCreations,
    /// `C2.document_worlds`: a `Page.createIsolatedWorld` an engine read
    /// sends, held per document and frame read.
    PageReadDocumentWorlds,
    /// `E4.configuration_reads`: a settings file read for a governor sample.
    GovernorConfigurationReads,
    /// `E4.meminfo_opens`: `/proc/meminfo` opened for a governor sample.
    GovernorMeminfoOpens,
    /// `E4.static_opens`: `self/cgroup` or `self/mountinfo` opened for a
    /// governor sample, which the engine's start could read once.
    GovernorStaticOpens,
    /// `E4.cpu_stat_opens`: the cgroup's `cpu.stat` opened for a governor
    /// sample.
    GovernorCpuStatOpens,
    /// `E4.process_reads`: one process's memory read for a governor sample.
    GovernorProcessReads,
    /// `E4.pool_walks`: a browser's processes walked for their memory as a
    /// page opens in the browser pool.
    PoolWalks,
    /// `C5.fetch_messages_no_rule`: a message of CDP's `Fetch` domain, a
    /// command sent or an event read, for a page's requests.
    InterceptedFetchMessages,
    /// `C5.spawns`: a task spawned for a refused request.
    InterceptedSpawns,
    /// `C5.file_opens`: a file opened or a folder created for a refused
    /// request, as the Run trace's request log does.
    InterceptedFileOpens,
    /// `C5.matches`: a paused request matched against the compiled Filter
    /// lists.
    InterceptedMatches,
    /// `C5.answers`: a CDP command that answers a paused request.
    InterceptedAnswers,
    /// `C5.compilations`: a glob or regular expression compiled for a paused
    /// request.
    InterceptedCompilations,
    /// `C5.entries`: an entry sent to the Run trace's request log.
    InterceptedEntries,
}

impl Row {
    /// Every row, in the order of [`CountedWork`]'s counts.
    pub const ALL: [Self; 97] = [
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
        Self::ActionScriptSends,
        Self::ActionCheckCommands,
        Self::ActionClickCommands,
        Self::ActionFillCommands,
        Self::ActionPressCommands,
        Self::RunTransactions,
        Self::WriterReads,
        Self::ConfigurationReads,
        Self::Prunes,
        Self::EffectSyncs,
        Self::ChargeStatements,
        Self::ChargeTransactions,
        Self::ChargeOpens,
        Self::NavigationDetectionReads,
        Self::NavigationCompilations,
        Self::NavigationScriptCompilations,
        Self::NavigationProvenanceEntries,
        Self::WorkingCopyMergeRoundTrips,
        Self::WorkingCopyDocumentLoads,
        Self::WorkingCopyDocumentEncodes,
        Self::WorkingCopyOpens,
        Self::WorkingCopyPrepares,
        Self::WorkingCopyTransactions,
        Self::WorkingCopySyncs,
        Self::WorkingCopyEncodes,
        Self::AuditConfigurationReads,
        Self::AuditOpens,
        Self::AuditPrunes,
        Self::AuditTransactions,
        Self::AuditSyncs,
        Self::LiveViewFileOpens,
        Self::LiveViewCompilations,
        Self::LiveViewSpawns,
        Self::LiveViewAcknowledgements,
        Self::LiveViewSeals,
        Self::RunTraceFileOpens,
        Self::RunTraceSpawns,
        Self::RunTraceFormattingOff,
        Self::PolitenessTransactions,
        Self::PolitenessOpens,
        Self::PolitenessScriptSends,
        Self::RobotsParses,
        Self::RobotsReads,
        Self::PageReadCommands,
        Self::PageReadWorldCreations,
        Self::PageReadDocumentWorlds,
        Self::GovernorConfigurationReads,
        Self::GovernorMeminfoOpens,
        Self::GovernorStaticOpens,
        Self::GovernorCpuStatOpens,
        Self::GovernorProcessReads,
        Self::PoolWalks,
        Self::InterceptedFetchMessages,
        Self::InterceptedSpawns,
        Self::InterceptedFileOpens,
        Self::InterceptedMatches,
        Self::InterceptedAnswers,
        Self::InterceptedCompilations,
        Self::InterceptedEntries,
    ];

    /// Its Shared path by name, the metric's `shared_path` label: a word, as every
    /// metric label's value is, so `page_analysis` for the register's F1 and
    /// `run_trace` for its E5.
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
            Self::ActionScriptSends
            | Self::ActionCheckCommands
            | Self::ActionClickCommands
            | Self::ActionFillCommands
            | Self::ActionPressCommands => "action",
            Self::RunTransactions
            | Self::WriterReads
            | Self::ConfigurationReads
            | Self::Prunes
            | Self::EffectSyncs => "run_state",
            Self::ChargeStatements | Self::ChargeTransactions | Self::ChargeOpens => "usage_charge",
            Self::NavigationDetectionReads
            | Self::NavigationCompilations
            | Self::NavigationScriptCompilations
            | Self::NavigationProvenanceEntries => "navigation",
            Self::WorkingCopyMergeRoundTrips
            | Self::WorkingCopyDocumentLoads
            | Self::WorkingCopyDocumentEncodes
            | Self::WorkingCopyOpens
            | Self::WorkingCopyPrepares
            | Self::WorkingCopyTransactions
            | Self::WorkingCopySyncs
            | Self::WorkingCopyEncodes => "working_copy_update",
            Self::AuditConfigurationReads
            | Self::AuditOpens
            | Self::AuditPrunes
            | Self::AuditTransactions
            | Self::AuditSyncs => "audit_entry",
            Self::LiveViewFileOpens
            | Self::LiveViewCompilations
            | Self::LiveViewSpawns
            | Self::LiveViewAcknowledgements
            | Self::LiveViewSeals => "live_view_frame",
            Self::RunTraceFileOpens | Self::RunTraceSpawns | Self::RunTraceFormattingOff => {
                "run_trace"
            }
            Self::PolitenessTransactions
            | Self::PolitenessOpens
            | Self::PolitenessScriptSends
            | Self::RobotsParses
            | Self::RobotsReads => "politeness_permit",
            Self::PageReadCommands
            | Self::PageReadWorldCreations
            | Self::PageReadDocumentWorlds => "page_read",
            Self::GovernorConfigurationReads
            | Self::GovernorMeminfoOpens
            | Self::GovernorStaticOpens
            | Self::GovernorCpuStatOpens
            | Self::GovernorProcessReads
            | Self::PoolWalks => "governor_sample",
            Self::InterceptedFetchMessages
            | Self::InterceptedSpawns
            | Self::InterceptedFileOpens
            | Self::InterceptedMatches
            | Self::InterceptedAnswers
            | Self::InterceptedCompilations
            | Self::InterceptedEntries => "intercepted_request",
        }
    }

    /// Its work, the metric's `work` label.
    #[must_use]
    pub const fn work(self) -> &'static str {
        match self {
            Self::PageParses => "parses",
            Self::PageSelectorCompilations => "selector_compilations",
            Self::PageReadabilityRuns => "readability_runs",
            Self::PageFileOpens
            | Self::CdpFileOpens
            | Self::LiveViewFileOpens
            | Self::RunTraceFileOpens
            | Self::InterceptedFileOpens => "file_opens",
            Self::PageLocaleFormBuilds => "locale_form_builds",
            Self::PagePluginSerializations => "plugin_serializations",
            Self::PagePluginAnswerParses => "plugin_answer_parses",
            Self::MessageEncodes | Self::CdpEncodes | Self::WorkingCopyEncodes => "encodes",
            Self::CdpSends => "sends",
            Self::CdpLogSendsOff => "log_sends_off",
            Self::InterceptedFetchMessages => "fetch_messages_no_rule",
            Self::InterceptedMatches => "matches",
            Self::InterceptedAnswers => "answers",
            Self::InterceptedEntries => "entries",
            Self::ActionCheckCommands => "check_commands",
            Self::ActionClickCommands => "click_commands",
            Self::ActionFillCommands => "fill_commands",
            Self::ActionPressCommands => "press_commands",
            Self::HookMessagesNoHook => "messages_no_hook",
            Self::HookCalls => "calls",
            Self::HookBuilds => "builds",
            Self::SinkOpens
            | Self::ChargeOpens
            | Self::WorkingCopyOpens
            | Self::AuditOpens
            | Self::PolitenessOpens => "opens",
            Self::ChargeStatements => "statements",
            Self::SinkWriterReads | Self::WriterReads => "writer_reads",
            Self::ConfigurationReads
            | Self::AuditConfigurationReads
            | Self::GovernorConfigurationReads => "configuration_reads",
            Self::Prunes | Self::AuditPrunes => "prunes",
            Self::EffectSyncs => "effect_syncs",
            Self::PolitenessScriptSends | Self::ActionScriptSends => "script_sends",
            Self::RobotsParses => "robots_parses",
            Self::RobotsReads => "robots_reads",
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
            | Self::SitemapTransactions
            | Self::RunTransactions
            | Self::ChargeTransactions
            | Self::WorkingCopyTransactions
            | Self::AuditTransactions
            | Self::PolitenessTransactions => "transactions",
            Self::BatchInvariantReads | Self::HopInvariantReads | Self::SitemapInvariantReads => {
                "invariant_reads"
            }
            Self::BatchCompilations
            | Self::HopCompilations
            | Self::SitemapCompilations
            | Self::PayloadCompilations
            | Self::NavigationCompilations
            | Self::LiveViewCompilations
            | Self::InterceptedCompilations => "compilations",
            Self::NavigationDetectionReads => "detection_reads",
            Self::NavigationScriptCompilations => "script_compilations",
            Self::NavigationProvenanceEntries => "provenance_entries",
            Self::WorkingCopyMergeRoundTrips => "merge_round_trips",
            Self::WorkingCopyDocumentLoads => "document_loads",
            Self::WorkingCopyDocumentEncodes => "document_encodes",
            Self::WorkingCopyPrepares => "prepares",
            Self::WorkingCopySyncs | Self::AuditSyncs => "syncs",
            Self::LiveViewSpawns | Self::RunTraceSpawns | Self::InterceptedSpawns => "spawns",
            Self::LiveViewAcknowledgements => "acknowledgements",
            Self::LiveViewSeals => "seals",
            Self::RunTraceFormattingOff => "formatting_off",
            Self::PageReadCommands => "commands",
            Self::PageReadWorldCreations => "world_creations",
            Self::PageReadDocumentWorlds => "document_worlds",
            Self::GovernorMeminfoOpens => "meminfo_opens",
            Self::GovernorStaticOpens => "static_opens",
            Self::GovernorCpuStatOpens => "cpu_stat_opens",
            Self::GovernorProcessReads => "process_reads",
            Self::PoolWalks => "pool_walks",
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
            Self::ActionScriptSends => "C3.script_sends",
            Self::ActionCheckCommands => "C3.check_commands",
            Self::ActionClickCommands => "C3.commands.click",
            Self::ActionFillCommands => "C3.commands.fill",
            Self::ActionPressCommands => "C3.commands.press",
            Self::RunTransactions => "F2.transactions",
            Self::WriterReads => "F2.writer_reads",
            Self::ConfigurationReads => "F2.configuration_reads",
            Self::Prunes => "F2.prunes",
            Self::EffectSyncs => "F2.effect_syncs",
            Self::ChargeStatements => "E3.statements",
            Self::ChargeTransactions => "E3.slice_transactions",
            Self::ChargeOpens => "E3.opens",
            Self::NavigationDetectionReads => "C4.detection_reads",
            Self::NavigationCompilations => "C4.compilations",
            Self::NavigationScriptCompilations => "C4.script_compilations",
            Self::NavigationProvenanceEntries => "C4.provenance_entries",
            Self::WorkingCopyMergeRoundTrips => "E7.merge_round_trips",
            Self::WorkingCopyDocumentLoads => "E7.document_loads",
            Self::WorkingCopyDocumentEncodes => "E7.document_encodes",
            Self::WorkingCopyOpens => "E7.opens",
            Self::WorkingCopyPrepares => "E7.prepares",
            Self::WorkingCopyTransactions => "E7.transactions",
            Self::WorkingCopySyncs => "E7.syncs",
            Self::WorkingCopyEncodes => "E7.encodes",
            Self::AuditConfigurationReads => "E2.configuration_reads",
            Self::AuditOpens => "E2.opens",
            Self::AuditPrunes => "E2.prunes",
            Self::AuditTransactions => "E2.transactions",
            Self::AuditSyncs => "E2.syncs",
            Self::LiveViewFileOpens => "C6.file_opens",
            Self::LiveViewCompilations => "C6.compilations",
            Self::LiveViewSpawns => "C6.spawns",
            Self::LiveViewAcknowledgements => "C6.acknowledgements",
            Self::LiveViewSeals => "C6.seals",
            Self::RunTraceFileOpens => "E5.file_opens",
            Self::RunTraceSpawns => "E5.spawns",
            Self::RunTraceFormattingOff => "E5.formatting_off",
            Self::PolitenessTransactions => "E6.transactions",
            Self::PolitenessOpens => "E6.opens",
            Self::PolitenessScriptSends => "E6.script_sends",
            Self::RobotsParses => "E6.robots_parses",
            Self::RobotsReads => "E6.robots_reads",
            Self::PageReadCommands => "C2.commands",
            Self::PageReadWorldCreations => "C2.world_creations",
            Self::PageReadDocumentWorlds => "C2.document_worlds",
            Self::GovernorConfigurationReads => "E4.configuration_reads",
            Self::GovernorMeminfoOpens => "E4.meminfo_opens",
            Self::GovernorStaticOpens => "E4.static_opens",
            Self::GovernorCpuStatOpens => "E4.cpu_stat_opens",
            Self::GovernorProcessReads => "E4.process_reads",
            Self::PoolWalks => "E4.pool_walks",
            Self::InterceptedFetchMessages => "C5.fetch_messages_no_rule",
            Self::InterceptedSpawns => "C5.spawns",
            Self::InterceptedFileOpens => "C5.file_opens",
            Self::InterceptedMatches => "C5.matches",
            Self::InterceptedAnswers => "C5.answers",
            Self::InterceptedCompilations => "C5.compilations",
            Self::InterceptedEntries => "C5.entries",
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

    /// Counts `units` units of `row`'s work.
    pub fn count_many(&self, row: Row, units: u64) {
        self.0.0[row.index()].add(units);
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
