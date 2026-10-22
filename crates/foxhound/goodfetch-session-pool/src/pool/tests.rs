//! Session pool cases (spec #582's pure rules, in process): each case drives
//! the pool through its interface, as a crawl's delivery does, on a
//! `Clock` reading and a seeded `Entropy`, and reads which Session serves
//! each request.
#![expect(
    clippy::disallowed_methods,
    reason = "tests draw from a seeded Entropy (ADR 0059)"
)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::entropy::Entropy;
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::session_pool::{ProxyTier, ServingSession};
use goodfetch_protocol::page::SessionId;
use rstest::rstest;
use rusqlite::Connection;

use super::{Effect, SessionPool};
use crate::settings::PoolSettings;
use crate::store::TABLES;

/// One Session at a time, so the next request goes to the same Session
/// while it serves, and to a new one once it is retired.
const ONE: PoolSettings = PoolSettings {
    max_size: 1,
    max_uses: 50,
    max_age: Duration::from_mins(50),
    max_error_score: 3,
};

/// 2026-01-05T09:00:00Z.
fn start() -> SystemTime {
    UNIX_EPOCH + Duration::from_hours(491_001)
}

/// An empty run file of a crawl, holding the pool's tables.
fn run_file() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    connection
}

fn pool(seed: u64) -> SessionPool {
    SessionPool::open(&run_file(), Entropy::seeded(seed)).unwrap()
}

fn serving(id: &str, proxy_tier: ProxyTier, revalidate: bool) -> ServingSession {
    ServingSession {
        id: SessionId(id.to_owned()),
        proxy_tier,
        revalidate,
    }
}

fn ids(served: &[ServingSession]) -> Vec<&str> {
    served.iter().map(|session| session.id.0.as_str()).collect()
}

/// Covers: story:6-250
#[rstest]
#[case::three_failed_page_checks(
    vec![Effect::PageCheckFailed, Effect::PageCheckFailed, Effect::PageCheckFailed],
    "session-2"
)]
#[case::two_failed_page_checks(vec![Effect::PageCheckFailed, Effect::PageCheckFailed], "session-1")]
#[case::a_success_takes_a_half_off(
    vec![Effect::PageCheckFailed, Effect::PageCheckFailed, Effect::Success, Effect::PageCheckFailed],
    "session-1"
)]
#[case::a_half_left_after_the_success(
    vec![
        Effect::PageCheckFailed,
        Effect::PageCheckFailed,
        Effect::Success,
        Effect::PageCheckFailed,
        Effect::PageCheckFailed,
    ],
    "session-2"
)]
#[case::successes_never_go_below_zero(
    vec![
        Effect::Success,
        Effect::Success,
        Effect::PageCheckFailed,
        Effect::PageCheckFailed,
        Effect::PageCheckFailed,
    ],
    "session-2"
)]
fn a_session_is_retired_once_its_error_score_reaches_the_maximum(
    #[case] effects: Vec<Effect>,
    #[case] next: &str,
) {
    let mut pool = pool(1);
    for effect in effects {
        let session = pool.draw("shop.test", &ONE, start());
        pool.settle(&session, "shop.test", effect, &ONE);
    }

    let session = pool.draw("shop.test", &ONE, start());

    assert_eq!(session.id.0, next);
}

/// Covers: story:6-250
#[test]
fn a_session_is_retired_after_its_uses() {
    let settings = PoolSettings { max_uses: 3, ..ONE };
    let mut pool = pool(1);

    let served: Vec<ServingSession> = (0..4)
        .map(|_| pool.draw("shop.test", &settings, start()))
        .collect();

    assert_eq!(
        ids(&served),
        ["session-1", "session-1", "session-1", "session-2"]
    );
}

/// Covers: story:6-250
#[rstest]
#[case::just_before(Duration::from_millis(2_999_999), "session-1")]
#[case::at_its_age(Duration::from_mins(50), "session-2")]
fn a_session_is_retired_at_its_age(#[case] later: Duration, #[case] next: &str) {
    let mut pool = pool(1);
    pool.draw("shop.test", &ONE, start());

    let session = pool.draw("shop.test", &ONE, start() + later);

    assert_eq!(session.id.0, next);
}

/// Covers: story:6-250
#[test]
fn requests_rotate_across_the_sessions_open_once_the_pool_is_full() {
    let settings = PoolSettings {
        max_size: 3,
        max_uses: 1_000,
        ..ONE
    };
    let mut pool = pool(5);

    let served: Vec<ServingSession> = (0..300)
        .map(|_| pool.draw("shop.test", &settings, start()))
        .collect();

    let count = |id: &str| {
        ids(&served)
            .into_iter()
            .filter(|served| *served == id)
            .count()
    };
    assert_eq!(
        (
            ids(&served[..3]),
            count("session-1") >= 60,
            count("session-2") >= 60,
            count("session-3") >= 60,
            count("session-4"),
        ),
        (
            vec!["session-1", "session-2", "session-3"],
            true,
            true,
            true,
            0
        )
    );
}

/// Covers: story:6-250
#[rstest]
#[case::the_same_seed(7, true)]
#[case::another_seed(8, false)]
fn which_session_serves_a_request_is_drawn_from_the_entropy_seed(
    #[case] seed: u64,
    #[case] same: bool,
) {
    let settings = PoolSettings {
        max_size: 4,
        max_uses: 1_000,
        ..ONE
    };
    let draws = |seed: u64| {
        let mut pool = pool(seed);
        (0..40)
            .map(|_| pool.draw("shop.test", &settings, start()).id.0)
            .collect::<Vec<String>>()
    };

    let (first, second) = (draws(7), draws(seed));

    assert_eq!(first == second, same);
}

/// Covers: story:6-251
#[rstest]
#[case::ip_block(BlockCause::IpBlock, ProxyTier::Datacenter)]
#[case::rate_limit(BlockCause::RateLimit, ProxyTier::Datacenter)]
#[case::challenge(BlockCause::Challenge, ProxyTier::None)]
#[case::captcha(BlockCause::Captcha, ProxyTier::None)]
fn a_block_that_escalates_retires_its_session_and_the_replacement_opens_at_its_tier(
    #[case] cause: BlockCause,
    #[case] tier: ProxyTier,
) {
    let mut pool = pool(1);
    let blocked = pool.draw("shop.test", &ONE, start());
    pool.settle(&blocked, "shop.test", Effect::Blocked(cause), &ONE);

    let replacement = pool.draw("shop.test", &ONE, start());
    let elsewhere = pool.draw("books.test", &ONE, start());

    assert_eq!(
        (replacement, elsewhere.proxy_tier),
        (serving("session-2", tier, false), ProxyTier::None)
    );
}

/// Covers: story:6-251
#[test]
fn each_escalation_moves_its_domain_one_proxy_tier_up_to_the_top() {
    let mut pool = pool(1);
    let mut tiers = Vec::new();
    for _ in 0..5 {
        let session = pool.draw("shop.test", &ONE, start());
        tiers.push(session.proxy_tier);
        pool.settle(
            &session,
            "shop.test",
            Effect::Blocked(BlockCause::IpBlock),
            &ONE,
        );
    }

    tiers.push(pool.draw("shop.test", &ONE, start()).proxy_tier);

    assert_eq!(
        tiers,
        [
            ProxyTier::None,
            ProxyTier::Datacenter,
            ProxyTier::Isp,
            ProxyTier::Residential,
            ProxyTier::Mobile,
            ProxyTier::Mobile,
        ]
    );
}

/// Covers: story:6-251
#[rstest]
#[case::payment_required(BlockCause::PaymentRequired)]
#[case::consent_required(BlockCause::ConsentRequired)]
#[case::decoy(BlockCause::Decoy)]
fn a_block_that_never_rotates_leaves_its_session_serving(#[case] cause: BlockCause) {
    let settings = PoolSettings {
        max_error_score: 1,
        ..ONE
    };
    let mut pool = pool(1);
    let blocked = pool.draw("shop.test", &settings, start());
    pool.settle(&blocked, "shop.test", Effect::Blocked(cause), &settings);

    let next = pool.draw("shop.test", &settings, start());

    assert_eq!(next, serving("session-1", ProxyTier::None, false));
}

/// Covers: story:6-251
#[test]
fn a_block_on_a_session_already_retired_still_moves_its_domain_up() {
    let settings = PoolSettings { max_uses: 1, ..ONE };
    let mut pool = pool(1);
    let blocked = pool.draw("shop.test", &settings, start());
    pool.draw("books.test", &settings, start());
    pool.settle(
        &blocked,
        "shop.test",
        Effect::Blocked(BlockCause::IpBlock),
        &settings,
    );

    let next = pool.draw("shop.test", &settings, start());

    assert_eq!(next, serving("session-3", ProxyTier::Datacenter, false));
}

/// Covers: story:6-255
#[test]
fn a_pool_read_again_from_run_state_keeps_its_sessions_and_revalidates_each_on_its_first_page() {
    let settings = PoolSettings { max_size: 2, ..ONE };
    let file = run_file();
    let mut pool = SessionPool::open(&file, Entropy::seeded(1)).unwrap();
    let blocked = pool.draw("shop.test", &settings, start());
    pool.settle(
        &blocked,
        "shop.test",
        Effect::Blocked(BlockCause::IpBlock),
        &settings,
    );
    pool.draw("shop.test", &settings, start());
    pool.draw("books.test", &settings, start());
    pool.changes().write(&file).unwrap();

    let mut resumed = SessionPool::open(&file, Entropy::seeded(2)).unwrap();
    let served = [
        resumed.draw("shop.test", &settings, start()),
        resumed.draw("books.test", &settings, start()),
        resumed.draw("books.test", &settings, start()),
    ];

    assert_eq!(
        served,
        [
            serving("session-2", ProxyTier::Datacenter, true),
            serving("session-3", ProxyTier::None, true),
            serving("session-3", ProxyTier::None, false),
        ]
    );
}
