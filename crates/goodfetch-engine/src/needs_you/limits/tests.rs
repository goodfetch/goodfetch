//! The Runs a Limit stopped, at the source's interface (spec #592's Seam
//! 3), with a real Run index and Usage ledger in SQLite and
//! `Clock::paused`: a Run stopped by the install's daily Limit is listed
//! while that Limit still holds it, set again since the stop with nothing
//! more left included, and not once a person raised it, once its period
//! started again, or when only another Limit has nothing left.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::{
    Completeness, Lifecycle, LimitScope, LimitSetter, Period, PeriodLength, PointInTime, Presence,
    ReachedLimit, RunId, RunIndexRow, RunKind, StopReason, Tags,
};
use goodfetch_protocol::usage::Limit;
use jiff::Timestamp;
use rstest::rstest;
use tokio::sync::watch;

use super::runs_stopped;
use crate::clock::Clock;
use crate::governor::StorageSample;
use crate::home::Home;
use crate::known_certificates::KnownCertificates;
use crate::ledger::{Holder, Ledger, SliceSizing};
use crate::needs_you::SourceContext;
use crate::run_index::{IndexFile, RunIndex};
use crate::settings::{Settings, SettingsFiles};
use crate::sqlite_store::StoreFile as _;
use crate::tasks::{BlockingPolicy, TaskOwner};

/// When the Run stopped: 10:00 UTC on a day whose Limit resets at midnight.
const STOPPED_AT: &str = "2026-10-24T10:00:00Z";

/// The install's daily Limit of one request, in UTC, set by `set_by` at
/// `amount`, or on `scope` instead.
fn daily(scope: &str, amount: u64, set_by: LimitSetter) -> Limit {
    Limit {
        scope: LimitScope {
            kind: scope.to_owned(),
            id: String::new(),
        },
        usage_kind: "requests".to_owned(),
        currency: None,
        period: Some(Period {
            every: PeriodLength::Day,
            time_zone: "UTC".to_owned(),
        }),
        amount,
        must_understand: true,
        set_by,
    }
}

/// The Run `capped`, stopped by the install's daily Limit.
fn stopped() -> RunIndexRow {
    let at = PointInTime::from_rfc3339(STOPPED_AT.to_owned());
    RunIndexRow {
        run_id: RunId::parse("capped").unwrap(),
        location: "runs/capped".to_owned(),
        kind: RunKind::Collie,
        needs: Product::Collie,
        tags: Tags::default(),
        lifecycle: Lifecycle::Stopped,
        verdict: None,
        completeness: Some(Completeness::Incomplete),
        started_at: at.clone(),
        ended_at: Some(at),
        owner: None,
        presence: Presence::Present,
        progress_message: None,
        stop_reason: Some(StopReason::Limit(Box::new(ReachedLimit {
            scope: LimitScope {
                kind: "install".to_owned(),
                id: String::new(),
            },
            usage_kind: "requests".to_owned(),
            currency: None,
            period: Some(Period {
                every: PeriodLength::Day,
                time_zone: "UTC".to_owned(),
            }),
            resets_at: Some(PointInTime::from_rfc3339("2026-10-25T00:00:00Z".to_owned())),
        }))),
        replayed: false,
    }
}

/// A Usage ledger where `limit` has nothing left on the day the Run
/// stopped: its one request is held by the Run.
fn used_up(ledger: &Ledger, limit: &Limit) {
    let now: Timestamp = STOPPED_AT.parse().unwrap();
    let id = ledger.add_limit(limit, now).unwrap();
    let stored = ledger
        .limits()
        .unwrap()
        .into_iter()
        .find(|stored| stored.id == id)
        .unwrap();
    let holder = Holder {
        run_id: "capped".to_owned(),
        epoch: 1,
        folder: "runs/capped".into(),
    };
    ledger
        .take_slice(&mut None, &stored, &holder, now, SliceSizing::default(), 1)
        .unwrap()
        .unwrap();
}

/// What the ledger holds besides the used-up Limit, by case.
#[derive(Clone, Copy)]
enum Then {
    Nothing,
    /// A person raised it to 5, half an hour after the stop.
    Raised,
    /// A person set it again at the 1 it allowed, half an hour after the stop.
    RaisedToNothingMore,
    OnlyAnotherScope,
}

/// The kinds and Run IDs the source lists, `after` the stop, for a ledger
/// set as `then` says.
fn listed(then: Then, after: Duration) -> Vec<(String, Option<String>)> {
    let folder = tempfile::tempdir().unwrap();
    let index = folder.path().join("run-index.sqlite");
    let version = ProductVersion::parse("1.0.0").unwrap();
    let file = IndexFile::of(index.clone(), &version);
    file.write(&file.open().unwrap(), &stopped()).unwrap();
    let ledger = Ledger::at(folder.path().join("usage.sqlite"), None);
    let half_an_hour_later = STOPPED_AT.parse::<Timestamp>().unwrap() + Duration::from_mins(30);
    match then {
        Then::Nothing => used_up(&ledger, &daily("install", 1, LimitSetter::Client)),
        Then::Raised | Then::RaisedToNothingMore => {
            used_up(&ledger, &daily("install", 1, LimitSetter::Client));
            let amount = if matches!(then, Then::Raised) { 5 } else { 1 };
            ledger
                .raise(
                    &daily("install", amount, LimitSetter::Person),
                    half_an_hour_later,
                )
                .unwrap();
        }
        Then::OnlyAnotherScope => used_up(&ledger, &daily("galaxy", 1, LimitSetter::Client)),
    }
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let stopped_at: Timestamp = STOPPED_AT.parse().unwrap();
        let context = SourceContext {
            clock: Clock::paused(SystemTime::from(stopped_at) + after),
            storage: watch::channel(Arc::new(StorageSample::default())).1,
            run_index: RunIndex::of(index, &version, None, Arc::new(owner.child())),
            settings: Settings::new(
                SettingsFiles::of(&Home::new(folder.path().to_owned()), &version),
                Product::Collie,
                owner.child(),
            ),
            ledger,
            known_certificates: KnownCertificates::of(
                &Home::new(folder.path().to_owned()),
                None,
                Arc::new(owner.child()),
            ),
            owner: Arc::new(owner.child()),
        };
        let items = runs_stopped().0.items(&context).await.unwrap();
        owner.stop().await;
        items
            .into_iter()
            .map(|item| (item.kind, item.run.map(|run| run.as_str().to_owned())))
            .collect()
    })
}

/// Covers: story:12-100, story:12-32
#[rstest]
#[case::while_its_limit_has_nothing_left(Then::Nothing, Duration::from_hours(1), true)]
#[case::not_once_a_person_raised_it(Then::Raised, Duration::from_hours(1), false)]
#[case::while_a_person_set_it_again_with_nothing_more_left(
    Then::RaisedToNothingMore,
    Duration::from_hours(1),
    true
)]
#[case::not_once_its_period_started_again(Then::Nothing, Duration::from_hours(15), false)]
#[case::not_when_only_another_limit_holds_nothing_left(
    Then::OnlyAnotherScope,
    Duration::from_hours(1),
    false
)]
fn a_run_a_limit_stopped_is_listed_while_that_limit_still_holds_it(
    #[case] then: Then,
    #[case] after: Duration,
    #[case] listed_then: bool,
) {
    let expected = if listed_then {
        vec![("run_stopped_by_limit".to_owned(), Some("capped".to_owned()))]
    } else {
        Vec::new()
    };

    assert_eq!(listed(then, after), expected);
}
