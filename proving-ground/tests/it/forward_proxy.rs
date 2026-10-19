//! The forward proxies `serve` starts (ADR 0059: proxies are tested against
//! a real one, never mocked): plain HTTP relayed in absolute form and
//! `CONNECT` tunnels, open or behind Basic authentication, one that refuses
//! tunnels, one that is down, and one per proxy kind with its own pool of
//! exits, which a provider's sticky username pins per Session. Each relay is
//! recorded in `GET /observations`'s `relays`, and the request it carries to
//! a task's host is observed for that task like any other.
//! Covers: story:2-82

use rstest::rstest;
use serde_json::{Value, json};

use crate::clients;
use crate::proving_ground::{self, ProvingGround};

const SEED: u64 = 311;

/// A plain HTTP request for `url` in absolute form, as a client sends it to
/// a forward proxy, with `authorization` as its `Proxy-Authorization`.
fn absolute(url: &str, authorization: Option<&str>) -> Vec<u8> {
    let host = url
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .expect("an http URL");
    let mut request = format!("GET {url} HTTP/1.1\r\nHost: {host}\r\n");
    if let Some(authorization) = authorization {
        request.push_str("Proxy-Authorization: ");
        request.push_str(authorization);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.into_bytes()
}

/// The fields of a relay a test checks.
fn relay(record: &Value, fields: &[&str]) -> Value {
    fields
        .iter()
        .map(|field| ((*field).to_owned(), record[*field].clone()))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

#[test]
fn a_request_relayed_by_the_open_proxy_is_observed_for_its_task() {
    let proving_ground = ProvingGround::start(SEED);
    let url = format!("http://1.catalog.test:{}/catalog/1", proving_ground.port());

    let exchange = clients::raw(&proving_ground.proxy("open"), &absolute(&url, None));

    let observations = proving_ground.observations();
    assert_eq!(exchange.status(), 200);
    assert_eq!(
        relay(
            &observations["relays"][0],
            &["proxy", "method", "target", "task", "exit", "outcome"]
        ),
        json!({
            "proxy": "open",
            "method": "GET",
            "target": url,
            "task": "catalog/1",
            "exit": "open",
            "outcome": "relayed",
        })
    );
    assert_eq!(observations["requests"][0]["task"], "catalog/1");
    assert_eq!(observations["requests"][0]["target"], "/catalog/1");
}

#[test]
fn a_tunnel_through_the_open_proxy_carries_tls_to_a_tasks_host() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let authority = format!("1.catalog.test:{}", proving_ground.port());

    let tunnel = clients::tunnel_tls(
        &proving_ground.proxy("open"),
        &authority,
        None,
        "1.catalog.test",
        config,
        b"GET /catalog/1 HTTP/1.1\r\nHost: 1.catalog.test\r\nConnection: close\r\n\r\n",
    );

    let observations = proving_ground.observations();
    assert_eq!(tunnel.status, 200);
    assert_eq!(tunnel.exchange.expect("a tunnel").status(), 200);
    assert_eq!(
        relay(
            &observations["relays"][0],
            &["method", "target", "task", "outcome"]
        ),
        json!({ "method": "CONNECT", "target": authority, "task": "catalog/1", "outcome": "relayed" })
    );
    assert_eq!(observations["requests"][0]["task"], "catalog/1");
    assert_eq!(observations["requests"][0]["tls"]["sni"], "1.catalog.test");
}

/// The `Proxy-Authorization` a case sends: none, the right username with a
/// wrong password, or the reported credentials.
#[derive(Clone, Copy)]
enum Credentials {
    None,
    WrongPassword,
    Reported,
}

#[rstest]
#[case::without_credentials(Credentials::None, 407)]
#[case::with_a_wrong_password(Credentials::WrongPassword, 407)]
#[case::with_the_reported_credentials(Credentials::Reported, 200)]
fn an_authenticated_proxy_opens_a_tunnel_only_with_its_credentials(
    #[case] credentials: Credentials,
    #[case] status: u16,
) {
    let proving_ground = ProvingGround::start(SEED);
    let authorization = match credentials {
        Credentials::None => None,
        Credentials::WrongPassword => Some(proving_ground::basic(
            &proving_ground.proxy_username(),
            "wrong",
        )),
        Credentials::Reported => {
            Some(proving_ground.proxy_authorization(&proving_ground.proxy_username()))
        }
    };

    let tunnel = clients::tunnel_tls(
        &proving_ground.proxy("residential"),
        &format!("catalog.test:{}", proving_ground.port()),
        authorization.as_deref(),
        "catalog.test",
        clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]),
        b"GET /catalog/1 HTTP/1.1\r\nHost: catalog.test\r\nConnection: close\r\n\r\n",
    );

    assert_eq!(tunnel.status, status);
}

#[test]
fn an_authenticated_proxy_asks_for_basic_credentials() {
    let proving_ground = ProvingGround::start(SEED);
    let url = format!("http://catalog.test:{}/catalog/1", proving_ground.port());

    let exchange = clients::raw(&proving_ground.proxy("datacenter"), &absolute(&url, None));

    let observations = proving_ground.observations();
    assert_eq!(exchange.status(), 407);
    assert_eq!(
        exchange.header("proxy-authenticate").as_deref(),
        Some("Basic realm=\"proving-ground\"")
    );
    assert_eq!(
        observations["relays"][0]["outcome"],
        "proxy_authentication_required"
    );
}

#[test]
fn the_proxy_that_refuses_tunnels_answers_connect_with_forbidden() {
    let proving_ground = ProvingGround::start(SEED);
    let request = format!(
        "CONNECT catalog.test:{0} HTTP/1.1\r\nHost: catalog.test:{0}\r\n\r\n",
        proving_ground.port()
    );

    let exchange = clients::raw(
        &proving_ground.proxy("refusing_tunnels"),
        request.as_bytes(),
    );

    let observations = proving_ground.observations();
    assert_eq!(exchange.status(), 403);
    assert_eq!(observations["relays"][0]["outcome"], "tunnel_refused");
}

#[test]
fn the_proxy_that_refuses_tunnels_still_relays_plain_http() {
    let proving_ground = ProvingGround::start(SEED);
    let url = format!("http://catalog.test:{}/catalog/1", proving_ground.port());

    let exchange = clients::raw(
        &proving_ground.proxy("refusing_tunnels"),
        &absolute(&url, None),
    );

    assert_eq!(exchange.status(), 200);
}

#[test]
fn the_down_proxy_refuses_every_connection() {
    let proving_ground = ProvingGround::start(SEED);

    let refused = clients::connection_refused(&proving_ground.proxy("down"));

    assert!(refused);
}

#[rstest]
#[case::datacenter("datacenter")]
#[case::isp("isp")]
#[case::residential("residential")]
#[case::mobile("mobile")]
fn each_proxy_kind_relays_through_its_own_exits(#[case] kind: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let url = format!("http://catalog.test:{}/catalog/1", proving_ground.port());
    let authorization = proving_ground.proxy_authorization(&proving_ground.proxy_username());

    let exchange = clients::raw(
        &proving_ground.proxy(kind),
        &absolute(&url, Some(&authorization)),
    );

    let observations = proving_ground.observations();
    assert_eq!(exchange.status(), 200);
    assert_eq!(
        relay(&observations["relays"][0], &["proxy", "kind", "exit"]),
        json!({ "proxy": kind, "kind": kind, "exit": format!("{kind}-1") })
    );
}

/// A provider's sticky username for `session`, built on the reported
/// username as each provider adapter builds it.
fn sticky(provider: &str, username: &str, session: &str) -> String {
    match provider {
        "bright_data" => format!("brd-customer-{username}-zone-residential-session-{session}"),
        "oxylabs" => format!("customer-{username}-cc-US-sessid-{session}-sesstime-10"),
        "apify" => format!("groups-RESIDENTIAL,session-{session},country-US"),
        other => panic!("no provider {other}"),
    }
}

/// The exits of the relays a run of requests went through, one request per
/// username, each on the residential proxy.
fn exits(proving_ground: &ProvingGround, usernames: &[String]) -> Vec<Value> {
    let url = format!("http://catalog.test:{}/catalog/1", proving_ground.port());
    for username in usernames {
        let authorization = proving_ground.proxy_authorization(username);
        let exchange = clients::raw(
            &proving_ground.proxy("residential"),
            &absolute(&url, Some(&authorization)),
        );
        assert_eq!(exchange.status(), 200, "{username}");
    }
    proving_ground.observations()["relays"]
        .as_array()
        .expect("relays")
        .iter()
        .map(|relay| relay["exit"].clone())
        .collect()
}

#[rstest]
#[case::bright_data("bright_data")]
#[case::oxylabs("oxylabs")]
#[case::apify("apify")]
fn a_sticky_session_keeps_its_exit_and_another_session_takes_the_next(#[case] provider: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let username = proving_ground.proxy_username();
    let first = sticky(provider, &username, "first");
    let second = sticky(provider, &username, "second");

    let exits = exits(&proving_ground, &[first.clone(), second, first]);

    assert_eq!(
        exits,
        [
            json!("residential-1"),
            json!("residential-2"),
            json!("residential-1")
        ]
    );
}

#[test]
fn without_a_sticky_session_each_relay_takes_the_next_exit() {
    let proving_ground = ProvingGround::start(SEED);
    let username = proving_ground.proxy_username();

    let exits = exits(&proving_ground, &[username.clone(), username]);

    assert_eq!(exits, [json!("residential-1"), json!("residential-2")]);
}

#[rstest]
#[case::bright_data("bright_data", json!({
    "provider": "bright_data", "session": "first", "country": null, "lifetime_minutes": null,
}))]
#[case::oxylabs("oxylabs", json!({
    "provider": "oxylabs", "session": "first", "country": "US", "lifetime_minutes": 10,
}))]
#[case::apify("apify", json!({
    "provider": "apify", "session": "first", "country": "US", "lifetime_minutes": null,
}))]
fn a_relay_records_the_sticky_session_its_username_asked_for(
    #[case] provider: &str,
    #[case] expected: Value,
) {
    let proving_ground = ProvingGround::start(SEED);
    let username = sticky(provider, &proving_ground.proxy_username(), "first");

    let _ = exits(&proving_ground, std::slice::from_ref(&username));

    let observations = proving_ground.observations();
    assert_eq!(observations["relays"][0]["sticky"], expected);
    assert_eq!(observations["relays"][0]["username"], json!(username));
}

#[rstest]
#[case::unresolved_name("http://nowhere.invalid/", 502, "unresolved")]
#[case::public_address("http://198.51.100.7/", 403, "destination_prohibited")]
fn a_proxy_relays_only_to_names_it_resolves_on_this_host(
    #[case] url: &str,
    #[case] status: u16,
    #[case] outcome: &str,
) {
    let proving_ground = ProvingGround::start(SEED);

    let exchange = clients::raw(&proving_ground.proxy("open"), &absolute(url, None));

    let observations = proving_ground.observations();
    assert_eq!(exchange.status(), status);
    assert_eq!(observations["relays"][0]["outcome"], outcome);
}
