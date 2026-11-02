//! B3's charge scenario (ADR 0140), shared by `examples/counts.rs` and its
//! tests: one run charges one Usage at a time against a Limit of the Usage
//! ledger, on the crash VFS, which counts its syncs. A charge its held
//! Slices cover and a charge that takes a Slice are each their own outcome,
//! so neither averages the other away: each charge's work goes to its own.
//!
//! With no Limit holding the run, no charge takes a Slice: that cell takes
//! the Slice's work away.

use std::path::Path;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::ledger::{Charging, Holder, Ledger, Placement, SliceId, SliceSizing};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{LimitScope, LimitSetter};
use goodfetch_protocol::usage::{INSTALL, Limit, REQUESTS};
use goodfetch_test_support::CrashVfs;
use jiff::Timestamp;
use rusqlite::{Connection, OpenFlags};

/// The charges one cell makes, one of Usage each.
const CHARGES: u64 = 40;

/// The Limit's amount, of which [`SliceSizing::default`] takes a Slice of
/// [`SLICE`].
const AMOUNT: u64 = 1_000;

/// What one Slice holds.
const SLICE: u64 = 10;

/// The outcome of one charge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The Slices it holds cover it.
    Covered,
    /// It took a Slice.
    SliceTaken,
}

/// What one cell counted, each by the outcome that did it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counted {
    /// The charges the held Slices covered.
    pub(crate) covered: u64,
    /// The Slices taken.
    pub(crate) slices: u64,
    /// The SQL statements of the covered charges.
    pub(crate) statements: u64,
    /// The transactions of the charges that took a Slice.
    pub(crate) transactions: u64,
    /// The syncs of the charges that took a Slice.
    pub(crate) syncs: u64,
    /// The connections opened by every charge.
    pub(crate) opens: u64,
}

/// One extra unit of `planted`'s work in a charge of `outcome`, on `ledger`,
/// whose reads open a connection and commit its set-up, or on `spare`, a
/// connection of the scenario's own.
fn plant(planted: &str, outcome: Outcome, ledger: &Ledger, spare: &Connection, work: &CountedWork) {
    match (planted, outcome) {
        ("E3.statements", Outcome::Covered) => {
            spare
                .query_row("SELECT 1", [], |_| Ok(()))
                .expect("invariant: a literal statement");
            work.count(Row::ChargeStatements);
        }
        ("E3.slice_transactions", Outcome::SliceTaken) | ("E3.opens", _) => {
            ledger.limits().expect("invariant: the ledger opens");
        }
        ("E3.prepares", Outcome::SliceTaken) => {
            spare
                .prepare("SELECT 1")
                .expect("invariant: a literal statement");
        }
        ("E3.statements" | "E3.slice_transactions" | "E3.prepares", _) => {}
        (other, _) => panic!("no plant for the row {other}"),
    }
}

/// Callgrind dumps its profile as this is entered (`--dump-before`): what
/// came before is outside any window.
#[inline(never)]
fn window_starts() {
    std::hint::black_box(1);
}

/// Callgrind dumps its profile as this is entered: its part holds what the
/// window called, as call counts need a dump and `--toggle-collect` stops
/// costs but not calls.
#[inline(never)]
fn window_ends() {
    std::hint::black_box(2);
}

/// ADR 0059's test `Clock` start, at which every charge is placed.
fn start() -> Timestamp {
    "2026-01-05T09:00:00Z"
        .parse()
        .expect("invariant: a literal moment")
}

fn install() -> LimitScope {
    LimitScope {
        kind: INSTALL.to_owned(),
        id: String::new(),
    }
}

/// The Limit of `requests` on the install that holds a limited cell's run.
fn limit() -> Limit {
    Limit {
        scope: install(),
        usage_kind: REQUESTS.to_owned(),
        currency: None,
        period: None,
        amount: AMOUNT,
        must_understand: true,
        set_by: LimitSetter::Person,
    }
}

/// The one run's charging on `ledger`, standing on the install.
fn run_charging(ledger: &Ledger, folder: &Path) -> Charging {
    Charging::new(
        ledger.clone(),
        Holder {
            run_id: "run".to_owned(),
            epoch: 1,
            folder: folder.join("run"),
        },
        Placement {
            product: Product::Foxhound,
            scopes: vec![install()],
        },
        SliceSizing::default(),
    )
}

/// One cell of the scenario, in the folder `folder`: under a Limit when
/// `limited`, with one extra unit of `planted`'s work where it belongs, and
/// the charges of the outcome `window` between [`window_starts`] and
/// [`window_ends`]. Fails here if a charge's outcome is not the one its
/// Slice's size predicts.
pub(crate) fn charges(
    folder: &Path,
    limited: bool,
    planted: Option<&str>,
    window: Option<Outcome>,
) -> Counted {
    let name = folder
        .file_name()
        .expect("invariant: a temporary folder has a name")
        .to_string_lossy()
        .into_owned();
    let vfs = CrashVfs::register(&name, 0).expect("invariant: a new folder's name is new");
    let now = start();
    let work = CountedWork::default();
    let setting = Ledger::at(folder.join("usage-ledger.sqlite"), Some(name.clone()));
    if limited {
        setting
            .add_limit(&limit(), now)
            .expect("invariant: the ledger is writable");
    }
    let ledger = setting.counting(work.clone());
    let mut charging = run_charging(&ledger, folder);
    let spare = Connection::open_with_flags_and_vfs(
        folder.join("spare.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        name.as_str(),
    )
    .expect("invariant: the crash VFS opens a file");
    let mut counted = Counted::default();
    let mut last_slice: Option<SliceId> = None;
    for index in 0..CHARGES {
        let outcome = if limited && index % SLICE == 0 {
            Outcome::SliceTaken
        } else {
            Outcome::Covered
        };
        let before = (
            work.read(Row::ChargeStatements),
            work.read(Row::ChargeTransactions),
            vfs.syncs(),
            work.read(Row::ChargeOpens),
        );
        let mut used = None;
        let mut charge = || {
            used = Some(
                charging
                    .charge(REQUESTS, 1, now)
                    .expect("invariant: the Limit has room")
                    .used,
            );
            if let Some(planted) = planted {
                plant(planted, outcome, &ledger, &spare, &work);
            }
        };
        let windowed = window == Some(outcome);
        if windowed {
            window_starts();
        }
        charge();
        if windowed {
            window_ends();
        }
        let slice = used
            .expect("invariant: the charge ran")
            .first()
            .map(|(slice, _)| *slice);
        assert_eq!(
            limited,
            slice.is_some(),
            "charge {index}: a Limit holds the run, or nothing does"
        );
        assert_eq!(
            slice != last_slice,
            outcome == Outcome::SliceTaken,
            "charge {index} is not the outcome a Slice of {SLICE} predicts"
        );
        last_slice = slice;
        counted.opens += work.read(Row::ChargeOpens) - before.3;
        match outcome {
            Outcome::Covered => {
                counted.covered += 1;
                counted.statements += work.read(Row::ChargeStatements) - before.0;
            }
            Outcome::SliceTaken => {
                counted.slices += 1;
                counted.transactions += work.read(Row::ChargeTransactions) - before.1;
                counted.syncs += vfs.syncs() - before.2;
            }
        }
    }
    charging
        .give_back()
        .expect("invariant: the ledger is writable");
    counted
}
