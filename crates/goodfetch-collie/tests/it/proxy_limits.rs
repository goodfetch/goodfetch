//! A proxy's limits over stdio (spec #577, ticket #709), through
//! `goodfetch-protocol`'s types, against the Proving ground's forward
//! proxies and the per-user Politeness file the engine shares them in
//! (ADRs 0053, 0088 and 0098): a site's 429 met on a program's Session
//! through a proxy is the site's, writing nothing to Politeness's domain
//! limits and lowering no proxy ceiling, while the proxy's own 429 does; a
//! Proxy provider's proxy that is down lowers none either.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;

use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::page::opened;

const SEED: u64 = 709;

/// The Politeness file's domain limits, which Foxhound's crawls write: one
/// row a test plants before the engine starts.
const DOMAIN_LIMITS: &str =
    "CREATE TABLE domain_limits (key TEXT NOT NULL PRIMARY KEY, ready_at INTEGER,
         backoff INTEGER, ceiling INTEGER, ceiling_changed_at INTEGER, last_start INTEGER) STRICT;
     INSERT INTO domain_limits VALUES ('scope.test', NULL, NULL, 4, 1767603600000, 1767603600000);";

/// Every row of `table` in the Politeness file at `file`, each as text.
fn rows(file: &std::path::Path, table: &str) -> Vec<String> {
    let connection = Connection::open(file).unwrap();
    let mut statement = connection
        .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
        .unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|column| {
                    row.get::<_, rusqlite::types::Value>(column)
                        .map(|value| format!("{value:?}"))
                })
                .collect::<rusqlite::Result<Vec<_>>>()
                .map(|values| values.join("|"))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// The ceiling the Politeness file holds for each proxy key, by key.
pub(crate) fn ceilings(file: &std::path::Path) -> BTreeSet<(String, Option<i64>)> {
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

/// Covers: story:5-90, story:5-91, setting:proxy.server, setting:proxy.username
#[cfg(unix)]
#[test]
fn a_sites_429_through_a_proxy_writes_no_domain_limit_and_no_proxy_ceiling_where_the_proxy_itself_does()
 {
    let env = TestEnv::new();
    let file = env.home().join("politeness.sqlite");
    Connection::open(&file)
        .unwrap()
        .execute_batch(DOMAIN_LIMITS)
        .unwrap();
    let planted = rows(&file, "domain_limits");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let proxies = &proving_ground.description().fixtures.proxies;
    let (open, limited) = (
        format!("http://{}", proxies["open"]),
        format!("http://{}", proxies["limited"]),
    );
    let mut settings = Map::new();
    settings.insert(
        "proxy".to_owned(),
        json!({ "server": open, "username": "site" }),
    );
    let mut engine =
        crate::page::started_with(&env, &cache, &proving_ground, "program", &settings, |_| {});
    let site = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "program" })));
    let opened_session = engine.ask(3,
        "run.new_session",
        &json!({ "run_id": "program", "settings": { "proxy": { "server": limited, "username": "own" } } }),
    );
    let mut own = Vec::new();
    for id in [4, 5] {
        own.push(opened(&engine.ask(
            id,
            "session.new_page",
            &json!({ "session_id": opened_session["result"]["session_id"] }),
        )));
    }

    let sites = engine.ask(6,
        "page.goto",
        &json!({ "page_id": site.page_id, "url": proving_ground.url("scope.test", "/quiet?mark=site&status=429") }),
    );
    for (id, page) in [(7, &own[0]), (8, &own[1])] {
        engine.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "page.goto",
            "params": { "page_id": page.page_id, "url": proving_ground.url("scope.test", &format!("/quiet?mark=own-{id}")) },
        }));
    }
    let mut answered: Vec<Value> = Vec::new();
    while answered.len() < 2 {
        let message = engine.next().expect("the engine answers both navigations");
        if message.get("method").is_none() && [json!(7), json!(8)].contains(&message["id"]) {
            answered.push(message);
        }
    }
    let _ = engine.ask(9, "run.finish", &json!({ "run_id": "program" }));
    let _ = engine.finish();

    let mut codes = answered
        .iter()
        .map(|answer| answer["error"]["data"]["code"].clone())
        .collect::<Vec<_>>();
    codes.sort_by_key(ToString::to_string);
    assert_eq!(
        (
            sites["error"]["data"]["code"] == json!("proxy_unavailable"),
            codes,
            rows(&file, "domain_limits") == planted,
            ceilings(&file),
        ),
        (
            false,
            vec![json!("proxy_unavailable"), Value::Null],
            true,
            BTreeSet::from([
                (format!("proxy {limited} own"), Some(1)),
                (format!("proxy {open} site"), None),
            ]),
        )
    );
}

/// A Proxy provider's proxy that is down: the Session's relay cannot reach
/// it, so a plain URL meets the relay's own 502 and a tunnelled one a tunnel
/// the relay could not open. Both raise `proxy_unavailable`, and neither is
/// the proxy refusing a request, so its learned ceiling stays unset.
///
/// Covers: story:5-90, setting:proxy.provider, setting:proxy.providers, error:proxy_unavailable
#[cfg(unix)]
#[test]
fn a_provider_proxy_that_is_down_raises_proxy_unavailable_and_lowers_no_ceiling() {
    let env = TestEnv::new();
    let file = env.home().join("politeness.sqlite");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let down = proving_ground.description().fixtures.proxies["down"].clone();
    env.user_configuration(&format!(
        "[settings.proxy.providers.list]\nservers = [{{ server = \"http://{down}\", kind = \"datacenter\" }}]\n"
    ));
    let mut settings = Map::new();
    settings.insert("proxy".to_owned(), json!({ "provider": "list" }));
    let mut engine =
        crate::page::started_with(&env, &cache, &proving_ground, "down", &settings, |_| {});
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "down" })));

    let plain = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": "http://scope.test/quiet?mark=plain" }),
    );
    let tunnelled = engine.ask(
        4,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": "https://scope.test/quiet?mark=tunnelled" }),
    );
    let _ = engine.ask(5, "run.finish", &json!({ "run_id": "down" }));
    let _ = engine.finish();

    assert_eq!(
        (
            plain["error"]["data"]["code"].clone(),
            tunnelled["error"]["data"]["code"].clone(),
            ceilings(&file),
        ),
        (
            json!("proxy_unavailable"),
            json!("proxy_unavailable"),
            BTreeSet::from([(format!("proxy http://{down} "), None)]),
        )
    );
}

/// A navigation `proxy.bypass` sends around the proxy reaches the site
/// directly, so it takes none of the proxy's Permits: the proxy's key never
/// starts a request.
///
/// Covers: story:5-90, setting:proxy.bypass
#[cfg(unix)]
#[test]
fn a_navigation_proxy_bypass_sends_around_the_proxy_takes_none_of_its_permits() {
    let env = TestEnv::new();
    let file = env.home().join("politeness.sqlite");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let open = format!(
        "http://{}",
        proving_ground.description().fixtures.proxies["open"]
    );
    let mut settings = Map::new();
    settings.insert(
        "proxy".to_owned(),
        json!({ "server": open, "username": "bypassing", "bypass": "scope.test" }),
    );
    let mut engine = crate::page::started_with(
        &env,
        &cache,
        &proving_ground,
        "bypassing",
        &settings,
        |_| {},
    );
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "bypassing" })));

    let navigated = engine.ask(3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("scope.test", "/quiet?mark=bypassed") }),
    );
    let _ = engine.ask(4, "run.finish", &json!({ "run_id": "bypassing" }));
    let _ = engine.finish();

    assert_eq!(
        (
            navigated["result"]["response"]["status"].clone(),
            rows(&file, "proxy_limits")
        ),
        (json!(200), Vec::<String>::new())
    );
}
