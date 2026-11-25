//! The forward proxies `serve` starts (ADR 0059: proxies are tested against
//! a real one, never mocked): plain HTTP relayed in absolute form and
//! `CONNECT` tunnels, open or behind Basic authentication, one that refuses
//! tunnels, one that is down, and one per proxy kind with its own pool of
//! exits, which a provider's sticky username pins per Session, and a SOCKS5
//! proxy behind the same credentials. Each relay is
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

/// The `https://` proxy speaks TLS first, under the site's authority's leaf
/// for its name, then relays as the open one does; a client that does not
/// trust the authority is relayed nowhere, its connection still recorded.
#[test]
fn the_https_proxy_relays_inside_tls_under_the_sites_authority() {
    let proving_ground = ProvingGround::start(SEED);
    let url = format!("http://1.catalog.test:{}/catalog/1", proving_ground.port());
    let trusting = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let other = ProvingGround::start(SEED + 1);
    let distrusting = clients::trusting(&other.certificate_authority(), &["http/1.1"]);

    let exchange = clients::raw_tls(
        &proving_ground.proxy("open_tls"),
        "tls-proxy.test",
        trusting,
        &absolute(&url, None),
    )
    .expect("a TLS connection to the proxy");
    let refused = clients::raw_tls(
        &proving_ground.proxy("open_tls"),
        "tls-proxy.test",
        distrusting,
        &absolute(&url, None),
    );

    let observations = proving_ground.observations();
    assert_eq!(exchange.status(), 200);
    assert!(refused.is_err());
    assert_eq!(
        observations["relays"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| relay(record, &["proxy", "method", "target", "task", "outcome"]))
            .collect::<Vec<_>>(),
        [json!({
            "proxy": "open_tls",
            "method": "GET",
            "target": url,
            "task": "catalog/1",
            "outcome": "relayed",
        })]
    );
    assert_eq!(
        observations["connections"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|connection| connection["listener"] == "open_tls")
            .count(),
        2
    );
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

/// Covers: story:5-90
#[test]
fn the_limited_proxy_answers_too_many_requests_to_one_sent_while_it_relays_another() {
    let proving_ground = ProvingGround::start(SEED);
    let url = format!("http://catalog.test:{}/catalog/1", proving_ground.port());
    let proxy = proving_ground.proxy("limited");

    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| clients::raw(&proxy, &absolute(&url, None)));
        let second = scope.spawn(|| clients::raw(&proxy, &absolute(&url, None)));
        (first.join().unwrap(), second.join().unwrap())
    });
    let after = clients::raw(&proxy, &absolute(&url, None));

    let mut statuses = [first.status(), second.status()];
    statuses.sort_unstable();
    let refused = [first, second]
        .into_iter()
        .find(|exchange| exchange.status() == 429)
        .and_then(|exchange| exchange.header("proxy-status"));
    let mut outcomes: Vec<Value> = proving_ground.observations()["relays"]
        .as_array()
        .unwrap()
        .iter()
        .map(|relay| relay["outcome"].clone())
        .collect();
    outcomes.sort_by_key(ToString::to_string);
    assert_eq!(
        (statuses, after.status(), refused, outcomes),
        (
            [200, 429],
            200,
            Some("proving-ground; error=http_request_denied".to_owned()),
            vec![
                json!("relayed"),
                json!("relayed"),
                json!("too_many_requests")
            ],
        )
    );
}

/// The limited proxy stops counting a request once it read the site's whole
/// answer, before its client receives any of it: a client that keeps its
/// connection open after reading the answer holds no room, so a request
/// sent once the answer arrived is relayed (FQ57: the engine admits the
/// next navigation once the page loaded, and nothing orders the client's
/// close before it).
/// Covers: story:5-90
#[test]
fn the_limited_proxy_relays_a_request_sent_once_the_answer_before_arrived() {
    let proving_ground = ProvingGround::start(SEED);
    let url = format!("http://catalog.test:{}/catalog/1", proving_ground.port());
    let proxy = proving_ground.proxy("limited");

    let (first, open) = clients::raw_kept_open(&proxy, &absolute(&url, None));
    let second = clients::raw(&proxy, &absolute(&url, None));
    drop(open);

    assert_eq!((first.status(), second.status()), (200, 200));
}

/// The limited proxy counts the pages a client loads, never a site's
/// robots.txt or its icon, which an engine reads and a browser asks for of
/// their own beside a page (ADRs 0086 and 0088): one sent while it relays a
/// page is relayed too, with the site's own answer (catalog.test has
/// neither, so 404).
/// Covers: story:5-90
#[rstest]
#[case::robots_txt("/robots.txt")]
#[case::icon("/favicon.ico")]
fn the_limited_proxy_counts_no_robots_txt_or_icon(#[case] path: &str) {
    let proving_ground = ProvingGround::start(SEED);
    let page = format!("http://catalog.test:{}/catalog/1", proving_ground.port());
    let beside = format!("http://catalog.test:{}{path}", proving_ground.port());
    let proxy = proving_ground.proxy("limited");

    let held = proving_ground.put_control("/limited-proxy-hold", r#"{ "held": true }"#);
    let (first, second, still_held, released) = std::thread::scope(|scope| {
        let first = scope.spawn(|| clients::raw(&proxy, &absolute(&page, None)));
        pause(LATE);
        let second = clients::raw(&proxy, &absolute(&beside, None));
        let still_held = !first.is_finished();
        let released = proving_ground.put_control("/limited-proxy-hold", r#"{ "held": false }"#);
        (first.join().unwrap(), second, still_held, released)
    });

    assert_eq!(
        (
            held.status,
            second.status(),
            still_held,
            released.status,
            first.status()
        ),
        (200, 404, true, 200, 200)
    );
}

/// While a harness holds the limited proxy, the first request it counts
/// waits until the hold is released, however late the second comes: the
/// second, sent well past the proxy's usual 300 ms hold, is still refused,
/// and the first is relayed once released.
/// Covers: story:5-90
#[test]
fn the_limited_proxy_held_keeps_its_request_until_released_and_refuses_another() {
    let proving_ground = ProvingGround::start(SEED);
    let url = format!("http://catalog.test:{}/catalog/1", proving_ground.port());
    let proxy = proving_ground.proxy("limited");

    let held = proving_ground.put_control("/limited-proxy-hold", r#"{ "held": true }"#);
    let (statuses, released) = std::thread::scope(|scope| {
        let first = scope.spawn(|| clients::raw(&proxy, &absolute(&url, None)));
        pause(LATE);
        let second = scope.spawn(|| clients::raw(&proxy, &absolute(&url, None)));
        // Whichever came first is held: the other's refusal ends first.
        while !first.is_finished() && !second.is_finished() {
            pause(POLL);
        }
        let released = proving_ground.put_control("/limited-proxy-hold", r#"{ "held": false }"#);
        let mut statuses = [
            first.join().unwrap().status(),
            second.join().unwrap().status(),
        ];
        statuses.sort_unstable();
        (statuses, released)
    });

    assert_eq!(
        (held.status, released.status, statuses),
        (200, 200, [200, 429])
    );
}

/// Past the limited proxy's usual 300 ms hold, as late as a request sent
/// at once may come under load.
const LATE: std::time::Duration = std::time::Duration::from_millis(800);

/// How often the test looks whether a request ended.
const POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// Waits `duration` on the test's own thread.
#[expect(
    clippy::disallowed_methods,
    reason = "the test paces its own threads, outside any runtime"
)]
fn pause(duration: std::time::Duration) {
    std::thread::sleep(duration);
}

/// Another program on the machine cannot take the `down` proxy's address,
/// so a connection to it stays refused for as long as the Proving ground
/// runs.
#[test]
fn no_other_program_can_take_the_down_proxy_address() {
    let proving_ground = ProvingGround::start(SEED);
    let down = proving_ground.proxy("down");

    let other = clients::listen(&down);
    let refused = clients::connection_refused(&down);

    assert_eq!(
        other.map(|_| ()).map_err(|error| error.kind()),
        Err(std::io::ErrorKind::AddrInUse)
    );
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

#[test]
fn a_socks5_client_with_the_credentials_reaches_a_tasks_host_and_its_username_is_recorded() {
    let proving_ground = ProvingGround::start(SEED);
    let username = proving_ground.proxy_username();
    let password = proving_ground.proxy_password();
    let port: u16 = proving_ground.port().parse().unwrap();
    let request = format!(
        "GET /catalog/1 HTTP/1.1\r\nHost: 1.catalog.test:{port}\r\nConnection: close\r\n\r\n"
    );

    let exchange = clients::socks5(
        &proving_ground.proxy("socks5"),
        (&username, &password),
        ("1.catalog.test", port),
        request.as_bytes(),
    )
    .unwrap();

    let observations = proving_ground.observations();
    assert_eq!(exchange.status(), 200);
    assert_eq!(
        relay(
            &observations["relays"][0],
            &["proxy", "method", "target", "task", "username", "outcome"]
        ),
        json!({
            "proxy": "socks5",
            "method": "CONNECT",
            "target": format!("1.catalog.test:{port}"),
            "task": "catalog/1",
            "username": username,
            "outcome": "relayed",
        })
    );
    assert_eq!(observations["requests"][0]["task"], "catalog/1");
}

#[test]
fn the_socks5_proxy_refuses_a_wrong_password_and_records_the_username_alone() {
    let proving_ground = ProvingGround::start(SEED);
    let username = proving_ground.proxy_username();
    let port: u16 = proving_ground.port().parse().unwrap();

    let refused = clients::socks5(
        &proving_ground.proxy("socks5"),
        (&username, "wrong"),
        ("1.catalog.test", port),
        b"",
    );

    let observations = proving_ground.observations();
    assert_eq!(refused.err(), Some(clients::Socks5Refusal::Credentials(1)));
    assert_eq!(
        relay(
            &observations["relays"][0],
            &["proxy", "username", "outcome"]
        ),
        json!({
            "proxy": "socks5",
            "username": username,
            "outcome": "proxy_authentication_required",
        })
    );
}
