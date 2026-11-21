//! A going run's look at the Usage ledger for another process's changes
//! (ADR 0041, issue 2602): a Limit an engine of another process sets
//! reaches the run's charges within its `usage.ledger_read_interval`, not
//! before, the Slices it holds covering what it charges meanwhile; and a run
//! no Limit holds leaves a ledger no engine created uncreated, however long
//! it charges.

use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_engine::ledger::{Charging, Holder, Ledger, Placement, Refusal, SliceSizing};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{LimitScope, LimitSetter};
use goodfetch_protocol::usage::{INSTALL, LEDGER_READ_INTERVAL_DEFAULT, Limit, REQUESTS};
use goodfetch_test_support::TestEnv;
use jiff::{SignedDuration, Timestamp};
use rstest::rstest;

/// The child that sets the Limit, run as a process of its own.
const CHILD: &str = "ledger_look::another_process_sets_a_limit_of_five_requests";

/// Where the child finds the ledger it sets the Limit in.
const LEDGER_VARIABLE: &str = "LEDGER_LOOK_LEDGER";

/// ADR 0059's test `Clock` start, at which the run starts.
fn start() -> Timestamp {
    "2026-01-05T09:00:00Z".parse().unwrap()
}

fn limit(amount: u64) -> Limit {
    Limit {
        scope: LimitScope {
            kind: INSTALL.to_owned(),
            id: String::new(),
        },
        usage_kind: REQUESTS.to_owned(),
        currency: None,
        period: None,
        amount,
        must_understand: true,
        set_by: LimitSetter::Person,
    }
}

/// A run standing on the install, charging against the ledger at `path`
/// and reading its Limits again each `read_interval`.
fn run_on(path: &Path, folder: &Path, read_interval: Duration) -> Charging {
    Charging::new(
        Ledger::at(path.to_owned(), None),
        Holder {
            run_id: "crawl".to_owned(),
            epoch: 1,
            folder: folder.join("crawl"),
        },
        Placement {
            product: Product::Foxhound,
            scopes: vec![LimitScope {
                kind: INSTALL.to_owned(),
                id: String::new(),
            }],
        },
        SliceSizing::default(),
    )
    .reading_every(read_interval)
}

/// Runs [`CHILD`] in a process of its own, started as `env` starts its
/// children, on the ledger at `path`, and waits for it to set its Limit.
fn set_in_another_process(env: &TestEnv, path: &Path) {
    let output = env
        .process_command(std::env::current_exe().unwrap())
        .args([CHILD, "--exact", "--include-ignored"])
        .env(LEDGER_VARIABLE, path)
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{output:?}"
    );
}

/// Covers: story:3-71
#[rstest]
#[case::a_short_interval(Duration::from_millis(100))]
#[case::the_default(LEDGER_READ_INTERVAL_DEFAULT)]
fn a_limit_another_process_sets_refuses_a_going_runs_charges_within_its_read_interval(
    #[case] read_interval: Duration,
) {
    let env = TestEnv::new();
    let path = env.home().join("usage-ledger.sqlite");
    Ledger::at(path.clone(), None)
        .add_limit(&limit(1_000_000), start())
        .unwrap();
    let mut run = run_on(&path, env.folder(), read_interval);
    run.may_start(start()).unwrap();
    run.charge(REQUESTS, 1, start()).unwrap();
    let interval = SignedDuration::try_from(read_interval).unwrap();

    set_in_another_process(&env, &path);
    let before = run.charge(
        REQUESTS,
        10,
        start() + interval - SignedDuration::from_millis(1),
    );
    let within = run.charge(REQUESTS, 10, start() + interval);

    assert!(
        before.is_ok() && matches!(within, Err(Refusal::Reached(_))),
        "{before:?} then {within:?}"
    );
}

/// A run no Limit holds reads a ledger no engine created once per read
/// interval at most, and no read creates it: the file is still absent after
/// a hundred charges over ten intervals.
///
/// Covers: story:3-71
#[test]
fn a_run_no_limit_holds_leaves_a_ledger_no_engine_created_uncreated_however_long_it_charges() {
    let env = TestEnv::new();
    let path = env.home().join("usage-ledger.sqlite");
    let read_interval = Duration::from_millis(100);
    let mut run = run_on(&path, env.folder(), read_interval);
    run.may_start(start()).unwrap();

    for charge in 0..100 {
        let at = start() + SignedDuration::from_millis(10 * charge);
        run.charge(REQUESTS, 1, at).unwrap();
    }

    assert!(!path.exists());
}

/// The other process: an engine that sets a Limit of five requests on the
/// install in the ledger [`LEDGER_VARIABLE`] names.
#[test]
#[ignore = "run as a child by ledger_look::a_limit_another_process_sets_refuses_a_going_runs_charges_within_its_read_interval"]
#[expect(
    clippy::disallowed_methods,
    reason = "the child's main is its test: it reads the ledger its parent names"
)]
fn another_process_sets_a_limit_of_five_requests() {
    let path = PathBuf::from(std::env::var_os(LEDGER_VARIABLE).unwrap());

    Ledger::at(path, None)
        .add_limit(&limit(5), start())
        .unwrap();
}
