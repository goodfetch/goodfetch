//! Session pool cases (spec #582's pure rules, in process): each case drives
//! the pool through its interface, as a crawl's delivery does, on a
//! `Clock` reading and a seeded `Entropy`, and reads which Session serves
//! each request.
#![expect(
    clippy::disallowed_methods,
    reason = "tests draw from a seeded Entropy (ADR 0059)"
)]

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::local_addresses::{BlockSizes, Host, Ipv6Prefix, LocalAddresses, Rotation};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::session_pool::{ProxyTier, ServingSession};
use goodfetch_protocol::page::SessionId;
use rstest::rstest;
use rusqlite::Connection;

use super::{Effect, LocalAddressing, Rotated, SessionPool};
use crate::labels::PoolKey;
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
        let session = pool.draw(&PoolKey::Run, "shop.test", &ONE, start());
        pool.settle(&session, "shop.test", effect, &ONE);
    }

    let session = pool.draw(&PoolKey::Run, "shop.test", &ONE, start());

    assert_eq!(session.id.0, next);
}

/// Covers: story:6-250
#[test]
fn a_session_is_retired_after_its_uses() {
    let settings = PoolSettings { max_uses: 3, ..ONE };
    let mut pool = pool(1);

    let served: Vec<ServingSession> = (0..4)
        .map(|_| pool.draw(&PoolKey::Run, "shop.test", &settings, start()))
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
    pool.draw(&PoolKey::Run, "shop.test", &ONE, start());

    let session = pool.draw(&PoolKey::Run, "shop.test", &ONE, start() + later);

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
        .map(|_| pool.draw(&PoolKey::Run, "shop.test", &settings, start()))
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
            .map(|_| {
                pool.draw(&PoolKey::Run, "shop.test", &settings, start())
                    .id
                    .0
            })
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
    let blocked = pool.draw(&PoolKey::Run, "shop.test", &ONE, start());
    pool.settle(&blocked, "shop.test", Effect::Blocked(cause), &ONE);

    let replacement = pool.draw(&PoolKey::Run, "shop.test", &ONE, start());
    let elsewhere = pool.draw(&PoolKey::Run, "books.test", &ONE, start());

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
        let session = pool.draw(&PoolKey::Run, "shop.test", &ONE, start());
        tiers.push(session.proxy_tier);
        pool.settle(
            &session,
            "shop.test",
            Effect::Blocked(BlockCause::IpBlock),
            &ONE,
        );
    }

    tiers.push(
        pool.draw(&PoolKey::Run, "shop.test", &ONE, start())
            .proxy_tier,
    );

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
    let blocked = pool.draw(&PoolKey::Run, "shop.test", &settings, start());
    pool.settle(&blocked, "shop.test", Effect::Blocked(cause), &settings);

    let next = pool.draw(&PoolKey::Run, "shop.test", &settings, start());

    assert_eq!(next, serving("session-1", ProxyTier::None, false));
}

/// Covers: story:6-251
#[test]
fn a_block_on_a_session_already_retired_still_moves_its_domain_up() {
    let settings = PoolSettings { max_uses: 1, ..ONE };
    let mut pool = pool(1);
    let blocked = pool.draw(&PoolKey::Run, "shop.test", &settings, start());
    pool.draw(&PoolKey::Run, "books.test", &settings, start());
    pool.settle(
        &blocked,
        "shop.test",
        Effect::Blocked(BlockCause::IpBlock),
        &settings,
    );

    let next = pool.draw(&PoolKey::Run, "shop.test", &settings, start());

    assert_eq!(next, serving("session-3", ProxyTier::Datacenter, false));
}

/// Covers: story:6-255
#[test]
fn a_pool_read_again_from_run_state_keeps_its_sessions_and_revalidates_each_on_its_first_page() {
    let settings = PoolSettings { max_size: 2, ..ONE };
    let file = run_file();
    let mut pool = SessionPool::open(&file, Entropy::seeded(1)).unwrap();
    let blocked = pool.draw(&PoolKey::Run, "shop.test", &settings, start());
    pool.settle(
        &blocked,
        "shop.test",
        Effect::Blocked(BlockCause::IpBlock),
        &settings,
    );
    pool.draw(&PoolKey::Run, "shop.test", &settings, start());
    pool.draw(&PoolKey::Run, "books.test", &settings, start());
    pool.changes().write(&file).unwrap();

    let mut resumed = SessionPool::open(&file, Entropy::seeded(2)).unwrap();
    let served = [
        resumed.draw(&PoolKey::Run, "shop.test", &settings, start()),
        resumed.draw(&PoolKey::Run, "books.test", &settings, start()),
        resumed.draw(&PoolKey::Run, "books.test", &settings, start()),
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

fn label(name: &str) -> PoolKey {
    PoolKey::Label(name.to_owned())
}

/// Covers: story:5-136
#[test]
fn a_labels_own_pool_keeps_its_sessions_apart_from_the_runs() {
    let mut pool = pool(1);

    let served = [
        pool.draw(&PoolKey::Run, "shop.test", &ONE, start()),
        pool.draw(&label("account"), "shop.test", &ONE, start()),
        pool.draw(&PoolKey::Run, "shop.test", &ONE, start()),
        pool.draw(&label("account"), "shop.test", &ONE, start()),
        pool.draw(&label("orders"), "shop.test", &ONE, start()),
    ];

    assert_eq!(
        ids(&served),
        [
            "session-1",
            "account.session-1",
            "session-1",
            "account.session-1",
            "orders.session-1",
        ]
    );
}

/// Covers: story:5-138
#[rstest]
#[case::the_runs_pool(PoolKey::Run, "session-2")]
#[case::a_labels_pool(label("account"), "account.session-2")]
fn a_block_on_a_pools_session_retires_it_and_the_replacement_is_a_new_session(
    #[case] key: PoolKey,
    #[case] replacement: &str,
) {
    let mut pool = pool(1);
    let blocked = pool.draw(&key, "shop.test", &ONE, start());
    pool.settle(
        &blocked,
        "shop.test",
        Effect::Blocked(BlockCause::IpBlock),
        &ONE,
    );

    let next = pool.draw(&key, "shop.test", &ONE, start());

    assert_eq!(next, serving(replacement, ProxyTier::Datacenter, false));
}

/// Covers: story:5-138
#[rstest]
#[case::challenge(BlockCause::Challenge, true)]
#[case::captcha(BlockCause::Captcha, true)]
#[case::ip_block(BlockCause::IpBlock, false)]
#[case::rate_limit(BlockCause::RateLimit, false)]
#[case::decoy(BlockCause::Decoy, false)]
fn a_challenge_moves_its_domain_to_the_browser_tier(
    #[case] cause: BlockCause,
    #[case] browser: bool,
) {
    let mut pool = pool(1);
    let blocked = pool.draw(&label("account"), "shop.test", &ONE, start());
    pool.settle(&blocked, "shop.test", Effect::Blocked(cause), &ONE);

    let moved = (pool.browser("shop.test"), pool.browser("books.test"));

    assert_eq!(moved, (browser, false));
}

/// Covers: story:5-138
#[test]
fn a_retired_session_closes_once_no_attempt_holds_it() {
    let mut pool = pool(1);
    let blocked = pool.draw(&PoolKey::Run, "shop.test", &ONE, start());
    let also = pool.draw(&PoolKey::Run, "shop.test", &ONE, start());
    pool.settle(
        &blocked,
        "shop.test",
        Effect::Blocked(BlockCause::Challenge),
        &ONE,
    );
    let while_held = pool.closable();

    pool.settle(&also, "shop.test", Effect::Success, &ONE);

    assert_eq!(
        (while_held, pool.closable(), pool.closable()),
        (
            Vec::new(),
            vec![SessionId("session-1".to_owned())],
            Vec::new()
        )
    );
}

/// Covers: story:5-136, story:5-138
#[test]
fn the_pools_read_again_from_run_state_keep_each_labels_sessions_and_the_browser_tier() {
    let file = run_file();
    let mut pool = SessionPool::open(&file, Entropy::seeded(1)).unwrap();
    pool.draw(&PoolKey::Run, "shop.test", &ONE, start());
    let blocked = pool.draw(&label("account"), "shop.test", &ONE, start());
    pool.settle(
        &blocked,
        "shop.test",
        Effect::Blocked(BlockCause::Challenge),
        &ONE,
    );
    pool.draw(&label("account"), "shop.test", &ONE, start());
    pool.changes().write(&file).unwrap();

    let mut resumed = SessionPool::open(&file, Entropy::seeded(2)).unwrap();
    let served = [
        resumed.draw(&PoolKey::Run, "shop.test", &ONE, start()),
        resumed.draw(&label("account"), "shop.test", &ONE, start()),
    ];

    assert_eq!(
        (served, resumed.browser("shop.test")),
        (
            [
                serving("session-1", ProxyTier::None, true),
                serving("account.session-2", ProxyTier::None, true),
            ],
            true
        )
    );
}

/// A host that holds every address, as Linux holds all of `127.0.0.0/8`.
struct EveryAddress;

impl Host for EveryAddress {
    fn holds(&self, _address: IpAddr) -> bool {
        true
    }

    fn interface(&self, _name: &str) -> Option<Vec<IpAddr>> {
        None
    }

    fn addresses(&self) -> Vec<IpAddr> {
        Vec::new()
    }

    fn binds_any_of(&self, _prefix: Ipv6Prefix) -> bool {
        false
    }
}

/// Room for ten Sessions, so each request opens a new one.
const ROOMY: PoolSettings = PoolSettings {
    max_size: 10,
    ..ONE
};

/// Sessions spread over `addresses` in /30 blocks, sending through a proxy
/// when `proxied`.
fn local(addresses: &[[u8; 4]], proxied: bool) -> LocalAddressing {
    LocalAddressing::new(
        Rotation::new(
            LocalAddresses::Addresses(
                addresses
                    .iter()
                    .map(|octets| IpAddr::from(*octets))
                    .collect(),
            ),
            BlockSizes { ipv4: 30, ipv6: 64 },
        ),
        BlockSizes { ipv4: 30, ipv6: 64 },
        Arc::new(EveryAddress),
        proxied,
    )
}

/// The IPv4 address the Session `served` holds.
fn address_of(pool: &SessionPool, served: &ServingSession) -> [u8; 4] {
    pool.local_addresses(&served.id)
        .unwrap()
        .unwrap()
        .ipv4
        .unwrap()
        .octets()
}

/// Draws a Session for `domain` and settles it with `effect`, answering
/// its address.
fn served(pool: &mut SessionPool, domain: &str, effect: Effect) -> [u8; 4] {
    let served = pool.draw(&PoolKey::Run, domain, &ROOMY, start());
    let address = address_of(pool, &served);
    pool.settle(&served, domain, effect, &ROOMY);
    address
}

/// Without a proxy, a Session a block retired is replaced by one outside
/// every /30 blocked on its domain during the run, where round robin
/// would have gone back into one; another domain keeps round robin.
///
/// Covers: story:4-109
#[rstest]
#[case::ip_block(BlockCause::IpBlock)]
#[case::rate_limit(BlockCause::RateLimit)]
#[case::challenge(BlockCause::Challenge)]
fn a_replacement_after_a_block_takes_an_address_outside_every_block_blocked_on_its_domain(
    #[case] cause: BlockCause,
) {
    let mut pool = pool(1).with_local_addresses(local(
        &[[127, 0, 0, 2], [127, 0, 0, 3], [127, 0, 0, 4]],
        false,
    ));

    let addresses = [
        served(&mut pool, "shop.test", Effect::Blocked(cause)),
        served(&mut pool, "shop.test", Effect::Success),
        served(&mut pool, "shop.test", Effect::Success),
        served(&mut pool, "books.test", Effect::Success),
    ];

    assert_eq!(
        (addresses, pool.rotated_within_block()),
        (
            [
                [127, 0, 0, 2],
                [127, 0, 0, 4],
                [127, 0, 0, 4],
                [127, 0, 0, 2]
            ],
            Vec::new()
        )
    );
}

/// With every block blocked on the domain, the replacement rotates within
/// a block, and the pool says which Session did, for the Run report.
///
/// Covers: story:4-109
#[test]
fn with_every_block_blocked_a_replacement_rotates_within_a_block_and_says_so() {
    let mut pool = pool(1).with_local_addresses(local(&[[127, 0, 0, 2], [127, 0, 0, 4]], false));
    let ip_block = Effect::Blocked(BlockCause::IpBlock);

    let addresses = [
        served(&mut pool, "shop.test", ip_block),
        served(&mut pool, "shop.test", ip_block),
        served(&mut pool, "shop.test", Effect::Success),
    ];

    assert_eq!(
        (addresses, pool.rotated_within_block()),
        (
            [[127, 0, 0, 2], [127, 0, 0, 4], [127, 0, 0, 2]],
            vec![Rotated {
                session: SessionId("session-3".to_owned()),
                domain_key: "shop.test".to_owned(),
            }]
        )
    );
}

/// With a proxy, whose exit rotates instead (ADR 0021), the replacement
/// keeps the local address of the Session the block retired.
///
/// Covers: story:4-109
#[test]
fn with_a_proxy_the_replacement_keeps_the_retired_sessions_address() {
    let mut pool = pool(1).with_local_addresses(local(&[[127, 0, 0, 2], [127, 0, 0, 4]], true));

    let addresses = [
        served(&mut pool, "shop.test", Effect::Blocked(BlockCause::IpBlock)),
        served(&mut pool, "shop.test", Effect::Success),
        served(&mut pool, "shop.test", Effect::Success),
    ];

    assert_eq!(
        (addresses, pool.rotated_within_block()),
        ([[127, 0, 0, 2], [127, 0, 0, 2], [127, 0, 0, 4]], Vec::new())
    );
}

/// A pool read again from run state keeps each Session's address and the
/// blocks blocked on each domain.
///
/// Covers: story:4-109
#[test]
fn a_pool_read_again_from_run_state_keeps_its_addresses_and_blocked_blocks() {
    let addresses = [[127, 0, 0, 2], [127, 0, 0, 3], [127, 0, 0, 4]];
    let file = run_file();
    let mut pool = SessionPool::open(&file, Entropy::seeded(1))
        .unwrap()
        .with_local_addresses(local(&addresses, false));
    served(&mut pool, "shop.test", Effect::Blocked(BlockCause::IpBlock));
    let open = pool.draw(&PoolKey::Run, "books.test", &ROOMY, start());
    let before = address_of(&pool, &open);
    pool.changes().write(&file).unwrap();

    let mut resumed = SessionPool::open(&file, Entropy::seeded(2))
        .unwrap()
        .with_local_addresses(local(&addresses, false));
    let kept = address_of(&resumed, &open);
    let replacement = served(&mut resumed, "shop.test", Effect::Success);

    assert_eq!(
        (before, kept, replacement),
        ([127, 0, 0, 4], [127, 0, 0, 4], [127, 0, 0, 4])
    );
}

/// A block met by an attempt whose Session the pool retired while it was
/// in flight, here for its uses, still blocks that Session's block on the
/// domain: the replacement for the domain skips the address round robin
/// would give it there.
///
/// Covers: story:4-109
#[test]
fn a_block_met_after_its_session_was_retired_still_blocks_its_block() {
    let settings = PoolSettings {
        max_uses: 1,
        ..ROOMY
    };
    let mut pool = pool(1).with_local_addresses(local(&[[127, 0, 0, 2], [127, 0, 0, 4]], false));
    let blocked = pool.draw(&PoolKey::Run, "shop.test", &settings, start());
    let other = pool.draw(&PoolKey::Run, "books.test", &settings, start());
    pool.settle(
        &blocked,
        "shop.test",
        Effect::Blocked(BlockCause::IpBlock),
        &settings,
    );
    let other_address = address_of(&pool, &other);
    pool.settle(&other, "books.test", Effect::Success, &settings);

    let replacement = pool.draw(&PoolKey::Run, "shop.test", &settings, start());

    assert_eq!(
        (other_address, address_of(&pool, &replacement)),
        ([127, 0, 0, 4], [127, 0, 0, 4])
    );
}
