//! Session pool cases with proxies (spec #600's story 235; ADRs 0093 and
//! 0117), on Foxhound's executable against the Proving ground's forward
//! proxies and Chrome for Testing: a Run's `proxy.provider` and
//! `proxy.kind` apply to every Session its pools open, the Run's pool's,
//! a Label's own pool's and a retired Session's replacement alike, so one
//! crawl presents one identity policy; and a Label setting a `proxy.*` key
//! fails the crawl's start with `setting_not_applicable` naming the key,
//! the Proving ground receiving nothing.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;
use std::time::Duration;

use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached_with;
use crate::fetch_tiers::settings;
use crate::frontier::crawl_declaring;
use crate::url_fragments::refused_start;
use url::Url;

/// The Proving ground's seed.
const SEED: u64 = 1160;

/// The variable the engine's environment holds the proxies' password in.
const PASSWORD_VARIABLE: &str = "PROVING_GROUND_PROXY_PASSWORD";

/// User configuration declaring the `list` provider of the Proving
/// ground's datacenter, isp and open proxies, the open one declared of the
/// datacenter kind too, with the proxies' credentials.
fn list_configuration(proving_ground: &ProvingGround) -> String {
    let fixtures = &proving_ground.description().fixtures;
    let server = |name: &str, kind: &str| {
        format!(
            "{{ server = \"http://{}\", kind = \"{kind}\" }}",
            fixtures.proxies[name]
        )
    };
    format!(
        "[settings.proxy.providers.list]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\nservers = [{}, {}, {}]\n",
        fixtures.proxy_credentials.username,
        server("datacenter", "datacenter"),
        server("isp", "isp"),
        server("open", "datacenter"),
    )
}

/// The proxies the Proving ground's relays carried requests for targets
/// holding `marker` through.
fn proxies_to(proving_ground: &ProvingGround, marker: &str) -> BTreeSet<String> {
    proving_ground.observations().unwrap()["relays"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|relay| {
            relay["outcome"] == json!("relayed")
                && relay["target"]
                    .as_str()
                    .is_some_and(|target| target.contains(marker))
        })
        .map(|relay| relay["proxy"].as_str().unwrap().to_owned())
        .collect()
}

/// A crawl whose Run sets `proxy.provider = "list"` and `proxy.kind =
/// "isp"`, on the browser tier with one Session in each pool at a time:
/// the Label `account`, which sets `session.source`, draws from a pool of
/// its own, `public` from the Run's, whose Session a 403 retires. Every
/// Session of both pools, the replacement included, leaves from the
/// Run's provider's only `isp` server, and so do the robots.txt reads of
/// the Run's own: nothing reaches the site past the proxy (#1640).
///
/// Covers: story:8-235, setting:proxy.provider, setting:proxy.kind
#[cfg(unix)]
#[test]
fn every_session_of_a_runs_pools_leaves_from_a_proxy_of_the_runs_kind_and_provider() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&list_configuration(&proving_ground));
    let password = proving_ground
        .description()
        .fixtures
        .proxy_credentials
        .password
        .clone();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
        command.env(PASSWORD_VARIABLE, &password);
    });
    let storage = |mark: &str| proving_ground.url("scope.test", &format!("/storage?mark={mark}"));
    let seed = |url: String, label: &str| json!({ "url": url, "label": label });

    let mut sessions = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "pool-proxies",
        &[
            seed(storage("account-1"), "account"),
            seed(storage("public-1"), "public"),
            seed(
                proving_ground.url("access.test", "/corpus/ip-block"),
                "public",
            ),
            seed(storage("public-2"), "public"),
        ],
        &json!({
            "account": { "settings": { "session": { "source": null } } },
            "public": {},
        }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "browser" },
                "session": { "pool": { "max_size": 1 } },
                "proxy": { "provider": "list", "kind": "isp" },
            }),
        ),
        &mut |_, handle| {
            sessions.push(handle["session"]["id"].as_str().unwrap().to_owned());
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    // The handled pages ran in the Label's own pool's Session and in the
    // replacement of the Run's pool Session the 403 retired (and in the
    // retired one too when the crawl admitted `public-1` before the 403).
    let mut sessions: BTreeSet<String> = sessions.into_iter().collect();
    sessions.remove("session-1");
    let isp = BTreeSet::from(["isp".to_owned()]);
    let relayed = relayed(&proving_ground);
    assert_eq!(
        (
            direct(&proving_ground, &relayed),
            proxies_to(&proving_ground, "/robots.txt"),
            &sessions,
            proxies_to(&proving_ground, "mark=account-"),
            proxies_to(&proving_ground, "mark=public-1"),
            proxies_to(&proving_ground, "/corpus/ip-block"),
            proxies_to(&proving_ground, "mark=public-2"),
            &finished["result"]["lifecycle"],
        ),
        (
            Vec::new(),
            isp.clone(),
            &BTreeSet::from(["account.session-1".to_owned(), "session-2".to_owned()]),
            isp.clone(),
            isp.clone(),
            isp.clone(),
            isp,
            &json!("finished"),
        )
    );
}

/// A crawl at the default tier whose Run sets `proxy.provider = "list"`
/// and `proxy.kind = "isp"` reads its site files, robots.txt and the
/// AI-use signal file, from a pool Session's own transport through the
/// Run's provider's `isp` server, as its browser's document goes: nothing
/// reaches the site past the proxy (#2422).
///
/// Covers: story:8-235, setting:proxy.provider, setting:proxy.kind,
/// setting:site_preferences.respect_use_signals
#[cfg(unix)]
#[test]
fn a_crawl_at_the_default_tier_reads_its_site_files_through_the_runs_proxy() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&list_configuration(&proving_ground));
    let password = proving_ground
        .description()
        .fixtures
        .proxy_credentials
        .password
        .clone();
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
        command.env(PASSWORD_VARIABLE, &password);
    });

    let finished = crawl_declaring(
        &mut engine,
        "site-files-through-the-proxy",
        &[page_seed(&proving_ground)],
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({
                "site_preferences": { "respect_use_signals": true, "purpose": "ai-train" },
                "proxy": { "provider": "list", "kind": "isp" },
            }),
        ),
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert!(engine.finished());

    let isp = BTreeSet::from(["isp".to_owned()]);
    let relayed = relayed(&proving_ground);
    assert_eq!(
        (
            direct(&proving_ground, &relayed),
            proxies_to(&proving_ground, "/robots.txt"),
            proxies_to(&proving_ground, "/.well-known/tdmrep.json"),
            proxies_to(&proving_ground, "/version"),
            &finished["result"]["lifecycle"],
        ),
        (
            Vec::new(),
            isp.clone(),
            isp.clone(),
            isp,
            &json!("finished")
        )
    );
}

/// A Label that sets a `proxy.*` key fails the crawl's start with
/// `setting_not_applicable` naming the key, before the Run begins: a
/// Run's proxy settings are its pools' identity policy.
///
/// Covers: story:8-235, error:setting_not_applicable
#[rstest]
/// Covers: setting:proxy.kind
#[case::its_kind(json!({ "proxy": { "kind": "residential" } }), "proxy.kind")]
/// Covers: setting:proxy.provider
#[case::its_provider(json!({ "proxy": { "provider": "list" } }), "proxy.provider")]
/// Covers: setting:proxy.country
#[case::its_country(json!({ "proxy": { "country": "US" } }), "proxy.country")]
/// Covers: setting:proxy.server
#[case::its_own_server(json!({ "proxy": { "server": "http://127.0.0.1:3128" } }), "proxy.server")]
fn a_label_setting_a_proxy_key_fails_the_start(#[case] label: Value, #[case] key: &str) {
    let refusal = refused_start(&json!({}), &label);

    assert_eq!(
        refusal,
        (json!(["setting_not_applicable", key]), Vec::<String>::new())
    );
}

/// The requests the Proving ground's proxies relayed in plain HTTP, each
/// its host and its target, the path and query, as the site received it.
fn relayed(proving_ground: &ProvingGround) -> Vec<(String, String)> {
    let mut relayed: Vec<(String, String)> = proving_ground.observations().unwrap()["relays"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|relay| relay["outcome"] == json!("relayed") && relay["method"] != json!("CONNECT"))
        .map(|relay| {
            let url = Url::parse(relay["target"].as_str().unwrap()).unwrap();
            let target = match url.query() {
                Some(query) => format!("{}?{query}", url.path()),
                None => url.path().to_owned(),
            };
            (url.host_str().unwrap().to_owned(), target)
        })
        .collect();
    relayed.sort();
    relayed
}

/// The requests the site received past every proxy: each one it received,
/// its host and its target, less those a proxy relayed.
fn direct(proving_ground: &ProvingGround, relayed: &[(String, String)]) -> Vec<(String, String)> {
    let mut relayed = relayed.to_vec();
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter_map(|request| {
            let received = (request.host.clone(), request.target.clone());
            match relayed.iter().position(|one| *one == received) {
                Some(at) => {
                    relayed.swap_remove(at);
                    None
                }
                None => Some(received),
            }
        })
        .collect()
}

/// The Run settings `run` with `proxy.server` the Proving ground's open
/// forward proxy.
fn through_the_open_proxy(proving_ground: &ProvingGround, run: &Value) -> Value {
    let mut run = run.clone();
    run["proxy"] = json!({
        "server": format!("http://{}", proving_ground.description().fixtures.proxies["open"]),
    });
    settings(proving_ground, &run)
}

/// A crawl under `run` settings through the open proxy, from `seed`, with
/// the Label `page` setting `label`: its lifecycle, the requests the site
/// received past the proxy, and the targets the proxy relayed.
fn crawl_beside_a_proxy(
    run: &Value,
    seed: fn(&ProvingGround) -> Value,
    label: &Value,
) -> (Value, Vec<(String, String)>, BTreeSet<String>) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let finished = crawl_declaring(
        &mut engine,
        "through-the-proxy",
        &[seed(&proving_ground)],
        &json!({ "page": { "settings": label } }),
        &through_the_open_proxy(&proving_ground, run),
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert!(engine.finished());

    let relayed = relayed(&proving_ground);
    let direct = direct(&proving_ground, &relayed);
    (
        finished["result"]["lifecycle"].clone(),
        direct,
        relayed.into_iter().map(|(_, target)| target).collect(),
    )
}

/// A crawl request of the Label `page`.
fn page_seed(proving_ground: &ProvingGround) -> Value {
    json!({ "url": proving_ground.url("access.test", "/version"), "label": "page" })
}

/// A sitemap seed whose entries take the Label `page`.
fn sitemap_seed(proving_ground: &ProvingGround) -> Value {
    json!({
        "sitemap": proving_ground.url("sitemap-seed.test", "/sitemap-index.xml"),
        "rules": [{ "pattern": "**", "label": "page" }],
    })
}

/// A crawl Run that sets a proxy reads a site only through it (#1640): a
/// document a forced `fetch.tier: http` fetches, the Run's or a Label's, a
/// sitemap seed's sitemap, a robots.txt and an AI-use signal file are
/// fetched through the Run's proxy as a Session's pages are, so the site
/// never sees the address the proxy hides.
///
/// Covers: story:8-235, setting:proxy.server, setting:fetch.tier
#[rstest]
#[case::the_run_forces_the_http_tier(
    json!({ "fetch": { "tier": "http" } }),
    page_seed,
    json!({}),
    &["/robots.txt", "/version"]
)]
#[case::a_label_forces_the_http_tier(
    json!({}),
    page_seed,
    json!({ "fetch": { "tier": "http" } }),
    &["/robots.txt", "/version"]
)]
#[case::a_sitemap_seed(
    json!({ "fetch": { "tier": "http" } }),
    sitemap_seed,
    json!({}),
    &["/robots.txt", "/sitemap-index.xml"]
)]
/// Covers: setting:site_preferences.respect_use_signals
#[case::the_use_signals_are_honoured(
    json!({
        "fetch": { "tier": "http" },
        "site_preferences": { "respect_use_signals": true, "purpose": "ai-train" },
    }),
    page_seed,
    json!({}),
    &["/.well-known/tdmrep.json", "/robots.txt", "/version"]
)]
fn a_run_that_sets_a_proxy_reads_its_sites_only_through_it(
    #[case] run: Value,
    #[case] seed: fn(&ProvingGround) -> Value,
    #[case] label: Value,
    #[case] read: &[&str],
) {
    let (lifecycle, direct, relayed) = crawl_beside_a_proxy(&run, seed, &label);

    let read: BTreeSet<String> = read.iter().map(|&target| target.to_owned()).collect();
    assert_eq!(
        (lifecycle, direct, relayed.is_superset(&read)),
        (json!("finished"), Vec::new(), true),
        "relayed: {relayed:?}"
    );
}

/// The proxy's own answer on the `http` tier, or its own failure, ends the
/// request with the proxy's error, as on the browser tier, never as the
/// site's page that a handler reads (#1640): a proxy that asks for
/// credentials the Run does not give answers `407`, `proxy_auth_failed`;
/// one that is down, HTTP or SOCKS5, which the transport reaches through
/// its own relay, `proxy_unavailable`.
///
/// Covers: setting:proxy.server
#[rstest]
/// Covers: error:proxy_auth_failed
#[case::its_407("http", "datacenter", "proxy_auth_failed")]
/// Covers: error:proxy_unavailable
#[case::an_http_proxy_that_is_down("http", "down", "proxy_unavailable")]
/// Covers: error:proxy_unavailable
#[case::a_socks5_proxy_that_is_down("socks5", "down", "proxy_unavailable")]
fn a_hop_the_proxy_answered_or_failed_on_the_http_tier_ends_with_its_error(
    #[case] scheme: &str,
    #[case] proxy: &str,
    #[case] code: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let mut run = settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "crawl": { "retry": { "max": 0 } },
            "site_preferences": { "respect_robots_txt": false },
        }),
    );
    let address = &proving_ground.description().fixtures.proxies[proxy];
    run["proxy"] = json!({ "server": format!("{scheme}://{address}") });
    let seed = page_seed(&proving_ground);
    let mut handled = Vec::new();

    crawl_declaring(
        &mut engine,
        "proxy-failure",
        std::slice::from_ref(&seed),
        &json!({ "page": {} }),
        &run,
        &mut |_, handle| {
            handled.push(handle.clone());
            json!({ "result": { "records": [] } })
        },
    );
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "proxy-failure" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let ended: Vec<Value> = outcomes["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {outcomes}"))
        .iter()
        .map(|outcome| {
            json!([
                outcome["request"]["url"],
                outcome["kind"],
                outcome["error"]["code"]
            ])
        })
        .collect();
    assert_eq!(
        (ended, handled),
        (vec![json!([seed["url"], "failed", code])], Vec::new()),
        "{outcomes}"
    );
}

/// A sitemap seed added to a live crawl Run that sets a proxy has its
/// sitemap fetched through the proxy, as the start's are (#1640).
///
/// Covers: setting:proxy.server, operation:run.add_seed
#[test]
fn a_sitemap_seed_added_to_a_live_run_that_sets_a_proxy_is_fetched_through_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);
    let run = json!({ "crawl": { "keep_alive": true }, "fetch": { "tier": "http" } });
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "live",
            "seeds": [page_seed(&proving_ground)],
            "labels": { "page": {} },
            "settings": through_the_open_proxy(&proving_ground, &run),
        }),
        &mut |_| Some(json!({ "result": { "records": [] } })),
    );
    assert_eq!(started["result"]["run_id"], "live", "{started}");

    let added = engine.call(
        "add",
        "run.add_seed",
        &json!({ "run_id": "live", "seed": sitemap_seed(&proving_ground) }),
        &mut |_| Some(json!({ "result": { "records": [] } })),
    );
    assert!(added.get("result").is_some(), "{added}");
    // `run.finish` ends the Run once its work, the added seed's included,
    // is done.
    let finished = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "live" }),
        &mut |_| Some(json!({ "result": { "records": [] } })),
    );
    assert!(finished.get("result").is_some(), "{finished}");
    assert!(engine.finished());

    let relayed = relayed(&proving_ground);
    let direct = direct(&proving_ground, &relayed);
    assert_eq!(
        (
            direct,
            relayed
                .iter()
                .any(|(_, target)| target == "/sitemap-index.xml")
        ),
        (Vec::new(), true),
        "relayed: {relayed:?}"
    );
}
