//! A Session's proxy over stdio (spec #577, ticket #708), through
//! `goodfetch-protocol`'s types: `proxy.server`, `proxy.bypass`,
//! `proxy.username` and `proxy.password` as a Session's settings, one proxy
//! per Session and none per page or request (ADRs 0021 and 0092; ADR 0037 /
//! SA-22), against the Proving ground's forward proxies, whose connection
//! record says which proxy each request went through.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

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

/// Covers: story:5-88, error:proxy_unavailable, error:proxy_auth_failed, setting:proxy.server, setting:proxy.username, setting:proxy.password
#[cfg(unix)]
#[rstest]
#[case::a_refused_tunnel(Failing::RefusedTunnel, "proxy_unavailable", true)]
#[case::a_proxy_that_is_down(Failing::Down, "proxy_unavailable", true)]
#[case::no_credentials(Failing::NoCredentials, "proxy_auth_failed", false)]
#[case::a_wrong_password(Failing::WrongPassword, "proxy_auth_failed", false)]
fn a_failing_proxy_is_the_proxys_failure_never_the_navigations(
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
            let marked = &target[target.find(marker).unwrap()..];
            (
                relay["proxy"].as_str().unwrap().to_owned(),
                marked.to_owned(),
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

/// Covers: story:5-89, operation:session.new_page, operation:run.new_page, operation:page.goto
#[rstest]
#[case::a_pages_own_proxy(
    "session.new_page",
    json!({ "session_id": "per-page.session-1", "proxy": { "server": "http://127.0.0.1:3128" } })
)]
#[case::a_pages_own_settings(
    "run.new_page",
    json!({ "run_id": "per-page", "settings": { "proxy": { "server": "http://127.0.0.1:3128" } } })
)]
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
fn a_socks5_proxys_credentials_reach_it_through_the_sessions_loopback_relay_and_nowhere_else(
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
