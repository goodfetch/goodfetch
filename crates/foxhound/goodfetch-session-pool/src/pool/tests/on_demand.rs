//! On-demand growth (#2178): a new Session opens only when no open
//! Session of the client wanted at the domain's proxy tier has room, or
//! when a block left the domain owing one, and one of the browser client
//! only while a browser-page place can be had.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::foxhound::block::BlockCause;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use rstest::rstest;

use super::{ONE, ROOMY, address_of, local, pool, start};
use crate::labels::PoolKey;
use crate::pool::{Effect, SendsFrom, SessionPool, Wanted};
use crate::settings::{Affinity, Growth, PoolSettings};

/// Room for a hundred Sessions, each serving one request at a time.
const ON_DEMAND: PoolSettings = PoolSettings {
    max_size: 100,
    max_uses: 1_000,
    ..ONE
};

/// The Session drawn for a request to `host`, on the domain `shop.test`,
/// sent from `sends_from`, a place to be had or not as `place` says.
fn draw(
    pool: &mut SessionPool,
    (host, sends_from): (&str, SendsFrom),
    settings: &PoolSettings,
    place: bool,
) -> Option<ServingSession> {
    let wanted = Wanted {
        domain_key: "shop.test",
        host,
        sends_from,
    };
    pool.draw(&PoolKey::Run, &wanted, settings, start(), || place)
}

/// [`draw`] from the browser to `shop.test`, a place always to be had.
fn browser(pool: &mut SessionPool, settings: &PoolSettings) -> ServingSession {
    draw(pool, ("shop.test", SendsFrom::Browser), settings, true).unwrap()
}

fn succeeded(pool: &mut SessionPool, served: &ServingSession, settings: &PoolSettings) {
    pool.settle(served, "shop.test", Effect::Success, settings);
}

/// How many Sessions `served` names.
fn sessions(served: &[ServingSession]) -> usize {
    let mut ids: Vec<&str> = served.iter().map(|served| served.id.0.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.len()
}

/// Requests rise to five in flight and fall again, twice: five Sessions
/// open, as many as the peak, where `eager` opens one per request.
///
/// Covers: setting:session.pool.growth
#[rstest]
#[case::on_demand(Growth::OnDemand, 5)]
#[case::eager(Growth::Eager, 14)]
fn on_demand_the_open_sessions_never_exceed_the_peak_requests_in_flight(
    #[case] growth: Growth,
    #[case] opened: usize,
) {
    let settings = PoolSettings {
        growth,
        ..ON_DEMAND
    };
    let mut pool = pool(3);
    let mut in_flight = std::collections::VecDeque::new();
    let mut served = Vec::new();
    let mut peak = 0;

    for wave in [3, 5, 2, 4] {
        while in_flight.len() < wave {
            let session = browser(&mut pool, &settings);
            served.push(session.clone());
            in_flight.push_back(session);
            peak = peak.max(in_flight.len());
        }
        while let Some(session) = in_flight.pop_front() {
            succeeded(&mut pool, &session, &settings);
        }
    }

    assert_eq!((peak, sessions(&served)), (5, opened));
}

/// Three requests sent from a Session's own transport and one sent from
/// the browser, in flight together, open three Sessions of the first
/// client and one of the second.
///
/// Covers: setting:session.pool.growth
#[test]
fn on_demand_each_client_opens_only_the_sessions_its_requests_in_flight_need() {
    let mut pool = pool(3);

    let served: Vec<(SendsFrom, ServingSession)> = [
        SendsFrom::Transport,
        SendsFrom::Browser,
        SendsFrom::Transport,
        SendsFrom::Transport,
    ]
    .into_iter()
    .map(|from| {
        let drawn = draw(&mut pool, ("shop.test", from), &ON_DEMAND, true).unwrap();
        (from, drawn)
    })
    .collect();

    let of = |client: SendsFrom| {
        let served: Vec<ServingSession> = served
            .iter()
            .filter(|(from, _)| *from == client)
            .map(|(_, session)| session.clone())
            .collect();
        sessions(&served)
    };
    assert_eq!((of(SendsFrom::Transport), of(SendsFrom::Browser)), (3, 1));
}

/// Four requests in flight together open one Session for each
/// `session_concurrency` of them, and one Session when it is unset.
///
/// Covers: setting:session.pool.session_concurrency
#[rstest]
#[case::one(Some(1), 4)]
#[case::two(Some(2), 2)]
#[case::unset(None, 1)]
fn session_concurrency_bounds_the_requests_one_session_serves_at_once(
    #[case] session_concurrency: Option<u64>,
    #[case] opened: usize,
) {
    let settings = PoolSettings {
        session_concurrency,
        ..ON_DEMAND
    };
    let mut pool = pool(3);

    let served: Vec<ServingSession> = (0..4).map(|_| browser(&mut pool, &settings)).collect();

    assert_eq!(sessions(&served), opened);
}

/// Two Sessions with room, one last used on `a.shop.test` and one on
/// `b.shop.test`, each seed's draw for `b.shop.test` as `affinity` says.
fn drawn_for_b_by_seed(affinity: Affinity) -> Vec<String> {
    let settings = PoolSettings {
        affinity,
        ..ON_DEMAND
    };
    (0..16)
        .map(|seed| {
            let mut pool = pool(seed);
            let on_a = draw(
                &mut pool,
                ("a.shop.test", SendsFrom::Browser),
                &settings,
                true,
            );
            let on_b = draw(
                &mut pool,
                ("b.shop.test", SendsFrom::Browser),
                &settings,
                true,
            );
            for served in [on_a, on_b].into_iter().flatten() {
                succeeded(&mut pool, &served, &settings);
            }
            draw(
                &mut pool,
                ("b.shop.test", SendsFrom::Browser),
                &settings,
                true,
            )
            .unwrap()
            .id
            .0
        })
        .collect()
}

/// Under `affinity: host`, a Session with room whose last request went to
/// the request's host is drawn whatever the seed; under `none`, the seed
/// draws among both, the same seed the same.
///
/// Covers: setting:session.pool.affinity
#[test]
fn affinity_host_draws_a_free_session_last_used_on_the_host_first() {
    let host = drawn_for_b_by_seed(Affinity::Host);
    let none = drawn_for_b_by_seed(Affinity::None);

    let on = |drawn: &[String], id: &str| drawn.iter().filter(|drawn| *drawn == id).count();
    assert_eq!(
        (
            on(&host, "session-2"),
            on(&none, "session-1") > 0,
            on(&none, "session-2") > 0,
            drawn_for_b_by_seed(Affinity::None) == none,
        ),
        (16, true, true, true)
    );
}

/// `eager` keeps #920's rule: a request goes to a new Session while the
/// pool has room, though an open one has room, then to one drawn from the
/// seed among those open; on demand the one free Session serves them all.
///
/// Covers: setting:session.pool.growth
#[rstest]
#[case::eager(
    Growth::Eager,
    &[
        "session-1", "session-2", "session-3", "session-2", "session-2",
        "session-1", "session-3", "session-1", "session-3", "session-2",
    ]
)]
#[case::on_demand(Growth::OnDemand, &["session-1"; 10])]
fn eager_reproduces_the_sequence_of_920_for_a_seed(
    #[case] growth: Growth,
    #[case] sequence: &[&str],
) {
    let settings = PoolSettings {
        max_size: 3,
        growth,
        ..ON_DEMAND
    };
    let mut pool = pool(7);

    let served: Vec<String> = (0..10)
        .map(|_| {
            let served = browser(&mut pool, &settings);
            succeeded(&mut pool, &served, &settings);
            served.id.0
        })
        .collect();

    assert_eq!(served, sequence);
}

/// A block that retires a Session leaves its domain owing a new one: the
/// domain's next draw opens one though another Session at its tier has
/// room, and the draw after it reuses as on demand.
///
/// Covers: setting:session.pool.growth
#[rstest]
#[case::challenge(Effect::Blocked(BlockCause::Challenge))]
#[case::captcha(Effect::Blocked(BlockCause::Captcha))]
#[case::refused(Effect::TransportRefused)]
fn after_a_block_that_retires_a_session_the_domains_next_draw_opens_a_new_one(
    #[case] block: Effect,
) {
    let mut pool = pool(3);
    let blocked = browser(&mut pool, &ON_DEMAND);
    let free = browser(&mut pool, &ON_DEMAND);
    succeeded(&mut pool, &free, &ON_DEMAND);
    pool.settle(&blocked, "shop.test", block, &ON_DEMAND);

    let replacement = browser(&mut pool, &ON_DEMAND);
    succeeded(&mut pool, &replacement, &ON_DEMAND);
    let next = browser(&mut pool, &ON_DEMAND);

    assert_eq!(
        (
            [&blocked, &free, &replacement].map(|served| served.id.0.as_str()),
            next.id.0 == "session-2" || next.id.0 == "session-3",
        ),
        (["session-1", "session-2", "session-3"], true)
    );
}

/// A domain owed a new Session in a full pool retires one to make room
/// (`pool_full`), though a Session at its tier has room.
///
/// Covers: setting:session.pool.growth
#[test]
fn escalation_opens_a_new_session_when_the_pool_is_full() {
    let settings = PoolSettings {
        max_size: 2,
        ..ON_DEMAND
    };
    let mut pool = pool(3);
    let blocked = browser(&mut pool, &settings);
    let free = browser(&mut pool, &settings);
    succeeded(&mut pool, &free, &settings);
    pool.settle(
        &blocked,
        "shop.test",
        Effect::Blocked(BlockCause::Challenge),
        &settings,
    );
    // Another domain fills the pool again with a Session of the other
    // client.
    let books = Wanted {
        domain_key: "books.test",
        host: "books.test",
        sends_from: SendsFrom::Transport,
    };
    pool.draw(&PoolKey::Run, &books, &settings, start(), || true)
        .unwrap();

    let replacement = browser(&mut pool, &settings);

    assert_eq!(
        (replacement.id.0.as_str(), pool.closable().len()),
        ("session-4", 2)
    );
}

/// Under `egress.local_addresses` with no proxy, once a block on a domain
/// blocked an address block there, no draw on that domain takes an open
/// Session whose addresses lie in it; another domain still does.
///
/// Covers: story:4-109
#[test]
fn no_draw_on_a_domain_takes_an_open_session_in_a_block_blocked_there() {
    let mut pool = pool(3).with_local_addresses(local(
        &[[127, 0, 0, 2], [127, 0, 0, 3], [127, 0, 0, 4]],
        false,
    ));
    let opened: Vec<ServingSession> = (0..3).map(|_| browser(&mut pool, &ON_DEMAND)).collect();
    let addresses: Vec<[u8; 4]> = opened
        .iter()
        .map(|served| address_of(&pool, served))
        .collect();
    for served in &opened[1..] {
        succeeded(&mut pool, served, &ON_DEMAND);
    }
    pool.settle(
        &opened[0],
        "shop.test",
        Effect::Blocked(BlockCause::Challenge),
        &ON_DEMAND,
    );

    let on_the_domain: Vec<[u8; 4]> = (0..30)
        .map(|_| {
            let served = browser(&mut pool, &ON_DEMAND);
            succeeded(&mut pool, &served, &ON_DEMAND);
            address_of(&pool, &served)
        })
        .collect();
    let elsewhere = {
        let wanted = Wanted {
            domain_key: "books.test",
            host: "books.test",
            sends_from: SendsFrom::Browser,
        };
        let settings = PoolSettings {
            affinity: Affinity::None,
            ..ON_DEMAND
        };
        (0..30)
            .map(|_| {
                let served = pool
                    .draw(&PoolKey::Run, &wanted, &settings, start(), || true)
                    .unwrap();
                pool.settle(&served, "books.test", Effect::Success, &settings);
                address_of(&pool, &served)
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        (
            addresses,
            on_the_domain.contains(&[127, 0, 0, 3]),
            elsewhere.contains(&[127, 0, 0, 3]),
        ),
        (
            vec![[127, 0, 0, 2], [127, 0, 0, 4], [127, 0, 0, 3]],
            false,
            true
        )
    );
}

/// With no browser-page place to be had, a request that needs the browser
/// opens no Session and waits; it is drawn once a Session of the browser
/// client has room, or once a place can be had. A request of the
/// transport never asks for a place.
///
/// Covers: setting:session.pool.growth
#[test]
fn with_no_place_a_browser_request_waits_for_a_session_with_room_or_a_place() {
    let mut pool = pool(3);
    let first = browser(&mut pool, &ON_DEMAND);

    let waits = draw(
        &mut pool,
        ("shop.test", SendsFrom::Browser),
        &ON_DEMAND,
        false,
    );
    let transport = draw(
        &mut pool,
        ("shop.test", SendsFrom::Transport),
        &ON_DEMAND,
        false,
    );
    let with_a_place = draw(
        &mut pool,
        ("shop.test", SendsFrom::Browser),
        &ON_DEMAND,
        true,
    );
    succeeded(&mut pool, &first, &ON_DEMAND);
    let wanted = Wanted {
        domain_key: "shop.test",
        host: "shop.test",
        sends_from: SendsFrom::Browser,
    };
    let with_room = pool.draw(&PoolKey::Run, &wanted, &ON_DEMAND, start(), || {
        panic!("a Session with room asks for no place")
    });

    let id = |served: Option<ServingSession>| served.map(|served| served.id.0);
    assert_eq!(
        [waits, transport, with_a_place, with_room].map(id),
        [
            None,
            Some("session-2".to_owned()),
            Some("session-3".to_owned()),
            Some("session-1".to_owned()),
        ]
    );
}

/// Among the open Sessions with room, one that holds a page is drawn
/// first, before one last used on the request's host.
///
/// Covers: setting:session.pool.growth
#[test]
fn among_the_sessions_with_room_one_that_holds_a_page_is_drawn_first() {
    let drawn: Vec<String> = (0..16)
        .map(|seed| {
            let mut pool = pool(seed);
            let opened: Vec<ServingSession> = ["a.shop.test", "b.shop.test", "c.shop.test"]
                .map(|host| draw(&mut pool, (host, SendsFrom::Browser), &ON_DEMAND, true).unwrap())
                .into();
            for served in &opened {
                succeeded(&mut pool, served, &ON_DEMAND);
            }
            pool.holds_page(&opened[1].id, true);
            draw(
                &mut pool,
                ("a.shop.test", SendsFrom::Browser),
                &ON_DEMAND,
                true,
            )
            .unwrap()
            .id
            .0
        })
        .collect();

    assert_eq!(drawn, vec!["session-2".to_owned(); 16]);
}

/// On demand, a Session that served `max_uses` requests is still retired,
/// and the next request opens its replacement.
///
/// Covers: story:6-250
#[test]
fn on_demand_max_uses_still_rotates() {
    let settings = PoolSettings {
        max_uses: 3,
        ..ROOMY
    };
    let settings = PoolSettings {
        growth: Growth::OnDemand,
        ..settings
    };
    let mut pool = pool(3);

    let served: Vec<String> = (0..7)
        .map(|_| {
            let served = browser(&mut pool, &settings);
            succeeded(&mut pool, &served, &settings);
            served.id.0
        })
        .collect();

    assert_eq!(
        served,
        [
            "session-1",
            "session-1",
            "session-1",
            "session-2",
            "session-2",
            "session-2",
            "session-3"
        ]
    );
}
