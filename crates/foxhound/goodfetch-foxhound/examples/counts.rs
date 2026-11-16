//! B1's counts cell (ADR 0140), beside the in-memory crawl's heap cell: runs
//! `benches/scenarios.rs`' crawl once with its default features on and once
//! per feature on its paths that can be switched off, and prints what each
//! counted as one JSON line, which `xtask counts` divides and holds to the
//! Quality register's limits. An example and not a `[[bench]]`, as `xtask
//! perf` runs every bench target under gungraun: counts need no optimized
//! build.
//!
//! `F3.prepares` is what no wrapper sees (the spec's "What wrappers cannot
//! see"): each outcome's is Callgrind's call count of SQLite's prepare inside
//! the one function that does that outcome's admission. Each cell runs small
//! under Valgrind once per outcome for it, so this needs Valgrind; without it
//! those counts are not printed, and `xtask counts` says so.
//!
//! `--plant <row>` does one extra unit of that row's work per page served,
//! through its one counted function, in every cell: a row's limit is its
//! worst cell's, often not the default's, which a unit more per page in the
//! default cell alone stays under. The crawl fails here if the work it counts
//! did not happen, or if a feature off did not take its work away.
//!
//! A page's rows of [`FROM_FIRST_PAGE`] leave out what the crawl's start did
//! before its first page: a selector checked and a locale's forms built once
//! per crawl are no page's work (ADRs 0138 and 0140), and a build moved onto
//! each page counts at once.
//!
//! The run file's work no wrapper sees is read from outside it too: syncs
//! through a counting VFS under the run file, and `F2.prepares` from
//! Callgrind's call count of `sqlite3_prepare_v3` beneath the `run_state`
//! writer's functions, for which each cell runs this example again as
//! `--callgrind <cell>`, on pages of about 1 kB, as Callgrind takes minutes
//! on a crawl with no large page and far longer on the whole site. A page's
//! size moves no statement. What each page more prepares is the row: each
//! cell runs a second time as `--callgrind <cell> --shorter`, on a site
//! linking [`SHORTER`] of its pages, and the row is the difference between
//! the two runs over the pages the whole site adds, so what a Run prepares
//! once (its file's tables, each statement's first preparation, its end)
//! counts for no page, and a statement its cache let go counts again
//! (ADR 0138). Without Valgrind `F2.prepares` goes unprinted, which `xtask
//! counts` fails as a row not counted.
//!
//! `F3.counted_work_constructions` is read from the same two runs, as a
//! `CountedWork` being built has no instance to count on: Callgrind's call
//! count of its constructor on the whole site, less on [`SHORTER`] of its
//! pages, over the pages the whole site adds. The engine's own and each
//! owner's placeholder are built once and count for no page; one an
//! admission builds counts for its page. The whole run must see the engine
//! build its own, or the constructor's name has changed. It goes unprinted
//! without Valgrind too.
//!
//! The cells of the run file's own switches (`concurrency_1`,
//! `concurrency_16`, `durability_fast`, `durability_full`,
//! `durability_off`) and the handler's cell crawl without the Frontier's
//! outcomes beyond a links batch: a hop's, a sitemap's and a Payload's rows
//! say "no hops" there.
//!
//! `F2.page_waits` divides by the pages of the cells at a level that batches
//! attempts, the unit `batched_pages`, and `F2.page_waits.full` by those of
//! the cell at `full`, `full_pages`: each counts nothing in the other's
//! cells, nor at `fast`. The cells at `batch` (`default`), `off` and `full`
//! are each made two calls once their crawl started that no page will make
//! again, a `key_value_store.set` outside a handler and a `run.add_seed` of
//! a duplicate: `F2.batches` is the batches the writer sealed before their
//! pages or their interval, less one for each such call, per Run.
//!
//! `F3.lease_reads` divides by the leases, a page's and a sitemap's, of the
//! cells at a level that batches attempts whose memory budget holds their
//! pending work items, the unit `batched_leases`, and
//! `F3.lease_reads.spilled` by those of the cell `memory_budget_0`, whose
//! budget holds none, `spilled_leases` (#2169). The cell
//! `memory_budget_1mib` runs at the test budget of 1 MiB, which the site
//! fits in.
//!
//! `F3.hop_round_trips.end` divides by the redirected chains that ended on
//! their document, the unit `chains`: the site's redirects are one hop
//! each, so its chains are its hops.
//!
//! `F3.actor_round_trips` and `F3.in_flight_serializations` divide by the
//! attempts that ended, the unit `attempts_ended`: the attempts the site
//! served a page to and the one that read each sitemap, which is a Work
//! item's attempt as a page's is and serves no page. No cell reads its Run's
//! progress or subscribes to the engine's events, so the second counts what
//! the attempts alone serialize.
//!
//! The cell `varying_parameter` crawls, beside the site, as many pages
//! `/item?id=<n>` as the site has pages, each its own, with learning on: the
//! input `F2.evidence_reads` and `F2.admission_evidence` are read on, and its
//! `--shorter` run serves half of them, so `F2.prepares` sees what each of
//! them prepares too. The cell `learning_off` crawls the same site with
//! `frontier.learning` off: `F2.learning_off_evidence` is the evidence rows
//! those two counted in it, over its pages, the unit `learning_off_pages`.
//!
//! The default cell's crawl meets a Limit of `requests`, so each request is
//! charged through the counted ledger (E3), and fails if nothing was; B3
//! holds E3's rows. The other cells meet none, so the ledger's prepares stay
//! out of the run-state cells' `F2.prepares`.
//!
//! The Run-trace rows count per entry: each cell prints, beside its pages,
//! the entries its Run trace holds, read from the trace's files once its
//! engine stopped, and fails if it wrote none.
//!
//! The site serves its robots.txt (E6). `E6.opens` is the Politeness file's
//! opens after the first page and the statements Callgrind sees prepared
//! beneath its functions in the `--callgrind` run, its opening's left out,
//! and goes unprinted without Valgrind too. `E6.off_work` is what the E6
//! counters counted in the cell with Politeness and robots.txt off, over its
//! attempts.
//!
//! `--strace` runs the cells as `xtask counts --strace` watches them, under
//! `strace`, with nothing beside them: no run under Callgrind, so their
//! prepares go unprinted. Each cell prints the files its counting VFS opened,
//! whose syncs `strace` is held to. `--strace --bypass <work>` has the default
//! cell alone do one raw unit of that work per page beside its counted
//! function (an `open` of a Run-trace file, a `directory` of a Run trace
//! created, a `sync` of a file the counting VFS holds), which no counter sees
//! and `strace` does, so the cross-check proves it fails.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]
#![expect(
    clippy::print_stderr,
    reason = "the counts harness (ADR 0140): why a row goes unprinted, which `xtask counts` fails, beside its finding"
)]
#![expect(
    clippy::print_stdout,
    reason = "the counts harness (ADR 0140): the benchmark's one output, which `xtask counts` reads"
)]

#[path = "../benches/plants.rs"]
mod plants;
#[path = "../benches/scenarios.rs"]
#[expect(
    dead_code,
    reason = "the fixture's plain crawl, which the heap benchmark runs and this one does not"
)]
mod scenarios;

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::metrics::Metrics;
use goodfetch_protocol::run::durability::{
    DURABILITY_BATCH, DURABILITY_FAST, DURABILITY_FULL, DURABILITY_OFF, DURABILITY_PERIODIC,
};
use goodfetch_test_support::CountingVfs;
use goodfetch_test_support::callgrind;
use plants::{Counted as PlantOn, politeness_plant, run_state_plant};
use scenarios::{
    AfterStart, BARRIER_CALLS, Frontier, Options, PAGES, Plant, body_bytes, crawl_with, entries,
    page_analysis_plant, run_trace_plant,
};
use serde_json::{Map, Value, json};

/// The Limit of `requests` the default cell's crawl meets, far above what it
/// charges.
const LIMIT: u64 = 1_000_000;

/// Each outcome's `F3.prepares`, and the one function that does its
/// admission: a links batch's is its attempt's commit's, which holds all the
/// admission work a commit does, a batch deferred to it and what a staged
/// one decided alike (#2245), named as Callgrind names a method.
const PREPARES: [(&str, &str); 3] = [
    ("F3.prepares", "<goodfetch_crawl::staging::Staged>::admit"),
    ("F3.prepares.hop", "goodfetch_frontier::identity::admit_hop"),
    (
        "F3.prepares.sitemap",
        "goodfetch_crawl::work_items::read_sitemap",
    ),
];

/// SQLite's prepare, whose calls Callgrind counts.
const PREPARE: &str = "sqlite3_prepare_v3";

/// [`planted_prepare`], as Callgrind names it.
const PLANTED_PREPARE: &str = "counts::planted_prepare";

/// The one prepare a planted `F3.prepares` does, in a function of its own, as
/// Callgrind counts only what its named functions call.
#[inline(never)]
fn planted_prepare(connection: &rusqlite::Connection) {
    connection
        .prepare("SELECT 1")
        .expect("invariant: a literal statement");
}

/// The plant of the row named `name`, a run-state row's on its own file
/// through the VFS `vfs`, a Politeness permit row's on a Politeness file of
/// its own.
fn plant(name: &str, work: &CountedWork, vfs: &str) -> Plant {
    if name.starts_with("F2.") {
        return run_state_plant(name, &PlantOn { work, vfs });
    }
    if name.starts_with("E6.") {
        return politeness_plant(name, work);
    }
    if name == LEASE_READS_SPILLED {
        return counting(work.clone(), Row::LeaseReads);
    }
    if name == CONSTRUCTIONS {
        return Arc::new(|_: &str| {
            std::hint::black_box(CountedWork::default());
        });
    }
    let work = work.clone();
    if name == "F1.parses" {
        return Arc::new(move |head: &str| {
            let _ = work.parse_html(Row::PageParses, head);
        });
    }
    if name == "F4.validator_builds" {
        return Arc::new(move |_: &str| {
            let _ = goodfetch_dataset::counted_validator(&work, &json!({}));
        });
    }
    if PREPARES.iter().any(|(row, _)| *row == name) {
        return Arc::new(move |_: &str| {
            let connection = rusqlite::Connection::open_in_memory()
                .expect("invariant: SQLite opens a database in memory");
            planted_prepare(&connection);
        });
    }
    let Some(row) = Row::ALL.into_iter().find(|row| row.name() == name) else {
        panic!("no plant for the row {name}");
    };
    page_analysis_plant(row, &work)
        .or_else(|| run_trace_plant(row, &work))
        .unwrap_or_else(|| counting(work, row))
}

/// The rows counted from the crawl's first page on, not from its start: the
/// Politeness file opens as the crawl starts.
const FROM_FIRST_PAGE: [Row; 7] = [
    Row::PageSelectorCompilations,
    Row::PageReadabilityRuns,
    Row::PageFileOpens,
    Row::PageLocaleFormBuilds,
    Row::PagePluginSerializations,
    Row::PagePluginAnswerParses,
    Row::PolitenessOpens,
];

/// The field whose Value transform has the crawl build a locale's forms:
/// the `h1`'s number, under `de-DE`, which only the pages holding one have.
fn number_field() -> Map<String, Value> {
    let field = json!({
        "locator": { "kind": "css", "selector": "h1" },
        "match": { "pattern": "[0-9]+", "group": 0 },
        "transform": { "kind": "number", "locale": "de-DE" },
    });
    Map::from_iter([("number".to_owned(), field)])
}

/// The run-state row Callgrind reads over a whole cell.
const RUN_STATE_PREPARES: &str = "F2.prepares";

/// The row Callgrind reads over a whole cell as the calls of
/// [`CONSTRUCTION`] each page adds.
const CONSTRUCTIONS: &str = "F3.counted_work_constructions";

/// `CountedWork`'s constructor, as Callgrind names it.
const CONSTRUCTION: &str =
    "<goodfetch_counted_work::CountedWork as core::default::Default>::default";

/// The row Callgrind adds the Politeness file's preparations to: those
/// beneath its functions, but its opening's.
const POLITENESS_OPENS: &str = "E6.opens";
const POLITENESS: &str = "goodfetch_engine::politeness::";
const OPENING: [&str; 2] = ["politeness::open_file", "politeness::create_tables"];

/// How many callers up Callgrind looks for the Politeness file's functions:
/// more than SQLite's wrapper, a contribution's rule and the file's actor take.
const CALLERS: u32 = 32;

/// The row of the cell with Politeness and robots.txt off, and that cell.
const OFF_WORK: &str = "E6.off_work";
const OFF_CELL: &str = "politeness_and_robots_txt_off";

/// The rows a cell with robots.txt off, and so pacing nothing under the
/// fixture's delay of 0, counts fewer of: no robots.txt read or parsed
/// (#1919), and no Politeness store consulted (#1920).
const OFF_REMOVES: &[Row] = &[
    Row::PolitenessTransactions,
    Row::RobotsParses,
    Row::RobotsReads,
];

/// The Politeness permit's rows a counter counts, whose sum in [`OFF_CELL`]
/// is its work.
const POLITENESS_ROWS: [Row; 5] = [
    Row::PolitenessTransactions,
    Row::PolitenessOpens,
    Row::PolitenessScriptSends,
    Row::RobotsParses,
    Row::RobotsReads,
];

/// The origins the fixture's robots.txt is read for: the site's one.
const ORIGINS: u64 = 1;

/// The owner of the run file's statements: each preparation `F2.prepares`
/// counts has one of its functions among its [`CALLERS`] nearest callers.
const RUN_STATE: &str = "goodfetch_engine::run_state::";

/// How many of [`PAGES`] a `--shorter` run's site links: half of them.
const SHORTER: usize = PAGES.len() / 2;

/// The row the counting VFS reads.
const SYNCS: &str = "F2.syncs";

/// A page's waits for the run file's writer, held at the levels that batch
/// attempts (`batch` and `off`) and, with a limit of its own, at `full`:
/// each row counts in its own cells alone, and none in the others (ADR
/// 0140's note on batching).
const PAGE_WAITS: &str = "F2.page_waits";
const PAGE_WAITS_FULL: &str = "F2.page_waits.full";

/// The Run state's read transactions on the run file per lease, held at
/// the levels that batch attempts, within the memory budget (#2169): none
/// counts in the other cells, whose leases read their rows.
const LEASE_READS: &str = "F3.lease_reads";

/// The same past the memory budget, in the cell whose budget holds no
/// pending work item ([`MEMORY_BUDGET_0`]): at most one a lease (ADR
/// 0140's F3 contract), from the first page.
const LEASE_READS_SPILLED: &str = "F3.lease_reads.spilled";

/// The batches the writer sealed before their plan, less the one each of
/// the crawl's [`BARRIER_CALLS`] may write: what is left is the Run's own,
/// at most its start's and its end's.
const BATCHES: &str = "F2.batches";

/// The cells whose crawl is made [`BARRIER_CALLS`], one at each level
/// `F2.batches` and `F2.page_waits` are held at: their batches for calls
/// that no page will make again are counted, and left out of the row.
const BARRIER_CELLS: [&str; 3] = ["default", "durability_off", "durability_full"];

/// The Durability level `cell` runs at, `batch` when it sets none.
fn level(cell: &Cell) -> &'static str {
    match cell.settings["run_state"]["durability"].as_str() {
        None => DURABILITY_BATCH,
        Some(DURABILITY_FULL) => DURABILITY_FULL,
        Some(DURABILITY_FAST) => DURABILITY_FAST,
        Some(DURABILITY_PERIODIC) => DURABILITY_PERIODIC,
        Some(DURABILITY_OFF) => DURABILITY_OFF,
        Some(other) => panic!("the cell {} sets the level {other}", cell.name),
    }
}

/// The units the rows of no outcome of this fixture divide by: one Run, and
/// no Effect until the Effect journal lands (#1550).
const RUNS: u64 = 1;
const EFFECTS: u64 = 0;

/// The paragraphs of the fixture's smallest page, about 1 kB: all a
/// `--callgrind` run serves a page.
const CALLGRIND_PAGE: usize = PAGES[0];

/// How a run is watched: counted alone, with its prepares read under Callgrind
/// beside it, or under `strace` with nothing beside it and a bypass planted.
#[derive(Clone, Copy)]
enum Watched<'a> {
    Counted,
    Traced { bypass: Option<&'a str> },
}

/// The plant of `bypass`: one raw unit of its work per page beside the
/// counted function, in a Run trace or on a file of `vfs` of its own under
/// `home`, made first through what counts.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140's strace cross-check: each bypass opens, creates or syncs beside the counted function on purpose, which no counter sees"
)]
fn bypass(bypass: &str, home: &Path, work: &CountedWork, vfs: &str) -> Plant {
    let folder = home.join("bypass").join("trace");
    work.create_dir_all(Row::RunTraceFileOpens, &folder)
        .expect("invariant: the bypass's Run trace is created");
    match bypass {
        "open" => Arc::new(move |_: &str| {
            drop(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(folder.join("bypass.jsonl"))
                    .expect("invariant: the bypass's file opens"),
            );
        }),
        "directory" => Arc::new(move |_: &str| {
            std::fs::create_dir_all(&folder).expect("invariant: the bypass's folder is there");
        }),
        "sync" => {
            let file = home.join("bypass.sqlite");
            rusqlite::Connection::open_with_flags_and_vfs(
                &file,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                    | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
                vfs,
            )
            .and_then(|connection| connection.execute_batch("CREATE TABLE items (value INTEGER)"))
            .expect("invariant: the bypass's database is made through the counting VFS");
            Arc::new(move |_: &str| {
                std::fs::File::open(&file)
                    .and_then(|opened| opened.sync_all())
                    .expect("invariant: the bypass's database syncs");
            })
        }
        other => panic!("no bypass {other}: open, directory or sync"),
    }
}

/// The plant of a row counted at its seam, which no function wraps.
fn counting(work: CountedWork, row: Row) -> Plant {
    Arc::new(move |_: &str| work.count(row))
}

/// One cell: a name, the settings it runs with beside the fixture's, the
/// Frontier's outcomes it meets beyond a links batch, and the rows it must
/// count fewer of than the default cell: a feature's work goes with the
/// feature (ADR 0140).
struct Cell {
    name: &'static str,
    settings: Value,
    frontier: Option<Frontier>,
    removes: &'static [Row],
    /// Whether it must sync less than the default cell.
    removes_syncs: bool,
    handler: bool,
    /// Whether its crawl meets a Limit, which each request charges (E3).
    limit: bool,
}

/// The Frontier's outcomes, the Payload under its schema.
const FRONTIER: Option<Frontier> = Some(Frontier {
    schema: true,
    items: 0,
    second_hop: false,
});

/// The Frontier's outcomes with a seed whose Payload has no schema to fit.
const UNTYPED: Option<Frontier> = Some(Frontier {
    schema: false,
    items: 0,
    second_hop: false,
});

/// How many items the cells [`VARYING_PARAMETER`] and [`LEARNING_OFF`]
/// serve: as many as their pages.
const ITEMS: usize = PAGES.len();

/// The Frontier's outcomes with [`ITEMS`] pages `/item?id=<n>` beside the
/// site's: URLs whose query varies, which learning reads and writes its
/// evidence for.
const WITH_ITEMS: Option<Frontier> = Some(Frontier {
    schema: true,
    items: ITEMS,
    second_hop: false,
});

/// The cell whose site also serves [`ITEMS`] pages `/item?id=<n>`, each its
/// own, with learning on: the second refuses `id`, and each later one reads
/// the pair that refused it and no more (`F2.evidence_reads`, ADR 0005's
/// note of 2026-11-10). Its crawl fails if learning read no evidence.
const VARYING_PARAMETER: &str = "varying_parameter";

fn varying_parameter() -> Cell {
    Cell {
        name: VARYING_PARAMETER,
        settings: scoped(json!({})),
        frontier: WITH_ITEMS,
        removes: &[],
        removes_syncs: false,
        handler: false,
        limit: false,
    }
}

/// A glob in the Scope, which is what a Frontier's reads of it compile.
fn scoped(mut settings: Value) -> Value {
    settings["crawl"] = json!({ "scope": { "exclude": ["http://pages.test/never/**"] } });
    settings
}

/// The cell `name` of the Durability level `level`, on the fixture's own
/// site, which syncs less than the default cell when `removes_syncs`.
fn durability(name: &'static str, level: &str, removes_syncs: bool) -> Cell {
    Cell {
        name,
        settings: json!({ "run_state": { "durability": level } }),
        frontier: None,
        removes: &[],
        removes_syncs,
        handler: false,
        limit: false,
    }
}

/// The cell whose memory budget holds nothing: every lease reads the run
/// file, counted on [`LEASE_READS_SPILLED`] and not on [`LEASE_READS`].
const MEMORY_BUDGET_0: &str = "memory_budget_0";

/// The cell `name` at `run_state.memory.budget: budget` (#2169), with the
/// Frontier's outcomes beyond a links batch, as the default cell meets
/// them.
fn memory_budget(name: &'static str, budget: u64) -> Cell {
    Cell {
        name,
        settings: scoped(json!({ "run_state": { "memory": { "budget": budget } } })),
        frontier: FRONTIER,
        removes: &[],
        removes_syncs: false,
        handler: false,
        limit: false,
    }
}

/// The cell with `frontier.learning` off, on the site of
/// [`VARYING_PARAMETER`]: its admissions read and write no evidence
/// ([`LEARNING_OFF_EVIDENCE`], #2169).
const LEARNING_OFF: &str = "learning_off";

/// The rows of learning's evidence the cell [`LEARNING_OFF`] read and
/// wrote, over its pages: 0, as learning off does no evidence work.
const LEARNING_OFF_EVIDENCE: &str = "F2.learning_off_evidence";

/// The counters of learning's evidence, whose sum in [`LEARNING_OFF`] is
/// its evidence work.
const EVIDENCE_ROWS: [Row; 2] = [Row::EvidenceReads, Row::AdmissionEvidence];

/// The cell with no format asked and `frontier.learning` off: the crawl
/// runs neither the clean-up nor the main-content step (ADR 0138), and no
/// reader of its untouched page needs a tree, so each page is read in one
/// streaming pass and parsed none (#1911). It compiles no selector a page,
/// as no page of the default cell does since the engine keeps its
/// compilations (#1912).
fn learning_off() -> Cell {
    Cell {
        name: LEARNING_OFF,
        settings: scoped(json!({ "frontier": { "learning": false } })),
        frontier: WITH_ITEMS,
        removes: &[Row::PageParses, Row::PageReadabilityRuns],
        removes_syncs: false,
        handler: false,
        limit: false,
    }
}

/// The cells of the per-page features ADR 0138's inventory switches off,
/// each against the default cell: the Provenance log, whose Run trace then
/// holds no entry; the Records, none of which is then validated, serialized
/// or committed, nor its fields read; and the Block classification, which
/// then reads no document (#1921); and the Block classification with
/// `frontier.learning` off, scenario 1's combination, whose pages no reader
/// needs a tree of and which then hands none to the analysis pool (#2189).
fn switched_off() -> [Cell; 4] {
    [
        Cell {
            name: "provenance_off",
            settings: scoped(json!({ "provenance": { "log": false } })),
            frontier: FRONTIER,
            removes: &[Row::RunTraceSpawns, Row::RunTraceFileOpens],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        Cell {
            name: RECORDS_OFF_CELL,
            settings: scoped(json!({ "dataset": { "records": false } })),
            frontier: FRONTIER,
            removes: &[Row::RecordValidations, Row::RecordSerializations],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        Cell {
            name: "block_classification_off",
            // The fixture's `fetch` with the switch beside it: a cell's
            // settings replace each of the fixture's top-level keys.
            settings: scoped(json!({
                "fetch": { "tier": "http", "block_classification": false },
            })),
            frontier: FRONTIER,
            removes: &[Row::PageBlockClassifications],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        Cell {
            name: "learning_and_block_classification_off",
            settings: scoped(json!({
                "frontier": { "learning": false },
                "fetch": { "tier": "http", "block_classification": false },
            })),
            frontier: FRONTIER,
            removes: &[
                Row::PageParses,
                Row::PageReadabilityRuns,
                Row::PageBlockClassifications,
                Row::PageAnalysisHandoffs,
            ],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
    ]
}

/// The cell that keeps no Record, whose crawl's checks of its Records it
/// skips.
const RECORDS_OFF_CELL: &str = "records_off";

/// Whether the cell's Run trace holds a Provenance log.
fn logs_provenance(cell: &Cell) -> bool {
    cell.settings["provenance"]["log"] != json!(false)
}

fn cells() -> Vec<Cell> {
    let mut cells = vec![
        Cell {
            name: "default",
            settings: scoped(json!({})),
            frontier: FRONTIER,
            removes: &[],
            removes_syncs: false,
            handler: false,
            limit: true,
        },
        Cell {
            name: "filter_lists_off",
            settings: scoped(json!({ "filter_lists": {
                "ads": false, "trackers": false, "annoyances": false, "first_party_scripts": false,
            } })),
            frontier: FRONTIER,
            // The crawl asks for no format, so it runs no clean-up with the
            // Filter lists on either (ADR 0138's demand plan).
            removes: &[],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        learning_off(),
        Cell {
            name: "document_tier_handler",
            settings: json!({}),
            frontier: None,
            removes: &[],
            removes_syncs: false,
            handler: true,
            limit: false,
        },
        Cell {
            name: "scope_globs_off",
            settings: json!({}),
            frontier: FRONTIER,
            // The Scope's globs compile once, as the crawl starts (#1896):
            // a page compiles none whichever the Scope.
            removes: &[],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        Cell {
            name: "payload_schema_off",
            settings: scoped(json!({})),
            frontier: UNTYPED,
            // The schema compiles once, as the crawl starts (#1896).
            removes: &[Row::PayloadValidations],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        // A transaction counts one job whatever group it commits in (ADR
        // 0138's note), so a row is read where commits group least and where
        // they group most. The crawl's own jobs differ between the two: one
        // request at a time runs a delivery pass per request. Each is held to
        // the limit, so neither hides the other's work.
        Cell {
            name: "concurrency_1",
            settings: json!({ "politeness": { "domain": { "delay": 0, "concurrency": 1 } } }),
            frontier: None,
            removes: &[],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        Cell {
            name: "concurrency_16",
            settings: json!({ "politeness": { "domain": { "delay": 0, "concurrency": 16 } } }),
            frontier: None,
            removes: &[],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        durability("durability_fast", "fast", true),
        // The levels `F2.page_waits` and `F2.batches` are held at beside
        // the default's `batch` (ADR 0142), on the same site as
        // `durability_fast`.
        durability("durability_full", "full", false),
        durability("durability_off", "off", true),
        // ADR 0140's F3 rows at the test budget of 1 MiB, which the
        // fixture's crawl fits in as at the default, and at 0, where it
        // fits in none from the first page (#2169).
        memory_budget("memory_budget_1mib", 1 << 20),
        memory_budget(MEMORY_BUDGET_0, 0),
        // robots.txt off reads and parses no robots.txt (#1919), and with
        // the fixture's delay of 0 waits out no Crawl-delay, so neither cell
        // paces its domains: admission consults no Politeness store and
        // takes no Permit (#1920).
        Cell {
            name: "robots_txt_off",
            settings: json!({ "site_preferences": { "respect_robots_txt": false } }),
            frontier: None,
            removes: OFF_REMOVES,
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        Cell {
            name: OFF_CELL,
            settings: json!({
                "politeness": { "domain": { "delay": 0, "concurrency": null } },
                "site_preferences": { "respect_robots_txt": false },
            }),
            frontier: None,
            removes: OFF_REMOVES,
            removes_syncs: false,
            handler: false,
            limit: false,
        },
    ];
    cells.extend(switched_off().into_iter().chain([varying_parameter()]));
    cells
}

/// What a cell's runs under Callgrind prepared and built.
#[derive(Clone, Copy)]
struct Prepared {
    /// Beneath the Politeness file's functions, its opening's left out, on
    /// the whole site: `E6.opens`' part.
    politeness: u64,
    /// Beneath the `run_state` writer's, on the whole site less on
    /// [`SHORTER`] of its pages: `F2.prepares`, over [`added_pages`].
    per_added_pages: u64,
    /// The `CountedWork` values built on the whole site less on [`SHORTER`]
    /// of its pages: `F3.counted_work_constructions`, over [`added_pages`].
    constructions: u64,
}

/// What one cell counted.
struct Counted {
    name: &'static str,
    pages: u64,
    /// The pages its whole site serves beyond a `--shorter` run's.
    added_pages: u64,
    hops: u64,
    sitemaps: u64,
    /// The entries its Run trace holds, read from its files.
    entries: u64,
    work: CountedWork,
    /// What [`FROM_FIRST_PAGE`] held when the first page was asked for.
    started: Arc<OnceLock<Vec<u64>>>,
    removes: &'static [Row],
    /// Each outcome's `F3.prepares`, when Valgrind ran.
    prepares: BTreeMap<&'static str, u64>,
    attempts: u64,
    /// Every request the site was sent, each under a Politeness permit.
    requests: u64,
    syncs: u64,
    /// The files the counting VFS opened, whose syncs `syncs` holds.
    vfs_files: Vec<PathBuf>,
    /// The syncs the fixture's own setup did through a counting VFS of its
    /// own, on files of `vfs_files`: no row's work.
    setup_syncs: u64,
    removes_syncs: bool,
    /// `F2.prepares`, and the preparations beneath the Politeness file's
    /// functions, when Valgrind ran.
    prepare_calls: Option<Prepared>,
    /// Whether it met the Frontier's outcomes beyond a links batch.
    frontier: bool,
    /// Whether a handler served its pages, following no links.
    handler: bool,
    /// Its Durability level.
    level: &'static str,
    /// The calls no page will make again its crawl was made.
    barrier_calls: u64,
}

impl Counted {
    /// What this cell counted on `row`.
    fn count(&self, row: Row) -> u64 {
        let before = FROM_FIRST_PAGE
            .iter()
            .position(|counted| *counted == row)
            .map_or(0, |at| self.started.get().map_or(0, |started| started[at]));
        self.work.read(row) - before
    }

    /// Whether its memory budget holds no pending work item.
    fn spills(&self) -> bool {
        self.name == MEMORY_BUDGET_0
    }
}

/// What a crawl of a cell answered and counted, what [`FROM_FIRST_PAGE`]
/// held when its first page was asked for, its run file's syncs and the
/// entries its Run trace holds.
struct Crawl {
    crawled: scenarios::Crawled,
    work: CountedWork,
    started: Arc<OnceLock<Vec<u64>>>,
    syncs: u64,
    vfs_files: Vec<PathBuf>,
    setup_syncs: u64,
    entries: u64,
}

/// The pages a crawl serves: whole, each without its paragraphs (a
/// `--small-cell` run), or none past [`CALLGRIND_PAGE`]'s (a `--callgrind`
/// run), on the whole site or on [`SHORTER`] of its pages (`--shorter`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pages {
    Whole,
    Small,
    Callgrind,
    CallgrindShorter,
}

/// The crawl of `cell`, on an engine and in a home of its own, with the row
/// `planted` or the bypass `bypassed` planted.
fn crawl(cell: &Cell, planted: Option<&str>, bypassed: Option<&str>, pages: Pages) -> Crawl {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let vfs = CountingVfs::register();
    let setup = CountingVfs::register();
    let settings: Map<String, Value> = match &cell.settings {
        Value::Object(settings) => settings.clone(),
        other => panic!("a cell's settings are an object, not {other}"),
    };
    let planted = planted
        .map(|name| plant(name, &work, vfs.name()))
        .or_else(|| bypassed.map(|name| bypass(name, home.path(), &work, vfs.name())));
    let started = Arc::new(OnceLock::new());
    let first_page: Plant = {
        let (work, started) = (work.clone(), Arc::clone(&started));
        Arc::new(move |head: &str| {
            started.get_or_init(|| FROM_FIRST_PAGE.map(|row| work.read(row)).to_vec());
            if let Some(planted) = &planted {
                planted(head);
            }
        })
    };
    let crawled = crawl_with(
        home.path(),
        std::time::UNIX_EPOCH,
        0,
        Options {
            metrics,
            settings,
            plant: Some(first_page),
            fields: if cell.handler {
                Map::new()
            } else {
                number_field()
            },
            handler: cell.handler,
            frontier: cell.frontier,
            small_pages: pages == Pages::Small,
            run_state_vfs: Some(vfs.name().to_owned()),
            largest_page: matches!(pages, Pages::Callgrind | Pages::CallgrindShorter)
                .then_some(CALLGRIND_PAGE),
            linked_pages: (pages == Pages::CallgrindShorter).then_some(SHORTER),
            limit: cell.limit.then_some(LIMIT),
            robots_txt: true,
            setup_vfs: Some(setup.name().to_owned()),
            formats: Map::new(),
            after_start: if BARRIER_CELLS.contains(&cell.name) {
                AfterStart::BarrierCalls
            } else {
                AfterStart::Finish
            },
        },
    );
    Crawl {
        crawled,
        work,
        started,
        syncs: vfs.syncs(),
        // The setup's own files too, such as the ledger's journal before
        // the engine opens it in WAL: `setup_syncs` counts their syncs.
        vfs_files: vfs.files().into_iter().chain(setup.files()).collect(),
        setup_syncs: setup.syncs(),
        entries: entries(home.path()),
    }
}

/// The pages a crawl of `cell` on a site linking `linked` of [`PAGES`]
/// serves: the index and its pages, and with the Frontier's outcomes the
/// redirect's target, the sitemap's entry, the seed with a Payload and
/// its items.
fn pages_of(cell: &Cell, linked: usize) -> usize {
    linked
        + cell
            .frontier
            .map_or(1, |frontier| 4 + frontier.items_on(linked))
}

/// The pages the whole site of `cell` serves beyond a `--shorter` run's:
/// the unit `F2.prepares` divides by.
fn added_pages(cell: &Cell) -> usize {
    pages_of(cell, PAGES.len()) - pages_of(cell, SHORTER)
}

/// Runs `cell`, and each outcome's prepares when `planted` asks for no other row
/// and nothing traces it.
fn run(cell: &Cell, planted: Option<&str>, watched: Watched) -> Counted {
    let (counted, bypassed) = match watched {
        Watched::Counted => (true, None),
        Watched::Traced { bypass } => (false, bypass),
    };
    let (prepare_calls, mut ran) = prepare_calls(cell.name, planted, counted, || {
        let Crawl {
            crawled,
            work,
            started,
            syncs,
            vfs_files,
            setup_syncs,
            entries,
        } = crawl(cell, planted, bypassed, Pages::Whole);
        assert!(
            entries > 0 || !logs_provenance(cell),
            "the crawl wrote no Run-trace entry"
        );
        assert!(
            !cell.limit
                || (work.read(Row::ChargeStatements) > 0 && work.read(Row::ChargeOpens) > 0),
            "the crawl charged nothing against its Limit"
        );
        assert_eq!(
            crawled.body_bytes,
            body_bytes(),
            "the site sends each body once"
        );
        let pages = pages_of(cell, PAGES.len());
        assert!(
            cell.name != VARYING_PARAMETER
                || (work.read(Row::EvidenceReads) > 0 && work.read(Row::AdmissionEvidence) > 0),
            "the crawl's learning read or its admissions kept no evidence"
        );
        if cell.name == RECORDS_OFF_CELL {
            assert!(crawled.records.is_empty(), "the crawl kept a Record");
        } else if cell.handler {
            assert_eq!(
                crawled.handled,
                u64::try_from(pages).expect("invariant: a dozen pages"),
                "the handler handles each page once"
            );
        } else {
            assert_eq!(
                crawled.records.len(),
                pages,
                "the crawl extracts each page once"
            );
            assert!(
                crawled
                    .records
                    .iter()
                    .any(|record| record["title"] == "Page 3" && record["number"] == 3),
                "the field's Value transform read no number: {:?}",
                crawled.records
            );
            let forms = FROM_FIRST_PAGE
                .iter()
                .position(|row| *row == Row::PageLocaleFormBuilds);
            let built = forms.and_then(|at| started.get().map(|started| started[at]));
            assert_eq!(
                built,
                Some(1),
                "the crawl builds its field's forms once, as it starts"
            );
        }
        Counted {
            name: cell.name,
            pages: u64::try_from(pages).expect("invariant: a dozen pages"),
            added_pages: u64::try_from(added_pages(cell)).expect("invariant: a dozen pages"),
            hops: crawled.hops,
            sitemaps: crawled.sitemaps,
            entries,
            work,
            started,
            removes: cell.removes,
            prepares: if counted {
                prepares(cell.name, cell.frontier.is_some(), cell.handler, planted)
            } else {
                BTreeMap::new()
            },
            attempts: crawled.attempts,
            requests: crawled.requests,
            syncs,
            vfs_files,
            setup_syncs,
            removes_syncs: cell.removes_syncs,
            prepare_calls: None,
            frontier: cell.frontier.is_some(),
            handler: cell.handler,
            level: level(cell),
            barrier_calls: if BARRIER_CELLS.contains(&cell.name) {
                BARRIER_CALLS
            } else {
                0
            },
        }
    });
    ran.prepare_calls = prepare_calls;
    ran
}

/// Runs `cell` under Callgrind for `F2.prepares`, `E6.opens` and
/// `F3.counted_work_constructions` beside `beside`, which runs on this
/// thread, when this run reads their rows and is `counted`: a plant of
/// another row has no use for their figures and would wait for them, and
/// `strace` would trace that run too. The run is scoped to this call, so its
/// Valgrind child never outlives a panic of `beside`.
#[expect(
    clippy::disallowed_methods,
    reason = "the ADR 0140 counts harness runs Callgrind beside the crawl on a scoped thread, which nothing else waits on"
)]
fn prepare_calls<T>(
    cell: &str,
    planted: Option<&str>,
    counted: bool,
    beside: impl FnOnce() -> T,
) -> (Option<Prepared>, T) {
    std::thread::scope(|scope| {
        let calls = scope.spawn(|| {
            if !counted
                || planted.is_some_and(|row| {
                    ![RUN_STATE_PREPARES, POLITENESS_OPENS, CONSTRUCTIONS].contains(&row)
                })
            {
                return None;
            }
            let executable = std::env::current_exe().expect("invariant: this process has a path");
            let owners: [(&str, &[&str]); 2] = [(POLITENESS, &OPENING), (RUN_STATE, &[])];
            let run = |shorter: bool| {
                let mut arguments = vec!["--callgrind", cell];
                if shorter {
                    arguments.push("--shorter");
                }
                if let Some(row) = planted {
                    arguments.extend(["--plant", row]);
                }
                callgrind::calls_beneath_each(
                    PREPARE,
                    CALLERS,
                    &owners,
                    &[CONSTRUCTION],
                    &executable,
                    &arguments,
                )
            };
            let (whole, shorter) = std::thread::scope(|scope| {
                let shorter = scope.spawn(|| run(true));
                let whole = run(false);
                (
                    whole,
                    shorter
                        .join()
                        .expect("invariant: the shorter run under Callgrind ended"),
                )
            });
            match whole.and_then(|whole| shorter.map(|shorter| (whole, shorter))) {
                Ok((whole, shorter)) => {
                    // A constructor renamed or inlined would otherwise
                    // count no construction, and pass.
                    assert!(
                        whole.others[0] > 0,
                        "Callgrind saw no `CountedWork` built, not even the engine's: {CONSTRUCTION} no longer names its constructor"
                    );
                    Some(Prepared {
                        politeness: whole.beneath[0].beneath,
                        per_added_pages: whole.beneath[1]
                            .beneath
                            .checked_sub(shorter.beneath[1].beneath)
                            .expect(
                                "invariant: the whole site prepares at least what half of it does",
                            ),
                        constructions: whole.others[0].checked_sub(shorter.others[0]).expect(
                            "invariant: the whole site builds at least what half of it does",
                        ),
                    })
                }
                Err(error) => {
                    eprintln!(
                        "{RUN_STATE_PREPARES}, {POLITENESS_OPENS} and {CONSTRUCTIONS} are not read: {error}"
                    );
                    None
                }
            }
        });
        let beside = beside();
        let calls = calls
            .join()
            .expect("invariant: the run under Callgrind ended");
        (calls, beside)
    })
}

/// Each outcome's `F3.prepares` in the cell `cell`, side by side, the planted
/// one's alone when a row is planted, and none when another row is. A cell
/// with no `frontier` meets no hop and no sitemap: theirs are 0, once
/// Valgrind ran. A handler's cell follows no links, so it admits no links
/// batch either, and every outcome's is 0.
#[expect(
    clippy::disallowed_methods,
    reason = "the ADR 0140 counts harness runs each outcome under Callgrind on a scoped thread, none waiting on another"
)]
fn prepares(
    cell: &'static str,
    frontier: bool,
    handler: bool,
    planted: Option<&str>,
) -> BTreeMap<&'static str, u64> {
    if handler {
        return PREPARES.iter().map(|(row, _)| (*row, 0)).collect();
    }
    let mut prepares: BTreeMap<&'static str, u64> = std::thread::scope(|scope| {
        let runs: Vec<_> = PREPARES
            .iter()
            .filter(|(row, _)| planted.is_none_or(|planted| planted == *row))
            .filter(|(row, _)| frontier || *row == PREPARES[0].0)
            .map(|(row, function)| {
                scope.spawn(move || {
                    under_callgrind(cell, function, planted).map(|count| (*row, count))
                })
            })
            .collect();
        runs.into_iter()
            .filter_map(|run| run.join().expect("invariant: a run under Callgrind ended"))
            .collect()
    });
    if !frontier && !prepares.is_empty() {
        for (row, _) in &PREPARES[1..] {
            prepares.insert(row, 0);
        }
    }
    prepares
}

/// The prepares inside `function` in the cell `cell`, with `planted`'s plant,
/// under Callgrind. `None` when Valgrind cannot run here.
///
/// Callgrind's call counts ignore its collection state, so `--toggle-collect`
/// scopes nothing: each call of `function` zeroes the counters on entry and
/// dumps them on exit, and each numbered dump holds that one call's prepares.
/// Its last, unnumbered dump holds what ran after and counts for nothing.
fn under_callgrind(cell: &str, function: &str, planted: Option<&str>) -> Option<u64> {
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let out = folder.path().join("out");
    let mut command = Command::new("valgrind");
    command
        .arg("--tool=callgrind")
        .arg(format!("--callgrind-out-file={}", out.display()))
        .args(["--compress-strings=no", "--compress-pos=no"])
        .arg("--log-file=/dev/null");
    let window = |command: &mut Command, function: &str| {
        command
            .arg(format!("--zero-before={function}"))
            .arg(format!("--dump-after={function}"));
    };
    window(&mut command, function);
    if planted.is_some() {
        window(&mut command, PLANTED_PREPARE);
    }
    command
        .arg(std::env::current_exe().expect("invariant: this executable has a path"))
        .args(["--small-cell", cell]);
    if let Some(planted) = planted {
        command.args(["--plant", planted]);
    }
    let Ok(status) =
        callgrind::bounded(|| command.stdout(Stdio::null()).stderr(Stdio::null()).status())
    else {
        return None;
    };
    assert!(status.success(), "Callgrind's run of {function} failed");
    let dumps: Vec<String> = std::fs::read_dir(folder.path())
        .expect("invariant: Callgrind's folder is there")
        .map(|entry| entry.expect("invariant: the folder reads").path())
        .filter(|path| path.extension().is_some())
        .map(|path| std::fs::read_to_string(path).expect("invariant: Callgrind wrote its dump"))
        .collect();
    // A function renamed would otherwise count no prepare, and pass.
    assert!(
        !dumps.is_empty(),
        "Callgrind never entered {function}: the function that does this outcome's admission has moved"
    );
    Some(dumps.iter().map(|dump| calls_to_prepare(dump)).sum())
}

/// The calls of [`PREPARE`] the Callgrind dump `dump` records.
fn calls_to_prepare(dump: &str) -> u64 {
    let mut lines = dump.lines();
    let mut calls = 0;
    while let Some(line) = lines.next() {
        if line.strip_prefix("cfn=") == Some(PREPARE) {
            calls += lines
                .next()
                .and_then(|line| line.strip_prefix("calls="))
                .and_then(|line| line.split(' ').next())
                .and_then(|count| count.parse::<u64>().ok())
                .expect("invariant: `calls=` follows a `cfn=`");
        }
    }
    calls
}

/// What `arguments` ask for: the pages a crawl serves, the one cell a child
/// run crawls, the row planted and how the cells are watched.
fn asked<'a>(arguments: &[&'a str]) -> (Pages, Option<&'a str>, Option<&'a str>, Watched<'a>) {
    match arguments {
        [] => (Pages::Whole, None, None, Watched::Counted),
        ["--plant", row] => (Pages::Whole, None, Some(*row), Watched::Counted),
        ["--strace"] => (Pages::Whole, None, None, Watched::Traced { bypass: None }),
        ["--strace", "--bypass", bypass] => (
            Pages::Whole,
            None,
            None,
            Watched::Traced {
                bypass: Some(*bypass),
            },
        ),
        ["--small-cell", cell] => (Pages::Small, Some(*cell), None, Watched::Counted),
        ["--small-cell", cell, "--plant", row] => {
            (Pages::Small, Some(*cell), Some(*row), Watched::Counted)
        }
        ["--callgrind", cell] => (Pages::Callgrind, Some(*cell), None, Watched::Counted),
        ["--callgrind", cell, "--plant", row] => {
            (Pages::Callgrind, Some(*cell), Some(*row), Watched::Counted)
        }
        ["--callgrind", cell, "--shorter"] => {
            (Pages::CallgrindShorter, Some(*cell), None, Watched::Counted)
        }
        ["--callgrind", cell, "--shorter", "--plant", row] => (
            Pages::CallgrindShorter,
            Some(*cell),
            Some(*row),
            Watched::Counted,
        ),
        other => panic!(
            "usage: counts [--small-cell <cell> | --callgrind <cell> [--shorter]] [--plant <row>] | --strace [--bypass <work>], not {other:?}"
        ),
    }
}

/// The shorter site's crawl of `cell`, `ran`, served [`added_pages`] pages
/// fewer than the whole site's, as its Records or its handler count them; a
/// cell that keeps no Record has none to count.
fn assert_shorter(cell: &Cell, ran: &Crawl) {
    let served = if cell.handler {
        usize::try_from(ran.crawled.handled).unwrap_or(usize::MAX)
    } else if cell.name == RECORDS_OFF_CELL {
        return;
    } else {
        ran.crawled.records.len()
    };
    assert_eq!(
        served,
        pages_of(cell, SHORTER),
        "the shorter site serves {} pages fewer",
        added_pages(cell)
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "the ADR 0140 counts harness runs each cell on a scoped thread, none waiting on another"
)]
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let (pages, cell, planted, watched) = asked(&arguments);
    let bypassed = matches!(watched, Watched::Traced { bypass: Some(_) });
    if let Some(name) = cell {
        let cell = cells()
            .into_iter()
            .find(|cell| cell.name == name)
            .expect("invariant: its parent named a cell");
        let ran = crawl(&cell, planted, None, pages);
        if pages == Pages::CallgrindShorter {
            assert_shorter(&cell, &ran);
        }
        return;
    }
    // Side by side, each on its own paused runtime: no cell waits on another,
    // so a family's rows stay within their minute. A plant runs in every
    // cell: a row's Today is its worst cell's, often not the default's
    // (`concurrency_1` for F2 and E6, the run-state cells for `F3.prepares`),
    // which a unit more per page in the default cell alone stays under, and
    // `E6.off_work` is held in its off cell alone. A bypass needs the default
    // cell alone, the one `strace` watches.
    let counted: Vec<Counted> = std::thread::scope(|scope| {
        let cells: Vec<_> = cells()
            .into_iter()
            .filter(|cell| !bypassed || cell.name == "default")
            .map(|cell| scope.spawn(move || run(&cell, planted, watched)))
            .collect();
        cells
            .into_iter()
            .map(|cell| cell.join().expect("invariant: a cell's checks passed"))
            .collect()
    });
    let default = &counted[0];
    for cell in &counted {
        for row in cell.removes {
            assert!(
                cell.count(*row) < default.count(*row),
                "the cell {} leaves {} as it is",
                cell.name,
                row.name()
            );
        }
        assert!(
            !cell.removes_syncs || cell.syncs < default.syncs,
            "the cell {} leaves {SYNCS} as it is",
            cell.name
        );
        let counts = counts_of(cell);
        let off = cell.name == OFF_CELL;
        let batched = matches!(cell.level, DURABILITY_BATCH | DURABILITY_OFF);
        let full = cell.level == DURABILITY_FULL;
        let leases = cell.attempts + cell.sitemaps;
        let units = json!({
            "pages": cell.pages,
            "added_pages": cell.added_pages,
            "records": if cell.name == RECORDS_OFF_CELL { 0 } else { cell.pages },
            "links_batches": if cell.handler { 0 } else { cell.pages },
            "hops": cell.hops,
            // Each of the site's redirects leads straight to a page: a
            // chain of one hop.
            "chains": cell.hops,
            "sitemaps": cell.sitemaps,
            "payloads": u64::from(cell.frontier),
            "attempts": cell.attempts,
            "attempts_ended": cell.attempts + cell.sitemaps,
            "runs": RUNS,
            "effects": EFFECTS,
            "entries": cell.entries,
            "permits": cell.requests,
            "requests": cell.requests,
            "origins": ORIGINS,
            "off_attempts": if off { cell.attempts } else { 0 },
            "learning_off_pages": if cell.name == LEARNING_OFF { cell.pages } else { 0 },
            "batched_pages": if batched { cell.pages } else { 0 },
            "batched_leases": if batched && !cell.spills() { leases } else { 0 },
            "spilled_leases": if batched && cell.spills() { leases } else { 0 },
            "full_pages": if full { cell.pages } else { 0 },
        });
        println!(
            "{}",
            json!({
                "cell": cell.name,
                "units": units,
                "counts": counts,
                "vfs_files": cell.vfs_files,
                "setup_syncs": cell.setup_syncs,
            })
        );
    }
}

/// The counts `cell` prints, each row's beside the ones read only in it.
fn counts_of(cell: &Counted) -> Map<String, Value> {
    let mut counts: Map<String, Value> = Row::ALL
        .iter()
        .map(|row| (row.name().to_owned(), json!(cell.count(*row))))
        .collect();
    for (row, count) in &cell.prepares {
        counts.insert((*row).to_owned(), json!(count));
    }
    counts.insert(SYNCS.to_owned(), json!(cell.syncs));
    match cell.prepare_calls {
        Some(calls) => {
            counts.insert(RUN_STATE_PREPARES.to_owned(), json!(calls.per_added_pages));
            counts.insert(CONSTRUCTIONS.to_owned(), json!(calls.constructions));
            let opens = cell.count(Row::PolitenessOpens) + calls.politeness;
            counts.insert(POLITENESS_OPENS.to_owned(), json!(opens));
        }
        None => {
            counts.remove(POLITENESS_OPENS);
        }
    }
    let off = cell.name == OFF_CELL;
    let off_work: u64 = POLITENESS_ROWS.iter().map(|row| cell.count(*row)).sum();
    counts.insert(OFF_WORK.to_owned(), json!(if off { off_work } else { 0 }));
    let evidence: u64 = EVIDENCE_ROWS.iter().map(|row| cell.count(*row)).sum();
    let learning_off = cell.name == LEARNING_OFF;
    counts.insert(
        LEARNING_OFF_EVIDENCE.to_owned(),
        json!(if learning_off { evidence } else { 0 }),
    );
    let batched = matches!(cell.level, DURABILITY_BATCH | DURABILITY_OFF);
    let full = cell.level == DURABILITY_FULL;
    let page_waits = cell.count(Row::RunPageWaits);
    counts.insert(
        PAGE_WAITS.to_owned(),
        json!(if batched { page_waits } else { 0 }),
    );
    counts.insert(
        PAGE_WAITS_FULL.to_owned(),
        json!(if full { page_waits } else { 0 }),
    );
    let lease_reads = cell.count(Row::LeaseReads);
    counts.insert(
        LEASE_READS.to_owned(),
        json!(if batched && !cell.spills() {
            lease_reads
        } else {
            0
        }),
    );
    counts.insert(
        LEASE_READS_SPILLED.to_owned(),
        json!(if batched && cell.spills() {
            lease_reads
        } else {
            0
        }),
    );
    let early = cell.count(Row::RunBatches);
    counts.insert(
        BATCHES.to_owned(),
        json!(early.saturating_sub(cell.barrier_calls)),
    );
    counts
}
