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
//! Callgrind's call count of `sqlite3_prepare_v3` over a whole cell, for
//! which each cell runs this example again as `--callgrind <cell>`, on pages
//! of about 1 kB, as Callgrind takes minutes on a crawl with no large page and
//! far longer on the whole site. A page's size moves no statement. Without
//! Valgrind `F2.prepares` goes unprinted, which `xtask counts` fails as a row
//! not counted.
//!
//! The cells of the run file's own switches (`concurrency_1`,
//! `concurrency_16`, `durability_fast`) and the handler's cell crawl without
//! the Frontier's outcomes beyond a links batch: a hop's, a sitemap's and a
//! Payload's rows say "no hops" there.
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
use goodfetch_test_support::CountingVfs;
use goodfetch_test_support::callgrind::{self, Calls};
use plants::{Counted as PlantOn, politeness_plant, run_state_plant};
use scenarios::{
    Frontier, Options, PAGES, Plant, body_bytes, crawl_with, entries, page_analysis_plant,
    run_trace_plant,
};
use serde_json::{Map, Value, json};

/// The Limit of `requests` the default cell's crawl meets, far above what it
/// charges.
const LIMIT: u64 = 1_000_000;

/// Each outcome's `F3.prepares`, and the one function that does its admission.
const PREPARES: [(&str, &str); 3] = [
    ("F3.prepares", "goodfetch_crawl::enqueue::links::add"),
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

/// The row the counting VFS reads.
const SYNCS: &str = "F2.syncs";

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
const FRONTIER: Option<Frontier> = Some(Frontier { schema: true });

/// A glob in the Scope, which is what a Frontier's reads of it compile.
fn scoped(mut settings: Value) -> Value {
    settings["crawl"] = json!({ "scope": { "exclude": ["http://pages.test/never/**"] } });
    settings
}

fn cells() -> [Cell; 10] {
    [
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
            removes: &[Row::PageParses, Row::PageSelectorCompilations],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
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
            removes: &[Row::BatchCompilations, Row::HopCompilations],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
        Cell {
            name: "payload_schema_off",
            settings: scoped(json!({})),
            frontier: Some(Frontier { schema: false }),
            removes: &[Row::PayloadCompilations, Row::PayloadValidations],
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
        Cell {
            name: "durability_fast",
            settings: json!({ "run_state": { "durability": "fast" } }),
            frontier: None,
            removes: &[],
            removes_syncs: true,
            handler: false,
            limit: false,
        },
        // robots.txt off reads and parses no robots.txt (#1919). Politeness
        // off takes none of E6's other work away yet: ADR 0138's "cost
        // nothing" is the Shared paths spec's, and X19 waives it meanwhile.
        Cell {
            name: "robots_txt_off",
            settings: json!({ "site_preferences": { "respect_robots_txt": false } }),
            frontier: None,
            removes: &[Row::RobotsParses, Row::RobotsReads],
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
            removes: &[Row::RobotsParses, Row::RobotsReads],
            removes_syncs: false,
            handler: false,
            limit: false,
        },
    ]
}

/// What one cell counted.
struct Counted {
    name: &'static str,
    pages: u64,
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
    /// `F2.prepares`, and those of its preparations beneath the Politeness
    /// file's functions, when Valgrind ran.
    prepare_calls: Option<Calls>,
    /// Whether it met the Frontier's outcomes beyond a links batch.
    frontier: bool,
    /// Whether a handler served its pages, following no links.
    handler: bool,
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
/// run).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pages {
    Whole,
    Small,
    Callgrind,
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
            largest_page: (pages == Pages::Callgrind).then_some(CALLGRIND_PAGE),
            limit: cell.limit.then_some(LIMIT),
            robots_txt: true,
            setup_vfs: Some(setup.name().to_owned()),
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
        assert!(entries > 0, "the crawl wrote no Run-trace entry");
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
        // The index and its pages, and with the Frontier's outcomes the
        // redirect's target, the sitemap's entry and the seed with a Payload.
        let pages = PAGES.len() + if cell.frontier.is_some() { 4 } else { 1 };
        if cell.handler {
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
        }
    });
    ran.prepare_calls = prepare_calls;
    ran
}

/// Runs `cell` under Callgrind for `F2.prepares` and `E6.opens` beside
/// `beside`, which runs on this thread, when this run reads their rows and is
/// `counted`: a plant of another row has no use for their figures and would
/// wait for them, and `strace` would trace that run too. The run is scoped to
/// this call, so its Valgrind child never outlives a panic of `beside`.
#[expect(
    clippy::disallowed_methods,
    reason = "the ADR 0140 counts harness runs Callgrind beside the crawl on a scoped thread, which nothing else waits on"
)]
fn prepare_calls<T>(
    cell: &str,
    planted: Option<&str>,
    counted: bool,
    beside: impl FnOnce() -> T,
) -> (Option<Calls>, T) {
    std::thread::scope(|scope| {
        let calls = scope.spawn(|| {
            if !counted
                || planted.is_some_and(|row| row != RUN_STATE_PREPARES && row != POLITENESS_OPENS)
            {
                return None;
            }
            let executable = std::env::current_exe().expect("invariant: this process has a path");
            let mut arguments = vec!["--callgrind", cell];
            if let Some(row) = planted {
                arguments.extend(["--plant", row]);
            }
            let calls = callgrind::calls_beneath(
                PREPARE,
                CALLERS,
                POLITENESS,
                &OPENING,
                &executable,
                &arguments,
            );
            match calls {
                Ok(calls) => Some(calls),
                Err(error) => {
                    eprintln!("{RUN_STATE_PREPARES} is not read: {error}");
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
        other => panic!(
            "usage: counts [--small-cell <cell> | --callgrind <cell>] [--plant <row>] | --strace [--bypass <work>], not {other:?}"
        ),
    }
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
        let _ = crawl(&cell, planted, None, pages);
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
                counts.insert(RUN_STATE_PREPARES.to_owned(), json!(calls.all));
                let opens = cell.count(Row::PolitenessOpens) + calls.beneath;
                counts.insert(POLITENESS_OPENS.to_owned(), json!(opens));
            }
            None => {
                counts.remove(POLITENESS_OPENS);
            }
        }
        let off = cell.name == OFF_CELL;
        let off_work: u64 = POLITENESS_ROWS.iter().map(|row| cell.count(*row)).sum();
        counts.insert(OFF_WORK.to_owned(), json!(if off { off_work } else { 0 }));
        let units = json!({
            "pages": cell.pages,
            "records": cell.pages,
            "links_batches": if cell.handler { 0 } else { cell.pages },
            "hops": cell.hops,
            "sitemaps": cell.sitemaps,
            "payloads": u64::from(cell.frontier),
            "attempts": cell.attempts,
            "runs": RUNS,
            "effects": EFFECTS,
            "entries": cell.entries,
            "permits": cell.requests,
            "requests": cell.requests,
            "origins": ORIGINS,
            "off_attempts": if off { cell.attempts } else { 0 },
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
