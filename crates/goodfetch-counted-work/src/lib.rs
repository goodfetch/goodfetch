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
//! declaration's values, which `xtask codegen` writes from this crate's
//! table of rows.

mod compiled;
mod sync;

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::Arc;

use compiled::Compiled;
use sync::Count;

/// A tree of an HTML document, parsed by [`CountedWork::parse_html`]: its
/// name for a signature, as `dom_query::Document` is refused outside this
/// crate (`clippy.toml`, ADR 0140).
#[expect(
    clippy::disallowed_types,
    reason = "the one name other crates' signatures hold a parsed tree under (ADR 0140)"
)]
pub type ParsedHtml = dom_query::Document;

/// Declares [`Row`] from one table, a line per row: its variant, its name in
/// the Quality register, its Shared path and its work. Every list of rows
/// (`ALL`, `name`, `path`, `work`) comes from that one line, and `xtask
/// codegen` reads the table's lines, `Variant => "name", "path", "work";`,
/// to write `goodfetch.counted_work`'s label values into
/// `goodfetch-protocol`.
macro_rules! rows {
    ($($(#[doc = $doc:literal])* $row:ident => $name:literal, $path:literal, $work:literal;)*) => {
        /// A row of the Quality register that a counted function counts: one work of
        /// one Shared path.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum Row {
            $($(#[doc = $doc])* $row,)*
        }

        impl Row {
            /// Every row, in the order of [`CountedWork`]'s counts.
            pub const ALL: [Self; [$(stringify!($row)),*].len()] = [$(Self::$row),*];

            /// Its Shared path by name, the metric's `shared_path` label: a word, as every
            /// metric label's value is, so `page_analysis` for the register's F1 and
            /// `run_trace` for its E5.
            #[must_use]
            pub const fn path(self) -> &'static str {
                match self {
                    $(Self::$row => $path,)*
                }
            }

            /// Its work, the metric's `work` label.
            #[must_use]
            pub const fn work(self) -> &'static str {
                match self {
                    $(Self::$row => $work,)*
                }
            }

            /// Its name in what a benchmark prints: the Quality register's row, and
            /// for a hop's, a sitemap's or a Payload's own count of a row that has
            /// several outcomes, the row with its outcome (`F3.compilations.hop`).
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$row => $name,)*
                }
            }

            const fn index(self) -> usize {
                self as usize
            }
        }
    };
}

rows! {
    /// `F1.parses`: an HTML parse of a fetched page.
    PageParses => "F1.parses", "page_analysis", "parses";
    /// `F1.selector_compilations`: a selector, glob or regular expression
    /// compiled for a fetched page.
    PageSelectorCompilations => "F1.selector_compilations", "page_analysis", "selector_compilations";
    /// `F1.readability_runs`: a run of the main-content step on a fetched page.
    PageReadabilityRuns => "F1.readability_runs", "page_analysis", "readability_runs";
    /// `F1.file_opens`: a file opened or a folder created for a fetched page.
    PageFileOpens => "F1.file_opens", "page_analysis", "file_opens";
    /// `F1.locale_form_builds`: a Value transform's locale forms built.
    PageLocaleFormBuilds => "F1.locale_form_builds", "page_analysis", "locale_form_builds";
    /// `F1.plugin_serializations`: a page serialized for an Extractor Plugin.
    PagePluginSerializations => "F1.plugin_serializations", "page_analysis", "plugin_serializations";
    /// `F1.plugin_answer_parses`: a parse of an Extractor Plugin's answer.
    PagePluginAnswerParses => "F1.plugin_answer_parses", "page_analysis", "plugin_answer_parses";
    /// `F1.tree_copies`: a copy of a fetched page's tree, made for a reader
    /// after a step that consumes it.
    PageTreeCopies => "F1.tree_copies", "page_analysis", "tree_copies";
    /// `F1.share_readability_runs`: a run of the main-content step on a
    /// fetched page as served, for ADR 0047's exact share.
    PageShareReadabilityRuns => "F1.share_readability_runs", "page_analysis", "share_readability_runs";
    /// `F1.block_classifications`: a fetched page's document read by the
    /// Block classification rules.
    PageBlockClassifications => "F1.block_classifications", "page_analysis", "block_classifications";
    /// `F1.rule_list_builds`: a page's list of cosmetic Clean-up rules built,
    /// from its URL and from the classes and IDs it holds.
    PageRuleListBuilds => "F1.rule_list_builds", "page_analysis", "rule_list_builds";
    /// `F1.rule_tests`: one Clean-up rule tested against one element of a
    /// fetched page, or one procedural rule run over one of its roots.
    PageRuleTests => "F1.rule_tests", "page_analysis", "rule_tests";
    /// `F1.analysis_handoffs`: a page's parsing or reading handed to the
    /// analysis pool (`TaskOwner::analysis`).
    PageAnalysisHandoffs => "F1.analysis_handoffs", "page_analysis", "analysis_handoffs";
    /// `F1.analysis_handoffs.handler`: a handler's read of its page handed
    /// to the analysis pool, apart from the page's own (#2189).
    HandlerAnalysisHandoffs => "F1.analysis_handoffs.handler", "page_analysis", "handler_analysis_handoffs";
    /// `F1.host_cache_entries`: a host whose Clean-up rules a Run's host
    /// cache holds as the Run ends.
    PageHostCacheEntries => "F1.host_cache_entries", "page_analysis", "host_cache_entries";
    /// `F1.host_cache_bytes`: a heap byte a Run's host cache holds as the
    /// Run ends.
    PageHostCacheBytes => "F1.host_cache_bytes", "page_analysis", "host_cache_bytes";
    /// `E1.deep_copies`: a whole copy of what a Client's message carries.
    MessageDeepCopies => "E1.deep_copies", "json_rpc_message", "deep_copies";
    /// `E1.encodes`: an encode of what the engine sends, into bytes or into
    /// a `Value` that is encoded again.
    MessageEncodes => "E1.encodes", "json_rpc_message", "encodes";
    /// `E1.decodes`: a decode of a request's params, by dispatch for what it
    /// reads of them or into the type their handler takes.
    MessageDecodes => "E1.decodes", "json_rpc_message", "decodes";
    /// `F5.messages_no_hook`: a message or its formatting for a stage with no
    /// Hook on it.
    HookMessagesNoHook => "F5.messages_no_hook", "hook_stage", "messages_no_hook";
    /// `F5.calls`: a request out to a Hook called.
    HookCalls => "F5.calls", "hook_stage", "calls";
    /// `F5.builds`: a Hook chain's build or compilation at a stage.
    HookBuilds => "F5.builds", "hook_stage", "builds";
    /// `F6.opens`: a store open or statement preparation for a delivered batch.
    SinkOpens => "F6.opens", "sink_delivery", "opens";
    /// `F6.transactions`: a transaction for a delivered batch.
    SinkTransactions => "F6.transactions", "sink_delivery", "transactions";
    /// `F6.writer_reads`: a read through the run file's writer for a delivered
    /// batch.
    SinkWriterReads => "F6.writer_reads", "sink_delivery", "writer_reads";
    /// `F4.declaration_reads`: a read of the Datasets' declarations to check a Record.
    RecordDeclarationReads => "F4.declaration_reads", "record", "declaration_reads";
    /// `F4.validator_builds`: a validator built to check a Record.
    RecordValidatorBuilds => "F4.validator_builds", "record", "validator_builds";
    /// `F4.validations`: a validation of a Record against its Dataset's schema.
    RecordValidations => "F4.validations", "record", "validations";
    /// `F4.serializations`: a Record's data written as the JSON text it is kept as.
    RecordSerializations => "F4.serializations", "record", "serializations";
    /// `F4.deep_copies`: a deep copy of a Record's data.
    RecordDeepCopies => "F4.deep_copies", "record", "deep_copies";
    /// `F4.constant_lookups`: a lookup of a value that stays constant for the Run, for a Record.
    RecordConstantLookups => "F4.constant_lookups", "record", "constant_lookups";
    /// `F4.transactions`: a run-file transaction of a Record's own, apart from its attempt's commit.
    RecordTransactions => "F4.transactions", "record", "transactions";
    /// `F3.invariant_reads`: a read of the Scope, Payload rules or Fragments
    /// from the run file, admitting a links batch.
    BatchInvariantReads => "F3.invariant_reads", "frontier_admission", "invariant_reads";
    /// `F3.compilations`: a glob compiled to admit a links batch.
    BatchCompilations => "F3.compilations", "frontier_admission", "compilations";
    /// `F3.transactions`: a write of its own a links batch asks the writer for.
    BatchTransactions => "F3.transactions", "frontier_admission", "transactions";
    /// `F3.actor_round_trips`: a message a page's task sends the actor that
    /// owns its crawl's Run state, whose answer the task waits for (ADR 0142).
    ActorRoundTrips => "F3.actor_round_trips", "frontier_admission", "actor_round_trips";
    /// `F3.in_flight_serializations`: a crawl's requests in flight encoded
    /// to JSON, for a value of its Run's progress that a reader takes.
    InFlightSerializations => "F3.in_flight_serializations", "frontier_admission", "in_flight_serializations";
    /// `F3.invariant_reads`, of a redirect hop.
    HopInvariantReads => "F3.invariant_reads.hop", "frontier_hop", "invariant_reads";
    /// `F3.compilations`, of a redirect hop.
    HopCompilations => "F3.compilations.hop", "frontier_hop", "compilations";
    /// `F3.transactions`, of a redirect hop.
    HopTransactions => "F3.transactions.hop", "frontier_hop", "transactions";
    /// `F3.hop_round_trips`: a message a page's task sends the actor that
    /// owns its crawl's Run state to have a redirect hop checked, whose
    /// answer the task waits for (ADR 0142).
    HopRoundTrips => "F3.hop_round_trips", "frontier_hop", "round_trips";
    /// `F3.hop_round_trips.end`: a message a page's task sends that actor to
    /// have a redirected chain's final URL recorded, whose answer the task
    /// waits for (ADR 0142).
    HopEndRoundTrips => "F3.hop_round_trips.end", "frontier_hop", "end_round_trips";
    /// `F3.invariant_reads`, of a sitemap's read.
    SitemapInvariantReads => "F3.invariant_reads.sitemap", "frontier_sitemap", "invariant_reads";
    /// `F3.compilations`, of a sitemap's read.
    SitemapCompilations => "F3.compilations.sitemap", "frontier_sitemap", "compilations";
    /// `F3.transactions`, of a sitemap's read.
    SitemapTransactions => "F3.transactions.sitemap", "frontier_sitemap", "transactions";
    /// `F3.compilations`: a validator built for one URL's Payload.
    PayloadCompilations => "F3.compilations.payload", "frontier_payload", "compilations";
    /// `F3.payload_serializations`: one URL's Payload serialized.
    PayloadSerializations => "F3.payload_serializations", "frontier_payload", "serializations";
    /// `F3.validations`: one URL's Payload validated against its Label's schema.
    PayloadValidations => "F3.validations", "frontier_payload", "validations";
    /// `C1.encodes`: a CDP command written into the bytes its pipe carries.
    CdpEncodes => "C1.encodes", "cdp_command", "encodes";
    /// `C1.sends`: a CDP command's bytes written to its pipe.
    CdpSends => "C1.sends", "cdp_command", "sends";
    /// `C1.file_opens`: a directory created or a file opened for a CDP
    /// command. The Run trace's one writer opens the CDP command log once a
    /// Run, on `E5.file_opens` (ADR 0140), so no command does.
    CdpFileOpens => "C1.file_opens", "cdp_command", "file_opens";
    /// `C1.log_sends_off`: a CDP command's method sent to the CDP command
    /// log's writer. The log has no off yet, so every send counts (X9).
    CdpLogSendsOff => "C1.log_sends_off", "cdp_command", "log_sends_off";
    /// `C3.script_sends`: a send of `check.js`'s source for a guard's check.
    ActionScriptSends => "C3.script_sends", "action", "script_sends";
    /// `C3.check_commands`: a CDP command a guard's check sent.
    ActionCheckCommands => "C3.check_commands", "action", "check_commands";
    /// `C3.commands`, of a click: a CDP command a click's attempt sent.
    ActionClickCommands => "C3.commands.click", "action", "click_commands";
    /// `C3.commands`, of a fill: a CDP command a fill's attempt sent.
    ActionFillCommands => "C3.commands.fill", "action", "fill_commands";
    /// `C3.commands`, of a press: a CDP command a key press's attempt sent.
    ActionPressCommands => "C3.commands.press", "action", "press_commands";
    /// `F2.transactions`: a transaction on a run file: a commit its commit
    /// hook saw, and each job the writer folded into a commit's group, so the
    /// count does not depend on how commits group (ADR 0138's note).
    RunTransactions => "F2.transactions", "run_state", "transactions";
    /// `F2.writer_reads`: a read through a run file's writer.
    WriterReads => "F2.writer_reads", "run_state", "writer_reads";
    /// `F2.page_waits`: a page's wait on the run file's writer: a lease, an
    /// admission or a settled outcome the page's task or the delivery loop
    /// awaits before it goes on (ADR 0140's note on batching).
    RunPageWaits => "F2.page_waits", "run_state", "page_waits";
    /// `F2.batches`: a batch the writer sealed before it reached its pages
    /// or its interval: a barrier, a seal, its bytes, or the run's end.
    RunBatches => "F2.batches", "run_state", "batches";
    /// `F2.evidence_reads`: a row of the evidence the Frontier learns its
    /// rules from, read for a fetched page or a redirect.
    EvidenceReads => "F2.evidence_reads", "run_state", "evidence_reads";
    /// `F2.admission_evidence`: a row of the evidence the Frontier learns
    /// its rules from, read or written for a URL it admitted.
    AdmissionEvidence => "F2.admission_evidence", "run_state", "admission_evidence";
    /// `F2.configuration_reads`: a read of the machine's settings files.
    ConfigurationReads => "F2.configuration_reads", "run_state", "configuration_reads";
    /// `F2.prunes`: a prune pass.
    Prunes => "F2.prunes", "run_state", "prunes";
    /// `F2.effect_syncs`: a synced commit of the Effect journal, which no code
    /// does until the journal lands (#1550).
    EffectSyncs => "F2.effect_syncs", "run_state", "effect_syncs";
    /// `E3.statements`: a SQL statement a Usage charge reads its Limits with.
    ChargeStatements => "E3.statements", "usage_charge", "statements";
    /// `E3.slice_transactions`: a transaction committed on the ledger's
    /// connections a charge goes through.
    ChargeTransactions => "E3.slice_transactions", "usage_charge", "transactions";
    /// `E3.opens`: a connection opened to the ledger for a charge.
    ChargeOpens => "E3.opens", "usage_charge", "opens";
    /// `C4.detection_reads`: a detector's read of a navigation's document,
    /// or the Consent prompt runner's start in it.
    NavigationDetectionReads => "C4.detection_reads", "navigation", "detection_reads";
    /// `C4.compilations`: a glob or a regular expression compiled to judge a
    /// navigation.
    NavigationCompilations => "C4.compilations", "navigation", "compilations";
    /// `C4.script_compilations`: a script compiled for a navigation.
    NavigationScriptCompilations => "C4.script_compilations", "navigation", "script_compilations";
    /// `C4.provenance_entries`: a Provenance entry a navigation's judgement
    /// writes.
    NavigationProvenanceEntries => "C4.provenance_entries", "navigation", "provenance_entries";
    /// `E7.merge_round_trips`: a round trip to the Merge host for a
    /// Working-copy update.
    WorkingCopyMergeRoundTrips => "E7.merge_round_trips", "working_copy_update", "merge_round_trips";
    /// `E7.document_loads`: a whole document loaded for a Working-copy update.
    WorkingCopyDocumentLoads => "E7.document_loads", "working_copy_update", "document_loads";
    /// `E7.document_encodes`: a whole document encoded for a Working-copy
    /// update.
    WorkingCopyDocumentEncodes => "E7.document_encodes", "working_copy_update", "document_encodes";
    /// `E7.opens`: a store opened for a Working-copy update.
    WorkingCopyOpens => "E7.opens", "working_copy_update", "opens";
    /// `E7.prepares`: a statement prepared for a Working-copy update.
    WorkingCopyPrepares => "E7.prepares", "working_copy_update", "prepares";
    /// `E7.transactions`: a transaction for a Working-copy update.
    WorkingCopyTransactions => "E7.transactions", "working_copy_update", "transactions";
    /// `E7.syncs`: a sync for a Working-copy update.
    WorkingCopySyncs => "E7.syncs", "working_copy_update", "syncs";
    /// `E7.encodes`: an encode of a Working-copy update for the Connections
    /// that follow it.
    WorkingCopyEncodes => "E7.encodes", "working_copy_update", "encodes";
    /// `E2.configuration_reads`: a read of the machine's settings files for
    /// an Audit entry.
    AuditConfigurationReads => "E2.configuration_reads", "audit_entry", "configuration_reads";
    /// `E2.opens`: an open of the Audit log's store, with its schema set-up.
    AuditOpens => "E2.opens", "audit_entry", "opens";
    /// `E2.prunes`: a prune pass over the Audit log.
    AuditPrunes => "E2.prunes", "audit_entry", "prunes";
    /// `E2.transactions`: a transaction the Audit log's store committed.
    AuditTransactions => "E2.transactions", "audit_entry", "transactions";
    /// `E2.syncs`: a write of the Audit log's head file; the store's own
    /// syncs are seen by a counting VFS, which no production store has.
    AuditSyncs => "E2.syncs", "audit_entry", "syncs";
    /// `C6.file_opens`: a file opened or a folder created for a Live view
    /// frame.
    LiveViewFileOpens => "C6.file_opens", "live_view_frame", "file_opens";
    /// `C6.compilations`: a selector, glob or regular expression compiled for
    /// a Live view frame.
    LiveViewCompilations => "C6.compilations", "live_view_frame", "compilations";
    /// `C6.spawns`: a task spawned for a Live view frame.
    LiveViewSpawns => "C6.spawns", "live_view_frame", "spawns";
    /// `C6.acknowledgements`: a Live view frame's acknowledgement command.
    LiveViewAcknowledgements => "C6.acknowledgements", "live_view_frame", "acknowledgements";
    /// `C6.seals`: a seal of a fragment of a relayed viewer's frame.
    LiveViewSeals => "C6.seals", "live_view_frame", "seals";
    /// `E5.file_opens`: a file opened or a folder created for a Run-trace
    /// entry, as the Run's one writer does once a Run for each file.
    RunTraceFileOpens => "E5.file_opens", "run_trace", "file_opens";
    /// `E5.spawns`: a task spawned for a Run-trace entry, as the Run's one
    /// writer is, once a Run.
    RunTraceSpawns => "E5.spawns", "run_trace", "spawns";
    /// `E5.formatting_off`: a Run-trace entry formatted though its kind is
    /// switched off.
    RunTraceFormattingOff => "E5.formatting_off", "run_trace", "formatting_off";
    /// `E6.transactions`: a transaction on a Politeness file, as its commit
    /// hook sees it.
    PolitenessTransactions => "E6.transactions", "politeness_permit", "transactions";
    /// `E6.opens`: a Politeness file opened. Its statement preparations are
    /// the benchmark's, read from Callgrind, as no wrapper sees them.
    PolitenessOpens => "E6.opens", "politeness_permit", "opens";
    /// `E6.script_sends`: a script sent whole to Valkey or Redis, which no
    /// code does until a Politeness store lands.
    PolitenessScriptSends => "E6.script_sends", "politeness_permit", "script_sends";
    /// `E6.robots_parses`: a robots.txt parsed for the copy a user's runs
    /// share.
    RobotsParses => "E6.robots_parses", "politeness_permit", "robots_parses";
    /// `E6.robots_reads`: a read of that copy from the Politeness file.
    RobotsReads => "E6.robots_reads", "politeness_permit", "robots_reads";
    /// `C2.commands`: a CDP command an engine read in a frame sends.
    PageReadCommands => "C2.commands", "page_read", "commands";
    /// `C2.world_creations`: a `Page.createIsolatedWorld` an engine read
    /// sends past its document's first. The driver keeps no registry of each
    /// frame's document yet, so every read's creation counts (X10).
    PageReadWorldCreations => "C2.world_creations", "page_read", "world_creations";
    /// `C2.document_worlds`: a `Page.createIsolatedWorld` an engine read
    /// sends, held per document and frame read.
    PageReadDocumentWorlds => "C2.document_worlds", "page_read", "document_worlds";
    /// `C2.serialization_commands`: a CDP command a page's serialization
    /// for its formats sends (`DOM.getDocument`, `DOM.getOuterHTML`), held
    /// per serialization: its own outcome, as no engine read in a frame
    /// makes it.
    PageSerializationCommands => "C2.serialization_commands", "page_read", "serialization_commands";
    /// `E4.configuration_reads`: a settings file read for a governor sample.
    GovernorConfigurationReads => "E4.configuration_reads", "governor_sample", "configuration_reads";
    /// `E4.meminfo_opens`: `/proc/meminfo` opened for a governor sample.
    GovernorMeminfoOpens => "E4.meminfo_opens", "governor_sample", "meminfo_opens";
    /// `E4.static_opens`: `self/cgroup` or `self/mountinfo` opened for a
    /// governor sample, which the engine's start could read once.
    GovernorStaticOpens => "E4.static_opens", "governor_sample", "static_opens";
    /// `E4.cpu_stat_opens`: the cgroup's `cpu.stat` opened for a governor
    /// sample.
    GovernorCpuStatOpens => "E4.cpu_stat_opens", "governor_sample", "cpu_stat_opens";
    /// `E4.process_reads`: one process's memory read for a governor sample.
    GovernorProcessReads => "E4.process_reads", "governor_sample", "process_reads";
    /// `C7.pool_walks`: a browser's processes walked for their memory as a
    /// page opens in the browser pool.
    PoolWalks => "C7.pool_walks", "page_opening", "pool_walks";
    /// `C7.opening_commands`: a CDP command a page target's opening awaits
    /// in turn before the page is ready, from its creation to its last
    /// set-up. Its Session's browser context is made as the Session opens,
    /// not in a page's opening.
    PageOpeningCommands => "C7.opening_commands", "page_opening", "opening_commands";
    /// `C5.fetch_messages_no_rule`: a message of CDP's `Fetch` domain, a
    /// command sent or an event read, for a page's requests.
    InterceptedFetchMessages => "C5.fetch_messages_no_rule", "intercepted_request", "fetch_messages_no_rule";
    /// `C5.spawns`: a task spawned for a refused request.
    InterceptedSpawns => "C5.spawns", "intercepted_request", "spawns";
    /// `C5.file_opens`: a file opened or a folder created for a refused
    /// request. The Run trace's one writer opens the request log once a
    /// Run, on `E5.file_opens` (ADR 0140), so no request does.
    InterceptedFileOpens => "C5.file_opens", "intercepted_request", "file_opens";
    /// `C5.matches`: a paused request matched against the compiled Filter
    /// lists.
    InterceptedMatches => "C5.matches", "intercepted_request", "matches";
    /// `C5.answers`: a CDP command that answers a paused request.
    InterceptedAnswers => "C5.answers", "intercepted_request", "answers";
    /// `C5.compilations`: a glob or regular expression compiled for a paused
    /// request.
    InterceptedCompilations => "C5.compilations", "intercepted_request", "compilations";
    /// `C5.entries`: an entry sent to the Run trace's writer for its
    /// request log.
    InterceptedEntries => "C5.entries", "intercepted_request", "entries";
}

/// The counts of one engine's Counted work, shared by every clone. A thread
/// may count while another reads: a read sees some count that was true. It
/// holds the engine's bounded cache of compilations too ([`Self::selector`]),
/// which every owner it is given to compiles through, so a text compiles once
/// for the engine and its compilation counts once. A default one builds no
/// part of that cache before a text of its kind is asked for, so the one an
/// owner holds until it is given the engine's costs it no cache.
#[derive(Clone, Debug)]
pub struct CountedWork(Arc<Shared>);

impl Default for CountedWork {
    /// Never inlined: `just counts` reads `F3.counted_work_constructions`
    /// as Callgrind's count of this function's calls, which a copy inlined
    /// into its caller would escape.
    #[inline(never)]
    fn default() -> Self {
        Self(Arc::default())
    }
}

/// What every clone of a [`CountedWork`] shares, behind its one `Arc`: a
/// clone costs one reference count and its handle stays a pointer wide, as
/// each dispatched message's handler future holds one (E1.instructions).
#[derive(Debug, Default)]
struct Shared {
    counts: Counts,
    compiled: Compiled,
}

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
        self.0.counts.0[row.index()].add_one();
    }

    /// Counts `units` units of `row`'s work.
    pub fn count_many(&self, row: Row, units: u64) {
        self.0.counts.0[row.index()].add(units);
    }

    /// The units of `row`'s work counted so far.
    #[must_use]
    pub fn read(&self, row: Row) -> u64 {
        self.0.counts.0[row.index()].read()
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

    /// A copy of the tree `tree`, counted on `row`: the one function a parsed
    /// tree is copied through (ADR 0138), never a parse.
    #[must_use]
    pub fn copy_tree(&self, row: Row, tree: &ParsedHtml) -> ParsedHtml {
        self.count(row);
        tree.clone()
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

    /// `selector` parsed for lol-html's streaming pass, counted on `row`,
    /// none when lol-html reads no such selector: the one function a
    /// streaming pass's selector is parsed through (ADR 0140), parsed once
    /// and lent to each pass (ADR 0138).
    #[must_use]
    pub fn compile_stream_selector(&self, row: Row, selector: &str) -> Option<lol_html::Selector> {
        self.count(row);
        selector.parse().ok()
    }

    /// The selector `selector`, from the engine's cache, compiled through
    /// [`Self::compile_selector`] and counted on `row` only when the cache
    /// does not keep it: what a tree reader matches with, never a
    /// `select("…")` that compiles on each call (ADR 0138).
    #[must_use]
    pub fn selector(&self, row: Row, selector: &str) -> Option<dom_query::Matcher> {
        self.0
            .compiled
            .selector(None, selector, || self.compile_selector(row, selector))
    }

    /// [`Self::selector`] of a selector of the Clean-up rules of the version
    /// whose digest is `rules`, kept apart from another version's.
    #[must_use]
    pub fn rules_selector(
        &self,
        row: Row,
        rules: &str,
        selector: &str,
    ) -> Option<dom_query::Matcher> {
        self.0.compiled.selector(Some(rules), selector, || {
            self.compile_selector(row, selector)
        })
    }

    /// [`Self::selector`] of a selector written in the code.
    ///
    /// # Panics
    ///
    /// When `selector` is no selector, which a test of its code shows.
    #[must_use]
    pub fn literal_selector(&self, row: Row, selector: &'static str) -> dom_query::Matcher {
        self.selector(row, selector)
            .expect("invariant: a selector written in the code is CSS")
    }

    /// The regular expression `pattern`, from the engine's cache, compiled
    /// through [`Self::compile_regex`] and counted on `row` only when the
    /// cache does not keep it.
    ///
    /// # Errors
    ///
    /// When `pattern` is no regular expression.
    pub fn regex(&self, row: Row, pattern: &str) -> Result<regex::Regex, regex::Error> {
        self.0
            .compiled
            .regex(pattern, || self.compile_regex(row, pattern))
    }

    /// The glob `pattern`, from the engine's cache, compiled through the one
    /// counted function of a glob's compilation and counted on `row` only
    /// when the cache does not keep it: a glob compiles only through it
    /// (ADR 0140).
    ///
    /// # Errors
    ///
    /// When `pattern` is no glob.
    pub fn glob(&self, row: Row, pattern: &str) -> Result<glob::Pattern, Arc<glob::PatternError>> {
        self.0
            .compiled
            .glob(pattern, || self.compile_glob(row, pattern))
    }

    /// How many selectors the engine's cache keeps, at most
    /// [`compiled::CAPACITY`].
    #[cfg(test)]
    fn kept(&self) -> u64 {
        self.0.compiled.selectors()
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
    fn compile_glob(&self, row: Row, pattern: &str) -> Result<glob::Pattern, glob::PatternError> {
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
