//! B3's counts cell (ADR 0140), as far as Usage charges take it: runs
//! `support/charge_counts.rs`' charges once under a Limit and once with none,
//! and prints what each counted as one JSON line, which `xtask counts` divides
//! and holds to the Quality register's limits. An example and not a
//! `[[bench]]`, as `xtask perf` runs every bench target under gungraun.
//!
//! What no wrapper sees comes from outside the process. `E3.prepares` is
//! Callgrind's call count of `sqlite3_prepare_v3` inside the charges that took
//! a Slice, run again under Valgrind by this same executable; the covered
//! charges' count must equal the statements the ledger counted, so a statement
//! added beside the counted read fails here. `E3.slice_transactions` is the
//! larger of the commit hook's transactions and the crash VFS's syncs, which
//! the row holds to one limit.
//!
//! `--plant <row>` does one extra unit of that row's work per unit of its
//! outcome.
#![expect(clippy::disallowed_methods, reason = "benchmark harness, ADR 0071")]
#![expect(
    clippy::print_stdout,
    reason = "the benchmark's one output, which `xtask counts` reads"
)]

#[path = "support/charge_counts.rs"]
mod charge_counts;

use std::path::Path;
use std::process::Command;

use charge_counts::{Counted, Outcome, charges};
use serde_json::json;

/// A cell: its name and whether a Limit holds its run. With none, the Slice's
/// work goes with the Limit.
const CELLS: [(&str, bool); 2] = [("default", true), ("no_limit", false)];

/// Runs one cell under this executable's `--window`: Callgrind's own process.
fn window(cell: &str, outcome: &str, planted: Option<&str>) {
    let limited = CELLS
        .iter()
        .find(|(name, _)| *name == cell)
        .unwrap_or_else(|| panic!("no cell {cell}"))
        .1;
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
    let output = command
        .output()
        .expect("invariant: Valgrind runs, as ADR 0071's benchmarks need it");
    assert!(
        output.status.success(),
        "Callgrind failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    prepares(folder.path())
}

/// One cell's line, its rows from `counted` and Callgrind's preparations.
fn line(cell: &str, counted: Counted, slice_prepares: u64) -> serde_json::Value {
    json!({
        "cell": cell,
        "units": {
            "covered_charges": counted.covered,
            "slices": counted.slices,
            "charges": counted.covered + counted.slices,
        },
        "counts": {
            "E3.statements": counted.statements,
            "E3.slice_transactions": counted.transactions.max(counted.syncs),
            "E3.opens": counted.opens,
            "E3.prepares": slice_prepares,
        },
    })
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let planted = match arguments.as_slice() {
        [] => None,
        ["--plant", row] => Some(*row),
        ["--window", cell, outcome] => return window(cell, outcome, None),
        ["--window", cell, outcome, row] => return window(cell, outcome, Some(row)),
        other => panic!("usage: counts-b3 [--plant <row>], not {other:?}"),
    };
    let mut lines = Vec::new();
    let mut default = None;
    for (cell, limited) in CELLS {
        let folder = tempfile::tempdir().expect("invariant: the OS gives a temporary folder");
        let counted = charges(folder.path(), limited, planted, None);
        // With no Limit, the first charge is a covered one and opens the
        // ledger: its set-up is no statement a charge reads with, so only the
        // cell whose first charge takes a Slice compares the two.
        let slice_prepares = if limited {
            if planted.is_none() {
                assert_eq!(
                    callgrind(cell, "covered", None),
                    counted.statements,
                    "the cell {cell}'s covered charges prepared what the ledger did not count"
                );
            }
            callgrind(cell, "slice", planted)
        } else {
            0
        };
        match default {
            None => default = Some(counted),
            Some(default) => assert!(
                counted.opens < default.opens && counted.transactions < default.transactions,
                "the cell {cell} leaves the Slice's work as it is: {counted:?} against {default:?}"
            ),
        }
        lines.push(line(cell, counted, slice_prepares));
    }
    for line in lines {
        println!("{line}");
    }
}
