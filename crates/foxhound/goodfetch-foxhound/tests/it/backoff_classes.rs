//! Politeness conformance cases, the backoff classes (spec #579; ADRs
//! 0021, 0053, 0086 and 0118), on Foxhound's executable against the
//! Proving ground, read from the per-user Politeness file every sharing run
//! obeys: a site's 429, 502, 503 and 504, a refused or timed-out connection
//! and an HTTP/2 `ENHANCE_YOUR_CALM` back its domain off on the `http` and
//! `browser` tiers, and a 500 does not; the site's `Retry-After` holds the
//! domain until the time it asks, and a request admitted meanwhile is never
//! sent; and through a forward proxy, the site's 429 backs its domain off
//! and lowers no proxy ceiling.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a test binds the refused case's address, outside the engine, only to see that no other program can take it (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the times the Politeness file holds a domain until against the wall clock, and bounds its own wait for them"
)]

use std::collections::BTreeSet;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_test_client::Attached;
use goodfetch_test_support::ObservedRequest;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround, ReceivedRequests, TestEnv, browser_cache,
};
use rstest::rstest;
use rusqlite::{Connection, OpenFlags, OptionalExtension as _};
use serde_json::{Value, json};

use crate::attached_with;
use crate::fetch_tiers::settings;

/// The Proving ground's seed.
const SEED: u64 = 766;

/// The first wait of a domain's backoff, long past any crawl here, so the
/// domain is still held when its Run has finished.
const BACKOFF_INITIAL_MS: u64 = 120_000;

/// The tier a crawl fetches its documents on.
#[derive(Clone, Copy, Debug)]
enum Tier {
    Http,
    Browser,
}

impl Tier {
    fn name(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Browser => "browser",
        }
    }
}

/// What a crawl's single page meets.
#[derive(Clone, Copy, Debug)]
enum Page {
    /// `scope.test`'s quiet page answering this status.
    Status(u16),
    /// A host of `scope.test` at the Proving ground's `down` address, where
    /// nothing listens.
    Refused,
    /// The Proving ground's port on the IPv6 loopback address, whose full
    /// listener's queue takes no connection.
    TimedOut,
    /// `fetch-edge.test/calm` over HTTP/2, reset with `ENHANCE_YOUR_CALM`.
    Calm,
}

/// A home whose configuration trusts the Proving ground's authority for
/// `fetch-edge.test`, which only configuration may.
fn home(proving_ground: &ProvingGround, env: &TestEnv) {
    env.user_configuration(&format!(
        "tls.trusted_cas.\"fetch-edge.test\" = ['{}']\n",
        proving_ground.description().ca.display()
    ));
}

/// An Attached engine under `env`, its pages launched from Chrome for
/// Testing.
fn engine(env: &TestEnv) -> Attached {
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    attached_with(env, Some(Duration::from_secs(90)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    })
}

/// The settings of a crawl on `tier` with `more` merged in: robots.txt
/// unread, no delay between a domain's starts, no retry, a backoff whose
/// first wait outlasts the crawl, and the hosts named `extra` resolved to
/// their addresses beside the Proving ground's.
fn crawl_settings(
    proving_ground: &ProvingGround,
    tier: Tier,
    extra: &[(&str, &str)],
    more: &Value,
) -> Value {
    let mut settings = settings(
        proving_ground,
        &json!({
            "fetch": { "tier": tier.name() },
            "crawl": { "retry": { "max": 0 } },
            "politeness": {
                "domain": { "concurrency": 1, "delay": 0 },
                "backoff": { "initial": BACKOFF_INITIAL_MS },
            },
            "site_preferences": { "respect_robots_txt": false },
            "timeout": { "first_byte": 3_000, "navigation": 4_000 },
        }),
    );
    for (host, address) in extra {
        settings["network"]["resolve"][*host] = json!([address]);
        settings["egress"]["private_ranges"]
            .as_array_mut()
            .expect("the private ranges are a list")
            .push(json!(host));
    }
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        for (key, value) in more {
            match (settings.get_mut(key), value) {
                (Some(Value::Object(into)), Value::Object(value)) => into.extend(value.clone()),
                _ => {
                    settings.insert(key.clone(), value.clone());
                }
            }
        }
    }
    settings
}

/// The address the refused case's page is met at: the Proving ground's
/// `down` address, where nothing listens and which no other program can
/// take while the Proving ground runs. A port bound and let go could be
/// taken, and answered, by any program before the engine connects.
fn refused_address(proving_ground: &ProvingGround) -> SocketAddr {
    proving_ground.description().fixtures.proxies["down"]
        .parse()
        .expect("the down address is a socket address")
}

/// The URL `page` is met at, its domain key, and the hosts it needs
/// resolved beside the Proving ground's.
fn url_of(
    proving_ground: &ProvingGround,
    page: Page,
) -> (String, &'static str, Vec<(&'static str, String)>) {
    let port = proving_ground.address().port();
    match page {
        Page::Status(status) => (
            proving_ground.url("scope.test", &format!("/quiet?mark=class&status={status}")),
            "scope.test",
            Vec::new(),
        ),
        Page::Refused => {
            let refused = refused_address(proving_ground);
            (
                format!(
                    "http://refused.scope.test:{}/quiet?mark=refused",
                    refused.port()
                ),
                "scope.test",
                vec![("refused.scope.test", refused.ip().to_string())],
            )
        }
        Page::TimedOut => (
            format!("http://stalled.scope.test:{port}/quiet?mark=stalled"),
            "scope.test",
            vec![("stalled.scope.test", "::1".to_owned())],
        ),
        Page::Calm => (
            format!("https://fetch-edge.test:{port}/calm"),
            "fetch-edge.test",
            Vec::new(),
        ),
    }
}

/// The time the Politeness file holds the domain key `key` until, if any:
/// none while the engine has made the file but not yet its tables.
fn ready_at(file: &Path, key: &str) -> Option<SystemTime> {
    let connection = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let tables: bool = connection
        .query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'domain_limits'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    if !tables {
        return None;
    }
    let ready_at: Option<Option<i64>> = connection
        .query_row(
            "SELECT ready_at FROM domain_limits WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    ready_at
        .flatten()
        .map(|at| UNIX_EPOCH + Duration::from_millis(u64::try_from(at).unwrap()))
}

/// Whether the Politeness file holds the domain key `key` for at least
/// `at_least` from now.
fn held_for(file: &Path, key: &str, at_least: Duration) -> bool {
    ready_at(file, key).is_some_and(|until| until > SystemTime::now() + at_least)
}

/// The ceiling the Politeness file learned for each proxy key, by key.
fn ceilings(file: &Path) -> BTreeSet<(String, Option<i64>)> {
    let connection = Connection::open(file).unwrap();
    let mut statement = connection
        .prepare("SELECT key, ceiling FROM proxy_limits ORDER BY key")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Crawls as `crawl.start`'s `params` say until the Run finished, the
/// handler handling every page and committing nothing: the answer of
/// `run.finish`.
fn crawled(engine: &mut Attached, params: &Value) -> Value {
    let run_id = params["run_id"].clone();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start", "params": params,
    }));
    loop {
        let message = engine
            .next()
            .expect("the engine closed before the Run ended");
        if message["method"] == "crawl_request.handle" {
            engine.send(&json!({
                "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] },
            }));
        } else if message["id"] == "start" {
            assert_eq!(message["result"]["run_id"], run_id, "{message}");
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": run_id },
            }));
        } else if message["id"] == "finish" {
            return message;
        }
    }
}

/// The refused case's address stays refused for as long as its Proving
/// ground runs: no other program on the machine can listen there, as one
/// could on a port bound and let go before the engine connects.
#[cfg(unix)]
#[test]
fn no_other_program_can_take_the_refused_cases_address() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let refused = refused_address(&proving_ground);

    let other = TcpListener::bind(refused);

    assert_eq!(
        other.map(|_| ()).map_err(|error| error.kind()),
        Err(std::io::ErrorKind::AddrInUse)
    );
}

/// A crawl of one page on each tier: the site's 429, 502, 503 and 504, a
/// refused or timed-out connection and an HTTP/2 `ENHANCE_YOUR_CALM` hold
/// its domain key in the Politeness file for the backoff's first wait,
/// past the crawl's end, whatever the handler made of the page; a 500
/// holds it not at all.
///
/// Covers: story:5-132, setting:politeness.backoff.initial, format:politeness-file
#[cfg(unix)]
#[rstest]
#[case::a_429(Page::Status(429), true)]
#[case::a_502(Page::Status(502), true)]
#[case::a_503(Page::Status(503), true)]
#[case::a_504(Page::Status(504), true)]
#[case::a_refused_connection(Page::Refused, true)]
#[case::a_timed_out_connection(Page::TimedOut, true)]
/// Covers: setting:tls.trusted_cas
#[case::enhance_your_calm(Page::Calm, true)]
#[case::a_500(Page::Status(500), false)]
fn a_failing_site_backs_its_domain_off_and_a_500_does_not(
    #[case] page: Page,
    #[case] backs_off: bool,
    #[values(Tier::Http, Tier::Browser)] tier: Tier,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    home(&proving_ground, &env);
    let file = env.home().join("politeness.sqlite");
    let (url, key, extra) = url_of(&proving_ground, page);
    let extra: Vec<(&str, &str)> = extra
        .iter()
        .map(|(host, address)| (*host, address.as_str()))
        .collect();
    // The browser reaches `fetch-edge.test` over TLS only through the
    // relay's termination for the authority `home` trusts (ADR 0042),
    // which passes the site's HTTP/2 frames on. An invalid-certificate
    // exception would not: its host is reached over HTTP/1.1 alone, where
    // no `ENHANCE_YOUR_CALM` is sent.
    let more = json!({});
    let mut engine = engine(&env);

    let finished = crawled(
        &mut engine,
        &json!({
            "run_id": "classes",
            "seeds": [{ "url": url, "label": "page" }],
            "settings": crawl_settings(&proving_ground, tier, &extra, &more),
        }),
    );
    assert!(engine.finished());

    assert_eq!(
        (
            finished["result"]["lifecycle"].clone(),
            held_for(&file, key, Duration::from_millis(BACKOFF_INITIAL_MS / 2)),
        ),
        (json!("finished"), backs_off),
        "{finished} {:?}",
        ready_at(&file, key)
    );
    // `ENHANCE_YOUR_CALM` is an HTTP/2 error: the page was asked for over
    // HTTP/2, on either tier.
    if matches!(page, Page::Calm) {
        let observations = proving_ground.observations().unwrap();
        let protocols: Vec<String> = ReceivedRequests::of(&observations)
            .of_the_case()
            .filter(|request| request.target == "/calm")
            .map(|request| request.protocol.clone())
            .collect();
        assert!(
            !protocols.is_empty() && protocols.iter().all(|protocol| protocol == "h2"),
            "{protocols:?}"
        );
    }
}

/// A crawl on each tier of a page whose 503 asks, in `Retry-After`, for no
/// request in the next hour, then of another page of its domain: the
/// domain is held for that hour, past the crawl's backoff, and the second
/// request, admitted while the site's time is further away than
/// `politeness.retry_after.max`, ends `failed` with `response_unavailable`,
/// never sent (ADR 0118).
///
/// Covers: story:5-132, setting:politeness.retry_after.max, error:response_unavailable
#[cfg(unix)]
#[rstest]
fn the_sites_retry_after_holds_its_domain_and_nothing_is_sent_before_it(
    #[values(Tier::Http, Tier::Browser)] tier: Tier,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let file = env.home().join("politeness.sqlite");
    let mut engine = engine(&env);
    let seed =
        |path: &str| json!({ "url": proving_ground.url("fetch-edge.test", path), "label": "page" });
    let settings = crawl_settings(
        &proving_ground,
        tier,
        &[],
        &json!({ "politeness": { "backoff": { "initial": 1_000 } } }),
    );

    let finished = crawled(
        &mut engine,
        &json!({
            "run_id": "retry-after",
            "seeds": [
                { "url": proving_ground.url("fetch-edge.test", "/retry-after/503/long"), "label": "page", "priority": 1 },
                seed("/target"),
            ],
            "settings": settings,
        }),
    );
    assert!(engine.finished());

    let targets: Vec<ObservedRequest> = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.target == "/target")
        .cloned()
        .collect();
    assert_eq!(
        (
            held_for(&file, "fetch-edge.test", Duration::from_mins(50)),
            targets,
            finished["result"]["requests"]["failed"].clone(),
        ),
        (true, Vec::new(), json!(1)),
        "{finished}"
    );
}

/// Crawls as `crawl.start`'s `params` say, the handler handling every
/// page and committing nothing, until the Politeness file at `file` holds
/// the domain key `key`, then aborts the Run: whether it held it within a
/// minute. (A site's 429 retires its Session and escalates its request a
/// proxy tier up, ADR 0021, which a Run with one `proxy.server` never
/// serves: the request waits, so the Run is aborted, not finished.)
fn crawled_until_held(engine: &mut Attached, params: &Value, file: &Path, key: &str) -> bool {
    let run_id = params["run_id"].clone();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start", "params": params,
    }));
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let held = loop {
        if file.exists() && ready_at(file, key).is_some_and(|until| until > SystemTime::now()) {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        if engine.is_quiet_for(Duration::from_millis(200)) {
            continue;
        }
        let message = engine
            .next()
            .expect("the engine closed before the Run ended");
        if message["method"] == "crawl_request.handle" {
            engine.send(&json!({
                "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] },
            }));
        }
    };
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "abort", "method": "run.abort", "params": { "run_id": run_id },
    }));
    loop {
        let message = engine
            .next()
            .expect("the engine closed before the Run ended");
        if message["id"] == "abort" {
            return held;
        }
    }
}

/// A crawl on the browser tier through the Proving ground's `open`
/// forward proxy of `scope.test`'s page answering the site's own 429: it
/// backs the site's domain off for the backoff's first wait and leaves the
/// proxy's learned ceiling unset (ADR 0053). (The proxy's own 429 backing
/// no domain off is the `backoff_classes` rule's, tested in process; its
/// ceiling learning is #709's page interception, which Collie's proxy
/// limit cases prove.)
///
/// Covers: story:5-133, setting:proxy.server, format:politeness-file
#[cfg(unix)]
#[test]
fn a_sites_429_through_a_proxy_backs_its_domain_off_and_lowers_no_ceiling() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let file = env.home().join("politeness.sqlite");
    let open = format!(
        "http://{}",
        proving_ground.description().fixtures.proxies["open"]
    );
    let mut engine = engine(&env);

    let held = crawled_until_held(
        &mut engine,
        &json!({
            "run_id": "through-open",
            "seeds": [{
                "url": proving_ground.url("scope.test", "/quiet?mark=site&status=429"),
                "label": "page",
            }],
            "settings": crawl_settings(
                &proving_ground,
                Tier::Browser,
                &[],
                &json!({ "proxy": { "server": open, "username": "site" } }),
            ),
        }),
        &file,
        "scope.test",
    );
    assert!(engine.finished());

    assert_eq!(
        (
            held && held_for(
                &file,
                "scope.test",
                Duration::from_millis(BACKOFF_INITIAL_MS / 2)
            ),
            ceilings(&file)
                .into_iter()
                .filter_map(|(key, ceiling)| ceiling.map(|ceiling| (key, ceiling)))
                .collect::<Vec<_>>(),
        ),
        (true, Vec::new()),
    );
}
