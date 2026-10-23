//! A Session's proxy over stdio (spec #577, ticket #708), through
//! `goodfetch-protocol`'s types: `proxy.server`, `proxy.bypass`,
//! `proxy.username` and `proxy.password` as a Session's settings, one proxy
//! per Session and none per page or request (ADRs 0021 and 0092; ADR 0037 /
//! SA-22), against the Proving ground's forward proxies, whose connection
//! record says which proxy each request went through; and the `list`
//! Proxy provider (spec #599, ticket #882): `proxy.provider`,
//! `proxy.kind`, `proxy.country` and `proxy.providers`, one of the user's
//! servers per Session for its life, and its bytes by proxy kind in Usage,
//! priced under a currency Limit (ADRs 0021, 0041, 0101 and 0129).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the SOCKS5 test reads the Goodfetch home's files, and the Usage test polls the Proving ground, on its own thread, never an async worker"
)]

use std::collections::BTreeSet;

use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached::{Attached, hello};
use crate::page::opened;
use crate::run::call;

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

/// The Proving ground's own seed for these tests.
const SEED: u64 = 708;

/// The variable the engine's environment holds a proxy password in.
const PASSWORD_VARIABLE: &str = "PROVING_GROUND_PROXY_PASSWORD";

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
    let (name, username, password) = match failing {
        Failing::RefusedTunnel => ("refusing_tunnels", None, ""),
        Failing::Down => ("down", None, ""),
        Failing::NoCredentials => ("datacenter", None, ""),
        Failing::WrongPassword => ("datacenter", Some(credentials.username.as_str()), "wrong"),
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
    let _ = call(
        &mut engine,
        4,
        "run.finish",
        &json!({ "run_id": "failing" }),
    );
    let _ = engine.finish();

    let data = &navigated["error"]["data"];
    assert_eq!(
        (&data["code"], &data["retryable"], &data["proxy"]),
        (
            &json!(code),
            &json!(retryable),
            &json!(format!("http://{}", proxy_address(&proving_ground, name)))
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

/// Covers: story:5-89
#[rstest]
/// Covers: operation:session.new_page
#[case::a_pages_own_proxy(
    "session.new_page",
    json!({ "session_id": "per-page.session-1", "proxy": { "server": "http://127.0.0.1:3128" } })
)]
/// Covers: operation:run.new_page
#[case::a_pages_own_settings(
    "run.new_page",
    json!({ "run_id": "per-page", "settings": { "proxy": { "server": "http://127.0.0.1:3128" } } })
)]
/// Covers: operation:page.goto
#[case::a_requests_own_proxy(
    "page.goto",
    json!({ "page_id": "page-1", "url": "http://scope.test/", "proxy": { "server": "http://127.0.0.1:3128" } })
)]
fn a_proxy_per_page_or_per_request_is_refused(#[case] method: &str, #[case] params: Value) {
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
        &refused["error"]["data"]["code"],
        &json!("invalid_params"),
        "{refused}"
    );
}

/// The files under `folder`, at any depth, whose bytes hold `secret`.
fn files_holding(folder: &std::path::Path, secret: &str) -> Vec<std::path::PathBuf> {
    let mut holding = Vec::new();
    let mut folders = vec![folder.to_owned()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if std::fs::read(&path).is_ok_and(|bytes| {
                bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes())
            }) {
                holding.push(path);
            }
        }
    }
    holding
}

/// Covers: story:5-88, setting:proxy.server, setting:proxy.username, setting:proxy.password
#[cfg(unix)]
#[rstest]
#[case::socks5("socks5")]
#[case::socks5h("socks5h")]
fn the_credentials_of_a_socks5_proxy_reach_it_through_the_sessions_loopback_relay_alone(
    #[case] scheme: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let credentials = proving_ground
        .description()
        .fixtures
        .proxy_credentials
        .clone();
    let proxy = proxy_settings(
        &proving_ground,
        scheme,
        "socks5",
        Some(&credentials.username),
    );
    let mut engine = started_through(
        &env,
        &cache,
        &proving_ground,
        "relayed",
        proxy,
        &credentials.password,
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
            arguments
                .iter()
                .any(|line| line.contains(&credentials.password)),
            stderr.contains(&credentials.password),
            files_holding(env.home(), &credentials.password),
        ),
        (
            &json!(200),
            Some(&json!([
                format!("scope.test:{port}"),
                credentials.username,
                "relayed"
            ])),
            false,
            false,
            false,
            Vec::<std::path::PathBuf>::new(),
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

/// Waits until the Proving ground's proxies have relayed `/storage`'s own
/// request for `/storage/found`, which its script sends once loaded, so the
/// page's every byte has gone through before the Run ends; a minute at
/// most.
fn storage_settled(proving_ground: &ProvingGround) {
    for _ in 0..600 {
        if !relays_to(proving_ground, "/storage/found").is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
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
    storage_settled(&proving_ground);
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
