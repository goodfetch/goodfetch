//! B3's charge scenario (ADR 0140), shared by `examples/counts.rs` and its
//! tests: one run starts as a Run does, its Limits read on the connection
//! it keeps, then charges one Usage at a time against a Limit of the Usage
//! ledger, on the crash VFS, which counts its syncs. Each round charges a
//! request, and reserves proxy bytes ahead and settles them as a provider
//! Session's relay does through `Run::reserve_part` and `Run::settle_part`
//! (#2392), and makes one paid call, a paid solve whose worst case it
//! reserves before it pays and settles at what it cost, as `Run::reserve`
//! and `Run::settle` do (#2356), each against a Limit on its own kind. A
//! paid call is one charge of the rows, its settlement a step in memory
//! within it. A charge its held Slices cover and a charge that takes a Slice
//! are each their own outcome, so neither averages the other away: each
//! charge's work goes to its own.
//!
//! With no Limit holding the run, no charge takes a Slice: that cell takes
//! the Slice's work away.

use std::path::Path;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::ledger::{
    Change, Charging, Holder, Ledger, Placement, Refusal, SliceId, SliceSizing, Unmade,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{LimitScope, LimitSetter};
use goodfetch_protocol::usage::{INSTALL, Limit, PAID_SOLVES, PROXY_BYTES, REQUESTS};
use goodfetch_test_support::CrashVfs;
use jiff::Timestamp;
use rusqlite::{Connection, OpenFlags};

/// The rounds one cell makes, each one charge of a request, one allowance
/// of proxy bytes and one paid call.
const ROUNDS: u64 = 40;

/// Each Limit's amount, in requests, in allowances of proxy bytes or in
/// paid calls' worst cases, of which [`SliceSizing::default`] takes a Slice
/// of [`SLICE`].
const AMOUNT: u64 = 1_000;

/// What one Slice holds, in requests, in allowances of proxy bytes or in
/// paid calls' worst cases.
const SLICE: u64 = 10;

/// The bytes a relayed connection's first allowance reserves ahead, as a
/// provider Session's relay does, and carries in full before it settles.
const ALLOWANCE: u64 = 16 * 1024;

/// The currency a paid call is paid in.
const CURRENCY: &str = "USD";

/// A paid call's worst case, in millionths of [`CURRENCY`], which it
/// reserves before it pays.
const WORST: u64 = 4_000;

/// What a paid call cost, half its worst case: its settlement gives the
/// rest back to the run's Slices, so a Slice of [`SLICE`] worst cases lasts
/// more calls than [`SLICE`].
const COST: u64 = WORST / 2;

/// The outcome of one charge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The Slices it holds cover it.
    Covered,
    /// It took a Slice.
    SliceTaken,
}

/// What one charge of a round charges, each against a Limit on its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Charge {
    /// One request, as `Run::charge` charges it.
    Request = 0,
    /// One allowance of proxy bytes ([`proxy_bytes`]).
    ProxyBytes = 1,
    /// One paid call ([`paid_call`]).
    PaidCall = 2,
}

/// Every kind of charge, in the order a round makes them.
const KINDS: [Charge; 3] = [Charge::Request, Charge::ProxyBytes, Charge::PaidCall];

impl Charge {
    /// Makes this charge on `charging`; answers what it used of each Slice.
    fn make(self, charging: &mut Charging, now: Timestamp) -> Vec<(SliceId, u64)> {
        match self {
            Self::Request => {
                charging
                    .charge(REQUESTS, 1, now)
                    .expect("invariant: the Limit has room")
                    .used
            }
            Self::ProxyBytes => proxy_bytes(charging, now),
            Self::PaidCall => paid_call(charging, now),
        }
    }

    /// What it takes of the run's Slices before it settles, which they
    /// must have room for, or it takes a Slice.
    fn worst(self) -> u64 {
        match self {
            Self::Request => 1,
            Self::ProxyBytes => ALLOWANCE,
            Self::PaidCall => WORST,
        }
    }

    /// What it leaves taken of the run's Slices once it settled.
    fn cost(self) -> u64 {
        match self {
            Self::Request => 1,
            Self::ProxyBytes => ALLOWANCE,
            Self::PaidCall => COST,
        }
    }
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
            #[expect(
                clippy::disallowed_methods,
                reason = "the planted statement the prove run counts, outside the ledger"
            )]
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

/// The Limit of `amount` on `usage_kind` in `currency` on the install,
/// which holds a limited cell's run.
fn limit(usage_kind: &str, currency: Option<&str>, amount: u64) -> Limit {
    Limit {
        scope: install(),
        usage_kind: usage_kind.to_owned(),
        currency: currency.map(str::to_owned),
        period: None,
        amount,
        must_understand: true,
        set_by: LimitSetter::Person,
    }
}

/// One allowance of [`ALLOWANCE`] proxy bytes, reserved ahead, down to one
/// byte as the Limits allow, and settled once the connection carried all of
/// it: the path of a provider Session's relay through `Run::reserve_part`
/// and `Run::settle_part`, a step in memory when the held Slices cover it
/// and a change on the ledger when not. Answers what it used of each Slice.
fn proxy_bytes(charging: &mut Charging, now: Timestamp) -> Vec<(SliceId, u64)> {
    let reserve = |change: &mut Change<'_>| {
        let mut amount = ALLOWANCE;
        loop {
            match change.reserve(PROXY_BYTES, None, amount) {
                Err(Unmade::Refused(Refusal::Reached(_))) if amount > 1 => {
                    amount = (amount / 2).max(1);
                }
                reserved => break reserved,
            }
        }
    };
    let (reservation, charged) = reserved(charging, now, reserve);
    // What the run would journal of the settlement: a step in memory.
    let _ = charging.settle(reservation, ALLOWANCE);
    charged.used
}

/// What `reserve` makes on `charging`, as `Run::reserve` and
/// `Run::reserve_part` make it: a step in memory when the held Slices cover
/// it, and a change on the ledger when not.
fn reserved<T>(
    charging: &mut Charging,
    now: Timestamp,
    reserve: impl Fn(&mut Change<'_>) -> Result<T, Unmade>,
) -> T {
    match charging.change_held(now, &reserve) {
        Some(reserved) => reserved,
        None => charging
            .change(now, reserve)
            .expect("invariant: the Limit has room"),
    }
}

/// One paid call, a paid solve's: its worst case of [`WORST`] reserved
/// before it is paid, as `Run::reserve` reserves it ([`reserved`]), then
/// settled at [`COST`], as `Run::settle` settles it, a step in memory.
/// Fails unless what the run journals of the two, the reservation before it
/// pays and the settlement after, counts the call's Usage once, at what it
/// cost. Answers what the reservation used of each Slice.
fn paid_call(charging: &mut Charging, now: Timestamp) -> Vec<(SliceId, u64)> {
    let reserve = |change: &mut Change<'_>| change.reserve(PAID_SOLVES, Some(CURRENCY), WORST);
    let (reservation, charged) = reserved(charging, now, reserve);
    let settled = charging.settle(reservation, COST);
    let journaled: i64 = charged
        .counted
        .iter()
        .chain(&settled.counted)
        .map(|counted| counted.delta)
        .sum();
    assert_eq!(
        journaled,
        i64::try_from(COST).expect("invariant: a small cost"),
        "a paid call's journals count its Usage once, at what it cost"
    );
    charged.used
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
/// [`window_ends`]. Fails here if a charge's outcome is not the one the
/// room left of its Slices predicts.
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
        for limit in [
            limit(REQUESTS, None, AMOUNT),
            limit(PROXY_BYTES, None, AMOUNT * ALLOWANCE),
            limit(PAID_SOLVES, Some(CURRENCY), AMOUNT * WORST),
        ] {
            setting
                .add_limit(&limit, now)
                .expect("invariant: the ledger is writable");
        }
    }
    let ledger = setting.counting(work.clone());
    let mut charging = run_charging(&ledger, folder);
    charging
        .may_start(now)
        .expect("invariant: the Limits have room for a run to start");
    let spare = Connection::open_with_flags_and_vfs(
        folder.join("spare.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        name.as_str(),
    )
    .expect("invariant: the crash VFS opens a file");
    let mut counted = Counted::default();
    let mut last_slices: [Option<SliceId>; KINDS.len()] = [None; KINDS.len()];
    // What is left of the Slices the run holds of each kind's Limit: a
    // charge that needs more takes a Slice of `SLICE` worst cases.
    let mut rooms = [0_u64; KINDS.len()];
    let rounds = (0..ROUNDS).flat_map(|round| KINDS.map(|kind| (round, kind)));
    for (round, kind) in rounds {
        let room = &mut rooms[kind as usize];
        let outcome = if limited && *room < kind.worst() {
            *room += SLICE * kind.worst();
            Outcome::SliceTaken
        } else {
            Outcome::Covered
        };
        *room = room.saturating_sub(kind.cost());
        let before = (
            work.read(Row::ChargeStatements),
            work.read(Row::ChargeTransactions),
            vfs.syncs(),
            work.read(Row::ChargeOpens),
        );
        let mut used = None;
        let mut charge = || {
            used = Some(kind.make(&mut charging, now));
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
        // The last Slice a charge drew on is its newest: a charge its
        // Slices fall short for draws on what is left of them first.
        let slice = used
            .expect("invariant: the charge ran")
            .last()
            .map(|(slice, _)| *slice);
        assert_eq!(
            limited,
            slice.is_some(),
            "round {round}'s {kind:?}: a Limit holds the run, or nothing does"
        );
        assert_eq!(
            slice != last_slices[kind as usize],
            outcome == Outcome::SliceTaken,
            "round {round}'s {kind:?} is not the outcome its Slices' room predicts"
        );
        last_slices[kind as usize] = slice;
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
