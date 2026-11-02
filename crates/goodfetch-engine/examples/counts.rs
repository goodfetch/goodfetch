//! B3's counts cell (ADR 0140): runs each of the family's scenarios once with
//! its default features on and once per feature on its paths that can be
//! switched off, on an engine and in a home of its own, and prints what each
//! cell counted as one JSON line, which `xtask counts` divides and holds to
//! the Quality register's limits. An example and not a `[[bench]]`, as `xtask
//! perf` runs every bench target under gungraun: counts need no Valgrind and
//! no optimized build.
//!
//! `--plant <row>` does one extra unit of that row's work per call or per
//! entry, through its count. A cell fails here if the work it counts did not
//! happen, or if a feature off did not take its work away.
//!
//! What no wrapper sees on the Audit entry path is read from outside it: the
//! store's syncs through a counting VFS, added to its head-file writes, and
//! statement preparations from Callgrind's call count of
//! `sqlite3_prepare_v3`, for which each cell runs this example again as
//! `--callgrind <cell>`, the Audit scenario alone. Without Valgrind
//! `E2.prepares` goes unprinted, which `xtask counts` fails as a row not
//! counted.
//!
//! A new scenario of the family (`benches/<scenario>.rs`) adds one line to
//! [`run`] that runs it and inserts its units, and its rows' arms to its
//! plant.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]
#![expect(
    clippy::print_stderr,
    reason = "why a row goes unprinted, which `xtask counts` fails, beside its finding"
)]
#![expect(
    clippy::print_stdout,
    reason = "the benchmark's one output, which `xtask counts` reads"
)]

#[path = "../benches/audit.rs"]
mod audit;
#[path = "../benches/messages.rs"]
#[expect(
    dead_code,
    reason = "what only the Valgrind targets read of the scenario"
)]
mod messages;

use std::collections::BTreeMap;
use std::sync::Arc;

use audit::{Audit, PLANTED_RETENTION};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::metrics::Metrics;
use goodfetch_test_support::{CountingVfs, callgrind};
use messages::{Logs, Plant, Traffic};
use serde_json::{Map, Value, json};

/// The fixture's traffic: every outcome of the message path, the small
/// params' share large enough that a row's figure is theirs as well.
const TRAFFIC: Traffic = Traffic {
    small: 20,
    large: 10,
    plugin: 10,
};

/// The row Callgrind reads, and the C symbol it reads it from.
const PREPARES: &str = "E2.prepares";
const PREPARE: &str = "sqlite3_prepare_v3";

/// The row the counting VFS adds the store's syncs to.
const SYNCS: &str = "E2.syncs";

/// The plant of the message row named `name`, if it is one.
fn message_plant(name: &str, work: &CountedWork) -> Option<Plant> {
    let work = work.clone();
    match name {
        "E1.deep_copies" => Some(Arc::new(move |params: &Value| {
            work.count(Row::MessageDeepCopies);
            drop(params.clone());
        })),
        "E1.encodes" => Some(Arc::new(move |params: &Value| {
            work.count(Row::MessageEncodes);
            drop(serde_json::to_vec(params));
        })),
        _ => None,
    }
}

/// The plant of the Audit entry row named `name`, if it is one: one extra
/// unit of its work per entry, done for real through the code that counts it.
fn audit_plant(name: &str, work: &CountedWork, vfs: &str) -> Option<audit::Plant> {
    match name {
        // One entry more, on a log of its own beside the scenario's: a
        // configuration read, a store open, its transactions and a head-file
        // write the store syncs.
        "E2.configuration_reads" | "E2.opens" | "E2.transactions" | SYNCS => {
            let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
            let log = audit::log(home.path(), Some(vfs), work);
            let home = Arc::new(home);
            Some(Arc::new(move |_: &AuditLog| {
                let _kept = &home;
                audit::run_one(&log, 0);
            }))
        }
        // A prune pass with nothing old enough to prune.
        "E2.prunes" => Some(Arc::new(|log: &AuditLog| {
            log.prune(PLANTED_RETENTION, None, jiff::Timestamp::UNIX_EPOCH)
                .expect("invariant: an Audit log with nothing to prune");
        })),
        // Callgrind counts the process's calls of SQLite's own function, on
        // any connection.
        PREPARES => Some(Arc::new(|_: &AuditLog| {
            let connection = rusqlite::Connection::open_in_memory()
                .expect("invariant: SQLite opens an in-memory database");
            let _ = connection
                .prepare("SELECT 1")
                .expect("invariant: a literal statement");
        })),
        _ => None,
    }
}

/// One cell: a name, how it runs and the rows it must count fewer of than the
/// default cell: a feature's work goes with the feature (ADR 0140).
struct Cell {
    name: &'static str,
    logs: Logs,
    audit: Audit,
    removes: &'static [Row],
}

const CELLS: [Cell; 4] = [
    Cell {
        name: "default",
        logs: Logs::On,
        audit: Audit::Chained,
        removes: &[],
    },
    Cell {
        name: "log_off",
        logs: Logs::Off,
        audit: Audit::Chained,
        removes: &[Row::MessageEncodes],
    },
    // `security.audit_chain` off: its entries are hashed alone, which takes
    // no row's work away.
    Cell {
        name: "audit_chain_off",
        logs: Logs::On,
        audit: Audit::Unchained,
        removes: &[],
    },
    // `audit.retention` set, off by default: each append prunes first.
    Cell {
        name: "audit_retention",
        logs: Logs::On,
        audit: Audit::Retained,
        removes: &[],
    },
];

/// What one cell counted.
struct Counted {
    cell: &'static Cell,
    units: BTreeMap<&'static str, u64>,
    work: CountedWork,
    syncs: u64,
    prepares: Option<u64>,
}

fn count(number: usize) -> u64 {
    u64::try_from(number).expect("invariant: a few dozen messages")
}

/// Runs `cell`'s scenarios.
fn run(cell: &'static Cell, planted: Option<&str>) -> Counted {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let metrics = Metrics::none();
    let work = metrics.counted_work().clone();
    let vfs = CountingVfs::register();
    let prepares = prepares(cell.name, planted);
    let sent = messages::run(
        TRAFFIC,
        cell.logs,
        metrics,
        planted.and_then(|name| message_plant(name, &work)),
        home.path(),
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
    let audit_home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let entries = audit::run(
        audit_home.path(),
        cell.audit,
        Some(vfs.name()),
        &work,
        planted
            .and_then(|name| audit_plant(name, &work, vfs.name()))
            .as_ref(),
    );
    let units = BTreeMap::from([
        ("incoming_messages", count(TRAFFIC.incoming())),
        ("outgoing_messages", count(sent.outgoing)),
        ("audit_entries", entries),
    ]);
    Counted {
        cell,
        units,
        work,
        syncs: vfs.syncs(),
        prepares: prepares.join().ok().flatten(),
    }
}

/// Runs `cell`'s Audit scenario under Callgrind and answers its statement
/// preparations; none when Valgrind cannot run.
fn prepares(cell: &str, planted: Option<&str>) -> std::thread::JoinHandle<Option<u64>> {
    let cell = cell.to_owned();
    let planted = planted == Some(PREPARES);
    std::thread::spawn(move || {
        let executable = std::env::current_exe().expect("invariant: this process has a path");
        let mut arguments = vec!["--callgrind", cell.as_str()];
        if planted {
            arguments.extend(["--plant", PREPARES]);
        }
        match callgrind::calls(PREPARE, &executable, &arguments) {
            Ok(calls) => Some(calls),
            Err(error) => {
                eprintln!("{PREPARES} is not read: {error}");
                None
            }
        }
    })
}

/// Runs `cell`'s Audit scenario alone, as Callgrind counts it.
fn run_under_callgrind(cell: &'static Cell, planted: Option<&str>) {
    let home = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
    let work = CountedWork::default();
    let plant = planted
        .filter(|name| *name == PREPARES)
        .and_then(|name| audit_plant(name, &work, ""));
    audit::run(home.path(), cell.audit, None, &work, plant.as_ref());
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (planted, callgrind_cell) = match arguments.as_slice() {
        [] => (None, None),
        [flag, row] if flag == "--plant" => (Some(row.as_str()), None),
        [flag, cell] if flag == "--callgrind" => (None, Some(cell.as_str())),
        [flag, cell, plant, row] if flag == "--callgrind" && plant == "--plant" => {
            (Some(row.as_str()), Some(cell.as_str()))
        }
        other => panic!("usage: counts [--callgrind <cell>] [--plant <row>], not {other:?}"),
    };
    if let Some(name) = callgrind_cell {
        let cell = CELLS
            .iter()
            .find(|cell| cell.name == name)
            .expect("invariant: its parent names one of the cells");
        run_under_callgrind(cell, planted);
        return;
    }
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
                cell.work.read(*row) < default.work.read(*row),
                "the cell {} leaves {} as it is",
                cell.cell.name,
                row.name()
            );
        }
        let mut counts: Map<String, Value> = Row::ALL
            .iter()
            .map(|row| (row.name().to_owned(), json!(cell.work.read(*row))))
            .collect();
        counts.insert(
            SYNCS.to_owned(),
            json!(cell.work.read(Row::AuditSyncs) + cell.syncs),
        );
        if let Some(prepares) = cell.prepares {
            counts.insert(PREPARES.to_owned(), json!(prepares));
        }
        println!(
            "{}",
            json!({ "cell": cell.cell.name, "units": cell.units, "counts": counts })
        );
    }
}
