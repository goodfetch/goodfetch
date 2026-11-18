//! The Usage ledger's writer for the Compatibility corpus (ADR 0066):
//! through the engine's own ledger at ADR 0059's test `Clock` start, adds a
//! monthly install Limit a person set, a daily Daemon Limit in Paris that
//! the operator set, a Client connection's own Limit, Foxhound's
//! `paid_fetch` Limit in dollars and a weekly install Limit in Tokyo that
//! major 0.0's managed Operator policy names, and the Limits a program gave
//! a Run at its start, in requests and in dollars of `paid_fetch`, which
//! hold its own `run` scope; then one run gives back what it did not use,
//! one whose engine died is reclaimed from what it journaled, one whose
//! state cannot be read is charged in full for review, and one still holds
//! its Slice. It writes the ledger, as SQL text, into the folder given as
//! the only argument, as `usage-ledger.sql`. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use goodfetch_engine::ledger::{Charging, Holder, Ledger, Placement, Probed, SliceSizing};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::LimitSetter;
use goodfetch_protocol::run::{LimitScope, Period, PeriodLength, RunLimit};
use goodfetch_protocol::usage::{Limit, ManagedLimit};

/// ADR 0059's test `Clock` start.
const START: &str = "2026-01-05T09:00:00Z";

/// Where the runs' folders were, as a home on Linux has them.
const RUNS: &str = "/home/someone/.local/share/goodfetch/0.0/runs";

fn scope(kind: &str, id: &str) -> LimitScope {
    LimitScope {
        kind: kind.to_owned(),
        id: id.to_owned(),
    }
}

/// The Limits it adds, each as its setter wrote it.
fn limits() -> [Limit; 4] {
    [
        Limit {
            scope: scope("install", ""),
            usage_kind: "requests".to_owned(),
            currency: None,
            period: Some(Period {
                every: PeriodLength::Month,
                time_zone: "UTC".to_owned(),
            }),
            amount: 100_000,
            must_understand: true,
            set_by: LimitSetter::Person,
        },
        Limit {
            scope: scope("daemon", "0.0"),
            usage_kind: "requests".to_owned(),
            currency: None,
            period: Some(Period {
                every: PeriodLength::Day,
                time_zone: "Europe/Paris".to_owned(),
            }),
            amount: 20_000,
            must_understand: true,
            set_by: LimitSetter::Person,
        },
        Limit {
            scope: scope("connection", "corpus-connection"),
            usage_kind: "requests".to_owned(),
            currency: None,
            period: None,
            amount: 500,
            must_understand: true,
            set_by: LimitSetter::Client,
        },
        Limit {
            scope: scope("install", ""),
            usage_kind: "paid_fetch".to_owned(),
            currency: Some("USD".to_owned()),
            period: Some(Period {
                every: PeriodLength::Month,
                time_zone: "America/New_York".to_owned(),
            }),
            amount: 25_000_000,
            must_understand: true,
            set_by: LimitSetter::Person,
        },
    ]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let scratch = tempfile::tempdir()?;
    let path = scratch.path().join("usage-ledger.sqlite");
    let ledger = Ledger::at(path.clone(), None);
    let start: jiff::Timestamp = START.parse()?;
    let limits = limits();
    for limit in &limits {
        ledger.add_limit(limit, start)?;
    }
    let managed = ManagedLimit {
        usage_kind: "requests".to_owned(),
        currency: None,
        period: Some(Period {
            every: PeriodLength::Week,
            time_zone: "Asia/Tokyo".to_owned(),
        }),
        amount: 50_000,
    };
    ledger.write_managed("0.0", &[managed.limit()], start)?;
    let given = [
        RunLimit {
            usage_kind: "requests".to_owned(),
            currency: None,
            amount: 1_000,
        },
        RunLimit {
            usage_kind: "paid_fetch".to_owned(),
            currency: Some("USD".to_owned()),
            amount: 2_000_000,
        },
    ];
    ledger
        .give_run_limits("live-run", &given, true, start)?
        .map_err(|refused| format!("the Run's Limits were refused: {refused:?}"))?;
    let placement = Placement {
        product: Product::Foxhound,
        scopes: vec![scope("install", ""), scope("daemon", "0.0")],
    };
    let charging = |run_id: &str| {
        Charging::new(
            ledger.clone(),
            Holder {
                run_id: run_id.to_owned(),
                epoch: 1,
                folder: PathBuf::from(RUNS).join(run_id),
            },
            placement.clone(),
            SliceSizing::default(),
        )
    };
    let mut ended = charging("ended-run");
    ended.charge("requests", 40, start)?;
    ended.give_back()?;
    let mut dead = charging("dead-run");
    let journaled = dead.charge("requests", 12, start)?;
    let mut unreadable = charging("unreadable-run");
    unreadable.charge("requests", 3, start)?;
    let mut live = charging("live-run");
    live.charge("requests", 5, start)?;
    let used: BTreeMap<i64, u64> = journaled.used.into_iter().collect();
    ledger.reclaim(&|holder| match holder.run_id.as_str() {
        "dead-run" => Probed::Gone {
            used: used.clone(),
            beyond: 0,
        },
        "unreadable-run" => Probed::Unreadable,
        _ => Probed::Live,
    })?;
    let connection = rusqlite::Connection::open(&path)?;
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("usage-ledger.sql"),
        goodfetch_test_support::sql_dump(&connection)?,
    )?;
    Ok(())
}
