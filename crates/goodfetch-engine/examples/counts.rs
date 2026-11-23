//! B3's counts cell (ADR 0140): runs each of the family's scenarios once with
//! its default features on and once per feature on its paths that can be
//! switched off, on an engine and in a home of its own, and prints what each
//! cell counted as one JSON line, which `xtask counts` divides and holds to
//! the Quality register's limits. An example and not a `[[bench]]`, as `xtask
//! perf` runs every bench target under gungraun: counts need no Valgrind and
//! no optimized build.
//!
//! `--plant <row>` does one extra unit of that row's work per call, charge or
//! entry, through its count. A cell fails here if the work it counts did not
//! happen, or if a feature off did not take its work away.
//!
//! Its Usage charge scenario (`support/charge_counts.rs`) charges requests,
//! proxy bytes as a provider Session's relay reserves them ahead, and paid
//! calls, each a paid solve's worst case reserved and then settled, under a
//! Limit on each in every cell but `no_limit`, whose run no Limit holds and
//! which takes the Slice's work away. What no wrapper sees of a charge comes from
//! outside the process: `E3.prepares` is Callgrind's call count of
//! `sqlite3_prepare_v3` inside the charges that took a Slice, run again under
//! Valgrind by this same executable (`--window`), so this needs Valgrind; the
//! covered charges' count must be 0, as a covered charge runs no statement
//! (#1888), so a statement added to one fails here unless it stays prepared,
//! which `E3.statements` counts.
//! `E3.slice_transactions` is the larger of the commit hook's transactions
//! and the crash VFS's syncs, which the row holds to one limit.
//!
//! What no wrapper sees on the Audit entry path is read from outside it too:
//! the store's syncs through a counting VFS, added to its head-file writes,
//! and statement preparations from Callgrind's call count of
//! `sqlite3_prepare_v3`, for which each cell runs this example again as
//! `--callgrind <cell>`, the Audit scenario alone. Its first entry opens the
//! store and prepares the statements its connection keeps, which count once
//! a log on `E2.log_prepares`; what the later entries prepare counts on
//! `E2.prepares`, read in a window of their own (ADR 0140's E2), and what
//! the scenario reads back after its entries counts on neither.
//!
//! A new scenario of the family (`benches/<scenario>.rs`) adds one line to
//! [`run`] that runs it and inserts its units and counts, and its rows' arms
//! to its plant.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]
#![expect(
    clippy::print_stdout,
    reason = "the counts harness (ADR 0140): the benchmark's one output, which `xtask counts` reads"
)]

#[path = "../benches/audit.rs"]
mod audit;

#[path = "../benches/governor.rs"]
mod governor;

#[path = "../benches/messages.rs"]
#[expect(
    dead_code,
    reason = "what only the Valgrind targets read of the scenario"
)]
mod messages;

#[path = "support/charge_counts.rs"]
mod charge_counts;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use audit::{Audit, PLANTED_RETENTION};
use charge_counts::{Outcome, charges};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::Params;
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::metrics::Metrics;
use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::CountingVfs;
use messages::{BODY_CHUNKS, Carriage, Logs, Plant, Traffic};
use serde_json::{Map, Value, json};

/// The fixture's traffic: every outcome of the message path, the small
/// params' share large enough that a row's figure is theirs as well.
const TRAFFIC: Traffic = Traffic {
    small: 20,
    large: 10,
    plugin: 10,
    bodies: 1,
};

/// The Audit rows Callgrind reads: the log's, of its first entry, and the
/// entries' of the later ones.
const LOG_PREPARES: &str = "E2.log_prepares";
const PREPARES: &str = "E2.prepares";

/// The row the counting VFS adds the Audit log's syncs to.
const SYNCS: &str = "E2.syncs";

/// The governor scenario's plant of the row named `name`, none for a row of
/// another scenario's.
fn governor_plant(name: &str) -> Option<governor::Plant> {
    name.starts_with("E4.").then(|| {
        Row::ALL
            .into_iter()
            .find(|row| row.name() == name)
            .unwrap_or_else(|| panic!("no plant for the row {name}"))
    })
}

/// The plant of the message row named `name`.
fn plant(name: &str, work: &CountedWork) -> Plant {
    let work = work.clone();
    match name {
        "E1.deep_copies" => Arc::new(move |params: Params<'_>| {
            work.count(Row::MessageDeepCopies);
            drop(params.json().to_owned());
        }),
        // The params' text encoded, never read: a read is a decode, which
        // `E1.decodes` counts.
        "E1.encodes" => Arc::new(move |params: Params<'_>| {
            work.count(Row::MessageEncodes);
            drop(serde_json::to_vec(params.json()));
        }),
        // One more read of the call's params, which counts itself.
        "E1.decodes" => Arc::new(|params: Params<'_>| drop(params.read::<Value>())),
        other => panic!("no plant for the row {other}"),
    }
}

/// The plant of the Audit entry row named `name`: one extra unit of its work
/// per entry, done for real through the code that counts it.
fn audit_plant(name: &str, work: &CountedWork, vfs: &str) -> audit::Plant {
    match name {
        // A read of the machine's settings files, as each entry made before
        // #1885: the read has no counted function, so it is counted here.
        "E2.configuration_reads" => {
            let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
            let work = work.clone();
            Arc::new(move |_: &AuditLog, _: &tokio::runtime::Runtime, _: u64| {
                work.count(Row::AuditConfigurationReads);
                audit::settings_files(home.path())
                    .load(Product::Collie)
                    .expect("invariant: a home with no settings file reads as empty");
            })
        }
        // One log more, beside the scenario's, at its first entry: its
        // writer's first open of its store and its schema set-up's commits.
        "E2.log_opens" | "E2.log_transactions" => {
            let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
            let (vfs, work) = (vfs.to_owned(), work.clone());
            Arc::new(
                move |_: &AuditLog, runtime: &tokio::runtime::Runtime, index: u64| {
                    if index == 0 {
                        audit::run_one(&audit::log(home.path(), Some(&vfs), &work), runtime, 0);
                    }
                },
            )
        }
        // A reader's open of the log's store beside its writer's, with its
        // schema set-up's commit.
        "E2.opens" => Arc::new(|log: &AuditLog, _: &tokio::runtime::Runtime, _: u64| {
            log.verify()
                .expect("invariant: the scenario's log verifies");
        }),
        // One entry more, on a log of its own beside the scenario's, made
        // anew for each: its append's commit and a head-file write the store
        // syncs.
        "E2.transactions" | SYNCS => {
            let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
            let (vfs, work) = (vfs.to_owned(), work.clone());
            Arc::new(
                move |_: &AuditLog, runtime: &tokio::runtime::Runtime, _: u64| {
                    let log = audit::log(home.path(), Some(&vfs), &work);
                    audit::run_one(&log, runtime, 0);
                },
            )
        }
        // A prune pass with nothing old enough to prune.
        "E2.prunes" => Arc::new(
            |log: &AuditLog, runtime: &tokio::runtime::Runtime, _: u64| {
                runtime
                    .block_on(log.prune(PLANTED_RETENTION, None, jiff::Timestamp::UNIX_EPOCH))
                    .expect("invariant: an Audit log with nothing to prune");
            },
        ),
        other => panic!("no plant for the row {other}"),
    }
}

/// One statement prepared, on a connection of its own: Callgrind counts the
/// process's calls of SQLite's own function, on any connection.
fn prepare_one() {
    let connection = rusqlite::Connection::open_in_memory()
        .expect("invariant: SQLite opens an in-memory database");
    let _ = connection
        .prepare("SELECT 1")
        .expect("invariant: a literal statement");
}

/// One cell: a name, the carriage and log level its messages run with,
/// whether a Limit holds its charges, how its Audit log runs, and the rows
/// it must count fewer of than the default cell: a feature's work goes with
/// the feature (ADR 0140).
struct Cell {
    name: &'static str,
    carriage: Carriage,
    logs: Logs,
    limited: bool,
    audit: Audit,
    removes: &'static [Row],
}

const CELLS: [Cell; 6] = [
    Cell {
        name: "default",
        carriage: Carriage::Stdio,
        logs: Logs::On,
        limited: true,
        audit: Audit::Chained,
        removes: &[],
    },
    // The messages over an Exposed Daemon's WebSocket, held to the rows
    // stdio is (ADR 0049): a feature off takes nothing away here.
    Cell {
        name: "websocket",
        carriage: Carriage::WebSocket,
        logs: Logs::On,
        limited: true,
        audit: Audit::Chained,
        removes: &[],
    },
    Cell {
        name: "log_off",
        carriage: Carriage::Stdio,
        logs: Logs::Off,
        limited: true,
        audit: Audit::Chained,
        removes: &[Row::MessageEncodes],
    },
    // With no Limit, the Slice's work goes with the Limit. No charge opens
    // the ledger in any cell, which a run opens as it starts (#1888), so no
    // open is left for this cell to take away.
    Cell {
        name: "no_limit",
        carriage: Carriage::Stdio,
        logs: Logs::On,
        limited: false,
        audit: Audit::Chained,
        removes: &[Row::ChargeTransactions],
    },
    // `security.audit_chain` off: its entries are hashed alone, which takes
    // no row's work away.
    Cell {
        name: "audit_chain_off",
        carriage: Carriage::Stdio,
        logs: Logs::On,
        limited: true,
        audit: Audit::Unchained,
        removes: &[],
    },
    // `audit.retention` set, off by default: no append prunes, the engine
    // pruning as it starts and once a day (#1887).
    Cell {
        name: "audit_retention",
        carriage: Carriage::Stdio,
        logs: Logs::On,
        limited: true,
        audit: Audit::Retained,
        removes: &[],
    },
];

/// What one cell counted: its units, the message and Audit scenarios' work,
/// and the counts by row that other scenarios count on their own.
struct Counted {
    cell: &'static Cell,
    units: BTreeMap<&'static str, u64>,
    work: CountedWork,
    by_row: BTreeMap<&'static str, u64>,
}

impl Counted {
    /// What this cell counted on `row`.
    fn count(&self, row: Row) -> u64 {
        self.by_row
            .get(row.name())
            .copied()
            .unwrap_or_else(|| self.work.read(row))
    }
}

fn count(number: usize) -> u64 {
    u64::try_from(number).expect("invariant: a few dozen messages")
}

/// A planted row, by the scenario that plants it.
#[derive(Clone, Copy)]
enum Planted<'a> {
    Message(&'a str),
    Charge(&'a str),
    Audit(&'a str),
    Governor(&'a str),
}

impl<'a> Planted<'a> {
    fn of(name: &'a str) -> Self {
        if name.starts_with("E3.") {
            Self::Charge(name)
        } else if name.starts_with("E2.") {
            Self::Audit(name)
        } else if name.starts_with("E4.") {
            Self::Governor(name)
        } else {
            Self::Message(name)
        }
    }
}

/// Runs the message scenario on `cell`'s carriage and log level, and fails
/// unless each of its outcomes left the engine.
fn run_messages(
    cell: &Cell,
    metrics: Metrics,
    plant: Option<Plant>,
    home: &Path,
) -> messages::Sent {
    let sent = messages::run(TRAFFIC, cell.logs, cell.carriage, metrics, plant, home);
    assert!(sent.event_batches > 0, "the Plugin's events went out");
    assert_eq!(
        sent.handler_requests, TRAFFIC.plugin,
        "the Plugin's handler requests went out"
    );
    assert_eq!(
        sent.chunks,
        TRAFFIC.bodies * BODY_CHUNKS,
        "the stored bodies' chunks went out"
    );
    assert_eq!(
        sent.log_lines > 0,
        cell.logs == Logs::On,
        "the log lines follow the Connection's level"
    );
    sent
}

/// Runs `cell`'s scenarios.
fn run(cell: &'static Cell, planted: Option<&str>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let planted = planted.map(Planted::of);
    let (message_plant, charge_plant, audit_planted, governor_planted) = match planted {
        Some(Planted::Message(name)) => (Some(name), None, None, None),
        Some(Planted::Charge(name)) => (None, Some(name), None, None),
        Some(Planted::Audit(name)) => (None, None, Some(name), None),
        Some(Planted::Governor(name)) => (None, None, None, Some(name)),
        None => (None, None, None, None),
    };
    let audit_prepares = audit_prepares(cell.name, audit_planted);
    let audit_planted = audit_planted.filter(|name| ![LOG_PREPARES, PREPARES].contains(name));
    let sent = run_messages(
        cell,
        metrics,
        message_plant.map(|name| plant(name, &work)),
        home.path(),
    );
    let sampled = governor::run(
        &home.path().join("governor"),
        &work,
        governor_planted.and_then(governor_plant),
    );
    assert_eq!(
        sampled.samples,
        governor::SAMPLES,
        "the governor took its samples"
    );
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let charged = charges(folder.path(), cell.limited, charge_plant, None);
    // A covered charge runs no statement (#1888), so it prepares nothing:
    // an uncached statement added to one prepares in the window and fails
    // here. A cell with no Limit has no ledger to prepare one on, so only
    // the cells a Limit holds check.
    let slice_prepares = if cell.limited {
        if charge_plant.is_none() {
            assert_eq!(
                callgrind(cell.name, "covered", None),
                0,
                "the cell {}'s covered charges prepared a statement, and a covered charge \
                 runs none",
                cell.name
            );
        }
        callgrind(cell.name, "slice", charge_plant)
    } else {
        0
    };
    let vfs = CountingVfs::register();
    let audit_home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let entries = audit::run(
        audit_home.path(),
        cell.audit,
        Some(vfs.name()),
        &work,
        audit_planted
            .map(|name| audit_plant(name, &work, vfs.name()))
            .as_ref(),
    );
    let units = BTreeMap::from([
        ("incoming_messages", count(TRAFFIC.incoming())),
        ("requests", count(TRAFFIC.requests())),
        ("answers_and_events", count(sent.answers_and_events())),
        ("covered_charges", charged.covered),
        ("slices", charged.slices),
        ("charges", charged.covered + charged.slices),
        ("audit_entries", entries),
        ("audit_logs", 1),
        ("later_audit_entries", entries - 1),
        ("samples", sampled.samples),
        ("process_samples", sampled.process_samples),
    ]);
    let mut by_row = BTreeMap::from([
        (Row::ChargeStatements.name(), charged.statements),
        (
            Row::ChargeTransactions.name(),
            charged.transactions.max(charged.syncs),
        ),
        (Row::ChargeOpens.name(), charged.opens),
        ("E3.prepares", slice_prepares),
        (SYNCS, work.read(Row::AuditSyncs) + vfs.syncs()),
    ]);
    let [log_prepares, prepares] = audit_prepares.join().expect("invariant: Callgrind read it");
    by_row.insert(LOG_PREPARES, log_prepares);
    by_row.insert(PREPARES, prepares);
    Counted {
        cell,
        units,
        work,
        by_row,
    }
}

/// Runs one cell's charges under this executable's `--window`: Callgrind's own
/// process.
fn window(cell: &str, outcome: &str, planted: Option<&str>) {
    let limited = CELLS
        .iter()
        .find(|each| each.name == cell)
        .unwrap_or_else(|| panic!("no cell {cell}"))
        .limited;
    let outcome = match outcome {
        "covered" => Outcome::Covered,
        "slice" => Outcome::SliceTaken,
        other => panic!("no outcome {other}"),
    };
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    charges(folder.path(), limited, planted, Some(outcome));
}

/// The calls of `sqlite3_prepare_v3` in the windows that end at the function
/// `end` of a Callgrind profile written in `folder` with
/// `--compress-strings=no`, which writes each called function by its name:
/// one part per dump, a window's dumped as it ends.
fn prepares(folder: &Path, end: &str) -> u64 {
    std::fs::read_dir(folder)
        .expect("invariant: Callgrind wrote its profile")
        .map(|part| std::fs::read_to_string(part.expect("invariant: a listed part").path()))
        .map(|part| part.expect("invariant: Callgrind's part is text"))
        .filter(|part| {
            part.lines()
                .find(|line| line.starts_with("desc: Trigger:"))
                .is_some_and(|trigger| trigger.contains(end))
        })
        .map(|part| calls_in(&part))
        .sum()
}

/// The calls of `sqlite3_prepare_v3` in one part.
fn calls_in(text: &str) -> u64 {
    let mut lines = text.lines();
    let mut calls = 0;
    while let Some(line) = lines.next() {
        if line == "cfn=sqlite3_prepare_v3" {
            calls += lines
                .next()
                .and_then(|line| line.strip_prefix("calls="))
                .and_then(|line| line.split(' ').next())
                .and_then(|count| count.parse::<u64>().ok())
                .expect("invariant: a call's line follows its callee's");
        }
    }
    calls
}

/// What Callgrind counts of `outcome`'s charges in `cell`.
fn callgrind(cell: &str, outcome: &str, planted: Option<&str>) -> u64 {
    let mut arguments = vec!["--window", cell, outcome];
    arguments.extend(planted);
    let [prepares] = windows(&arguments, &["window_starts"], ["window_ends"]);
    prepares
}

/// The calls of `sqlite3_prepare_v3` this executable makes, run under
/// Callgrind with `arguments`, in each window that ends as it enters one of
/// the functions `ends`; a window starts at the previous dump: the start of
/// the process, one of `ends` or one of the functions `starts`.
fn windows<const N: usize>(arguments: &[&str], starts: &[&str], ends: [&str; N]) -> [u64; N] {
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let profile = folder.path().join("callgrind.out");
    let mut command = Command::new("valgrind");
    command
        .args(["--tool=callgrind", "--compress-strings=no"])
        .args(
            starts
                .iter()
                .chain(&ends)
                .map(|function| format!("--dump-before=*{function}*")),
        )
        .arg(format!("--callgrind-out-file={}", profile.display()))
        .arg(std::env::current_exe().expect("invariant: this executable has a path"))
        .args(arguments);
    let output = goodfetch_test_support::callgrind::bounded(|| command.output())
        .expect("invariant: Valgrind runs, as ADR 0071's benchmarks need it");
    assert!(
        output.status.success(),
        "Callgrind failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ends.map(|end| prepares(folder.path(), end))
}

/// Callgrind dumps its profile as this is entered: its part holds the Audit
/// log's first entry and all before it, the store's first open among them.
#[inline(never)]
fn first_entry_ends() {
    std::hint::black_box(3);
}

/// Callgrind dumps its profile as this is entered: its part holds the Audit
/// log's later entries, and not what the scenario reads back after them.
#[inline(never)]
fn later_entries_end() {
    std::hint::black_box(4);
}

/// Runs `cell`'s Audit scenario under Callgrind on a thread of its own and
/// answers its statement preparations: the log's, then the later entries'.
fn audit_prepares(cell: &str, planted: Option<&str>) -> std::thread::JoinHandle<[u64; 2]> {
    let cell = cell.to_owned();
    let planted = [LOG_PREPARES, PREPARES]
        .into_iter()
        .find(|row| planted == Some(*row));
    std::thread::spawn(move || {
        let mut arguments = vec!["--callgrind", cell.as_str()];
        arguments.extend(planted.iter().flat_map(|name| ["--plant", name]));
        windows(&arguments, &[], ["first_entry_ends", "later_entries_end"])
    })
}

/// Runs `cell`'s Audit scenario alone, as Callgrind counts it, with one
/// statement more for the log or for each entry when `planted` names its row.
fn audit_under_callgrind(cell: &str, planted: Option<&str>) {
    let cell = CELLS
        .iter()
        .find(|each| each.name == cell)
        .unwrap_or_else(|| panic!("no cell {cell}"));
    if planted == Some(LOG_PREPARES) {
        prepare_one();
    }
    let each_entry = planted == Some(PREPARES);
    let marked: audit::Plant = Arc::new(
        move |_: &AuditLog, _: &tokio::runtime::Runtime, index: u64| {
            if index == 0 {
                first_entry_ends();
            }
            if each_entry && index > 0 {
                prepare_one();
            }
            if index + 1 == audit::ENTRIES {
                later_entries_end();
            }
        },
    );
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    audit::run(
        home.path(),
        cell.audit,
        None,
        &CountedWork::default(),
        Some(&marked),
    );
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let planted = match arguments.as_slice() {
        [] => None,
        ["--plant", row] => Some(*row),
        ["--window", cell, outcome] => return window(cell, outcome, None),
        ["--window", cell, outcome, row] => return window(cell, outcome, Some(row)),
        ["--callgrind", cell] => return audit_under_callgrind(cell, None),
        ["--callgrind", cell, "--plant", row] => return audit_under_callgrind(cell, Some(row)),
        other => panic!("usage: counts [--plant <row>], not {other:?}"),
    };
    // Side by side, each on its own paused runtime: no cell waits on another,
    // so a family's rows stay within their minute.
    let counted: Vec<Counted> = std::thread::scope(|scope| {
        let cells: Vec<_> = CELLS
            .iter()
            .map(|cell| scope.spawn(move || run(cell, planted)))
            .collect();
        cells
            .into_iter()
            .map(|cell| cell.join().expect("invariant: a cell's checks passed"))
            .collect()
    });
    let default = &counted[0];
    for cell in &counted {
        for row in cell.cell.removes {
            assert!(
                cell.count(*row) < default.count(*row),
                "the cell {} leaves {} as it is",
                cell.cell.name,
                row.name()
            );
        }
        let mut counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(cell.work.read(*row))))
            .collect();
        for (row, count) in &cell.by_row {
            counts.insert((*row).to_owned(), json!(count));
        }
        println!(
            "{}",
            json!({ "cell": cell.cell.name, "units": cell.units, "counts": counts })
        );
    }
}
