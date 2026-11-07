//! A Session's proxy over stdio (spec #577, ticket #708), through
//! `goodfetch-protocol`'s types: `proxy.server`, `proxy.bypass`,
//! `proxy.username` and `proxy.password` as a Session's settings, one proxy
//! per Session and none per page or request (ADRs 0021 and 0092; ADR 0037 /
//! SA-22), against the Proving ground's forward proxies, whose connection
//! record says which proxy each request went through; and the `list`
//! Proxy provider (spec #599, ticket #882): `proxy.provider`,
//! `proxy.kind`, `proxy.country` and `proxy.providers`, one of the user's
//! servers per Session for its life, and its bytes by proxy kind in Usage,
//! priced under a currency Limit (ADRs 0021, 0041, 0101 and 0129), the Run
//! that Limit stopped waiting in "Needs you" until a person raises it at
//! the interactive prompt (ADRs 0031 and 0041); and the `bright_data`,
//! `oxylabs` and `apify` adapters (ticket #891), against a forward proxy
//! that reads each provider's sticky username: one sticky exit per Session
//! for its life, and `proxy_exit_may_change` for a Session that outlives
//! its sticky lifetime.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the SOCKS5 test reads the Goodfetch home's files, the Usage tests poll the Proving ground and run a sink proxy, on their own threads, never an async worker"
)]

use std::collections::BTreeSet;
use std::sync::mpsc;

use goodfetch_test_client::robots_txt_read;
use goodfetch_test_support::{
    Canaries, Canary, DISPATCH_TARGET, Hit, OUTCOME_TARGET, Output, ProvingGround,
    ReceivedRequests, SecretKind, TestEnv, browser_cache, cli, pty,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::page::{BROWSER_DEADLINE, opened};
use crate::run::call;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Error code of `answer`'s error, the key it names and its layer.
fn refusal_of(answer: &Value) -> (Value, Value, Value) {
    let data = &answer["error"]["data"];
    (
        data["code"].clone(),
        data["key"].clone(),
        data["layer"].clone(),
    )
}

/// Whether `answer`'s refusal's reason names both credential settings.
fn names_the_credential_settings(answer: &Value) -> bool {
    answer["error"]["data"]["reason"]
        .as_str()
        .is_some_and(|reason| {
            reason.contains("proxy.username") && reason.contains("proxy.password")
        })
}

/// Covers: story:5-88, setting:proxy.server, error:setting_invalid
#[rstest]
#[case::socks4("socks4://127.0.0.1:1080")]
#[case::credentials_in_the_url("http://user:secret@127.0.0.1:8080")]
#[case::a_username_alone_in_the_url("socks5://user@127.0.0.1:1080")]
fn a_proxy_server_on_socks4_or_with_credentials_in_its_url_is_refused_naming_the_credential_settings(
    #[case] server: &str,
) {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();

    let started = call(
        &mut engine,
        1,
        "run.start",
        &json!({ "run_id": "refused", "settings": { "proxy": { "server": server } } }),
    );
    let _ = engine.finish();

    assert_eq!(
        (
            refusal_of(&started),
            names_the_credential_settings(&started)
        ),
        (
            (
                json!("setting_invalid"),
                json!("proxy.server"),
                json!("run")
            ),
            true
        )
    );
}

/// A `hello` from the MCP server, a Client acting for a model.
fn hello_for_a_model() -> Value {
    let mut hello = hello("collie", "0.1");
    hello["params"]["surface"] = json!("mcp");
    hello
}

/// Covers: story:5-88, setting:proxy.server, setting:proxy.password, error:setting_locked
#[test]
fn a_client_acting_for_a_model_sets_no_proxy_on_a_run_or_a_session() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello_for_a_model());
    let _ = engine.next();

    let on_the_run = call(
        &mut engine,
        1,
        "run.start",
        &json!({
            "run_id": "proxied",
            "settings": { "proxy": { "server": "http://127.0.0.1:3128" } },
        }),
    );
    let _ = call(&mut engine, 2, "run.start", &json!({ "run_id": "plain" }));
    let on_a_session = call(
        &mut engine,
        3,
        "run.new_session",
        &json!({
            "run_id": "plain",
            "settings": { "proxy": { "password": "env:PROXY_PASSWORD" } },
        }),
    );
    let _ = call(&mut engine, 4, "run.finish", &json!({ "run_id": "plain" }));
    let _ = engine.finish();

    assert_eq!(
        (refusal_of(&on_the_run), refusal_of(&on_a_session)),
        (
            (json!("setting_locked"), json!("proxy.server"), json!("run")),
            (
                json!("setting_locked"),
                json!("proxy.password"),
                json!("run")
            ),
        )
    );
}

/// The project configuration a Client acting for a model sends with
/// `run.start` is the model's too, so it sets no proxy either.
///
/// Covers: story:5-88, setting:proxy.server, error:setting_locked
#[test]
fn a_client_acting_for_a_model_sets_no_proxy_through_the_project_configuration_it_sends() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello_for_a_model());
    let _ = engine.next();

    let started = call(
        &mut engine,
        1,
        "run.start",
        &json!({
            "run_id": "proxied",
            "project": {
                "path": "goodfetch.toml",
                "settings": { "proxy": { "server": "http://127.0.0.1:3128" } },
            },
        }),
    );
    let _ = engine.finish();

    assert_eq!(
        refusal_of(&started),
        (
            json!("setting_locked"),
            json!("proxy.server"),
            json!("project_configuration")
        )
    );
}

/// The Proving ground's own seed for these tests.
const SEED: u64 = 708;

/// The variable the engine's environment holds a proxy password in.
pub(crate) const PASSWORD_VARIABLE: &str = "PROVING_GROUND_PROXY_PASSWORD";

/// The address of the Proving ground's proxy `name`.
fn proxy_address(proving_ground: &ProvingGround, name: &str) -> String {
    proving_ground.description().fixtures.proxies[name].clone()
}

/// `proxy` settings: through the Proving ground's proxy `name` on
/// `scheme`, with `username` and the password in [`PASSWORD_VARIABLE`].
fn proxy_settings(
    proving_ground: &ProvingGround,
    scheme: &str,
    name: &str,
    username: Option<&str>,
) -> Value {
    let server = format!("{scheme}://{}", proxy_address(proving_ground, name));
    let mut proxy = json!({ "server": server });
    if let Some(username) = username {
        proxy["username"] = json!(username);
        proxy["password"] = json!(format!("env:{PASSWORD_VARIABLE}"));
    }
    proxy
}

/// Starts an engine whose environment holds `password`, and a Run whose
/// `proxy` settings are `proxy`.
fn started_through<'env>(
    env: &'env TestEnv,
    cache: &std::path::Path,
    proving_ground: &ProvingGround,
    run_id: &str,
    proxy: Value,
    password: &str,
) -> Attached<'env> {
    let mut settings = serde_json::Map::new();
    settings.insert("proxy".to_owned(), proxy);
    crate::page::started_with(env, cache, proving_ground, run_id, &settings, |command| {
        command.env(PASSWORD_VARIABLE, password);
    })
}

/// How a failing proxy is met.
#[derive(Clone, Copy, Debug)]
enum Failing {
    /// The tunnel to an `https://` page is refused.
    RefusedTunnel,
    /// Nothing listens where the proxy should.
    Down,
    /// The proxy asks for credentials the Session does not give.
    NoCredentials,
    /// The proxy refuses the Session's password.
    WrongPassword,
}

/// Covers: story:5-88, setting:proxy.server, setting:proxy.username, setting:proxy.password
#[cfg(unix)]
#[rstest]
/// Covers: error:proxy_unavailable
#[case::a_refused_tunnel(Failing::RefusedTunnel, "proxy_unavailable", true)]
/// Covers: error:proxy_unavailable
#[case::a_proxy_that_is_down(Failing::Down, "proxy_unavailable", true)]
/// Covers: error:proxy_auth_failed
#[case::no_credentials(Failing::NoCredentials, "proxy_auth_failed", false)]
/// Covers: error:proxy_auth_failed
#[case::a_wrong_password(Failing::WrongPassword, "proxy_auth_failed", false)]
fn a_failing_proxy_raises_a_proxy_error_never_navigation_failed(
    #[case] failing: Failing,
    #[case] code: &str,
    #[case] retryable: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let credentials = &proving_ground.description().fixtures.proxy_credentials;
    // The wrong password the proxy refuses with its 407, a canary the
    // engine reads from its environment through the `env:` Secret provider.
    let mut canaries = Canaries::new(env.seed());
    let wrong = canaries.plant(SecretKind::ProxyPassword).value().to_owned();
    let (name, username, password) = match failing {
        Failing::RefusedTunnel => ("refusing_tunnels", None, ""),
        Failing::Down => ("down", None, ""),
        Failing::NoCredentials => ("datacenter", None, ""),
        Failing::WrongPassword => (
            "datacenter",
            Some(credentials.username.as_str()),
            wrong.as_str(),
        ),
    };
    let proxy = proxy_settings(&proving_ground, "http", name, username);
    let mut engine = started_through(&env, &cache, &proving_ground, "failing", proxy, password);
    let url = match failing {
        Failing::RefusedTunnel => proving_ground
            .url("scope.test", "/storage?mark=tunnel")
            .replacen("http://", "https://", 1),
        Failing::Down | Failing::NoCredentials | Failing::WrongPassword => {
            proving_ground.url("scope.test", "/storage?mark=plain")
        }
    };

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "failing" }),
    ));
    let navigated = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    );
    let arguments = crate::page::browser_command_lines(&env);
    let _ = call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "failing" }),
    );
    let (_, stderr) = engine.finish_with_stderr();

    let data = &navigated["error"]["data"];
    assert_eq!(
        (
            &data["code"],
            &data["retryable"],
            &data["proxy"],
            leaks_of(&canaries, env.home(), &stderr, &arguments),
        ),
        (
            &json!(code),
            &json!(retryable),
            &json!(format!("http://{}", proxy_address(&proving_ground, name))),
            Vec::new(),
        ),
        "{navigated}"
    );
}

/// How a failing SOCKS5 proxy is met by an `http:` navigation.
#[derive(Clone, Copy, Debug)]
enum FailingSocks5 {
    /// Nothing listens where the proxy should, and the Session gives no
    /// credentials: the egress relay speaks SOCKS5 to it itself.
    Down,
    /// Nothing listens where the proxy should, and the Session gives
    /// credentials: its loopback relay authenticates to it.
    DownWithCredentials,
    /// The proxy refuses the Session's password.
    WrongPassword,
}

/// A plain `http:` request through a SOCKS5 proxy goes through the
/// Session's own relays, whose `502` for a proxy they could not use is
/// never the site's answer (spec #577's pitfall H12; finding 577-1): a
/// proxy that is down raises `proxy_unavailable` and one that refuses the
/// password `proxy_auth_failed`, as an HTTP proxy's failures do.
///
/// Covers: story:5-88, setting:proxy.server, setting:proxy.username, setting:proxy.password
#[cfg(unix)]
#[rstest]
/// Covers: error:proxy_unavailable
#[case::a_proxy_that_is_down(FailingSocks5::Down, "proxy_unavailable", true)]
/// Covers: error:proxy_unavailable
#[case::a_proxy_that_is_down_behind_the_loopback_relay(
    FailingSocks5::DownWithCredentials,
    "proxy_unavailable",
    true
)]
/// Covers: error:proxy_auth_failed
#[case::a_wrong_password(FailingSocks5::WrongPassword, "proxy_auth_failed", false)]
fn a_failing_socks5_proxy_raises_a_proxy_error_on_an_http_navigation(
    #[case] failing: FailingSocks5,
    #[case] code: &str,
    #[case] retryable: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let credentials = &proving_ground.description().fixtures.proxy_credentials;
    let mut canaries = Canaries::new(env.seed());
    let wrong = canaries.plant(SecretKind::ProxyPassword).value().to_owned();
    let (name, username, password) = match failing {
        FailingSocks5::Down => ("down", None, ""),
        FailingSocks5::DownWithCredentials => {
            ("down", Some(credentials.username.as_str()), wrong.as_str())
        }
        FailingSocks5::WrongPassword => (
            "socks5",
            Some(credentials.username.as_str()),
            wrong.as_str(),
        ),
    };
    let proxy = proxy_settings(&proving_ground, "socks5", name, username);
    let mut engine = started_through(&env, &cache, &proving_ground, "socks", proxy, password);

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "socks" }),
    ));
    let navigated = call(
        &mut engine,
        3,
        "page.goto",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/storage?mark=socks"),
        }),
    );
    let arguments = crate::page::browser_command_lines(&env);
    let _ = call(&mut engine, 4, "run.finish", &json!({ "run_id": "socks" }));
    let (_, stderr) = engine.finish_with_stderr();

    let data = &navigated["error"]["data"];
    assert_eq!(
        (
            &data["code"],
            &data["retryable"],
            &data["proxy"],
            leaks_of(&canaries, env.home(), &stderr, &arguments),
        ),
        (
            &json!(code),
            &json!(retryable),
            &json!(format!("socks5://{}", proxy_address(&proving_ground, name))),
            Vec::new(),
        ),
        "{navigated}"
    );
}

/// The relays the Proving ground's proxies carried out for targets that
/// hold `marker`, in arrival order: each one's proxy and target.
fn relays_to(proving_ground: &ProvingGround, marker: &str) -> Vec<(String, String)> {
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
        .map(|relay| {
            let target = relay["target"].as_str().unwrap();
            let from_marker = &target[target.find(marker).unwrap()..];
            (
                relay["proxy"].as_str().unwrap().to_owned(),
                from_marker.to_owned(),
            )
        })
        .collect()
}

/// Covers: story:5-88, operation:run.new_session, setting:proxy.server, setting:proxy.username, setting:proxy.password
#[cfg(unix)]
#[test]
fn two_sessions_of_one_run_reach_the_site_each_through_its_own_proxy() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let credentials = proving_ground
        .description()
        .fixtures
        .proxy_credentials
        .clone();
    let username = Some(credentials.username.as_str());
    let runs = proxy_settings(&proving_ground, "http", "datacenter", username);
    let own = proxy_settings(&proving_ground, "http", "isp", username);
    let mut engine = started_through(
        &env,
        &cache,
        &proving_ground,
        "two-proxies",
        runs,
        &credentials.password,
    );
    let url = |mark: &str| proving_ground.url("scope.test", &format!("/storage?mark={mark}"));

    let default_page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "two-proxies" }),
    ));
    let session = call(
        &mut engine,
        3,
        "run.new_session",
        &json!({ "run_id": "two-proxies", "settings": { "proxy": own } }),
    );
    let own_page = opened(&call(
        &mut engine,
        4,
        "session.new_page",
        &json!({ "session_id": session["result"]["session_id"] }),
    ));
    let statuses = [
        (5, &default_page, "default"),
        (6, &own_page, "own"),
        (7, &default_page, "default"),
    ]
    .map(|(id, page, mark)| {
        call(
            &mut engine,
            id,
            "page.goto",
            &json!({ "page_id": page.page_id, "url": url(mark) }),
        )["result"]["response"]["status"]
            .clone()
    });
    let _ = call(
        &mut engine,
        8,
        "run.finish",
        &json!({ "run_id": "two-proxies" }),
    );
    let _ = engine.finish();

    assert_eq!(
        (
            statuses.to_vec(),
            relays_to(&proving_ground, "/storage?mark=")
        ),
        (
            vec![json!(200), json!(200), json!(200)],
            vec![
                ("datacenter".to_owned(), "/storage?mark=default".to_owned()),
                ("isp".to_owned(), "/storage?mark=own".to_owned()),
                ("datacenter".to_owned(), "/storage?mark=default".to_owned()),
            ]
        )
    );
}

/// A SOCKS5 proxy with credentials whose host has no address fails the
/// Session's open, before any request: `proxy_unavailable` names the proxy
/// and no URL.
///
/// Covers: story:5-88, setting:proxy.server, error:proxy_unavailable
#[cfg(unix)]
#[test]
fn a_proxy_that_fails_the_sessions_open_names_no_url() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let proxy = json!({
        "server": "socks5://nowhere.invalid:1080",
        "username": "user",
        "password": format!("env:{PASSWORD_VARIABLE}"),
    });
    let mut engine = started_through(&env, &cache, &proving_ground, "nowhere", proxy, "secret");

    let opened = call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "nowhere" }),
    );
    let _ = call(
        &mut engine,
        3,
        "run.finish",
        &json!({ "run_id": "nowhere" }),
    );
    let _ = engine.finish();

    let data = &opened["error"]["data"];
    assert_eq!(
        (&data["code"], &data["proxy"], data.get("url")),
        (
            &json!("proxy_unavailable"),
            &json!("socks5://nowhere.invalid:1080"),
            None
        ),
        "{opened}"
    );
}

/// Covers: story:5-88, setting:proxy.bypass
#[cfg(unix)]
#[test]
fn a_host_proxy_bypass_names_is_reached_without_the_proxy() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut proxy = proxy_settings(&proving_ground, "http", "open", None);
    proxy["bypass"] = json!("other.test, scope.test");
    let mut engine = started_through(&env, &cache, &proving_ground, "bypassed", proxy, "");

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "bypassed" }),
    ));
    let navigated = call(
        &mut engine,
        3,
        "page.goto",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/storage?mark=bypassed"),
        }),
    );
    let _ = call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "bypassed" }),
    );
    let _ = engine.finish();

    assert_eq!(
        (
            &navigated["result"]["response"]["status"],
            relays_to(&proving_ground, "/storage?mark=")
        ),
        (&json!(200), vec![])
    );
}

/// A proxy is a Session's, never a page's or a request's (ADR 0092): no
/// call takes a `proxy` argument, so one is `invalid_params`, and a page's
/// own `settings`, which hold only what one page can use, refuse
/// `proxy.server` with `setting_not_applicable` naming it (ADR 0083's
/// refusals, as for `browser.viewport` on a page).
///
/// Covers: story:5-89
#[rstest]
/// Covers: operation:session.new_page
#[case::a_pages_own_proxy(
    "session.new_page",
    json!({ "session_id": "per-page.session-1", "proxy": { "server": "http://127.0.0.1:3128" } }),
    ("invalid_params", None)
)]
/// Covers: operation:run.new_page
#[case::a_pages_own_settings(
    "run.new_page",
    json!({ "run_id": "per-page", "settings": { "proxy": { "server": "http://127.0.0.1:3128" } } }),
    ("setting_not_applicable", Some("proxy.server"))
)]
/// Covers: operation:page.goto
#[case::a_requests_own_proxy(
    "page.goto",
    json!({ "page_id": "page-1", "url": "http://scope.test/", "proxy": { "server": "http://127.0.0.1:3128" } }),
    ("invalid_params", None)
)]
fn a_proxy_per_page_or_per_request_is_refused(
    #[case] method: &str,
    #[case] params: Value,
    #[case] refusal: (&str, Option<&str>),
) {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let _ = call(
        &mut engine,
        1,
        "run.start",
        &json!({ "run_id": "per-page" }),
    );

    let refused = call(&mut engine, 2, method, &params);
    let _ = call(
        &mut engine,
        3,
        "run.finish",
        &json!({ "run_id": "per-page" }),
    );
    let _ = engine.finish();

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["key"]
        ),
        (&json!(refusal.0), &json!(refusal.1)),
        "{refused}"
    );
}

/// Where the engine left any of `canaries` in what it writes: the files of
/// the Goodfetch home `home`, at any depth, its stderr and the browser's
/// command lines, in every encoding a search reads.
fn leaks_of(
    canaries: &Canaries,
    home: &std::path::Path,
    stderr: &str,
    arguments: &[String],
) -> Vec<Hit> {
    let arguments = arguments.join(
        "
",
    );
    canaries
        .leaks(&[
            Output::Path(home),
            Output::Bytes {
                name: "stderr",
                bytes: stderr.as_bytes(),
            },
            Output::Bytes {
                name: "the browser's command lines",
                bytes: arguments.as_bytes(),
            },
        ])
        .unwrap()
}

/// Covers: story:5-88, setting:proxy.server, setting:proxy.username, setting:proxy.password
#[cfg(unix)]
#[rstest]
#[case::socks5("socks5", false, true)]
#[case::socks5h("socks5h", false, true)]
#[case::socks5h_named_by_network_resolve("socks5h", true, true)]
/// The browser itself is the relay's client, given its credentials at
/// `Fetch.authRequired`, with no egress relay in front of it.
///
/// Covers: setting:security.raw_cdp_egress_proxy
#[case::socks5_with_the_egress_relay_off("socks5", false, false)]
fn the_credentials_of_a_socks5_proxy_reach_it_through_the_sessions_loopback_relay_alone(
    #[case] scheme: &str,
    #[case] named: bool,
    #[case] egress_relay: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let credentials = proving_ground
        .description()
        .fixtures
        .proxy_credentials
        .clone();
    // The proxy's password, which the engine reads from its environment
    // through the `env:` Secret provider.
    let mut canaries = Canaries::new(env.seed());
    canaries.add(Canary::known(
        "proxy-password",
        SecretKind::ProxyPassword,
        &credentials.password,
    ));
    let mut proxy = proxy_settings(
        &proving_ground,
        scheme,
        "socks5",
        Some(&credentials.username),
    );
    if named {
        // The proxy's host by a name the Run's `network.resolve` maps.
        let address = proxy_address(&proving_ground, "socks5");
        let (ip, port) = address.rsplit_once(':').unwrap();
        let name = proving_ground
            .description()
            .resolve
            .iter()
            .find_map(|(host, mapped)| (mapped == ip).then_some(host))
            .unwrap();
        proxy["server"] = json!(format!("{scheme}://{name}:{port}"));
    }
    let mut settings = serde_json::Map::new();
    settings.insert("proxy".to_owned(), proxy);
    if !egress_relay {
        settings.insert(
            "security".to_owned(),
            json!({ "raw_cdp_egress_proxy": false }),
        );
    }
    let mut engine = crate::page::started_with(
        &env,
        &cache,
        &proving_ground,
        "relayed",
        &settings,
        |command| {
            command.env(PASSWORD_VARIABLE, &credentials.password);
        },
    );

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "relayed" }),
    ));
    let navigated = call(
        &mut engine,
        3,
        "page.goto",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/storage?mark=relayed"),
        }),
    );
    let arguments = crate::page::browser_command_lines(&env);
    let _ = call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "relayed" }),
    );
    let (_, stderr) = engine.finish_with_stderr();

    let port = proving_ground
        .url("scope.test", "")
        .rsplit(':')
        .next()
        .unwrap()
        .to_owned();
    let relays: Vec<Value> = proving_ground.observations().unwrap()["relays"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|relay| relay["proxy"] == json!("socks5"))
        .map(|relay| json!([relay["target"], relay["username"], relay["outcome"]]))
        .collect();
    assert_eq!(
        (
            &navigated["result"]["response"]["status"],
            relays.first(),
            arguments.is_empty(),
            leaks_of(&canaries, env.home(), &stderr, &arguments),
        ),
        (
            &json!(200),
            Some(&json!([
                format!("scope.test:{port}"),
                credentials.username,
                "relayed"
            ])),
            false,
            Vec::new(),
        ),
        "{navigated}"
    );
}

/// A `list` server: the Proving ground's proxy `name`, declared of `kind`,
/// its exit in `country` if given.
struct Listed<'a> {
    name: &'a str,
    kind: &'a str,
    country: Option<&'a str>,
}

/// User configuration declaring the `list` provider of `servers`, with the
/// Proving ground's proxy credentials and `price` per GB if given.
fn list_configuration(
    proving_ground: &ProvingGround,
    servers: &[Listed<'_>],
    price: Option<&str>,
) -> String {
    let credentials = &proving_ground.description().fixtures.proxy_credentials;
    let servers: Vec<String> = servers
        .iter()
        .map(|listed| {
            let country = listed
                .country
                .map(|country| format!(", country = \"{country}\""))
                .unwrap_or_default();
            format!(
                "{{ server = \"http://{}\", kind = \"{}\"{country} }}",
                proxy_address(proving_ground, listed.name),
                listed.kind
            )
        })
        .collect();
    let price = price
        .map(|price| format!("price = \"{price}\"\n"))
        .unwrap_or_default();
    format!(
        "[settings.proxy.providers.list]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\n{price}servers = [{}]\n",
        credentials.username,
        servers.join(", ")
    )
}

/// The Proving ground's datacenter proxy, then its isp proxy, then its open
/// proxy, declared of the datacenter kind too.
const THREE_SERVERS: [Listed<'static>; 3] = [
    Listed {
        name: "datacenter",
        kind: "datacenter",
        country: Some("US"),
    },
    Listed {
        name: "isp",
        kind: "isp",
        country: None,
    },
    Listed {
        name: "open",
        kind: "datacenter",
        country: Some("DE"),
    },
];

/// Starts an engine whose user configuration is `configuration` and whose
/// environment holds the Proving ground's proxy password, has it answer
/// `before`, then starts a Run of `settings`.
fn started_listing<'env>(
    env: &'env TestEnv,
    cache: &std::path::Path,
    proving_ground: &ProvingGround,
    run_id: &str,
    configuration: &str,
    settings: &Value,
    before: &[(&str, Value)],
) -> Attached<'env> {
    env.user_configuration(configuration);
    let password = proving_ground
        .description()
        .fixtures
        .proxy_credentials
        .password
        .clone();
    let settings = settings.as_object().cloned().unwrap_or_default();
    crate::page::started_after(
        env,
        cache,
        proving_ground,
        run_id,
        &settings,
        |command| {
            command.env(PASSWORD_VARIABLE, &password);
        },
        before,
    )
}

/// Covers: story:8-88, setting:proxy.provider, setting:proxy.providers, setting:proxy.kind, operation:run.new_session
#[cfg(unix)]
#[test]
fn each_session_of_a_list_provider_leaves_from_one_of_its_servers_for_its_life() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let configuration = list_configuration(&proving_ground, &THREE_SERVERS, None);
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "listed",
        &configuration,
        &json!({ "proxy": { "provider": "list" } }),
        &[],
    );
    let url = |mark: &str| proving_ground.url("scope.test", &format!("/storage?mark={mark}"));
    let mut id = 10;
    let mut visit = |engine: &mut Attached, page: &str, mark: &str| {
        id += 1;
        let navigated = call(
            engine,
            id,
            "page.goto",
            &json!({ "page_id": page, "url": url(mark) }),
        );
        assert!(navigated.get("result").is_some(), "{navigated}");
    };

    let default = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "listed" }),
    ));
    let mut sessions = Vec::new();
    for (position, own) in [(3, json!({})), (4, json!({ "proxy": { "kind": "isp" } }))] {
        let session = call(
            &mut engine,
            position,
            "run.new_session",
            &json!({ "run_id": "listed", "settings": own }),
        );
        let session = session["result"]["session_id"].clone();
        let page = call(
            &mut engine,
            position + 10,
            "session.new_page",
            &json!({ "session_id": session }),
        );
        sessions.push(opened(&page).page_id);
    }
    for round in ["first", "second"] {
        visit(&mut engine, &default.page_id.0, &format!("default-{round}"));
        visit(&mut engine, &sessions[0].0, &format!("second-{round}"));
        visit(&mut engine, &sessions[1].0, &format!("third-{round}"));
    }
    let _ = call(
        &mut engine,
        50,
        "run.finish",
        &json!({ "run_id": "listed" }),
    );
    let _ = engine.finish();

    // The proxies each Session's requests left from, and the rounds that
    // reached the site through them.
    let proxies_of = |marker: &str| -> (BTreeSet<String>, BTreeSet<String>) {
        relays_to(&proving_ground, marker)
            .into_iter()
            .map(|(proxy, target)| {
                let round = if target.contains("first") {
                    "first"
                } else {
                    "second"
                };
                (proxy, round.to_owned())
            })
            .unzip()
    };
    let rounds = BTreeSet::from(["first".to_owned(), "second".to_owned()]);
    let only = |proxy: &str| BTreeSet::from([proxy.to_owned()]);
    assert_eq!(
        (
            proxies_of("mark=default-"),
            proxies_of("mark=second-"),
            proxies_of("mark=third-")
        ),
        (
            (only("datacenter"), rounds.clone()),
            (only("open"), rounds.clone()),
            (only("isp"), rounds)
        )
    );
}

/// Covers: story:8-106, error:setting_invalid, setting:proxy.provider, setting:proxy.kind, setting:proxy.country, setting:proxy.server
#[rstest]
#[case::a_kind_the_list_lacks(json!({ "provider": "list" }), json!({ "proxy": { "kind": "mobile" } }), "proxy.kind")]
#[case::a_country_the_kind_lacks(json!({ "provider": "list", "kind": "isp" }), json!({ "proxy": { "country": "FR" } }), "proxy.country")]
#[case::a_server_beside_the_provider(json!({ "server": "http://127.0.0.1:3128" }), json!({ "proxy": { "provider": "list" } }), "proxy.provider")]
#[case::an_unknown_provider(json!({ "provider": "nowhere" }), json!({}), "proxy.provider")]
fn a_session_the_provider_cannot_serve_is_refused_with_setting_invalid(
    #[case] runs: Value,
    #[case] own: Value,
    #[case] key: &str,
) {
    let env = TestEnv::new();
    env.user_configuration(
        "[settings.proxy.providers.list]\nservers = [{ server = \"http://127.0.0.1:3001\", kind = \"datacenter\" }, { server = \"socks5://127.0.0.1:3002\", kind = \"isp\", country = \"US\" }]\n",
    );
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();

    let started = call(
        &mut engine,
        1,
        "run.start",
        &json!({ "run_id": "refused", "settings": { "proxy": runs } }),
    );
    let opened = call(
        &mut engine,
        2,
        "run.new_session",
        &json!({ "run_id": "refused", "settings": own }),
    );
    let _ = call(
        &mut engine,
        3,
        "run.finish",
        &json!({ "run_id": "refused" }),
    );
    let _ = engine.finish();

    let data = &opened["error"]["data"];
    assert_eq!(
        (started.get("result").is_some(), &data["code"], &data["key"]),
        (true, &json!("setting_invalid"), &json!(key)),
        "{opened}"
    );
}

/// What the Proving ground's proxies of `kind` carried, in bytes, on every
/// relay.
fn carried_by(proving_ground: &ProvingGround, kind: &str) -> u64 {
    proving_ground.observations().unwrap()["relays"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|relay| relay["kind"] == json!(kind))
        .map(|relay| {
            relay["bytes"]["received"].as_u64().unwrap() + relay["bytes"]["sent"].as_u64().unwrap()
        })
        .sum()
}

/// The requests a page of `scope.test` makes after its document's that no
/// load waits for: the `/favicon.ico` Chrome asks for once the page loaded,
/// since the page names no icon, and the engine's own read of the site's
/// `/robots.txt`, which starts once the document's response arrived and
/// which nothing the page does waits for. Either may come last.
const AFTER_THE_LOAD: [&str; 2] = ["/favicon.ico", "/robots.txt"];

/// Whether the Proving ground's proxies relayed each of a page of
/// `scope.test`'s requests [`AFTER_THE_LOAD`] and ended every relay: a
/// relay's bytes are recorded when it ends, and each received at least the
/// request it is for.
fn page_load_relayed(observations: &Value) -> bool {
    let relays = observations["relays"].as_array().unwrap();
    let paths: BTreeSet<String> = relays
        .iter()
        .filter_map(|relay| relay["target"].as_str())
        .filter_map(|target| url::Url::parse(target).ok())
        .map(|target| target.path().to_owned())
        .collect();
    AFTER_THE_LOAD.iter().all(|path| paths.contains(*path))
        && relays
            .iter()
            .all(|relay| relay["bytes"]["received"].as_u64() > Some(0))
}

/// Waits until [`page_load_relayed`], a minute at most, so no byte of the
/// page's load, nor of the engine's robots.txt read beside it, is under
/// way when the Run ends.
fn page_load_settled(proving_ground: &ProvingGround) {
    for _ in 0..600 {
        if page_load_relayed(&proving_ground.observations().unwrap()) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!(
        "the page's load was still under way after a minute: {}",
        proving_ground.observations().unwrap()["relays"]
    );
}

/// A relay of `target` on `scope.test` that the Proving ground's isp proxy
/// recorded: ended, with the bytes it carried, or still under way, with
/// none recorded yet.
fn relay_of(target: &str, ended: bool) -> Value {
    let bytes = if ended {
        json!({ "received": 500, "sent": 200 })
    } else {
        json!({ "received": 0, "sent": 0 })
    };
    json!({ "kind": "isp", "target": format!("http://scope.test:8080{target}"), "bytes": bytes })
}

#[rstest]
// The order a failing run of the counted test met (#1788): Chrome's icon
// relayed before the engine's own read of the site's robots.txt reached
// the proxy, so the Run ended under it.
#[case::before_the_robots_txt_read_reached_the_proxy(&[("/storage?mark=counted", true), ("/storage/found?mark=none", true), ("/favicon.ico", true)], false)]
#[case::while_the_robots_txt_read_is_under_way(&[("/storage?mark=counted", true), ("/storage/found?mark=none", true), ("/favicon.ico", true), ("/robots.txt", false)], false)]
#[case::before_the_icon_reached_the_proxy(&[("/storage?mark=counted", true), ("/storage/found?mark=none", true), ("/robots.txt", true)], false)]
#[case::once_the_icon_and_the_robots_txt_read_ended(&[("/storage?mark=counted", true), ("/storage/found?mark=none", true), ("/robots.txt", true), ("/favicon.ico", true)], true)]
fn a_page_load_is_relayed_once_its_icon_and_the_engines_robots_txt_read_ended(
    #[case] relays: &[(&str, bool)],
    #[case] relayed: bool,
) {
    let observations = json!({
        "relays": relays.iter().map(|(target, ended)| relay_of(target, *ended)).collect::<Vec<_>>()
    });

    assert_eq!(page_load_relayed(&observations), relayed, "{observations}");
}

/// Covers: story:8-92, setting:proxy.providers, operation:run.usage
#[cfg(unix)]
#[test]
fn a_sessions_proxy_bytes_count_by_proxy_kind_at_the_providers_price() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    // A millionth of a dollar a byte, so the price is the bytes' count.
    let configuration = list_configuration(&proving_ground, &THREE_SERVERS, Some("1000 USD"));
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "counted",
        &configuration,
        &json!({ "proxy": { "provider": "list", "kind": "isp" } }),
        &[],
    );

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "counted" }),
    ));
    let navigated = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("scope.test", "/storage?mark=counted") }),
    );
    page_load_settled(&proving_ground);
    let finished = call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "counted" }),
    );
    let used = call(&mut engine, 5, "run.usage", &json!({ "run_id": "counted" }));
    let _ = engine.finish();

    let carried = carried_by(&proving_ground, "isp");
    let usage = &used["result"];
    assert_eq!(
        (
            navigated.get("result").is_some(),
            carried > 0,
            &usage["proxy_bytes"],
            &usage["proxy_bytes_by_kind"],
            &usage["proxy_price"],
            &usage["uncapped"],
            &finished["result"]["usage"]["proxy_bytes_by_kind"],
        ),
        (
            true,
            true,
            &json!(carried),
            &json!([{ "kind": "isp", "bytes": carried }]),
            &json!([{ "currency": "USD", "amount": carried }]),
            &json!(true),
            &json!([{ "kind": "isp", "bytes": carried }]),
        ),
        "{used}"
    );
}

/// What the sink proxy saw, as it saw it.
enum Sunk {
    /// It accepted a connection from this address.
    Opened(std::net::SocketAddr),
    /// It read this many bytes on one.
    Read(u64),
    /// One of its connections reached its end.
    Ended,
}

/// A proxy stand-in on loopback, on threads of its own, that reads every
/// byte each connection sends it to the connection's end and answers
/// nothing: its address, and what it sees.
#[expect(
    clippy::disallowed_types,
    reason = "the sink proxy is a loopback listener on the test's own threads, never an async worker"
)]
fn sink() -> (std::net::SocketAddr, mpsc::Receiver<Sunk>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (seen, sunk) = mpsc::channel();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().map_while(Result::ok) {
            let _ = seen.send(Sunk::Opened(stream.peer_addr().unwrap()));
            let seen = seen.clone();
            std::thread::spawn(move || {
                let mut buffer = vec![0; 64 * 1024];
                while let Ok(read @ 1..) = std::io::Read::read(&mut stream, &mut buffer) {
                    let _ = seen.send(Sunk::Read(read as u64));
                }
                let _ = seen.send(Sunk::Ended);
            });
        }
    });
    (address, sunk)
}

/// Every byte the sink proxy at `address` read, once each of its
/// connections ended: a connection of the test's own, opened last, closes
/// at once, so every earlier one was accepted by the time it is.
#[expect(
    clippy::disallowed_types,
    reason = "the test's last connection to the sink proxy is a loopback socket on the test's own thread"
)]
fn sunk_once_ended(
    address: std::net::SocketAddr,
    sunk: &mpsc::Receiver<Sunk>,
    mut read: u64,
) -> u64 {
    let last = std::net::TcpStream::connect(address).unwrap();
    let last_address = last.local_addr().unwrap();
    drop(last);
    let (mut opened, mut ended, mut seen_last) = (0, 0, false);
    while !(seen_last && opened == ended) {
        match sunk.recv_timeout(BROWSER_DEADLINE).unwrap() {
            Sunk::Opened(peer) => {
                opened += 1;
                seen_last |= peer == last_address;
            }
            Sunk::Read(bytes) => read += bytes,
            Sunk::Ended => ended += 1,
        }
    }
    read
}

/// The bytes a page uploads through its proxy: far more than it carries
/// before the Run ends.
const UPLOAD: u64 = 8 * 1024 * 1024;

/// What the sink proxy must have read before the Run ends: the upload is
/// under way.
const UNDER_WAY: u64 = 1024 * 1024;

/// Covers: story:8-92, setting:proxy.providers, operation:run.usage
#[cfg(unix)]
#[test]
fn a_run_ending_mid_upload_counts_every_byte_its_proxy_took() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let (address, sunk) = sink();
    let configuration = format!(
        "[settings.proxy.providers.list]\nservers = [{{ server = \"http://{address}\", kind = \"isp\" }}]\n"
    );
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "uploading",
        &configuration,
        &json!({ "proxy": { "provider": "list", "kind": "isp" } }),
        &[],
    );

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "uploading" }),
    ));
    // The sink never answers, so the request is still sending when the Run
    // ends, which answers it.
    let zeros = "A".repeat(usize::try_from(UPLOAD / 3 * 4).unwrap());
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 3, "method": "page.request",
        "params": {
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/upload"),
            "method": "POST",
            "post_data": zeros,
        },
    }));
    let mut read = 0;
    while read < UNDER_WAY {
        match sunk.recv_timeout(BROWSER_DEADLINE).unwrap() {
            Sunk::Read(bytes) => read += bytes,
            Sunk::Opened(_) | Sunk::Ended => {}
        }
    }
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 4, "method": "run.finish", "params": { "run_id": "uploading" },
    }));
    let finished = std::iter::from_fn(|| engine.next())
        .find(|answer| answer["id"] == json!(4))
        .unwrap();
    let used = call(
        &mut engine,
        5,
        "run.usage",
        &json!({ "run_id": "uploading" }),
    );
    let _ = engine.finish();
    let read = sunk_once_ended(address, &sunk, read);

    assert_eq!(
        (
            finished.get("result").is_some(),
            read < UPLOAD,
            &used["result"]["proxy_bytes"],
        ),
        (true, true, &json!(read)),
        "{used}"
    );
}

/// An install Limit of `amount` millionths of a dollar on `proxy_bytes`.
fn a_dollar_limit(amount: u64) -> (&'static str, Value) {
    (
        "usage.set",
        json!({ "scope": "install", "usage_kind": "proxy_bytes", "currency": "USD", "amount": amount }),
    )
}

/// Covers: story:8-92, setting:proxy.providers, error:limit_reached
#[cfg(unix)]
#[test]
fn a_currency_limit_caps_the_bytes_of_a_priced_proxy_and_stops_the_run() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let configuration = list_configuration(&proving_ground, &THREE_SERVERS, Some("1000 USD"));
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "capped",
        &configuration,
        &json!({ "proxy": { "provider": "list" } }),
        &[a_dollar_limit(100)],
    );

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "capped" }),
    ));
    let navigated = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("scope.test", "/storage?mark=capped") }),
    );
    let finished = call(&mut engine, 4, "run.finish", &json!({ "run_id": "capped" }));
    let _ = engine.finish();

    let report = &finished["result"];
    let price = report["usage"]["proxy_price"][0]["amount"]
        .as_u64()
        .unwrap_or(0);
    assert_eq!(
        (
            &navigated["error"]["data"]["code"],
            &navigated["error"]["data"]["usage_kind"],
            &report["stop_reason"]["kind"],
            &report["stop_reason"]["usage_kind"],
            &report["stop_reason"]["currency"],
            price <= 100,
        ),
        (
            &json!("limit_reached"),
            &json!("proxy_bytes"),
            &json!("limit"),
            &json!("proxy_bytes"),
            &json!("USD"),
            true
        ),
        "{navigated} {finished}"
    );
}

/// The kinds, Runs and actions of the items `needs-you list --json` prints
/// in `env`'s home.
fn needs_you(env: &TestEnv) -> Vec<(Value, Value, Value)> {
    let listed = cli(env, COLLIE)
        .args(["needs-you", "list", "--json"])
        .run()
        .unwrap();
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap_or_else(|_| panic!("{listed}"));
    listed["items"]
        .as_array()
        .unwrap_or_else(|| panic!("{listed}"))
        .iter()
        .map(|item| {
            (
                item["kind"].clone(),
                item["run"].clone(),
                item["actions"].clone(),
            )
        })
        .collect()
}

/// A Run a currency Limit stopped is listed in "Needs you", offering a
/// person's `usage.raise`, and leaves it once a person raises the Limit at
/// the interactive prompt (ADRs 0031 and 0041), the Run then waiting for
/// its script.
///
/// Covers: story:12-100, command:needs-you.list, command:usage.raise
#[cfg(unix)]
#[test]
fn a_run_a_limit_stopped_waits_in_needs_you_until_a_person_raises_the_limit() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let configuration = list_configuration(&proving_ground, &THREE_SERVERS, Some("1000 USD"));
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "capped",
        &configuration,
        &json!({ "proxy": { "provider": "list" } }),
        &[a_dollar_limit(100)],
    );
    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "capped" }),
    ));
    call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("scope.test", "/storage?mark=capped") }),
    );
    let finished = call(&mut engine, 4, "run.finish", &json!({ "run_id": "capped" }));
    let _ = engine.finish();
    assert_eq!(
        finished["result"]["stop_reason"]["kind"],
        json!("limit"),
        "{finished}"
    );

    let stopped = needs_you(&env);
    let mut raise = pty(&env, COLLIE)
        .args([
            "usage",
            "raise",
            "--scope",
            "install",
            "--usage-kind",
            "proxy_bytes",
            "--currency",
            "USD",
            "--amount",
            "1000000",
        ])
        .spawn()
        .unwrap();
    raise.expect("Raise the install Limit").unwrap();
    raise.send_line("y").unwrap();
    let raised = raise.finish().unwrap();

    assert_eq!(
        (stopped, raised.exit, needs_you(&env)),
        (
            vec![(
                json!("run_stopped_by_limit"),
                json!("capped"),
                json!(["usage.raise", "needs_you.dismiss"]),
            )],
            Some(0),
            Vec::new(),
        ),
        "{}",
        raised.stripped()
    );
}

/// Covers: story:8-92, setting:proxy.providers, error:setting_invalid
#[test]
fn an_unpriced_provider_under_a_currency_limit_is_refused_naming_its_price() {
    let env = TestEnv::new();
    env.user_configuration(
        "[settings.proxy.providers.list]\nservers = [{ server = \"http://127.0.0.1:3001\", kind = \"datacenter\" }]\n",
    );
    let mut engine = Attached::start(&env);
    engine.send(&hello("collie", "0.1"));
    let _ = engine.next();
    let (method, limit) = a_dollar_limit(1_000_000);

    let limited = call(&mut engine, 1, method, &limit);
    let _ = call(
        &mut engine,
        2,
        "run.start",
        &json!({ "run_id": "unpriced", "settings": { "proxy": { "provider": "list" } } }),
    );
    let opened = call(
        &mut engine,
        3,
        "run.new_session",
        &json!({ "run_id": "unpriced" }),
    );
    let _ = call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "unpriced" }),
    );
    let _ = engine.finish();

    let data = &opened["error"]["data"];
    assert_eq!(
        (
            limited.get("result").is_some(),
            &data["code"],
            &data["key"],
            data["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("proxy.providers.list.price")),
        ),
        (
            true,
            &json!("setting_invalid"),
            &json!("proxy.providers"),
            true
        ),
        "{opened}"
    );
}

/// The engine's pages of the browser-fetch tier send their one request
/// each and nothing more (ADRs 0020 and 0121): the robots.txt read of a
/// document's origin and a `page.request` each reach the site once,
/// through the Session's proxy, and Chromium asks neither site for
/// `/favicon.ico` for the text document the engine's page loads, a
/// request outside the page's interception that Politeness and Usage
/// would never count. The `page.request` is sent while the robots.txt
/// read is under way, as when an engine's page the browser showed let
/// its icon's request out (4 runs of 5). The page the program opened names its
/// icon inline, so it asks for none either; the Run's end closes the
/// browser, after which no request can come.
///
/// Covers: story:5-88, operation:page.request, setting:proxy.server
#[test]
fn the_robots_txt_read_and_page_request_each_send_one_request_through_the_sessions_proxy() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let proxy = proxy_settings(&proving_ground, "http", "open", None);
    let run_id = "engine-pages";
    let mut engine = started_through(&env, &cache, &proving_ground, run_id, proxy, "");
    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": run_id }),
    ));
    let document = proving_ground.url("scope.test", "/quiet?mark=read");
    let requested = proving_ground.url("1.incidents.test", "/robots.txt");

    let went = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": document }),
    );
    let sent = call(
        &mut engine,
        4,
        "page.request",
        &json!({ "page_id": page.page_id, "url": requested }),
    );
    robots_txt_read(&env, run_id);
    let _ = call(&mut engine, 5, "run.finish", &json!({ "run_id": run_id }));
    let _ = engine.finish();
    let observations = proving_ground.observations().unwrap();
    let relays: Vec<(Value, Value)> = observations["relays"]
        .as_array()
        .unwrap()
        .iter()
        .map(|relay| (relay["proxy"].clone(), relay["target"].clone()))
        .collect();
    let received: Vec<(Value, Value)> = ReceivedRequests::of(&observations)
        .every()
        .map(|request| (request["host"].clone(), request["target"].clone()))
        .collect();

    assert_eq!(
        (
            went["result"]["response"]["status"].clone(),
            sent["result"]["status"].clone(),
            relays,
            received,
        ),
        (
            json!(200),
            json!(404),
            vec![
                (json!("open"), json!(document)),
                (
                    json!("open"),
                    json!(proving_ground.url("scope.test", "/robots.txt"))
                ),
                (json!("open"), json!(requested)),
            ],
            vec![
                (json!("scope.test"), json!("/quiet?mark=read")),
                (json!("scope.test"), json!("/robots.txt")),
                (json!("1.incidents.test"), json!("/robots.txt")),
            ],
        )
    );
}

/// User configuration declaring the adapter `name` on the Proving ground's
/// proxy account, its `endpoint` the Proving ground's residential proxy,
/// which reads every provider's sticky username.
pub(crate) fn adapter_configuration(proving_ground: &ProvingGround, name: &str) -> String {
    let credentials = &proving_ground.description().fixtures.proxy_credentials;
    format!(
        "[settings.proxy.providers.{name}]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\nendpoint = \"http://{}\"\n",
        credentials.username,
        proxy_address(proving_ground, "residential"),
    )
}

/// Opens a Session of `run_id` with `own` settings and a page in it, as
/// the `position`-th call: the Session's and the page's IDs.
fn session_page(engine: &mut Attached, position: i64, run_id: &str, own: &Value) -> (Value, Value) {
    let session = call(
        engine,
        position,
        "run.new_session",
        &json!({ "run_id": run_id, "settings": own }),
    );
    let session = session["result"]["session_id"].clone();
    assert!(!session.is_null(), "{session}");
    let page = call(
        engine,
        position + 1,
        "session.new_page",
        &json!({ "session_id": session }),
    );
    (session, json!(opened(&page).page_id.0))
}

/// Visits `/storage?mark=<mark>` of `scope.test` in `page`, as the
/// `position`-th call.
pub(crate) fn visit_marked(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    position: i64,
    page: &Value,
    mark: &str,
) {
    let url = proving_ground.url("scope.test", &format!("/storage?mark={mark}"));
    let went = call(
        engine,
        position,
        "page.goto",
        &json!({ "page_id": page, "url": url }),
    );
    assert!(went.get("result").is_some(), "{went}");
}

/// The relayed requests whose target holds `marker`: each one's sticky
/// session as the proxy read it, and the exit it left from.
pub(crate) fn sticky_relays(proving_ground: &ProvingGround, marker: &str) -> Vec<(Value, Value)> {
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
        .map(|relay| (relay["sticky"].clone(), relay["exit"].clone()))
        .collect()
}

/// Covers: story:8-89, story:8-90, setting:proxy.provider, setting:proxy.providers, setting:proxy.country, operation:run.new_session
#[cfg(unix)]
#[rstest]
#[case::bright_data("bright_data", "us")]
#[case::oxylabs("oxylabs", "US")]
#[case::apify("apify", "US")]
fn each_adapters_session_leaves_from_one_sticky_exit_named_in_its_providers_username(
    #[case] name: &str,
    #[case] country_as_read: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let configuration = adapter_configuration(&proving_ground, name);
    let run_id = format!("through-{name}");
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        &run_id,
        &configuration,
        &json!({ "proxy": { "provider": name, "kind": "residential", "country": "US" } }),
        &[],
    );

    let (_, first) = session_page(&mut engine, 2, &run_id, &json!({}));
    let (_, second) = session_page(&mut engine, 4, &run_id, &json!({}));
    let mut position = 10;
    for round in ["one", "two"] {
        for (page, session) in [(&first, "first"), (&second, "second")] {
            position += 1;
            visit_marked(
                &mut engine,
                &proving_ground,
                position,
                page,
                &format!("{session}-{round}"),
            );
        }
    }
    let _ = call(&mut engine, 50, "run.finish", &json!({ "run_id": run_id }));
    let _ = engine.finish();

    // Whether each Session's two visits were relayed (a page may ask for
    // more), its sticky sessions and exits, and what its username named.
    let of = |session: &str| {
        let relays = sticky_relays(&proving_ground, &format!("mark={session}-"));
        let stickies: BTreeSet<String> = relays
            .iter()
            .map(|(sticky, _)| sticky["session"].to_string())
            .collect();
        let exits: BTreeSet<String> = relays.iter().map(|(_, exit)| exit.to_string()).collect();
        let named: BTreeSet<String> = relays
            .iter()
            .map(|(sticky, _)| format!("{} {}", sticky["provider"], sticky["country"]))
            .collect();
        (relays.len() >= 2, stickies, exits.len(), named)
    };
    let (first, second) = (of("first"), of("second"));
    let named = BTreeSet::from([format!("{} {}", json!(name), json!(country_as_read))]);
    assert_eq!(
        (
            (first.0, first.1.len(), first.2, &first.3),
            (second.0, second.1.len(), second.2, &second.3),
            first.1.is_disjoint(&second.1),
        ),
        ((true, 1, 1, &named), (true, 1, 1, &named), true),
        "{first:?} {second:?}"
    );
}

/// Covers: story:8-90, setting:proxy.sticky_lifetime, operation:session.close, operation:run.finish
#[cfg(unix)]
#[test]
fn a_session_that_outlives_its_sticky_lifetime_is_named_by_proxy_exit_may_change() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let configuration = adapter_configuration(&proving_ground, "oxylabs");
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "outlived",
        &configuration,
        &json!({ "proxy": { "provider": "oxylabs" } }),
        &[],
    );

    // The first Session asks its exit for a millisecond, which its page's
    // visit outlives; the second keeps the provider's longest, 24 hours.
    let (short, short_page) = session_page(
        &mut engine,
        2,
        "outlived",
        &json!({ "proxy": { "sticky_lifetime": 1 } }),
    );
    let (long, long_page) = session_page(&mut engine, 4, "outlived", &json!({}));
    visit_marked(&mut engine, &proving_ground, 6, &short_page, "short");
    visit_marked(&mut engine, &proving_ground, 7, &long_page, "long");
    for (position, session) in [(8, &short), (9, &long)] {
        let closed = call(
            &mut engine,
            position,
            "session.close",
            &json!({ "session_id": session }),
        );
        assert!(closed.get("result").is_some(), "{closed}");
    }
    let finished = call(
        &mut engine,
        10,
        "run.finish",
        &json!({ "run_id": "outlived" }),
    );
    let _ = engine.finish();

    let warned: Vec<Value> = finished["result"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|warning| warning["code"] == json!("proxy_exit_may_change"))
        .map(|warning| warning["sessions"].clone())
        .collect();
    let lifetimes: Vec<Value> = ["short", "long"]
        .iter()
        .flat_map(|mark| sticky_relays(&proving_ground, &format!("mark={mark}")))
        .map(|(sticky, _)| sticky["lifetime_minutes"].clone())
        .collect();
    assert_eq!(
        (warned, lifetimes),
        (vec![json!([short])], vec![json!(1), json!(1440)]),
        "{finished}"
    );
}

/// The Sessions a Run's end closes, its default Session and one the program
/// left open, are named by `proxy_exit_may_change` once they outlived their
/// sticky lifetime, though no page opened or Session closed after it: the
/// check runs as the Run stops.
///
/// Covers: story:8-90, setting:proxy.sticky_lifetime, operation:run.finish
#[cfg(unix)]
#[test]
fn sessions_that_outlive_their_sticky_lifetime_and_end_with_their_run_are_named_by_proxy_exit_may_change()
 {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let configuration = adapter_configuration(&proving_ground, "oxylabs");
    let lifetime = std::time::Duration::from_secs(2);
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "ended-with-run",
        &configuration,
        &json!({ "proxy": {
            "provider": "oxylabs",
            "sticky_lifetime": u64::try_from(lifetime.as_millis()).unwrap(),
        } }),
        &[],
    );

    let default = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "ended-with-run" }),
    ));
    let (left_open, left_open_page) = session_page(&mut engine, 3, "ended-with-run", &json!({}));
    visit_marked(
        &mut engine,
        &proving_ground,
        5,
        &json!(default.page_id.0),
        "default",
    );
    visit_marked(
        &mut engine,
        &proving_ground,
        6,
        &left_open_page,
        "left-open",
    );
    // Both Sessions outlive their lifetime before the Run finishes; a
    // navigation is no check (ADR 0021's #891 note).
    std::thread::sleep(lifetime + std::time::Duration::from_millis(500));
    visit_marked(&mut engine, &proving_ground, 7, &left_open_page, "again");
    let finished = call(
        &mut engine,
        10,
        "run.finish",
        &json!({ "run_id": "ended-with-run" }),
    );
    let _ = engine.finish();

    let warned: BTreeSet<String> = finished["result"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|warning| warning["code"] == json!("proxy_exit_may_change"))
        .flat_map(|warning| warning["sessions"].as_array().cloned().unwrap_or_default())
        .filter_map(|session| session.as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        warned,
        BTreeSet::from([
            default.session_id.0,
            left_open.as_str().unwrap_or_default().to_owned()
        ]),
        "{finished}"
    );
}

/// Two Sessions of one hosted provider's account each build their own
/// sticky username, yet hold the account's one limit, under the provider
/// and the configured username (ADR 0088; the deep review's 599-4).
///
/// Covers: story:5-90, setting:proxy.provider, setting:proxy.providers
#[cfg(unix)]
#[test]
fn the_sessions_of_one_providers_account_share_its_proxy_limit() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let configuration = adapter_configuration(&proving_ground, "oxylabs");
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "one-account",
        &configuration,
        &json!({ "proxy": { "provider": "oxylabs" } }),
        &[],
    );

    let (_, first) = session_page(&mut engine, 2, "one-account", &json!({}));
    let (_, second) = session_page(&mut engine, 4, "one-account", &json!({}));
    visit_marked(&mut engine, &proving_ground, 6, &first, "first");
    visit_marked(&mut engine, &proving_ground, 7, &second, "second");
    let _ = call(
        &mut engine,
        8,
        "run.finish",
        &json!({ "run_id": "one-account" }),
    );
    let _ = engine.finish();

    let stickies: BTreeSet<String> = sticky_relays(&proving_ground, "mark=")
        .iter()
        .map(|(sticky, _)| sticky["session"].to_string())
        .collect();
    let username = &proving_ground
        .description()
        .fixtures
        .proxy_credentials
        .username;
    assert_eq!(
        (
            stickies.len(),
            crate::proxy_limits::ceilings(&env.home().join("politeness.sqlite")),
        ),
        (
            2,
            BTreeSet::from([(format!("provider oxylabs {username}"), None)]),
        )
    );
}

/// A hosted provider that refuses the account's credentials raises
/// `proxy_auth_failed` naming the provider's own keys, never `proxy.username`
/// and `proxy.password`, which are refused beside `proxy.provider` (the
/// deep review's 599-6).
///
/// Covers: story:8-89, setting:proxy.providers, error:proxy_auth_failed
#[cfg(unix)]
#[test]
fn a_provider_refusing_the_accounts_credentials_names_its_own_keys() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    env.user_configuration(&adapter_configuration(&proving_ground, "oxylabs"));
    let settings = json!({ "proxy": { "provider": "oxylabs" } });
    let mut engine = crate::page::started_after(
        &env,
        &cache,
        &proving_ground,
        "refused",
        settings.as_object().unwrap(),
        |command| {
            command.env(PASSWORD_VARIABLE, "not-the-accounts-password");
        },
        &[],
    );

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "refused" }),
    ));
    let navigated = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("scope.test", "/storage?mark=refused") }),
    );
    let _ = call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "refused" }),
    );
    let _ = engine.finish();

    let message = navigated["error"]["message"].as_str().unwrap_or_default();
    assert_eq!(
        (
            &navigated["error"]["data"]["code"],
            message.contains("`proxy.providers.oxylabs.username`")
                && message.contains("`proxy.providers.oxylabs.password`"),
            message.contains("`proxy.username`"),
        ),
        (&json!("proxy_auth_failed"), true, false),
        "{navigated}"
    );
}

/// An install Limit on `proxy_bytes` far above what the test carries, so
/// the Run holds Slices of it and the ledger traces every charge.
fn a_bytes_limit() -> (&'static str, Value) {
    (
        "usage.set",
        json!({ "scope": "install", "usage_kind": "proxy_bytes", "amount": 1_000_000_000_000_u64 }),
    )
}

/// The ledger's trace lines, in `stderr`, that charged or reserved
/// `proxy_bytes`: one per charge before, one per allowance (two when
/// priced) since.
fn proxy_bytes_charges(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|line| {
            line.contains("goodfetch::ledger")
                && line.contains("proxy_bytes")
                && (line.contains("reserve") || line.contains("charge"))
        })
        .count()
}

/// A provider Session's upload is charged ahead, in allowances that grow to
/// a MiB, so 8 MiB costs a few dozen ledger writes rather than one per
/// 16 KiB, and its bytes all count; their price, a millionth of a dollar a
/// GB, is rounded up once for the Session, not once per allowance (the deep
/// review's 599-12 and 599-10).
///
/// Covers: story:8-92, setting:proxy.providers, operation:run.usage
#[cfg(unix)]
#[test]
fn a_provider_sessions_upload_is_charged_ahead_in_few_allowances_and_priced_once() {
    let env = TestEnv::new();
    env.log_level(&format!(
        "info,{DISPATCH_TARGET}=trace,{OUTCOME_TARGET}=trace,goodfetch::ledger=trace"
    ));
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let (address, sunk) = sink();
    let configuration = format!(
        "[settings.proxy.providers.list]\nprice = \"0.000001 USD\"\nservers = [{{ server = \"http://{address}\", kind = \"isp\" }}]\n"
    );
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "allowances",
        &configuration,
        &json!({ "proxy": { "provider": "list", "kind": "isp" } }),
        &[a_bytes_limit()],
    );

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "allowances" }),
    ));
    let zeros = "A".repeat(usize::try_from(UPLOAD / 3 * 4).unwrap());
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 3, "method": "page.request",
        "params": {
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/upload"),
            "method": "POST",
            "post_data": zeros,
        },
    }));
    // The sink never answers: the Run's end answers the request once the
    // whole upload reached it.
    let mut read = 0;
    while read < UPLOAD {
        match sunk.recv_timeout(BROWSER_DEADLINE).unwrap() {
            Sunk::Read(bytes) => read += bytes,
            Sunk::Opened(_) | Sunk::Ended => {}
        }
    }
    engine.send(&json!({
        "jsonrpc": "2.0", "id": 4, "method": "run.finish", "params": { "run_id": "allowances" },
    }));
    let finished = std::iter::from_fn(|| engine.next())
        .find(|answer| answer["id"] == json!(4))
        .unwrap();
    let used = call(
        &mut engine,
        5,
        "run.usage",
        &json!({ "run_id": "allowances" }),
    );
    let (_, stderr) = engine.finish_with_stderr();
    let read = sunk_once_ended(address, &sunk, read);

    let charges = proxy_bytes_charges(&stderr);
    assert_eq!(
        (
            finished.get("result").is_some(),
            &used["result"]["proxy_bytes"],
            &used["result"]["proxy_price"],
            charges > 0 && charges < 64,
        ),
        (
            true,
            &json!(read),
            &json!([{ "currency": "USD", "amount": 1 }]),
            true
        ),
        "{charges} ledger charges of proxy_bytes: {used}"
    );
}

/// A `list` provider's SOCKS5 server: the relay charges its handshake, its
/// tunnel and the page's bytes ahead and lets every one pass, so the page
/// loads and `proxy_bytes` counts what the proxy carried.
///
/// Covers: story:8-92, setting:proxy.providers, operation:run.usage
#[cfg(unix)]
#[test]
fn a_socks5_list_servers_handshake_and_bytes_are_charged_ahead_and_all_pass() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let credentials = &proving_ground.description().fixtures.proxy_credentials;
    let configuration = format!(
        "[settings.proxy.providers.list]\nusername = \"{}\"\npassword = \"env:{PASSWORD_VARIABLE}\"\nservers = [{{ server = \"socks5://{}\", kind = \"isp\" }}]\n",
        credentials.username,
        proxy_address(&proving_ground, "socks5"),
    );
    let mut engine = started_listing(
        &env,
        &cache,
        &proving_ground,
        "socks-counted",
        &configuration,
        &json!({ "proxy": { "provider": "list", "kind": "isp" } }),
        &[a_bytes_limit()],
    );

    let page = opened(&call(
        &mut engine,
        2,
        "run.new_page",
        &json!({ "run_id": "socks-counted" }),
    ));
    let navigated = call(
        &mut engine,
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("scope.test", "/storage?mark=socks-counted") }),
    );
    tunnels_settled(&proving_ground);
    let _ = call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "socks-counted" }),
    );
    let used = call(
        &mut engine,
        5,
        "run.usage",
        &json!({ "run_id": "socks-counted" }),
    );
    let _ = engine.finish();

    let carried = carried_by_every_relay(&proving_ground);
    assert_eq!(
        (
            navigated["result"]["response"]["status"].clone(),
            carried > 0,
            &used["result"]["proxy_bytes"],
        ),
        (json!(200), true, &json!(carried)),
        "{navigated} {used}"
    );
}

/// Waits until the site received the page, its robots.txt and the icon
/// Chromium asks for, a minute at most, and the SOCKS5 proxy ended a relay
/// for each: the site answers each with `Connection: close`, so a relay
/// ends once the browser read its whole answer, and no byte is under way
/// when the Run ends. A SOCKS5 tunnel names no path, so the relays alone
/// cannot tell the page's last request, as [`page_load_relayed`] does.
fn tunnels_settled(proving_ground: &ProvingGround) {
    for _ in 0..600 {
        let observations = proving_ground.observations().unwrap();
        let requests = ReceivedRequests::of(&observations);
        let targets: BTreeSet<String> = requests
            .every()
            .filter_map(|request| request["target"].as_str())
            .map(|target| target.split('?').next().unwrap_or_default().to_owned())
            .collect();
        let relays = observations["relays"].as_array().unwrap().len();
        if ["/storage", "/robots.txt", "/favicon.ico"]
            .iter()
            .all(|target| targets.contains(*target))
            && relays == requests.every().count()
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!(
        "the page's tunnels had not all ended after a minute: {}",
        proving_ground.observations().unwrap()
    );
}

/// What the Proving ground's proxies carried, in bytes, on every relay
/// that ended.
fn carried_by_every_relay(proving_ground: &ProvingGround) -> u64 {
    proving_ground.observations().unwrap()["relays"]
        .as_array()
        .unwrap()
        .iter()
        .map(|relay| {
            relay["bytes"]["received"].as_u64().unwrap() + relay["bytes"]["sent"].as_u64().unwrap()
        })
        .sum()
}
