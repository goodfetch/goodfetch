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

use crate::attached;
use crate::attached_with;
use crate::fetch_tiers::settings;
use crate::frontier::crawl_declaring;
use crate::url_fragments::refused_start;

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
/// Run's provider's only `isp` server.
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
                // A robots.txt would be read past the proxy, which the
                // start refuses (#1640).
                "site_preferences": { "respect_robots_txt": false },
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
    assert_eq!(
        (
            &sessions,
            proxies_to(&proving_ground, "mark=account-"),
            proxies_to(&proving_ground, "mark=public-1"),
            proxies_to(&proving_ground, "/corpus/ip-block"),
            proxies_to(&proving_ground, "mark=public-2"),
            &finished["result"]["lifecycle"],
        ),
        (
            &BTreeSet::from(["account.session-1".to_owned(), "session-2".to_owned()]),
            isp.clone(),
            isp.clone(),
            isp.clone(),
            isp,
            &json!("finished"),
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

/// What a crawl's start under `run` settings, from `seed`, with the Label
/// `page` setting `label`, was refused with, its Error code, key and kind,
/// and how many requests the Proving ground received, through a proxy or
/// not.
fn start_beside_a_proxy(run: &Value, seed: fn(&ProvingGround) -> Value, label: &Value) -> Value {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = attached(&env);
    let mut run = run.clone();
    run["proxy"] = json!({
        "server": format!("http://{}", proving_ground.description().fixtures.proxies["open"]),
    });

    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "past-the-proxy",
            "seeds": [seed(&proving_ground)],
            "labels": { "page": { "settings": label } },
            "settings": settings(&proving_ground, &run),
        }),
        &mut |_| None,
    );
    assert!(engine.finished());
    let refusal = &started["error"]["data"];
    json!([
        refusal["code"],
        refusal["key"],
        refusal["kind"],
        proving_ground.received_requests().unwrap().every().count(),
    ])
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

/// A crawl Run that sets a proxy and can read a site past it fails its
/// start with `capability_unavailable` naming the proxy key and the read,
/// before the Run begins (#1640's interim refusal): a document a forced
/// `fetch.tier: http` fetches, the Run's or a Label's, a sitemap seed's
/// sitemap, a robots.txt and an AI-use signal file are fetched on the
/// Run's HTTP tier, which leaves directly and would show the site the
/// address the proxy hides. The Proving ground receives nothing.
///
/// Covers: setting:proxy.server, error:capability_unavailable
#[rstest]
#[case::the_run_forces_the_http_tier(
    json!({ "fetch": { "tier": "http" }, "site_preferences": { "respect_robots_txt": false } }),
    page_seed,
    json!({}),
    "http"
)]
#[case::a_label_forces_the_http_tier(
    json!({ "site_preferences": { "respect_robots_txt": false } }),
    page_seed,
    json!({ "fetch": { "tier": "http" } }),
    "http"
)]
#[case::a_sitemap_seed(
    json!({ "fetch": { "tier": "browser" }, "site_preferences": { "respect_robots_txt": false } }),
    sitemap_seed,
    json!({}),
    "sitemap"
)]
#[case::robots_txt_is_respected(json!({ "fetch": { "tier": "browser" } }), page_seed, json!({}), "robots_txt")]
#[case::robots_txt_marks_the_records_taken_against_it(
    json!({
        "fetch": { "tier": "browser" },
        "site_preferences": { "respect_robots_txt": false, "mark_robots_txt_ignored": true },
    }),
    page_seed,
    json!({}),
    "robots_txt"
)]
#[case::the_use_signals_are_honoured(
    json!({
        "fetch": { "tier": "browser" },
        "site_preferences": {
            "respect_robots_txt": false, "respect_use_signals": true, "purpose": "ai-train",
        },
    }),
    page_seed,
    json!({}),
    "use_signals"
)]
fn a_run_that_can_read_a_site_past_its_proxy_fails_its_start(
    #[case] run: Value,
    #[case] seed: fn(&ProvingGround) -> Value,
    #[case] label: Value,
    #[case] kind: &str,
) {
    let refused = start_beside_a_proxy(&run, seed, &label);

    assert_eq!(
        refused,
        json!(["capability_unavailable", "proxy.server", kind, 0])
    );
}
