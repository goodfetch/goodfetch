//! A proxy's limits over stdio (spec #577, ticket #709), through
//! `goodfetch-protocol`'s types, against the Proving ground's forward
//! proxies and the per-user Politeness file the engine shares them in
//! (ADRs 0053, 0088 and 0098): a site's 429 met on a program's Session
//! through a proxy is the site's, writing nothing to Politeness's domain
//! limits and lowering no proxy ceiling, while the proxy's own 429 does.

use std::collections::BTreeSet;

use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::page::opened;
use crate::run::call;

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
fn ceilings(file: &std::path::Path) -> BTreeSet<(String, Option<i64>)> {
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
    let site = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "program" }),
    ));
    let opened_session = call(
        &mut engine,
        3,
        "run.new_session",
        &json!({ "run_id": "program", "settings": { "proxy": { "server": limited, "username": "own" } } }),
    );
    let mut own = Vec::new();
    for id in [4, 5] {
        own.push(opened(&call(
            &mut engine,
            id,
            "session.new_page",
            &json!({ "session_id": opened_session["result"]["session_id"] }),
        )));
    }

    let sites = call(
        &mut engine,
        6,
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
    let _ = call(
        &mut engine,
        9,
        "run.finish",
        &json!({ "run_id": "program" }),
    );
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
