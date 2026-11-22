//! A proxy's limits through the engine's rule interface (spec #213's Seam
//! 4, as the per-domain rule is tested), two runs' rules under their own
//! settings on one Politeness file's table, at wall readings the tests
//! give: each run's own cap against the count in flight together, the
//! interval since any sharing run's last start, the ceiling a proxy's own
//! refusal halves for every run, and its growing back after a quiet
//! period, which a site's answer never touches; and a wait for room on an
//! in-memory Politeness file, on Tokio's paused clock, ending with its Run.
#![expect(
    clippy::disallowed_methods,
    reason = "a wait's test runs on the test Clock and Tokio's paused clock (ADR 0059); its SQL statements are its own, on no Shared path (ADR 0140)"
)]

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::politeness::{
    Ending, Hold, Holder, Politeness, PolitenessOptions, SharedRule, Verdict,
};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{ProxyRule, TABLES, taken};

/// A rule's steps on a Politeness file's connection, each loading the
/// key's state and saving it as a Permit's take and give-back do.
mod on_file {
    use std::time::SystemTime;

    use goodfetch_engine::politeness::{Ending, KeyState, PermitRule, Verdict};
    use rusqlite::Connection;

    use super::ProxyRule;
    use crate::proxy::limits::Limits;

    pub(super) trait OnFile {
        fn admit(
            &self,
            file: &Connection,
            key: &str,
            in_flight: u64,
            now: SystemTime,
        ) -> rusqlite::Result<Verdict>;

        fn started(&self, file: &Connection, key: &str, now: SystemTime) -> rusqlite::Result<()>;

        fn ended(
            &self,
            file: &Connection,
            key: &str,
            in_flight: u64,
            ending: Ending,
            now: SystemTime,
        ) -> rusqlite::Result<Option<SystemTime>>;
    }

    impl OnFile for ProxyRule {
        fn admit(
            &self,
            file: &Connection,
            key: &str,
            in_flight: u64,
            now: SystemTime,
        ) -> rusqlite::Result<Verdict> {
            let limits = Limits::load(file, key, ())?;
            Ok(PermitRule::admit(self, &limits, in_flight, now))
        }

        fn started(&self, file: &Connection, key: &str, now: SystemTime) -> rusqlite::Result<()> {
            let mut limits = Limits::load(file, key, ())?;
            PermitRule::started(self, &mut limits, now);
            limits.save(file, key, &())
        }

        fn ended(
            &self,
            file: &Connection,
            key: &str,
            in_flight: u64,
            ending: Ending,
            now: SystemTime,
        ) -> rusqlite::Result<Option<SystemTime>> {
            let mut limits = Limits::load(file, key, ())?;
            let (retry_at, ()) = PermitRule::ended(self, &mut limits, in_flight, ending, now);
            limits.save(file, key, &())?;
            Ok(retry_at)
        }
    }
}

use on_file::OnFile as _;

const KEY: &str = "proxy http://127.0.0.1:3128 ";

fn file() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    connection
}

fn rule_of(run: Value) -> Result<ProxyRule, String> {
    let Value::Object(run) = run else {
        panic!("settings are a table");
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &run, Product::Collie).unwrap(),
        ..Layers::default()
    };
    ProxyRule::of(&resolve(&layers, Product::Collie, Product::Collie).unwrap())
}

fn rule(run: Value) -> ProxyRule {
    rule_of(run).unwrap()
}

fn concurrency(cap: u64) -> ProxyRule {
    rule(json!({ "politeness": { "proxy": { "concurrency": cap } } }))
}

fn rate(per_second: u64) -> ProxyRule {
    rule(json!({ "politeness": { "proxy": { "rate": per_second } } }))
}

fn unlimited() -> ProxyRule {
    rule(json!({}))
}

fn at(milliseconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_hours(491_001) + Duration::from_millis(milliseconds)
}

/// Covers: story:5-90, setting:politeness.proxy.concurrency
#[test]
fn a_stricter_run_waits_while_a_looser_one_goes_up_to_its_own_cap_through_one_proxy() {
    let file = file();

    let verdicts = (
        concurrency(1).admit(&file, KEY, 1, at(0)).unwrap(),
        concurrency(2).admit(&file, KEY, 1, at(0)).unwrap(),
        concurrency(2).admit(&file, KEY, 2, at(0)).unwrap(),
        unlimited().admit(&file, KEY, 50, at(0)).unwrap(),
    );

    assert_eq!(
        verdicts,
        (Verdict::Busy, Verdict::Go, Verdict::Busy, Verdict::Go)
    );
}

/// A concurrency of 0 would admit no request through the proxy, so every
/// navigation through it would wait forever: it is refused, and 1 is the
/// least it takes.
///
/// Covers: story:5-90, setting:politeness.proxy.concurrency
#[test]
fn a_proxy_concurrency_of_0_is_refused() {
    assert_eq!(
        (
            rule_of(json!({ "politeness": { "proxy": { "concurrency": 0 } } })).is_err(),
            rule_of(json!({ "politeness": { "proxy": { "concurrency": 1 } } })).is_ok(),
        ),
        (true, true)
    );
}

/// Covers: story:5-90, setting:politeness.proxy.rate
#[test]
fn a_run_starts_through_a_proxy_only_once_its_interval_passed_since_any_runs_last_start() {
    let file = file();
    unlimited().started(&file, KEY, at(1_000)).unwrap();

    let verdicts = (
        rate(2).admit(&file, KEY, 0, at(1_200)).unwrap(),
        rate(2).admit(&file, KEY, 0, at(1_500)).unwrap(),
        rate(1).admit(&file, KEY, 0, at(1_500)).unwrap(),
    );

    assert_eq!(
        verdicts,
        (
            Verdict::Until(at(1_500), Hold::Delay),
            Verdict::Go,
            Verdict::Until(at(2_000), Hold::Delay),
        )
    );
}

/// Covers: story:5-90
#[rstest]
#[case::from_the_count_in_flight(&[3], 2)]
#[case::from_the_ceiling_it_left(&[3, 1], 1)]
#[case::never_below_one(&[0, 0, 0], 1)]
fn a_refusal_by_the_proxy_itself_halves_its_ceiling_for_every_run(
    #[case] others_in_flight: &[u64],
    #[case] ceiling: u64,
) {
    let file = file();

    for others in others_in_flight {
        unlimited()
            .ended(&file, KEY, *others, Ending::TooManyRequests(None), at(0))
            .unwrap();
    }

    assert_eq!(
        (
            unlimited().admit(&file, KEY, ceiling - 1, at(0)).unwrap(),
            concurrency(5).admit(&file, KEY, ceiling, at(0)).unwrap(),
        ),
        (Verdict::Go, Verdict::Busy)
    );
}

/// Covers: story:5-90
#[test]
fn a_ceiling_grows_back_by_one_with_each_success_after_a_quiet_period() {
    let file = file();
    unlimited()
        .ended(&file, KEY, 1, Ending::TooManyRequests(None), at(0))
        .unwrap();

    unlimited()
        .ended(&file, KEY, 0, Ending::Done, at(5_000))
        .unwrap();
    let early = unlimited().admit(&file, KEY, 1, at(5_000)).unwrap();
    unlimited()
        .ended(&file, KEY, 0, Ending::Done, at(10_000))
        .unwrap();
    let grown = (
        unlimited().admit(&file, KEY, 1, at(10_000)).unwrap(),
        unlimited().admit(&file, KEY, 2, at(10_000)).unwrap(),
    );

    assert_eq!(
        (early, grown),
        (Verdict::Busy, (Verdict::Go, Verdict::Busy))
    );
}

/// A ceiling grows back only while it binds: a request that ended well
/// with fewer in flight than the ceiling teaches nothing, so the ceiling
/// never grows past what was tried.
///
/// Covers: story:5-90
#[test]
fn a_ceiling_that_does_not_bind_grows_no_further() {
    let file = file();
    unlimited()
        .ended(&file, KEY, 7, Ending::TooManyRequests(None), at(0))
        .unwrap();

    for quiet in 1..=3 {
        unlimited()
            .ended(&file, KEY, 0, Ending::Done, at(quiet * 10_000))
            .unwrap();
    }

    assert_eq!(
        (
            unlimited().admit(&file, KEY, 3, at(30_000)).unwrap(),
            unlimited().admit(&file, KEY, 4, at(30_000)).unwrap(),
        ),
        (Verdict::Go, Verdict::Busy)
    );
}

/// A refusal halves the ceiling from what was in flight when it came, when
/// that is below the ceiling: the proxy refused that many, not the ceiling.
///
/// Covers: story:5-90
#[test]
fn a_refusal_halves_from_the_count_in_flight_when_it_is_below_the_ceiling() {
    let file = file();
    unlimited()
        .ended(&file, KEY, 15, Ending::TooManyRequests(None), at(0))
        .unwrap();

    unlimited()
        .ended(&file, KEY, 3, Ending::TooManyRequests(None), at(1_000))
        .unwrap();

    assert_eq!(
        (
            unlimited().admit(&file, KEY, 1, at(1_000)).unwrap(),
            unlimited().admit(&file, KEY, 2, at(1_000)).unwrap(),
        ),
        (Verdict::Go, Verdict::Busy)
    );
}

/// Covers: story:5-90
#[test]
fn a_request_that_ended_well_leaves_an_unlearned_proxy_without_a_ceiling() {
    let file = file();

    for others in [0, 4, 9] {
        unlimited()
            .ended(&file, KEY, others, Ending::Done, at(0))
            .unwrap();
    }

    assert_eq!(
        unlimited().admit(&file, KEY, 100, at(0)).unwrap(),
        Verdict::Go
    );
}

/// A Run's wait for room through a proxy every Permit of which another
/// request holds ends once the Run is cancelled, with no Permit, so its end
/// never waits for another Run's request.
///
/// Covers: story:5-90
#[tokio::test(start_paused = true)]
async fn a_wait_for_room_through_a_proxy_ends_once_its_run_is_cancelled() {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let politeness = Politeness::in_memory(&PolitenessOptions::default(), &owner).unwrap();
    let clock = Clock::paused(at(0));
    let holder = Holder {
        run_id: "waiting".to_owned(),
        epoch: 1,
        run_folder: PathBuf::from("waiting"),
        delegates: true,
    };
    let rule = SharedRule::new(concurrency(1));
    let ending = CancellationToken::new();
    let held = taken(&politeness, KEY, &holder, &rule, &clock, &ending).await;

    let (waited, ()) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(600),
            taken(&politeness, KEY, &holder, &rule, &clock, &ending),
        ),
        async {
            clock.sleep(Duration::from_secs(45)).await;
            ending.cancel();
        },
    );

    assert_eq!((held.is_some(), waited.ok()), (true, Some(None)));
}
