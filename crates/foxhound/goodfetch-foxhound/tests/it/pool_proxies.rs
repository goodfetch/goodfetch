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
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, browser_cache};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

#[cfg(unix)]
use crate::attached_with;
use crate::fetch_tiers::settings;
use crate::frontier::crawl_declaring;
use crate::url_fragments::refused_start;
#[cfg(unix)]
use goodfetch_test_client::Attached;
use url::Url;

/// The Proving ground's seed.
const SEED: u64 = 1160;

/// The variable the engine's environment holds the proxies' password in.
#[cfg(unix)]
const PASSWORD_VARIABLE: &str = "PROVING_GROUND_PROXY_PASSWORD";

/// User configuration declaring the `list` provider of the Proving
/// ground's datacenter, isp and open proxies, the open one declared of the
/// datacenter kind too, with the proxies' credentials.
#[cfg(unix)]
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
#[cfg(unix)]
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
/// Covers: story:3-82, error:proxy_auth_failed
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

/// With `escalation.proxy_kinds: ["datacenter", "isp"]` over a `list`
/// provider that offers `datacenter`, `isp` and `residential`, a 403 on
/// `datacenter` retires its Session, opens the replacement on `isp`, never
/// climbs to `residential`, and keeps `isp` across a `run.resume` (#1137).
///
/// Covers: story:8-218, story:8-219, story:8-221, setting:escalation.proxy_kinds, operation:run.resume
#[cfg(unix)]
#[test]
fn a_403_escalates_within_escalation_proxy_kinds_and_survives_resume() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let fixtures = &proving_ground.description().fixtures;
    let server = |name: &str, kind: &str| {
        format!(
            "{{ server = \"http://{}\", kind = \"{kind}\" }}",
            fixtures.proxies[name]
        )
    };
    env.user_configuration(&format!(
        "[settings.proxy.providers.list]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\nservers = [{}, {}, {}]\n",
        fixtures.proxy_credentials.username,
        server("datacenter", "datacenter"),
        server("isp", "isp"),
        server("residential", "residential"),
    ));
    let password = fixtures.proxy_credentials.password.clone();
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(PASSWORD_VARIABLE, &password);
    });
    let run_settings = settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
            "session": { "pool": { "max_size": 1 } },
            "proxy": { "provider": "list" },
            "escalation": { "proxy_kinds": ["datacenter", "isp"] },
            "crawl": { "retry": { "max": 0 } },
            "run_state": { "durability": "full" },
        }),
    );

    engine.send(&json!({
        "jsonrpc": "2.0",
        "id": "start",
        "method": "crawl.start",
        "params": {
            "run_id": "escalate-resume",
            "seeds": [
                { "url": proving_ground.url("access.test", "/corpus/ip-block"), "label": "page" },
                { "url": proving_ground.url("access.test", "/corpus/article?n=1"), "label": "page" },
                { "url": proving_ground.url("access.test", "/corpus/article?n=2"), "label": "page" },
            ],
            "labels": { "page": {} },
            "settings": run_settings,
        },
    }));
    // Wait until `/corpus/article?n=1` is handed over (after `/corpus/ip-block`
    // escalated the domain from `datacenter` to `isp`), answer it, wait for
    // `/corpus/article?n=2` to be leased, then kill the engine before answering.
    let mut handled = 0usize;
    while handled < 2 {
        let message = engine.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            handled += 1;
            if handled == 1 {
                engine.send(&json!({
                    "jsonrpc": "2.0",
                    "id": message["id"],
                    "result": { "records": [] },
                }));
            }
        }
    }
    engine.kill();

    // Resume in a fresh engine process: `/corpus/article?n=2` must start on
    // `isp` without dropping back to `datacenter`.
    let mut resumed_engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(PASSWORD_VARIABLE, &password);
    });
    let mut answer = |_: &Value| Some(json!({ "result": { "records": [] } }));
    let resumed = resumed_engine.call(
        "resume",
        "run.resume",
        &json!({ "run_id": "escalate-resume", "settings": run_settings }),
        &mut answer,
    );
    assert_eq!(resumed["result"]["run_id"], "escalate-resume", "{resumed}");
    let finished = resumed_engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "escalate-resume" }),
        &mut answer,
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert!(resumed_engine.finished());

    let isp = BTreeSet::from(["isp".to_owned()]);
    assert_eq!(
        (
            proxies_to(&proving_ground, "/corpus/ip-block"),
            proxies_to(&proving_ground, "/corpus/article?n=1"),
            proxies_to(&proving_ground, "/corpus/article?n=2"),
        ),
        (
            BTreeSet::from(["datacenter".to_owned(), "isp".to_owned()]),
            isp.clone(),
            isp,
        )
    );
}

/// The distinct sticky exits the Proving ground's relays used for targets
/// holding `marker`, in order of observation.
#[cfg(unix)]
fn exits_to(proving_ground: &ProvingGround, marker: &str) -> Vec<String> {
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
        .filter_map(|relay| relay["exit"].as_str().map(str::to_owned))
        .collect()
}

/// With a hosted provider (`oxylabs`) narrowed to a single kind (`residential`),
/// a 403 retires the Session and opens the replacement with a fresh sticky
/// exit (never reusing the retired exit at once): against `/limits/2`, the
/// rotated exit succeeds; against `/corpus/ip-block`, once the escalation
/// budget (1 retirement per kind) is spent, the request ends `blocked` (#1141).
///
/// Covers: story:8-220
#[cfg(unix)]
#[test]
fn rotation_within_a_provider_uses_fresh_sticky_exits_and_ends_blocked_when_budget_is_spent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let fixtures = &proving_ground.description().fixtures;
    env.user_configuration(&format!(
        "[settings.proxy.providers.oxylabs]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\nendpoint = \"http://{}\"\n",
        fixtures.proxy_credentials.username,
        fixtures.proxies["residential"],
    ));
    let password = fixtures.proxy_credentials.password.clone();
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(PASSWORD_VARIABLE, &password);
    });
    let run_settings = settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
            "session": { "pool": { "max_size": 1 } },
            "proxy": { "provider": "oxylabs" },
            "escalation": { "proxy_kinds": ["residential"] },
            "crawl": { "retry": { "max": 0 } },
        }),
    );

    // 1) `limits/2` lets each address fetch 10 pages and flags the 11th with
    // 403; rotation gives a fresh sticky exit so all 11 requests succeed.
    let mut handled_limits = 0usize;
    let limit_seeds: Vec<Value> = (1..=11)
        .map(|n| {
            json!({
                "url": proving_ground.url("2.limits.test", &format!("/items/L-{n:04}")),
                "label": "page"
            })
        })
        .collect();
    let finished_limits = crawl_declaring(
        &mut engine,
        "rotate-limits",
        &limit_seeds,
        &json!({ "page": {} }),
        &run_settings,
        &mut |_, _| {
            handled_limits += 1;
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(finished_limits["result"]["lifecycle"], "finished");
    assert_eq!(handled_limits, 11);
    let rotated = (1..=11).find_map(|n| {
        let exits = exits_to(&proving_ground, &format!("/items/L-{n:04}"));
        (exits.len() == 2).then_some((n, exits))
    });
    let (_, rotated_exits) = rotated.expect(
        "one request past the 10-page per-IP quota should hit 403 on exit 1 then succeed on a fresh exit",
    );
    assert_ne!(
        rotated_exits[0], rotated_exits[1],
        "replacement Session must leave from a fresh sticky exit: {rotated_exits:?}"
    );

    // 2) `/corpus/ip-block` always 403s: with budget = 1 (1 kind in
    // `escalation.proxy_kinds`), it rotates once to a fresh sticky exit, then
    // ends `blocked`.
    let finished_blocked = crawl_declaring(
        &mut engine,
        "rotate-budget",
        &[json!({
            "url": proving_ground.url("access.test", "/corpus/ip-block"),
            "label": "page"
        })],
        &json!({ "page": {} }),
        &run_settings,
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert_eq!(finished_blocked["result"]["lifecycle"], "finished");
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "rotate-budget" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let block_exits = exits_to(&proving_ground, "/corpus/ip-block");
    assert_eq!(
        block_exits.len(),
        2,
        "initial attempt + 1 rotation (budget = 1): {block_exits:?}"
    );
    assert_ne!(
        block_exits[0], block_exits[1],
        "rotated Session must use a fresh sticky exit"
    );
    assert_eq!(
        outcomes["result"]["outcomes"][0]["kind"],
        json!("blocked"),
        "{outcomes}"
    );
}

/// Runs one periodic-trial crawl case, pausing past the 150 ms
/// `trial_interval` while handling the URL containing `pause_on`.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "synchronous integration test handler waiting past the 150 ms trial_interval"
)]
fn run_trial_case(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    run_id: &str,
    paths: &[&str],
    pause_on: &str,
    trial_interval: &Value,
) {
    let run_settings = settings(
        proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
            "session": { "pool": { "max_size": 1 } },
            "proxy": { "provider": "list" },
            "escalation": { "proxy_kinds": ["datacenter", "isp"], "trial_interval": trial_interval },
            "crawl": { "retry": { "max": 0 } },
        }),
    );
    let seeds: Vec<Value> = paths
        .iter()
        .map(|path| json!({ "url": proving_ground.url("access.test", path), "label": "page" }))
        .collect();
    let finished = crawl_declaring(
        engine,
        run_id,
        &seeds,
        &json!({ "page": {} }),
        &run_settings,
        &mut |_, handle| {
            if handle["crawl_request"]["url"]
                .as_str()
                .is_some_and(|u| u.contains(pause_on))
            {
                std::thread::sleep(Duration::from_millis(200));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(finished["result"]["lifecycle"], "finished");
}

/// With `escalation.trial_interval` set short (150 ms), a periodic trial on
/// the cheaper kind brings the domain back down after it succeeds and leaves
/// it up after it is blocked again; `escalation.trial_interval: null` runs no
/// trial (#1141).
///
/// Covers: story:8-222, setting:escalation.trial_interval
#[cfg(unix)]
#[test]
fn periodic_trial_steps_down_on_success_stays_up_on_block_and_null_runs_no_trial() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let fixtures = &proving_ground.description().fixtures;
    let server = |name: &str, kind: &str| {
        format!(
            "{{ server = \"http://{}\", kind = \"{kind}\" }}",
            fixtures.proxies[name]
        )
    };
    env.user_configuration(&format!(
        "[settings.proxy.providers.list]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\nservers = [{}, {}]\n",
        fixtures.proxy_credentials.username,
        server("datacenter", "datacenter"),
        server("isp", "isp"),
    ));
    let password = fixtures.proxy_credentials.password.clone();
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(PASSWORD_VARIABLE, &password);
    });
    let dc = BTreeSet::from(["datacenter".to_owned()]);
    let isp = BTreeSet::from(["isp".to_owned()]);
    let both = BTreeSet::from(["datacenter".to_owned(), "isp".to_owned()]);

    // 1) Short trial_interval + trial succeeds -> domain steps back down to datacenter.
    run_trial_case(
        &mut engine,
        &proving_ground,
        "trial-down",
        &[
            "/corpus/ip-block?case=down",
            "/corpus/article?case=down&n=1",
            "/corpus/article?case=down&n=2",
            "/corpus/article?case=down&n=3",
        ],
        "n=1",
        &json!(150),
    );
    // 2) Short trial_interval + trial blocked -> domain stays on isp.
    run_trial_case(
        &mut engine,
        &proving_ground,
        "trial-up",
        &[
            "/corpus/ip-block?case=up1",
            "/corpus/article?case=up&n=1",
            "/corpus/ip-block?case=up2",
            "/corpus/article?case=up&n=2",
        ],
        "case=up&n=1",
        &json!(150),
    );
    // 3) trial_interval: null -> no trial runs even after waiting past 200ms.
    run_trial_case(
        &mut engine,
        &proving_ground,
        "trial-null",
        &[
            "/corpus/ip-block?case=null",
            "/corpus/article?case=null&n=1",
            "/corpus/article?case=null&n=2",
        ],
        "case=null&n=1",
        &Value::Null,
    );
    assert!(engine.finished());

    assert_eq!(
        (
            proxies_to(&proving_ground, "case=down&n=1"),
            proxies_to(&proving_ground, "case=down&n=2"),
            proxies_to(&proving_ground, "case=down&n=3"),
            proxies_to(&proving_ground, "case=up2"),
            proxies_to(&proving_ground, "case=up&n=2"),
            proxies_to(&proving_ground, "case=null&n=1"),
            proxies_to(&proving_ground, "case=null&n=2"),
        ),
        (
            isp.clone(),
            dc.clone(),
            dc,
            both,
            isp.clone(),
            isp.clone(),
            isp,
        )
    );
}

/// Starts an attached Foxhound engine configured with a `list` Proxy provider
/// serving `kinds` (with optional `price` per GB).
#[cfg(unix)]
fn list_engine(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    kinds: &[&str],
    price: Option<&str>,
) -> Attached {
    let fixtures = &proving_ground.description().fixtures;
    let servers = kinds
        .iter()
        .map(|kind| {
            format!(
                "{{ server = \"http://{}\", kind = \"{kind}\" }}",
                fixtures.proxies[*kind]
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let price_line = price.map_or_else(String::new, |p| format!("price = \"{p}\"\n"));
    env.user_configuration(&format!(
        "[settings.proxy.providers.list]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\n{price_line}servers = [{servers}]\n",
        fixtures.proxy_credentials.username,
    ));
    let password = fixtures.proxy_credentials.password.clone();
    attached_with(env, Some(Duration::from_secs(60)), |command| {
        command.env(PASSWORD_VARIABLE, &password);
    })
}

/// Common HTTP-tier settings for a `list`-provider escalation test.
#[cfg(unix)]
fn list_settings(proving_ground: &ProvingGround, kinds: &[&str], retry: &Value) -> Value {
    settings(
        proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
            "session": { "pool": { "max_size": 1 } },
            "proxy": { "provider": "list" },
            "escalation": { "proxy_kinds": kinds },
            "crawl": { "retry": retry },
        }),
    )
}

/// Stopping causes (`payment_required`, `consent_required`, `decoy`) never
/// escalate, rotate or retry even with a 2-tier proxy ladder and
/// `crawl.retry.max = 2`, and the Run report counts requests by Block cause
/// (`requests.by_block_cause`) (#1145).
///
/// Covers: story:8-223, story:8-224
#[cfg(unix)]
#[test]
fn stopping_causes_never_escalate_or_retry_and_report_counts_by_block_cause() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = list_engine(&env, &proving_ground, &["datacenter", "isp"], None);
    let run_settings = list_settings(
        &proving_ground,
        &["datacenter", "isp"],
        &json!({ "max": 2 }),
    );
    let finished = crawl_declaring(
        &mut engine,
        "stopping-causes",
        &[
            json!({ "url": proving_ground.url("access.test", "/corpus/payment"), "label": "page" }),
            json!({ "url": proving_ground.url("access.test", "/corpus/article?case=consent"), "label": "page" }),
            json!({ "url": proving_ground.url("access.test", "/corpus/labyrinth"), "label": "decoy_page" }),
        ],
        &json!({
            "page": {},
            "decoy_page": {
                "settings": {
                    "session": {
                        "page_check": {
                            "expected_page": { "kind": "css", "selector": "article.never-present" }
                        }
                    }
                }
            },
        }),
        &run_settings,
        &mut |_, handle| {
            let blocked = |cause: &str| {
                json!({
                    "error": {
                        "code": -32003,
                        "message": cause,
                        "data": { "code": "blocked", "retryable": false, "cause": cause }
                    }
                })
            };
            match handle["crawl_request"]["url"].as_str().unwrap_or_default() {
                url if url.contains("/corpus/payment") => blocked("payment_required"),
                url if url.contains("case=consent") => blocked("consent_required"),
                _ => json!({ "result": { "records": [] } }),
            }
        },
    );
    assert_eq!(finished["result"]["lifecycle"], "finished");
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "stopping-causes" }),
        &mut |_| None,
    );
    let report = engine.call(
        "report",
        "run.report",
        &json!({ "run_id": "stopping-causes" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let dc = BTreeSet::from(["datacenter".to_owned()]);
    assert_eq!(proxies_to(&proving_ground, "/corpus/payment"), dc);
    assert_eq!(proxies_to(&proving_ground, "case=consent"), dc);
    assert_eq!(proxies_to(&proving_ground, "/corpus/labyrinth"), dc);
    let attempts: Vec<(Value, Value)> = outcomes["result"]["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["cause"].clone(), o["attempts"].clone()))
        .collect();
    assert_eq!(
        attempts,
        vec![
            (json!("payment_required"), json!(1)),
            (json!("consent_required"), json!(1)),
            (json!("decoy"), json!(1)),
        ],
        "{outcomes}"
    );
    let by_cause: BTreeSet<(String, u64)> = report["result"]["requests"]["by_block_cause"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().unwrap().to_owned(),
                entry["count"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        by_cause,
        BTreeSet::from([
            ("consent_required".to_owned(), 1),
            ("decoy".to_owned(), 1),
            ("payment_required".to_owned(), 1),
        ]),
        "{report}"
    );
}

/// A run that escalates twice (`datacenter` -> `isp` -> `residential`) still
/// has its full `crawl.retry.max` (= 1) left for the request (#1145).
///
/// Covers: story:8-217
#[cfg(unix)]
#[test]
fn escalating_twice_keeps_full_crawl_retry_max_for_the_request() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = list_engine(
        &env,
        &proving_ground,
        &["datacenter", "isp", "residential"],
        None,
    );
    let quota_seeds = |prefix: &str| -> Vec<Value> {
        (1..=10)
            .map(|n| {
                json!({
                    "url": proving_ground.url("2.limits.test", &format!("/items/{prefix}-{n:02}")),
                    "label": "page"
                })
            })
            .collect()
    };
    // Spend `datacenter` and `isp` 10-page quotas on `2.limits.test`.
    for (run_id, kind, prefix) in [
        ("spend-dc", "datacenter", "DC"),
        ("spend-isp", "isp", "ISP"),
    ] {
        let mut s = list_settings(&proving_ground, &[kind], &json!({ "max": 0 }));
        s["proxy"]["kind"] = json!(kind);
        let fin = crawl_declaring(
            &mut engine,
            run_id,
            &quota_seeds(prefix),
            &json!({ "page": {} }),
            &s,
            &mut |_, _| json!({ "result": { "records": [] } }),
        );
        assert_eq!(fin["result"]["lifecycle"], "finished");
    }

    // Now one request escalates twice (dc -> isp -> residential), then fails
    // once in the handler and still gets its full `crawl.retry.max = 1`.
    let run_settings = list_settings(
        &proving_ground,
        &["datacenter", "isp", "residential"],
        &json!({ "max": 1, "handler_failed": true }),
    );
    let mut handler_calls = 0usize;
    let fin = crawl_declaring(
        &mut engine,
        "two-escalations-plus-retry",
        &[json!({
            "url": proving_ground.url("2.limits.test", "/items/FINAL-01"),
            "label": "page"
        })],
        &json!({ "page": {} }),
        &run_settings,
        &mut |_, _| {
            handler_calls += 1;
            if handler_calls == 1 {
                json!({
                    "error": {
                        "code": -32001,
                        "message": "transient failure after two escalations",
                        "data": {
                            "code": "handler_failed",
                            "retryable": true
                        }
                    }
                })
            } else {
                json!({ "result": { "records": [] } })
            }
        },
    );
    assert_eq!(fin["result"]["lifecycle"], "finished");
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "two-escalations-plus-retry" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    assert_eq!(handler_calls, 2);
    assert_eq!(
        proxies_to(&proving_ground, "/items/FINAL-01"),
        BTreeSet::from([
            "datacenter".to_owned(),
            "isp".to_owned(),
            "residential".to_owned(),
        ])
    );
    assert_eq!(
        (
            &outcomes["result"]["outcomes"][0]["kind"],
            &outcomes["result"]["outcomes"][0]["attempts"],
        ),
        (&json!("succeeded"), &json!(2)),
        "{outcomes}"
    );
}

/// Usage records `proxy_bytes` per proxy kind (`datacenter`, `isp`) during
/// escalation, and a money Limit on `proxy_bytes` stops the escalation climb
/// with `limit_reached` (#1145).
///
/// Covers: story:8-224
#[cfg(unix)]
#[test]
fn usage_tracks_proxy_bytes_per_kind_and_money_limit_stops_escalation_with_limit_reached() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let fixtures = &proving_ground.description().fixtures;
    let server = |name: &str, kind: &str| {
        format!(
            "{{ server = \"http://{}\", kind = \"{kind}\" }}",
            fixtures.proxies[name]
        )
    };
    // High price per GB so a tiny USD limit stops escalation cleanly.
    env.user_configuration(&format!(
        "[settings.proxy.providers.list]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\nprice = \"100000 USD\"\nservers = [{}, {}]\n",
        fixtures.proxy_credentials.username,
        server("datacenter", "datacenter"),
        server("isp", "isp"),
    ));
    let password = fixtures.proxy_credentials.password.clone();
    let mut engine = attached_with(&env, Some(Duration::from_secs(60)), |command| {
        command.env(PASSWORD_VARIABLE, &password);
    });
    let run_settings = settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
            "session": { "pool": { "max_size": 1 } },
            "proxy": { "provider": "list" },
            "escalation": { "proxy_kinds": ["datacenter", "isp"] },
            "crawl": { "retry": { "max": 0 } },
        }),
    );

    // 1) Uncapped escalation: Usage shows `proxy_bytes_by_kind` for both kinds.
    let uncapped = crawl_declaring(
        &mut engine,
        "usage-per-kind",
        &[json!({
            "url": proving_ground.url("access.test", "/corpus/ip-block"),
            "label": "page"
        })],
        &json!({ "page": {} }),
        &run_settings,
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert_eq!(uncapped["result"]["lifecycle"], "finished");
    let usage = engine.call(
        "usage",
        "run.usage",
        &json!({ "run_id": "usage-per-kind" }),
        &mut |_| None,
    );
    let kinds: BTreeSet<String> = usage["result"]["proxy_bytes_by_kind"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["bytes"].as_u64().unwrap_or(0) > 0)
        .map(|entry| entry["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        kinds,
        BTreeSet::from(["datacenter".to_owned(), "isp".to_owned()]),
        "{usage}"
    );

    // 2) Tight currency limit (1 millionth of a USD = $0.000001): stops the
    //    escalation climb with `limit_reached`.
    let started = engine.call(
        "start-limited",
        "crawl.start",
        &json!({
            "run_id": "money-limited",
            "seeds": [{
                "url": proving_ground.url("access.test", "/corpus/ip-block?limited=1"),
                "label": "page"
            }],
            "labels": { "page": {} },
            "settings": run_settings,
            "limits": [{ "usage_kind": "proxy_bytes", "currency": "USD", "amount": 1 }],
        }),
        &mut |_| None,
    );
    assert!(started.get("result").is_some(), "{started}");
    let report = engine.call(
        "report-limited",
        "run.report",
        &json!({ "run_id": "money-limited" }),
        &mut |msg| {
            (msg.get("method").and_then(Value::as_str) == Some("crawl_request.handle"))
                .then(|| json!({ "result": { "records": [] } }))
        },
    );
    assert!(engine.finished());
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &report["result"]["stop_reason"]["kind"],
            &report["result"]["requests"]["by_error_code"],
        ),
        (
            &json!("stopped"),
            &json!("limit"),
            &json!([{ "count": 1, "name": "limit_reached" }]),
        ),
        "{report}"
    );
}
