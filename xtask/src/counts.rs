//! `xtask counts` (ADR 0140): runs a family's benchmark or a path's rows,
//! reads the Counted work each cell did, divides each count by the units of
//! the outcome that did the work, reads each row's limit from `TESTING.md`'s
//! Quality register, prints every figure and fails over a limit, naming the
//! row, its figure and its limit. It is the "Checked by" of every counted
//! row, and runs at implementer done (`just counts`) and in the merge gate's
//! `perf` step.
//!
//! `--plant <row>` has the benchmark do one extra unit of the row's work, as
//! `--prove` does for every row and expects the failure: each counter proves
//! it can fail (the register's rules).
//!
//! `--strace` runs B1 and B2 under `strace` and fails when what it sees of
//! the Run trace's folders and files, and of the syncs of the files a
//! counting VFS holds, differs from the counters (ADR 0140), so a bypass of
//! the counted functions fails; `--bypass <work>` plants one.
//!
//! `--valgrind` runs the rows read under Valgrind in place of the counted ones
//! (ADR 0140's "Runs at"): each family's Callgrind and DHAT targets, whose
//! figure per unit is the difference between a scenario and the run of its
//! benchmark that sends nothing, so no start-up cost hides a change, and
//! B1's example in the benchmarks' profile, which runs its handled cell
//! under Callgrind for `E1.handled_page_wire_share`.
//!
//! Size: one concern, the command's one register of counted rows (`ROWS`, a
//! row per counted work of every family) and the run, verdict and proof that
//! hold them to the register; the rows' table is most of its length.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

mod register;
mod strace;

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::check::{self, Finding};
use crate::environment::{CALLGRIND_AT_ONCE_VARIABLE, Environment};
use crate::output;
use crate::perf::{self, Figure, Measured};
use crate::products;

use register::Limit;

/// A benchmark family: the one target whose run reads every row it counts.
struct Family {
    /// Its name in ADR 0140.
    name: &'static str,
    /// The package holding its target.
    package: &'static str,
    /// Its example target: `<target> [--plant <row>]` prints one JSON line
    /// per cell.
    target: &'static str,
    /// Its benchmark under Valgrind, if it has rows read there.
    valgrind: Option<ValgrindBenchmark>,
    /// The bypasses of a counted function its benchmark plants under
    /// `strace`, each one raw unit of that work: none when `--strace` does
    /// not run it.
    bypasses: &'static [&'static str],
}

/// A family's benchmark in its Callgrind and DHAT targets.
struct ValgrindBenchmark {
    /// The `#[library_benchmark]` function, which filters the run.
    function: &'static str,
    /// Its `#[bench::…]` that sends nothing, which each scenario is measured
    /// against.
    empty: &'static str,
    /// The messages each of its scenarios dispatches beyond `empty`'s.
    units: u64,
    /// Each cell, by the suffix of its `#[bench::…]`s.
    cells: &'static [(&'static str, &'static str)],
}

const B1: Family = Family {
    name: "B1",
    package: products::FOXHOUND.package,
    target: "counts",
    valgrind: None,
    bypasses: &[strace::OPEN, strace::DIRECTORY, strace::SYNC],
};

const B3: Family = Family {
    name: "B3",
    package: "goodfetch-engine",
    target: "counts",
    valgrind: Some(ValgrindBenchmark {
        function: "dispatch_messages",
        empty: "no_messages",
        units: 100,
        cells: &[(DEFAULT_CELL, ""), ("log_off", "_log_off")],
    }),
    bypasses: &[],
};

const B2: Family = Family {
    name: "B2",
    package: "goodfetch-page",
    target: "counts",
    valgrind: None,
    bypasses: &[strace::OPEN, strace::DIRECTORY],
};

/// The extraction path's benchmark (ADR 0140's note of 2026-11-14): the
/// in-memory crawl, as B1's, over the Proving ground's article site.
const B4: Family = Family {
    name: "B4",
    package: products::FOXHOUND.package,
    target: "counts-extraction",
    valgrind: None,
    bypasses: &[],
};

const FAMILIES: [Family; 4] = [B1, B2, B3, B4];

/// The variable a Valgrind target reads its plant from: gungraun takes no
/// argument for it.
const PLANT_VARIABLE: &str = "GOODFETCH_COUNTS_PLANT";

/// The folder under the workspace's `target/` of the Valgrind rows' reports.
const VALGRIND_OUT: &str = "target/counts";

/// The folder under the workspace's `target/` of each family's built
/// benchmark, under its family's name.
const EXECUTABLES_OUT: &str = "target/counts/executables";

/// The `cfg` of the build a row is read in under Callgrind, and the name of
/// that build's folder in the workspace's target folder: the wire's
/// functions are never inlined there, so that Callgrind names them in
/// `E1.handled_page_wire_share` (ADR 0140).
const CALLGRIND_NAMES: &str = "goodfetch_callgrind_names";

/// One counted row of the register.
struct Row {
    /// Its name in the register, `<path>.<work>`, and the name its
    /// benchmark prints its count under.
    name: &'static str,
    /// The family that counts it. A row the register puts on two
    /// families is counted on the one its entry names (`E5.file_opens`, on
    /// B1), or has an entry for each, which holds each family's count to
    /// that family's own figure (`F1.analysis_handoffs` and `F1.parses`, on
    /// B1 and B4), or to one cell's where the register names it
    /// (`F1.tree_copies` in B4's `single_attempt`).
    family: &'static str,
    /// The key of the units of the outcome that does the work, in its
    /// benchmark's line.
    unit: &'static str,
    /// Whether the default cell must do this work: a benchmark that did
    /// none of what its row counts proves nothing. A row whose contract is
    /// 0 says no; its plant still proves its counter.
    happens: bool,
    /// Where a row read under Valgrind gets its figure.
    valgrind: Option<UnderValgrind>,
}

/// Where a row read under Valgrind gets its figure.
#[derive(Clone, Copy)]
enum UnderValgrind {
    /// A scenario of its family's benchmark under Valgrind.
    Benchmark(ValgrindFigure),
    /// Its family's example, run as `<target> --valgrind`, which runs itself
    /// under Callgrind and prints its cells as `xtask counts` reads them.
    Example,
}

/// A figure read under Valgrind, of one scenario of its family's benchmark.
#[derive(Clone, Copy)]
struct ValgrindFigure {
    /// The bench target that measures it: `instructions` under Callgrind,
    /// `heap` under DHAT.
    target: &'static str,
    figure: Figure,
    /// The `#[bench::…]`, its cell's suffix left out.
    scenario: &'static str,
}

const fn message_row(
    name: &'static str,
    target: &'static str,
    figure: Figure,
    scenario: &'static str,
) -> Row {
    Row {
        name,
        family: "B3",
        unit: "messages",
        happens: true,
        valgrind: Some(UnderValgrind::Benchmark(ValgrindFigure {
            target,
            figure,
            scenario,
        })),
    }
}

/// The two scenarios of [`message_row`]s.
const SMALL: &str = "small_params";
const LARGE: &str = "large_params";

/// Every row with a counter. Adding one here, with its counter and its
/// plant in the family's benchmark, is what gives a row its command.
const ROWS: [Row; 140] = [
    Row {
        name: "F1.parses",
        family: "B1",
        unit: "pages",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F1.selector_compilations",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.readability_runs",
        family: "B1",
        unit: "pages",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F1.file_opens",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.locale_form_builds",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.plugin_serializations",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.plugin_answer_parses",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.tree_copies",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.share_readability_runs",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F1.block_classifications",
        family: "B1",
        unit: "pages",
        happens: true,
        valgrind: None,
    },
    b1("F1.analysis_handoffs", "pages", true),
    b4("F1.analysis_handoffs", "pages", true),
    // A handler's own reads of its page's tree, counted apart from the
    // page's hand-off (#2189): B1's default cell runs no handler.
    b1("F1.analysis_handoffs.handler", "pages", false),
    // A code-free crawl that asks for a format reads it in its page's one
    // hand-off, on a copy of the tree its untouched readers parsed (#2189).
    b4("F1.parses", "pages", true),
    // The read ahead's copy for the landmarks fallback, once a page, and
    // twice in `single_attempt`, a cell the register names (#2639).
    b4("F1.tree_copies", "pages", true),
    b4("F1.rule_list_builds", "pages", true),
    b4("F1.rule_tests", "pages", true),
    // Read as each cell's crawl ended.
    b4("F1.host_cache_entries", CRAWLS, true),
    b4("F1.host_cache_bytes", CRAWLS, true),
    // Callgrind's count beneath the clean-up's removal, which B4's own run
    // reads on its default cell alone: the other cells print none of its
    // unit.
    b4(CLEAN_UP_INSTRUCTIONS, CALLGRIND_PAGES, true),
    // Callgrind's count beneath each selector's compilation over the
    // default cell's one crawl on an empty cache, read per crawl as
    // `F1.clean_up_instructions` is: work once per engine and rules version.
    b4(SELECTOR_COMPILATION_INSTRUCTIONS, CALLGRIND_CRAWLS, true),
    // Callgrind's count beneath the build of the version's generic set,
    // which the Filter lists' load does once per version (issue 2191),
    // over the same crawl as `F1.clean_up_instructions`, per crawl.
    b4(GENERIC_SET_INSTRUCTIONS, CALLGRIND_CRAWLS, true),
    Row {
        name: "F2.transactions",
        family: "B1",
        unit: "attempts",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F2.syncs",
        family: "B1",
        unit: "attempts",
        happens: true,
        valgrind: None,
    },
    // What each page more prepares: the whole site's run less a run on half
    // its pages, over the pages it adds (ADR 0138).
    Row {
        name: "F2.prepares",
        family: "B1",
        unit: ADDED_PAGES,
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F2.writer_reads",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    // A page's waits for the run file's writer, held per page at the levels
    // that batch attempts and, apart, at `full` (ADR 0140's note on
    // batching): each row's unit is its own cells' pages, none elsewhere.
    Row {
        name: "F2.page_waits",
        family: "B1",
        unit: "batched_pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F2.page_waits.full",
        family: "B1",
        unit: "full_pages",
        happens: false,
        valgrind: None,
    },
    // The batches sealed before their plan, less one for each call the
    // crawl was made that no page will make again, per Run.
    Row {
        name: "F2.batches",
        family: "B1",
        unit: "runs",
        happens: false,
        valgrind: None,
    },
    // The rows of evidence the Frontier's learning reads, where it runs
    // (ADR 0140's note of 2026-11-10): none on a site whose URLs have no
    // query, so the default cell does none.
    Row {
        name: "F2.evidence_reads",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    // The rows of evidence the Frontier's admissions read and write, where
    // learning runs (#2169): none with `frontier.learning` off.
    Row {
        name: "F2.admission_evidence",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    // The lessons the Frontier learns on a page's path, outside the end of
    // a batch the writer commits (#2169): none, as a crawl's pages and
    // admissions note them for that end.
    Row {
        name: "F2.page_learning",
        family: "B1",
        unit: "pages",
        happens: false,
        valgrind: None,
    },
    // Its unit is the pages of the cell with learning off: none in any
    // other cell, where its count is 0.
    Row {
        name: "F2.learning_off_evidence",
        family: "B1",
        unit: "learning_off_pages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F2.configuration_reads",
        family: "B1",
        unit: "runs",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F2.prunes",
        family: "B1",
        unit: "attempts",
        happens: false,
        valgrind: None,
    },
    // Its unit is the Effects its benchmark meets: none until the Effect
    // journal lands (#1550), so 0 passes and a planted one fails.
    Row {
        name: "F2.effect_syncs",
        family: "B1",
        unit: "effects",
        happens: false,
        valgrind: None,
    },
    b1("F3.invariant_reads", LINKS_BATCHES, false),
    b1("F3.invariant_reads.hop", HOPS, false),
    b1("F3.invariant_reads.sitemap", SITEMAPS, false),
    b1("F3.compilations", LINKS_BATCHES, false),
    b1("F3.compilations.hop", HOPS, false),
    b1("F3.compilations.sitemap", SITEMAPS, false),
    b1("F3.compilations.payload", PAYLOADS, false),
    b1("F3.prepares", LINKS_BATCHES, false),
    b1("F3.prepares.hop", HOPS, false),
    b1("F3.prepares.sitemap", SITEMAPS, false),
    b1("F3.transactions", LINKS_BATCHES, false),
    b1("F3.transactions.hop", HOPS, false),
    b1("F3.transactions.sitemap", SITEMAPS, false),
    b1("F3.payload_serializations", PAYLOADS, true),
    b1("F3.validations", PAYLOADS, true),
    b1("F3.actor_round_trips", ATTEMPTS_ENDED, true),
    // A second copy's take of its Permit, a round trip to the actor that
    // holds its Run's Politeness (#2171): at most one per second copy. Its
    // unit is the cell whose handler sends each page's navigation to
    // another domain key, none elsewhere, where its count is 0.
    b1("F3.second_copy_round_trips", SECOND_COPIES, false),
    // A message whose answer a crawl's task waits for from the task that
    // holds its Run's own Politeness, held in memory (#2171, the owner's
    // option A): a paced key's start after its robots.txt, a proxy's
    // Permit. A robots.txt copy's read counts on `E6.robots_reads` and a
    // second copy's take on the row above instead; on the file it counts
    // none, nor where no key is paced and no proxy is set.
    b1("F3.politeness_read_round_trips", ATTEMPTS_ENDED, false),
    b1("F3.hop_round_trips", HOPS, true),
    b1("F3.hop_round_trips.end", CHAINS, true),
    b1("F3.in_flight_serializations", ATTEMPTS_ENDED, false),
    // The read transactions on the run file per lease, at the levels that
    // batch attempts, within the memory budget (#2169): none, as the Run
    // state holds every pending work item. Its unit is those cells'
    // leases, a page's and a sitemap's, none elsewhere.
    b1("F3.lease_reads", BATCHED_LEASES, false),
    // The same past the memory budget, from the first page in the cell
    // whose budget holds no pending work item: at most one a lease (ADR
    // 0140's F3 contract). Its unit is that cell's leases.
    b1("F3.lease_reads.spilled", SPILLED_LEASES, false),
    // The Frontier's state an admission reads from the run file per links
    // batch, in the same cells as `F3.lease_reads` (#2169): none, as the
    // crawl's opening read the learned rules and identities, the Bounds'
    // counts and the trap marks into memory. Its unit is those cells'
    // links batches, none elsewhere.
    b1("F3.state_reads", HELD_LINKS_BATCHES, false),
    // What each page more builds, read as `F2.prepares` is: the whole
    // site's run less a run on half its pages, over the pages it adds.
    b1("F3.counted_work_constructions", ADDED_PAGES, false),
    Row {
        name: "C1.encodes",
        family: "B2",
        unit: "commands",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C1.sends",
        family: "B2",
        unit: "commands",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C1.file_opens",
        family: "B2",
        unit: "commands",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C1.log_sends_off",
        family: "B2",
        unit: "commands",
        happens: false,
        valgrind: None,
    },
    // C7, page opening in the browser pool: B2's pool cell opens its pages
    // in a pool its scripted launcher starts (ADR 0143); the Borrowed
    // browser of its other cells skips the pool's walk.
    Row {
        name: "C7.pool_walks",
        family: "B2",
        unit: "pages_opened",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C7.opening_commands",
        family: "B2",
        unit: "pages_opened",
        happens: true,
        valgrind: None,
    },
    // A Session's browser context, made as it opens in the pool; the
    // Borrowed browser of the default cell makes none.
    Row {
        name: "C7.contexts_created",
        family: "B2",
        unit: "sessions_opened",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E4.configuration_reads",
        family: "B3",
        unit: "samples",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E4.meminfo_opens",
        family: "B3",
        unit: "samples",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E4.static_opens",
        family: "B3",
        unit: "samples",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E4.process_reads",
        family: "B3",
        unit: "process_samples",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E1.deep_copies",
        family: "B3",
        unit: "incoming_messages",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E1.encodes",
        family: "B3",
        unit: "answers_and_events",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E1.decodes",
        family: "B3",
        unit: "requests",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E2.configuration_reads",
        family: "B3",
        unit: "audit_entries",
        happens: false,
        valgrind: None,
    },
    // 0 in every cell: the writer opens its store and prepares its
    // statements once a log, which the log's rows count (#1887). Callgrind
    // reads `E2.prepares` over the entries after the log's first, whose
    // preparations are the log's.
    Row {
        name: "E2.opens",
        family: "B3",
        unit: "audit_entries",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E2.prepares",
        family: "B3",
        unit: "later_audit_entries",
        happens: false,
        valgrind: None,
    },
    // 0 in every cell: no append prunes, the engine pruning as it starts
    // and once a day (#1887).
    Row {
        name: "E2.prunes",
        family: "B3",
        unit: "audit_entries",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E2.transactions",
        family: "B3",
        unit: "audit_entries",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E2.syncs",
        family: "B3",
        unit: "audit_entries",
        happens: true,
        valgrind: None,
    },
    // The writer's own work, once a log: the store's first open, its
    // schema set-up's commits and what its first entry prepares (#1887).
    Row {
        name: "E2.log_opens",
        family: "B3",
        unit: "audit_logs",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E2.log_transactions",
        family: "B3",
        unit: "audit_logs",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E2.log_prepares",
        family: "B3",
        unit: "audit_logs",
        happens: true,
        valgrind: None,
    },
    message_row(
        "E1.instructions",
        "instructions",
        Figure::Instructions,
        SMALL,
    ),
    message_row(
        "E1.instructions_large",
        "instructions",
        Figure::Instructions,
        LARGE,
    ),
    message_row("E1.allocations", "heap", Figure::TotalAllocations, SMALL),
    message_row(
        "E1.allocations_large",
        "heap",
        Figure::TotalAllocations,
        LARGE,
    ),
    message_row("E1.bytes_allocated", "heap", Figure::TotalBytes, SMALL),
    message_row(
        "E1.bytes_allocated_large",
        "heap",
        Figure::TotalBytes,
        LARGE,
    ),
    // The wire's share of a handled page (#2407): B1's example runs its
    // handled cell's crawl under Callgrind, with the reference handler, and
    // prints the instructions spent on messages, both sides, over a
    // hundredth of the page's, so the figure reads as a percent.
    Row {
        name: "E1.handled_page_wire_share",
        family: "B1",
        unit: HANDLED_PAGE_HUNDREDTHS,
        happens: true,
        valgrind: Some(UnderValgrind::Example),
    },
    Row {
        name: "E3.statements",
        family: "B3",
        unit: "covered_charges",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E3.slice_transactions",
        family: "B3",
        unit: "slices",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E3.opens",
        family: "B3",
        unit: "charges",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E3.prepares",
        family: "B3",
        unit: "slices",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F4.declaration_reads",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F4.validator_builds",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F4.validations",
        family: "B1",
        unit: "records",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F4.serializations",
        family: "B1",
        unit: "records",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "F4.deep_copies",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F4.constant_lookups",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "F4.transactions",
        family: "B1",
        unit: "records",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E5.file_opens",
        family: "B1",
        unit: "entries",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E5.spawns",
        family: "B1",
        unit: "entries",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E5.formatting_off",
        family: "B1",
        unit: "entries",
        happens: false,
        valgrind: None,
    },
    // The Run's one writer: its start, its trace's folder and each file's
    // first open, once a Run.
    Row {
        name: "E5.run_file_opens",
        family: "B1",
        unit: "runs",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "E5.run_spawns",
        family: "B1",
        unit: "runs",
        happens: true,
        valgrind: None,
    },
    // The default cell's Run serves each Permit from its engine's
    // Delegation and takes none in a transaction (ADR 0142, #2172): its
    // figure is 0, and `E6.undelegated_transactions` counts the work.
    Row {
        name: "E6.transactions",
        family: "B1",
        unit: "attempts",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E6.opens",
        family: "B1",
        unit: "permits",
        happens: false,
        valgrind: None,
    },
    // No Politeness store sends a script until one lands: 0 passes and a
    // planted one fails.
    Row {
        name: "E6.script_sends",
        family: "B1",
        unit: "permits",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "E6.robots_parses",
        family: "B1",
        unit: "origins",
        happens: true,
        valgrind: None,
    },
    // A Run reads each origin's copy from the Politeness file once, then
    // keeps its parse (« ok a tous », 2026-11-16).
    Row {
        name: "E6.robots_reads",
        family: "B1",
        unit: "origins",
        happens: true,
        valgrind: None,
    },
    // Its unit is the attempts of the cell with Politeness and robots.txt
    // off: none in any other cell, where its count is 0.
    Row {
        name: "E6.off_work",
        family: "B1",
        unit: "off_attempts",
        happens: false,
        valgrind: None,
    },
    // Its unit is the attempts of the cell whose Run keeps its Politeness
    // to itself (`politeness.sharing: run`), in memory: none in any other
    // cell, where its count is 0 (#2171).
    Row {
        name: "E6.private_transactions",
        family: "B1",
        unit: "private_attempts",
        happens: false,
        valgrind: None,
    },
    // Its unit is the attempts of the cell whose Run takes each Permit in
    // the Politeness file (`politeness.delegation: false`): none in any
    // other cell, where `E6.transactions` counts the rest (#2172).
    Row {
        name: "E6.undelegated_transactions",
        family: "B1",
        unit: "undelegated_attempts",
        happens: false,
        valgrind: None,
    },
    // Its unit is the attempts of the cell where two engines crawl the
    // site, each its own Run, and its count every transaction on the
    // Politeness file they share: none in any other cell, where its count
    // is 0 (#2173).
    Row {
        name: "E6.shared_transactions",
        family: "B1",
        unit: "shared_attempts",
        happens: false,
        valgrind: None,
    },
    // The Delegations a Run's engine takes, per Run: one for the site's
    // key, given back at the Run's end (ADR 0142, #2172).
    Row {
        name: "E6.delegations",
        family: "B1",
        unit: "runs",
        happens: true,
        valgrind: None,
    },
    // The transactions that take Delegations, per Delegation taken: at most
    // one, fewer when a group asks for several keys.
    Row {
        name: "E6.delegation_transactions",
        family: "B1",
        unit: "delegations",
        happens: true,
        valgrind: None,
    },
    // The file's other transactions, per attempt, as ADR 0140's E6 counts
    // the file's work: a Run's open, its robots.txt copy, a site's ask, a
    // Delegation given back and the Run's end, which a Run of more attempts
    // spreads thinner.
    Row {
        name: "E6.other_transactions",
        family: "B1",
        unit: "attempts",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C2.commands",
        family: "B2",
        unit: "reads",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C2.world_creations",
        family: "B2",
        unit: "reads",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C2.document_worlds",
        family: "B2",
        unit: "documents",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C2.serialization_commands",
        family: "B2",
        unit: "serializations",
        happens: true,
        valgrind: None,
    },
    // Each `page.read` the program makes, at one field and at twenty: the
    // benchmark's test holds the two equal.
    Row {
        name: "C2.read_commands",
        family: "B2",
        unit: "field_reads",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C5.fetch_messages_no_rule",
        family: "B2",
        unit: "requests",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C5.spawns",
        family: "B2",
        unit: "refusals",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C5.file_opens",
        family: "B2",
        unit: "refusals",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C5.matches",
        family: "B2",
        unit: "pauses",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C5.answers",
        family: "B2",
        unit: "pauses",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C5.compilations",
        family: "B2",
        unit: "pauses",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C5.entries",
        family: "B2",
        unit: "refusals",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C3.script_sends",
        family: "B2",
        unit: "checks",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C3.check_commands",
        family: "B2",
        unit: "checks",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C3.commands.click",
        family: "B2",
        unit: "click_attempts",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C3.commands.fill",
        family: "B2",
        unit: "fill_attempts",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C3.commands.press",
        family: "B2",
        unit: "press_attempts",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C4.detection_reads",
        family: "B2",
        unit: "navigations",
        happens: true,
        valgrind: None,
    },
    Row {
        name: "C4.compilations",
        family: "B2",
        unit: "navigations",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C4.script_compilations",
        family: "B2",
        unit: "navigations",
        happens: false,
        valgrind: None,
    },
    Row {
        name: "C4.provenance_entries",
        family: "B2",
        unit: "navigations",
        happens: true,
        valgrind: None,
    },
];

/// A row of a Shared path whose code does not exist yet: it holds its limit
/// and says it has not run, until the ticket that builds the path counts its
/// work and moves it to [`ROWS`].
struct NotRunYet {
    /// Its name in the register, and the name its benchmark prints it under.
    name: &'static str,
    /// The family that will count it.
    family: &'static str,
    /// The ticket that builds its path.
    builder: &'static str,
    /// Whether its Contract caps it: a ratchet's (`no cap (ratchet)`) is
    /// held only to its first figure, so it has no limit before one.
    capped: bool,
}

const fn working_copy_row(name: &'static str, builder: &'static str) -> NotRunYet {
    NotRunYet {
        name,
        family: "B3",
        builder,
        capped: true,
    }
}

/// The ticket that builds the Working-copy update (E7) and the one that moves
/// its merge into the Merge host.
const WORKING_COPY: &str = "#1689";
const MERGE_HOST: &str = "#1998";

const NOT_RUN_YET: [NotRunYet; 26] = [
    NotRunYet {
        name: "F5.messages_no_hook",
        family: "B1",
        builder: "#1481",
        capped: true,
    },
    NotRunYet {
        name: "F5.calls",
        family: "B1",
        builder: "#1481",
        capped: true,
    },
    NotRunYet {
        name: "F5.builds",
        family: "B1",
        builder: "#1481",
        capped: true,
    },
    NotRunYet {
        name: "F6.opens",
        family: "B1",
        builder: "#1121",
        capped: true,
    },
    NotRunYet {
        name: "F6.transactions",
        family: "B1",
        builder: "#1121",
        capped: true,
    },
    NotRunYet {
        name: "F6.writer_reads",
        family: "B1",
        builder: "#1121",
        capped: true,
    },
    NotRunYet {
        name: "C2.closed_root_commands",
        family: "B2",
        builder: "#726",
        capped: true,
    },
    NotRunYet {
        name: "C2.live_view_commands",
        family: "B2",
        builder: "#1683",
        capped: false,
    },
    working_copy_row("E7.merge_round_trips", MERGE_HOST),
    working_copy_row("E7.document_loads", WORKING_COPY),
    working_copy_row("E7.document_encodes", WORKING_COPY),
    working_copy_row("E7.opens", WORKING_COPY),
    working_copy_row("E7.prepares", WORKING_COPY),
    working_copy_row("E7.transactions", WORKING_COPY),
    working_copy_row("E7.syncs", WORKING_COPY),
    working_copy_row("E7.encodes", WORKING_COPY),
    NotRunYet {
        name: "C6.file_opens",
        family: "B2",
        builder: "#959",
        capped: true,
    },
    NotRunYet {
        name: "C6.compilations",
        family: "B2",
        builder: "#959",
        capped: true,
    },
    NotRunYet {
        name: "C6.spawns",
        family: "B2",
        builder: "#959",
        capped: true,
    },
    NotRunYet {
        name: "C6.acknowledgements",
        family: "B2",
        builder: "#959",
        capped: true,
    },
    NotRunYet {
        name: "C6.seals",
        family: "B2",
        builder: "#959",
        capped: true,
    },
    NotRunYet {
        name: "E4.cpu_stat_opens",
        family: "B3",
        builder: "#1306",
        capped: true,
    },
    NotRunYet {
        name: "C5.cache_lookups",
        family: "B2",
        builder: "#793",
        capped: true,
    },
    NotRunYet {
        name: "C5.cache_file_opens",
        family: "B2",
        builder: "#793",
        capped: true,
    },
    NotRunYet {
        name: "C5.body_reads",
        family: "B2",
        builder: "#1291",
        capped: true,
    },
    NotRunYet {
        name: "C5.charset_refetches",
        family: "B2",
        builder: "#1783",
        capped: true,
    },
];

/// The pages B1's whole site serves beyond a run on half of it.
const ADDED_PAGES: &str = "added_pages";

/// The links batches B1's crawl admitted.
const LINKS_BATCHES: &str = "links_batches";
/// The redirect hops it took through the Frontier.
const HOPS: &str = "hops";
/// The redirected chains that ended on their document.
const CHAINS: &str = "chains";
/// The sitemaps it read.
const SITEMAPS: &str = "sitemaps";
/// The URLs it enqueued with a Payload.
const PAYLOADS: &str = "payloads";

/// The attempts that ended in it: a page's, and each sitemap's read.
const ATTEMPTS_ENDED: &str = "attempts_ended";
/// The second copies of requests its Run admitted that it sent: a
/// handler's navigations to another domain key.
const SECOND_COPIES: &str = "second_copies";

/// The leases of a cell at a level that batches attempts whose memory
/// budget holds its pending work items, a page's and a sitemap's: none in
/// any other cell.
const BATCHED_LEASES: &str = "batched_leases";

/// The leases of the cell at a level that batches attempts whose memory
/// budget holds no pending work item: none in any other cell.
const SPILLED_LEASES: &str = "spilled_leases";

/// The links batches of a cell at a level that batches attempts whose
/// memory budget holds its pending work items: none in any other cell.
const HELD_LINKS_BATCHES: &str = "held_links_batches";

const fn b1(name: &'static str, unit: &'static str, happens: bool) -> Row {
    Row {
        name,
        family: "B1",
        unit,
        happens,
        valgrind: None,
    }
}

/// A cell's one crawl, the unit of what is read as the crawl ended.
const CRAWLS: &str = "crawls";
/// The pages of the cell that B4's benchmark runs again under Callgrind.
const CALLGRIND_PAGES: &str = "callgrind_pages";
/// The one crawl of that cell its run under Callgrind counts.
const CALLGRIND_CRAWLS: &str = "callgrind_crawls";

/// A hundredth of the instructions the pages B1's handled cell adds take,
/// the unit of `E1.handled_page_wire_share`, which reads as a percent.
const HANDLED_PAGE_HUNDREDTHS: &str = "handled_page_hundredths";

/// The clean-up's instructions per page, which B4's benchmark reads under
/// Callgrind in its own run: a figure that varies, so held within the
/// tolerance of a figure read under Valgrind (the register's ratchet).
const CLEAN_UP_INSTRUCTIONS: &str = "F1.clean_up_instructions";

/// The instructions of a crawl's selector compilations on an empty cache,
/// which B4's benchmark reads under Callgrind as it reads
/// [`CLEAN_UP_INSTRUCTIONS`], and holds within the same tolerance.
const SELECTOR_COMPILATION_INSTRUCTIONS: &str = "F1.selector_compilation_instructions";

/// The instructions of the generic set's build as a crawl's Filter lists
/// load, which B4's benchmark reads under Callgrind as it reads
/// [`CLEAN_UP_INSTRUCTIONS`], and holds within the same tolerance.
const GENERIC_SET_INSTRUCTIONS: &str = "F1.generic_set_instructions";

const fn b4(name: &'static str, unit: &'static str, happens: bool) -> Row {
    Row {
        name,
        family: "B4",
        unit,
        happens,
        valgrind: None,
    }
}

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// Only this family's rows (B1, B2, B3, B4).
    #[arg(long)]
    family: Option<String>,
    /// Only this path's rows (F1 to F4, C1 to C6, E1 to E7).
    #[arg(long)]
    path: Option<String>,
    /// Only this row, by its name in the register (F1.parses), each of its
    /// kinds for a row with a figure per kind (C3.commands).
    #[arg(long)]
    row: Option<String>,
    /// Has the benchmark do one extra unit of this row's work.
    #[arg(long, conflicts_with = "prove")]
    plant: Option<String>,
    /// Plants one extra unit of each row's work in turn and fails unless
    /// each is caught, naming its row.
    #[arg(long)]
    prove: bool,
    /// Runs this executable as the family `<family>=<path>` names, in place
    /// of building its target.
    #[arg(long, value_name = "FAMILY=PATH")]
    executable: Vec<String>,
    /// Runs the rows read under Valgrind, in place of the counted ones.
    #[arg(long, conflicts_with = "executable")]
    valgrind: bool,
    /// Runs B1 and B2 under `strace`, in place of the counted rows, and fails
    /// when what it sees of the opens, folders and syncs the counters count
    /// differs from them.
    #[arg(long, conflicts_with_all = ["valgrind", "plant", "path", "row"])]
    strace: bool,
    /// With `--strace`, has the benchmark do one raw unit of this work
    /// (`open`, `directory`, `sync`) beside its counted function.
    #[arg(long, requires = "strace", conflicts_with = "prove")]
    bypass: Option<String>,
    /// Lets each benchmark run at most this many Callgrind processes at
    /// once, below its own bound of half the cores, at most eight; without
    /// it, `GOODFETCH_COUNTS_CALLGRIND_AT_ONCE` sets it.
    #[arg(long, value_name = "N")]
    callgrind_at_once: Option<NonZeroUsize>,
}

/// The cell that runs with every default feature on.
const DEFAULT_CELL: &str = "default";

/// One line a benchmark prints: what one cell of it counted.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cell {
    /// `default`, or the feature it runs with off.
    #[serde(rename = "cell")]
    name: String,
    /// The units of each outcome.
    units: BTreeMap<String, u64>,
    /// Each row's whole count.
    counts: BTreeMap<String, u64>,
    /// The files a counting VFS opened, whose syncs it counts: what
    /// `--strace` holds its syncs to.
    #[serde(default)]
    vfs_files: Vec<PathBuf>,
    /// The syncs of those files the benchmark's own setup did through a
    /// counting VFS of its own: no row's work, and no bypass either.
    #[serde(default)]
    setup_syncs: u64,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    // A cap that is no whole number of 1 or more fails before anything runs.
    callgrind_at_once(arguments, environment)?;
    if arguments.strace {
        return strace::run(arguments, environment);
    }
    let root = environment.root.as_path();
    let register = register::read(root)?;
    let named: Vec<&str> = arguments
        .executable
        .iter()
        .filter_map(|named| named.split_once('=').map(|(family, _)| family))
        .collect();
    let uncounted = uncounted(&register.rows, &named, arguments);
    if !uncounted.is_empty() {
        return Ok(check::report(&uncounted));
    }
    let limits = register.limits;
    let rows: Vec<&Row> = ROWS
        .iter()
        .filter(|row| row.valgrind.is_some() == arguments.valgrind)
        // A family named by its executable is what this run asks for.
        .filter(|row| named.is_empty() || named.contains(&row.family))
        .filter(|row| arguments.asks_for(row.family, row.name))
        .collect();
    // A row not run yet is a counted row: no Valgrind run holds it.
    let not_run_yet: Vec<&NotRunYet> = NOT_RUN_YET
        .iter()
        .filter(|_| !arguments.valgrind)
        .filter(|row| named.is_empty() || named.contains(&row.family))
        .filter(|row| arguments.asks_for(row.family, row.name))
        .collect();
    if rows.is_empty() && not_run_yet.is_empty() {
        bail!("no counted row matches the family, path and row asked for");
    }
    if let Some(planted) = &arguments.plant
        && !rows.iter().any(|row| row.name == planted)
    {
        if let Some(row) = not_run_yet.iter().find(|row| row.name == planted) {
            bail!("{planted} has not run yet: {} builds its path", row.builder);
        }
        bail!("{planted} is no counted row of what runs");
    }
    let unheld = unheld(&rows, &register.rows);
    if !unheld.is_empty() {
        return Ok(check::report(&unheld));
    }
    for row in &not_run_yet {
        let limit = match limits.get(row.name) {
            Some(limit) => format!("limit {limit}"),
            None if !row.capped => "a ratchet with no figure yet".to_owned(),
            None => bail!("the Quality register has no row {}", row.name),
        };
        output::line(&format!(
            "{}: not run yet, {limit}, until {} builds its path",
            row.name, row.builder
        ));
    }
    let executables = executables(arguments, environment, &rows)?;
    if arguments.prove {
        return prove(&rows, &executables, &limits, arguments, environment);
    }
    let mut findings = Vec::new();
    let mut families: Vec<&str> = rows.iter().map(|row| row.family).collect();
    families.sort_unstable();
    families.dedup();
    for family in families {
        let of_family: Vec<&Row> = rows
            .iter()
            .copied()
            .filter(|row| row.family == family)
            // A plant's run answers for what it planted: it counts no other
            // row Valgrind would have to run for.
            .filter(|row| {
                arguments
                    .plant
                    .as_deref()
                    .is_none_or(|plant| plant == row.name)
            })
            .collect();
        // A family's benchmark plants only its own rows.
        let plant = arguments
            .plant
            .as_deref()
            .filter(|planted| of_family.iter().any(|row| row.name == *planted));
        let cells = family_cells(family, &rows, &executables, plant, arguments, environment)?;
        findings.extend(verdict(&of_family, &cells, &limits)?);
        findings.extend(
            not_run_yet
                .iter()
                .filter(|row| row.family == family)
                .filter_map(|row| done_though_not_run(row, &cells)),
        );
    }
    Ok(check::report(&findings))
}

/// Each enforced row of the register's counted work, of a family and name
/// this run asks for, that [`ROWS`] has no counter for: it would go uncounted
/// and pass.
fn uncounted(held: &[register::Held], named: &[&str], arguments: &Arguments) -> Vec<Finding> {
    held.iter()
        .filter(|row| row.enforced)
        .filter(|row| {
            row.families.iter().any(|family| {
                (named.is_empty() || named.contains(&family.as_str()))
                    && arguments.asks_for(family, &row.name)
            })
        })
        .filter(|row| {
            !row.counted_as
                .iter()
                .all(|name| ROWS.iter().any(|counter| counter.name == name))
        })
        .map(|row| {
            Finding::new(
                "row-without-counter",
                &row.name,
                "the register enforces this row, and `xtask counts`' ROWS has no counter for it",
            )
        })
        .collect()
}

/// Each of `rows`, the counters this run reads, that no row of the register
/// holds.
fn unheld(rows: &[&Row], held: &[register::Held]) -> Vec<Finding> {
    rows.iter()
        .filter(|counter| {
            !held
                .iter()
                .any(|row| row.counted_as.iter().any(|name| name == counter.name))
        })
        .map(|counter| {
            Finding::new(
                "counter-without-row",
                counter.name,
                "`xtask counts`' ROWS counts this row, and the Quality register holds no row of it",
            )
        })
        .collect()
}

/// Plants one extra unit of each of `rows`' work in turn and fails each
/// counter that does not see it.
fn prove(
    rows: &[&Row],
    executables: &BTreeMap<&'static str, PathBuf>,
    limits: &BTreeMap<String, Limit>,
    arguments: &Arguments,
    environment: &Environment,
) -> Result<ExitCode> {
    let mut findings = Vec::new();
    // A Valgrind plant does the same work whichever row it names, so one
    // run of a family's targets proves all its rows.
    let mut planted: BTreeMap<&str, Vec<Cell>> = BTreeMap::new();
    for row in rows {
        if !planted.contains_key(row.family) || !arguments.valgrind {
            let cells = family_cells(
                row.family,
                rows,
                executables,
                Some(row.name),
                arguments,
                environment,
            )?;
            planted.insert(row.family, cells);
        }
        let caught = verdict(&[row], &planted[row.family], limits)?
            .iter()
            .any(|finding| finding.kind == "over-limit" && finding.subject == row.name);
        output::line(&format!(
            "{}: one planted unit {}",
            row.name,
            if caught { "fails" } else { "passes unseen" }
        ));
        if !caught {
            findings.push(Finding::new(
                "counter-blind",
                row.name,
                "one planted unit of its work did not fail it",
            ));
        }
    }
    Ok(check::report(&findings))
}

/// What `family`'s rows counted, each extra unit of `plant` done.
fn family_cells(
    family: &str,
    rows: &[&Row],
    executables: &BTreeMap<&'static str, PathBuf>,
    plant: Option<&str>,
    arguments: &Arguments,
    environment: &Environment,
) -> Result<Vec<Cell>> {
    let at_once = callgrind_at_once(arguments, environment)?;
    if !arguments.valgrind {
        cells(&executables[family], plant, false, at_once)
    } else if let Some(executable) = executables.get(family) {
        cells(executable, plant, true, at_once)
    } else {
        valgrind_cells(family, rows, plant, environment)
    }
}

/// Runs `family`'s Valgrind targets for `rows` and answers each cell's
/// figures, as a counted benchmark prints its own.
fn valgrind_cells(
    family: &str,
    rows: &[&Row],
    plant: Option<&str>,
    environment: &Environment,
) -> Result<Vec<Cell>> {
    let Some(of_family) = FAMILIES.iter().find(|of| of.name == family) else {
        bail!("no family {family}");
    };
    let Some(benchmark) = &of_family.valgrind else {
        bail!("the family {family} has no benchmark under Valgrind");
    };
    let figures: Vec<(&Row, ValgrindFigure)> = rows
        .iter()
        .filter(|row| row.family == family)
        .filter_map(|row| match row.valgrind {
            Some(UnderValgrind::Benchmark(figure)) => Some((*row, figure)),
            Some(UnderValgrind::Example) | None => None,
        })
        .collect();
    let mut targets: Vec<&str> = figures.iter().map(|(_, figure)| figure.target).collect();
    targets.sort_unstable();
    targets.dedup();
    let mut measured = Vec::new();
    for target in targets {
        measured.extend(run_valgrind_target(
            of_family,
            benchmark,
            target,
            plant,
            environment,
        )?);
    }
    let read = |target: &str, id: String, figure: Figure| {
        let end = format!("{}/{id}", benchmark.function);
        measured
            .iter()
            .find(|one: &&Measured| {
                one.figure == figure
                    && one.subject.ends_with(&end)
                    && one.subject.contains(&format!("/{target}::"))
            })
            .map(|one| one.head)
    };
    Ok(benchmark
        .cells
        .iter()
        .map(|(name, suffix)| Cell {
            name: (*name).to_owned(),
            units: BTreeMap::from([("messages".to_owned(), benchmark.units)]),
            counts: figures
                .iter()
                .filter_map(|(row, figure)| {
                    let scenario = read(
                        figure.target,
                        format!("{}{suffix}", figure.scenario),
                        figure.figure,
                    )?;
                    let empty = read(
                        figure.target,
                        format!("{}{suffix}", benchmark.empty),
                        figure.figure,
                    )?;
                    Some((row.name.to_owned(), scenario.saturating_sub(empty)))
                })
                .collect(),
            vfs_files: Vec::new(),
            setup_syncs: 0,
        })
        .collect())
}

/// Runs one Valgrind target's benchmark, each extra unit of `plant` done, and
/// answers its figures.
fn run_valgrind_target(
    family: &Family,
    benchmark: &ValgrindBenchmark,
    target: &str,
    plant: Option<&str>,
    environment: &Environment,
) -> Result<Vec<Measured>> {
    let out = environment.root.join(VALGRIND_OUT);
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let report = out.join(format!("{}-{target}.jsonl", family.name));
    std::fs::File::create(&report).with_context(|| format!("creating {}", report.display()))?;
    // Absolute: `cargo bench` runs the benchmark in its package's folder,
    // where a relative path would make a `target/` of the package's own.
    let gungraun_home =
        std::path::absolute(out.join("gungraun")).context("resolving gungraun's folder")?;
    let mut command = crate::child_cargo::command(&environment.cargo);
    command
        .args(["bench", "--locked", "--package", family.package])
        .args(["--bench", target, "--"])
        .arg(format!("*::{}::*", benchmark.function))
        .current_dir(&environment.root)
        .env("GUNGRAUN_HOME", &gungraun_home)
        .stderr(Stdio::inherit());
    if let Some(plant) = plant {
        command.env(PLANT_VARIABLE, plant);
    }
    let status = perf::gungraun_output(&mut command, &report)?
        .status()
        .with_context(|| format!("running {}'s {target} benchmark", family.name))?;
    if !status.success() {
        bail!("{}'s {target} benchmark failed", family.name);
    }
    perf::read_gungraun_summaries(&[&report])
}

impl Arguments {
    /// Whether the family, path and row asked for hold the row `name` of
    /// `family`.
    fn asks_for(&self, family: &str, name: &str) -> bool {
        self.family.as_deref().is_none_or(|asked| asked == family)
            && self
                .path
                .as_deref()
                .is_none_or(|path| name.split('.').next() == Some(path))
            && self.row.as_deref().is_none_or(|asked| {
                asked == name
                    || name
                        .strip_prefix(asked)
                        .is_some_and(|kind| kind.starts_with('.'))
            })
    }
}

/// A finding when a cell did the work of `row`, which says it has not run: the
/// ticket that builds its path gives it its limit's verdict by moving it.
fn done_though_not_run(row: &NotRunYet, cells: &[Cell]) -> Option<Finding> {
    let (cell, count) = cells.iter().find_map(|cell| {
        let count = *cell.counts.get(row.name)?;
        (count > 0).then_some((cell, count))
    })?;
    Some(Finding::new(
        "row-listed-as-not-run",
        row.name,
        format!(
            "the cell `{}` counted {count} of its work: move it to `ROWS` in `xtask counts`",
            cell.name
        ),
    ))
}

/// Each family's executable that this run runs: the one `--executable`
/// names, else its built target. Under `--valgrind`, only a family whose
/// example reads a row, in the benchmarks' optimized build.
fn executables(
    arguments: &Arguments,
    environment: &Environment,
    rows: &[&Row],
) -> Result<BTreeMap<&'static str, PathBuf>> {
    let named: BTreeMap<&str, &str> = arguments
        .executable
        .iter()
        .filter_map(|named| named.split_once('='))
        .collect();
    let mut executables = BTreeMap::new();
    for family in FAMILIES {
        if !rows.iter().any(|row| {
            row.family == family.name
                && (!arguments.valgrind || matches!(row.valgrind, Some(UnderValgrind::Example)))
        }) {
            continue;
        }
        let executable = match named.get(family.name) {
            Some(path) => environment.path(Path::new(path)),
            None => build(&family, arguments.valgrind, environment)?,
        };
        executables.insert(family.name, executable);
    }
    Ok(executables)
}

/// Builds `family`'s target and answers its executable: in the benchmarks'
/// profile when it runs `optimized`, as what it reads under Valgrind is
/// read in the build `just perf` measures, else in the dev profile.
///
/// The optimized build also sets [`CALLGRIND_NAMES`], under which the wire's
/// functions are never inlined so that Callgrind names them, in a build
/// folder of its own: the product, and every other benchmark, keep them
/// inlined. A `RUSTFLAGS` in the environment would replace the setting, and
/// the run would then fail on a wire function it never saw entered.
fn build(family: &Family, optimized: bool, environment: &Environment) -> Result<PathBuf> {
    let mut command = crate::child_cargo::command(&environment.cargo);
    command
        .args(["build", "--message-format", "json-render-diagnostics"])
        .args(["-p", family.package, "--example", family.target]);
    if optimized {
        let folder = crate::metadata::target_directory(&environment.cargo, &environment.root)?
            .join(CALLGRIND_NAMES);
        command
            .args(["--profile", "bench", "--target-dir"])
            .arg(folder)
            .args([
                "--config",
                &format!("build.rustflags = [\"--cfg\", \"{CALLGRIND_NAMES}\"]"),
            ]);
    }
    let output = command
        .current_dir(&environment.root)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("building {}'s benchmark", family.name))?;
    if !output.status.success() {
        bail!("cargo build of {}'s benchmark failed", family.name);
    }
    let built = crate::child_cargo::messages(&output.stdout)?
        .into_iter()
        .find_map(|message| message.executable)
        .with_context(|| format!("cargo build reported no executable for {}", family.name))?;
    // Every family's example is `counts`, which Cargo leaves at one path, so
    // the next family's build would replace this one before it runs.
    let own = environment.root.join(EXECUTABLES_OUT).join(format!(
        "{}-{}{}",
        family.name,
        family.target,
        if optimized { "-bench" } else { "" }
    ));
    if let Some(folder) = own.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::copy(&built, &own)
        .with_context(|| format!("copying {} to {}", built.display(), own.display()))?;
    Ok(own)
}

/// What `executable` counted, each extra unit of `plant` done: its rows
/// read under Valgrind when `valgrind`, its counted rows else.
fn cells(
    executable: &Path,
    plant: Option<&str>,
    valgrind: bool,
    at_once: Option<NonZeroUsize>,
) -> Result<Vec<Cell>> {
    let mut command = Command::new(executable);
    if valgrind {
        command.arg("--valgrind");
    }
    if let Some(plant) = plant {
        command.args(["--plant", plant]);
    }
    printed_cells(command, executable, at_once)
}

/// The cap on the Callgrind processes a benchmark runs at once:
/// `--callgrind-at-once`, else `GOODFETCH_COUNTS_CALLGRIND_AT_ONCE`, else
/// none. Read before anything runs, so a value that is no whole number of 1
/// or more fails the run before it starts.
fn callgrind_at_once(
    arguments: &Arguments,
    environment: &Environment,
) -> Result<Option<NonZeroUsize>> {
    if arguments.callgrind_at_once.is_some() {
        return Ok(arguments.callgrind_at_once);
    }
    let Some(value) = &environment.callgrind_at_once else {
        return Ok(None);
    };
    match value.to_str().and_then(|text| text.parse().ok()) {
        Some(cap) => Ok(Some(cap)),
        None => bail!(
            "{CALLGRIND_AT_ONCE_VARIABLE} is `{}`, not a whole number of 1 or more",
            value.to_string_lossy()
        ),
    }
}

/// What `command`, which runs `executable` with at most `at_once` Callgrind
/// processes at once, printed of each cell.
fn printed_cells(
    mut command: Command,
    executable: &Path,
    at_once: Option<NonZeroUsize>,
) -> Result<Vec<Cell>> {
    if let Some(at_once) = at_once {
        command.env(CALLGRIND_AT_ONCE_VARIABLE, at_once.to_string());
    }
    let output = command
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("running {}", executable.display()))?;
    if !output.status.success() {
        bail!("the benchmark {} failed", executable.display());
    }
    let cells = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<Vec<Cell>, _>>()
        .with_context(|| format!("reading what {} printed", executable.display()))?;
    if cells.is_empty() {
        bail!("the benchmark {} printed no cell", executable.display());
    }
    Ok(cells)
}

/// One of the units `unit` names, as a figure's line says it.
fn one_of(unit: &str) -> &str {
    match unit {
        LINKS_BATCHES | HELD_LINKS_BATCHES => "links batch",
        "audit_entries" => "audit_entry",
        "later_audit_entries" => "later_audit_entry",
        "entries" => "entry",
        "batched_pages" | "full_pages" | CALLGRIND_PAGES => "page",
        ADDED_PAGES => "page added",
        ATTEMPTS_ENDED => "attempt",
        BATCHED_LEASES | SPILLED_LEASES => "lease",
        CALLGRIND_CRAWLS => "crawl",
        HANDLED_PAGE_HUNDREDTHS => "hundredth of a handled page",
        SECOND_COPIES => "second copy",
        unit => unit.trim_end_matches('s'),
    }
}

/// Prints each row's figure in each of `cells` and answers what fails.
fn verdict(
    rows: &[&Row],
    cells: &[Cell],
    limits: &BTreeMap<String, Limit>,
) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for row in rows {
        for cell in cells {
            let limit = register::limit(limits, row.name, row.family, &cell.name)
                .with_context(|| format!("the Quality register has no row {}", row.name))?;
            let (Some(count), Some(units)) = (cell.counts.get(row.name), cell.units.get(row.unit))
            else {
                findings.push(Finding::new(
                    "row-not-counted",
                    row.name,
                    format!(
                        "the cell `{}` printed no count or no {}",
                        cell.name, row.unit
                    ),
                ));
                continue;
            };
            if *units == 0 && !row.happens {
                if *count == 0 {
                    output::line(&format!(
                        "{} [{}]: no {} in this cell",
                        row.name, cell.name, row.unit
                    ));
                } else {
                    findings.push(Finding::new(
                        "over-limit",
                        row.name,
                        format!(
                            "{count} with no {} in the cell `{}`, over its limit of {limit}",
                            row.unit, cell.name
                        ),
                    ));
                }
                continue;
            }
            // A feature off may take the whole outcome away, which is what
            // its cell shows.
            if *units == 0 && cell.name != DEFAULT_CELL {
                output::line(&format!("{} [{}]: no {}", row.name, cell.name, row.unit));
                continue;
            }
            if *units == 0 || (row.happens && *count == 0 && cell.name == DEFAULT_CELL) {
                findings.push(Finding::new(
                    "work-not-done",
                    row.name,
                    format!(
                        "the cell `{}` did none of the work it counts: {count} over {units} {}",
                        cell.name, row.unit
                    ),
                ));
                continue;
            }
            let figure = Limit::figure(*count, *units);
            output::line(&format!(
                "{} [{}]: {figure} per {}, limit {limit} ({count} over {units})",
                row.name,
                cell.name,
                one_of(row.unit),
            ));
            let limit_held = if row.valgrind.is_some()
                || [
                    CLEAN_UP_INSTRUCTIONS,
                    SELECTOR_COMPILATION_INSTRUCTIONS,
                    GENERIC_SET_INSTRUCTIONS,
                ]
                .contains(&row.name)
            {
                limit.with_valgrind_tolerance()
            } else {
                *limit
            };
            if limit_held.exceeded_by(*count, *units) {
                findings.push(Finding::new(
                    "over-limit",
                    row.name,
                    format!(
                        "{figure} per {} in the cell `{}`, over its limit of {limit}",
                        one_of(row.unit),
                        cell.name
                    ),
                ));
            }
        }
    }
    Ok(findings)
}

#[cfg(test)]
mod tests;
