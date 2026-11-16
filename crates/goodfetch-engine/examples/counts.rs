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
//! Its Usage charge scenario (`support/charge_counts.rs`) charges under a
//! Limit in every cell but `no_limit`, whose run no Limit holds and which
//! takes the Slice's work away. What no wrapper sees of a charge comes from
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
//! `--callgrind <cell>`, the Audit scenario alone. Without Valgrind
//! `E2.prepares` goes unprinted, which `xtask counts` fails as a row not
//! counted.
//!
//! A new scenario of the family (`benches/<scenario>.rs`) adds one line to
//! [`run`] that runs it and inserts its units and counts, and its rows' arms
//! to its plant.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]
#![expect(
    clippy::print_stderr,
    reason = "the counts harness (ADR 0140): why a row goes unprinted, which `xtask counts` fails, beside its finding"
)]
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
use messages::{Logs, Plant, Traffic};
use serde_json::{Map, Value, json};

/// The fixture's traffic: every outcome of the message path, the small
/// params' share large enough that a row's figure is theirs as well.
const TRAFFIC: Traffic = Traffic {
    small: 20,
    large: 10,
    plugin: 10,
};

/// The Audit entry row Callgrind reads, and the C symbol it reads it from.
const PREPARES: &str = "E2.prepares";
const PREPARE: &str = "sqlite3_prepare_v3";

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
            Arc::new(move |_: &AuditLog, _: &tokio::runtime::Runtime| {
                work.count(Row::AuditConfigurationReads);
                audit::settings_files(home.path())
                    .load(Product::Collie)
                    .expect("invariant: a home with no settings file reads as empty");
            })
        }
        // One entry more, on a log of its own beside the scenario's, made
        // anew for each so that its writer opens its store again: a store
        // open, its transactions and a head-file write the store syncs.
        "E2.opens" | "E2.transactions" | SYNCS => {
            let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
            let (vfs, work) = (vfs.to_owned(), work.clone());
            Arc::new(move |_: &AuditLog, runtime: &tokio::runtime::Runtime| {
                let log = audit::log(home.path(), Some(&vfs), &work);
                audit::run_one(&log, runtime, 0);
            })
        }
        // A prune pass with nothing old enough to prune.
        "E2.prunes" => Arc::new(|log: &AuditLog, runtime: &tokio::runtime::Runtime| {
            runtime
                .block_on(log.prune(PLANTED_RETENTION, None, jiff::Timestamp::UNIX_EPOCH))
                .expect("invariant: an Audit log with nothing to prune");
        }),
        // Callgrind counts the process's calls of SQLite's own function, on
        // any connection.
        PREPARES => Arc::new(|_: &AuditLog, _: &tokio::runtime::Runtime| {
            let connection = rusqlite::Connection::open_in_memory()
                .expect("invariant: SQLite opens an in-memory database");
            let _ = connection
                .prepare("SELECT 1")
                .expect("invariant: a literal statement");
        }),
        other => panic!("no plant for the row {other}"),
    }
}

/// One cell: a name, how it runs, whether a Limit holds its charges, how its
/// Audit log runs, and the rows it must count fewer of than the default cell:
/// a feature's work goes with the feature (ADR 0140).
struct Cell {
    name: &'static str,
    logs: Logs,
    limited: bool,
    audit: Audit,
    removes: &'static [Row],
}

const CELLS: [Cell; 5] = [
    Cell {
        name: "default",
        logs: Logs::On,
        limited: true,
        audit: Audit::Chained,
        removes: &[],
    },
    Cell {
        name: "log_off",
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
        logs: Logs::On,
        limited: false,
        audit: Audit::Chained,
        removes: &[Row::ChargeTransactions],
    },
    // `security.audit_chain` off: its entries are hashed alone, which takes
    // no row's work away.
    Cell {
        name: "audit_chain_off",
        logs: Logs::On,
        limited: true,
        audit: Audit::Unchained,
        removes: &[],
    },
    // `audit.retention` set, off by default: no append prunes, the engine
    // pruning as it starts and once a day (#1887).
    Cell {
        name: "audit_retention",
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
    let sent = messages::run(
        TRAFFIC,
        cell.logs,
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
    assert!(sent.event_batches > 0, "the Plugin's events went out");
    assert_eq!(
        sent.handler_requests, TRAFFIC.plugin,
        "the Plugin's handler requests went out"
    );
    assert_eq!(
        sent.log_lines > 0,
        cell.logs == Logs::On,
        "the log lines follow the Connection's level"
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
    if let Some(prepares) = audit_prepares.join().ok().flatten() {
        by_row.insert(PREPARES, prepares);
    }
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

/// The calls of `sqlite3_prepare_v3` in the windows of a Callgrind profile
/// written in `folder` with `--compress-strings=no`, which writes each called
/// function by its name: one part per dump, a window's dumped as it ends.
fn prepares(folder: &Path) -> u64 {
    std::fs::read_dir(folder)
        .expect("invariant: Callgrind wrote its profile")
        .map(|part| std::fs::read_to_string(part.expect("invariant: a listed part").path()))
        .map(|part| part.expect("invariant: Callgrind's part is text"))
        .filter(|part| {
            part.lines()
                .find(|line| line.starts_with("desc: Trigger:"))
                .is_some_and(|trigger| trigger.contains("window_ends"))
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
    let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let profile = folder.path().join("callgrind.out");
    let mut command = Command::new("valgrind");
    command
        .args([
            "--tool=callgrind",
            "--compress-strings=no",
            "--dump-before=*window_starts*",
            "--dump-before=*window_ends*",
        ])
        .arg(format!("--callgrind-out-file={}", profile.display()))
        .arg(std::env::current_exe().expect("invariant: this executable has a path"))
        .args(["--window", cell, outcome]);
    if let Some(planted) = planted {
        command.arg(planted);
    }
    let output = goodfetch_test_support::callgrind::bounded(|| command.output())
        .expect("invariant: Valgrind runs, as ADR 0071's benchmarks need it");
    assert!(
        output.status.success(),
        "Callgrind failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    prepares(folder.path())
}

/// Runs `cell`'s Audit scenario under Callgrind and answers its statement
/// preparations; none when Valgrind cannot run.
fn audit_prepares(cell: &str, planted: Option<&str>) -> std::thread::JoinHandle<Option<u64>> {
    let cell = cell.to_owned();
    let planted = planted == Some(PREPARES);
    std::thread::spawn(move || {
        let executable = std::env::current_exe().expect("invariant: this process has a path");
        let mut arguments = vec!["--callgrind", cell.as_str()];
        if planted {
            arguments.extend(["--plant", PREPARES]);
        }
        match goodfetch_test_support::callgrind::calls(PREPARE, &executable, &arguments) {
            Ok(calls) => Some(calls),
            Err(error) => {
                eprintln!("{PREPARES} is not read: {error}");
                None
            }
        }
    })
}

/// Runs `cell`'s Audit scenario alone, as Callgrind counts it.
fn audit_under_callgrind(cell: &str, planted: Option<&str>) {
    let cell = CELLS
        .iter()
        .find(|each| each.name == cell)
        .unwrap_or_else(|| panic!("no cell {cell}"));
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let work = CountedWork::default();
    let plant = planted
        .filter(|name| *name == PREPARES)
        .map(|name| audit_plant(name, &work, ""));
    audit::run(home.path(), cell.audit, None, &work, plant.as_ref());
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
