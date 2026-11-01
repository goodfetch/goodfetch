//! The Proxy-provider driver's cases in process (ADR 0129; spec #599's
//! Driver cases), with a fake provider that fills the two primitives, and
//! the `list` provider read from configuration.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

mod ahead;

use std::time::Duration;

use goodfetch_protocol::catalog::PriceValue;
use goodfetch_protocol::page::proxy::{
    CountryCode, ProxyKind, ProxyProviderConfiguration, ProxyServer,
};
use rstest::rstest;
use serde_json::json;

use super::failure::{ProviderAnswer, ProviderFailure};
use super::{
    Chosen, Endpoint, Offer, ProxyDriver, ProxyProvider, Refusal, Sticky, Wanted, apify,
    bright_data, list, oxylabs, priced,
};

/// A provider with one proxy of two kinds, its exits in two countries,
/// kept 30 minutes at most, whose username names everything it is given.
struct Fake {
    offer: Offer,
}

impl Fake {
    fn new() -> Self {
        Self {
            offer: Offer {
                endpoints: vec![Endpoint {
                    server: server("http://fake.example:8000"),
                    kinds: vec![ProxyKind::Datacenter, ProxyKind::Residential],
                    countries: Some(vec![country("US"), country("DE")]),
                    longest_lifetime: Some(Duration::from_mins(30)),
                }],
            },
        }
    }
}

impl ProxyProvider for Fake {
    fn offer(&self) -> &Offer {
        &self.offer
    }

    fn username(&self, sticky: &Sticky<'_>) -> Option<String> {
        Some(format!(
            "account-session-{}-kind-{}-country-{}-life-{}",
            sticky.session,
            sticky.kind,
            sticky.country.map_or("any", CountryCode::as_str),
            sticky.lifetime.as_deref().unwrap_or("none"),
        ))
    }

    fn format_lifetime(&self, lifetime: Duration) -> String {
        format!("{}m", lifetime.as_secs() / 60)
    }
}

fn server(text: &str) -> ProxyServer {
    ProxyServer::parse(text).unwrap()
}

fn country(code: &str) -> CountryCode {
    CountryCode::parse(code).unwrap()
}

fn fake() -> ProxyDriver {
    ProxyDriver::new("fake", Fake::new())
}

fn wanted(kind: Option<ProxyKind>, country_code: Option<&str>) -> Wanted {
    Wanted {
        kind,
        country: country_code.map(country),
        sticky_lifetime: None,
    }
}

/// Covers: story:8-106, setting:proxy.kind, setting:proxy.country
#[rstest]
#[case::a_kind_it_lacks(
    wanted(Some(ProxyKind::Mobile), None),
    Refusal::KindNotOffered {
        provider: "fake".to_owned(),
        asked: ProxyKind::Mobile,
        offered: vec![ProxyKind::Datacenter, ProxyKind::Residential],
    }
)]
#[case::a_country_it_lacks(
    wanted(Some(ProxyKind::Residential), Some("fr")),
    Refusal::CountryNotOffered {
        provider: "fake".to_owned(),
        asked: country("FR"),
        kind: ProxyKind::Residential,
        offered: vec![country("DE"), country("US")],
    }
)]
fn the_driver_refuses_a_kind_or_a_country_the_provider_lacks(
    #[case] wanted: Wanted,
    #[case] refusal: Refusal,
) {
    assert_eq!(fake().choose(&wanted, "run.session-1", 1), Err(refusal));
}

/// Covers: story:8-106, setting:proxy.kind, setting:proxy.country, setting:proxy.sticky_lifetime
#[rstest]
#[case::the_providers_default_kind_and_longest_lifetime(
    wanted(None, None),
    "account-session-run.session-1-kind-datacenter-country-any-life-30m",
    Some(30 * 60)
)]
#[case::its_kind_and_country(
    wanted(Some(ProxyKind::Residential), Some("de")),
    "account-session-run.session-1-kind-residential-country-DE-life-30m",
    Some(30 * 60)
)]
#[case::a_shorter_sticky_lifetime(
    Wanted { sticky_lifetime: Some(Duration::from_mins(10)), ..wanted(None, None) },
    "account-session-run.session-1-kind-datacenter-country-any-life-10m",
    Some(10 * 60)
)]
#[case::a_longer_one_held_to_the_providers_longest(
    Wanted { sticky_lifetime: Some(Duration::from_mins(90)), ..wanted(None, None) },
    "account-session-run.session-1-kind-datacenter-country-any-life-30m",
    Some(30 * 60)
)]
fn the_provider_builds_the_sessions_sticky_username_and_lifetime(
    #[case] wanted: Wanted,
    #[case] username: &str,
    #[case] lifetime_seconds: Option<u64>,
) {
    let chosen = fake().choose(&wanted, "run.session-1", 1).unwrap();

    assert_eq!(
        (chosen.username.as_deref(), chosen.lifetime),
        (Some(username), lifetime_seconds.map(Duration::from_secs))
    );
}

/// Covers: story:8-106, setting:proxy.sticky_lifetime
#[rstest]
#[case::within_its_lifetime(29 * 60, None)]
#[case::past_its_lifetime(31 * 60, Some(vec!["run.session-1".to_owned()]))]
fn a_session_that_outlives_its_sticky_lifetime_is_warned_of(
    #[case] lived_seconds: u64,
    #[case] warned: Option<Vec<String>>,
) {
    let chosen = fake()
        .choose(&wanted(None, None), "run.session-1", 1)
        .unwrap();

    let warning = chosen.exit_warning("run.session-1", Duration::from_secs(lived_seconds));

    assert_eq!(
        warning.map(|warning| (warning.code, warning.sessions)),
        warned.map(|sessions| ("proxy_exit_may_change".to_owned(), sessions))
    );
}

/// The `list` provider of `servers`, with `username`.
fn listed(servers: &serde_json::Value) -> ProxyDriver {
    let configuration = ProxyProviderConfiguration::read(&json!({
        "username": "me",
        "password": "env:PROXY_PASSWORD",
        "servers": servers,
    }))
    .unwrap();
    ProxyDriver::new("list", list::List::new(&configuration))
}

/// What a Session opened `ordinal`-th was given: its server, kind and
/// username.
fn given(chosen: &Chosen) -> (String, ProxyKind, Option<String>) {
    (
        chosen.server.to_string(),
        chosen.kind,
        chosen.username.clone(),
    )
}

/// Covers: story:8-88, story:8-106, setting:proxy.provider, setting:proxy.providers
#[test]
fn the_list_provider_gives_each_session_one_of_the_users_servers_of_its_kind_in_turn() {
    let driver = listed(&json!([
        { "server": "http://127.0.0.1:3001", "kind": "datacenter", "country": "US" },
        { "server": "socks5://127.0.0.1:3002", "kind": "residential" },
        { "server": "http://127.0.0.1:3003", "kind": "datacenter", "country": "DE" },
    ]));

    let sessions: Vec<_> = (1..=3)
        .map(|ordinal| {
            given(
                &driver
                    .choose(
                        &wanted(None, None),
                        &format!("run.session-{ordinal}"),
                        ordinal,
                    )
                    .unwrap(),
            )
        })
        .collect();
    let residential = given(
        &driver
            .choose(
                &wanted(Some(ProxyKind::Residential), None),
                "run.session-4",
                4,
            )
            .unwrap(),
    );
    let in_germany = driver
        .choose(&wanted(None, Some("DE")), "run.session-5", 5)
        .unwrap();
    let lacking = driver.choose(&wanted(Some(ProxyKind::Mobile), None), "run.session-6", 6);

    let me = Some("me".to_owned());
    assert_eq!(
        (
            sessions,
            residential,
            given(&in_germany),
            in_germany.lifetime,
            lacking
        ),
        (
            vec![
                (
                    "http://127.0.0.1:3001".to_owned(),
                    ProxyKind::Datacenter,
                    me.clone()
                ),
                (
                    "http://127.0.0.1:3003".to_owned(),
                    ProxyKind::Datacenter,
                    me.clone()
                ),
                (
                    "http://127.0.0.1:3001".to_owned(),
                    ProxyKind::Datacenter,
                    me.clone()
                ),
            ],
            (
                "socks5://127.0.0.1:3002".to_owned(),
                ProxyKind::Residential,
                me.clone()
            ),
            (
                "http://127.0.0.1:3003".to_owned(),
                ProxyKind::Datacenter,
                me
            ),
            None,
            Err(Refusal::KindNotOffered {
                provider: "list".to_owned(),
                asked: ProxyKind::Mobile,
                offered: vec![ProxyKind::Datacenter, ProxyKind::Residential],
            }),
        )
    );
}

/// Covers: story:8-92, setting:proxy.providers
#[rstest]
#[case::no_currency_limit(&[], None, Ok(None))]
#[case::priced_without_a_currency_limit(&[], Some("5 USD"), Ok(Some("5 USD")))]
#[case::unpriced_under_a_currency_limit(&["USD"], None, Err(Refusal::Unpriced { provider: "fake".to_owned(), currency: "USD".to_owned() }))]
#[case::priced_in_another_currency(&["EUR"], Some("5 USD"), Err(Refusal::Unpriced { provider: "fake".to_owned(), currency: "EUR".to_owned() }))]
#[case::priced_in_the_limits_currency(&["USD"], Some("5 USD"), Ok(Some("5 USD")))]
fn a_metered_price_is_needed_under_a_currency_limit(
    #[case] currencies: &[&str],
    #[case] price: Option<&str>,
    #[case] expected: Result<Option<&str>, Refusal>,
) {
    let price = price.map(|price| PriceValue::parse(price).unwrap());

    let checked = priced("fake", currencies, price.as_ref());

    assert_eq!(
        checked,
        expected.map(|price| price.map(|price| PriceValue::parse(price).unwrap()))
    );
}

/// A provider's `endpoint` is spoken to through the Session's loopback
/// relay, which speaks no TLS to it, so an `https://` endpoint is refused as
/// a `list` server's is, never spoken to in clear with the account's Basic
/// credentials (deep review finding 599-14); `http://` and SOCKS5 ones
/// stand.
///
/// Covers: setting:proxy.providers
#[rstest]
#[case::https("https://brd.superproxy.io:33335", false)]
#[case::http("http://brd.superproxy.io:33335", true)]
#[case::socks5("socks5://pr.oxylabs.io:7777", true)]
fn an_https_endpoint_is_refused_as_an_https_list_server_is(
    #[case] endpoint: &str,
    #[case] read: bool,
) {
    let entry = json!({ "username": "me", "password": "env:PROXY_PASSWORD", "endpoint": endpoint });

    let configuration = ProxyProviderConfiguration::read(&entry);

    assert_eq!(
        (
            configuration.is_ok(),
            configuration
                .err()
                .is_some_and(|reason| reason.contains("`endpoint`") && reason.contains(endpoint))
        ),
        (read, !read)
    );
}

/// The SHA-256 of `run.session-1`, its first 16 hex digits: the sticky
/// session every adapter names for it.
const SESSION_ONE: &str = "79f5a6526323bee7";

/// The configuration of an account `account`, with `extra` fields.
fn account(account: Option<&str>, extra: &serde_json::Value) -> ProxyProviderConfiguration {
    let mut entry = json!({ "password": "env:PROXY_PASSWORD" });
    if let Some(account) = account {
        entry["username"] = json!(account);
    }
    for (field, value) in extra.as_object().into_iter().flatten() {
        entry[field] = value.clone();
    }
    ProxyProviderConfiguration::read(&entry).unwrap()
}

/// The driver of the built-in adapter `name`, on the account `me`, with
/// `extra` fields.
fn adapter(name: &str, extra: &serde_json::Value) -> ProxyDriver {
    let configuration = account(Some("me"), extra);
    match name {
        bright_data::NAME => {
            ProxyDriver::new(name, bright_data::BrightData::new(&configuration).unwrap())
        }
        oxylabs::NAME => ProxyDriver::new(name, oxylabs::Oxylabs::new(&configuration).unwrap()),
        apify::NAME => ProxyDriver::new(name, apify::Apify::new(&configuration)),
        other => panic!("no adapter `{other}`"),
    }
}

/// What a Session was given: its server, kind, username and lifetime in
/// seconds.
fn sticky_given(chosen: &Chosen) -> (String, ProxyKind, Option<String>, Option<u64>) {
    (
        chosen.server.to_string(),
        chosen.kind,
        chosen.username.clone(),
        chosen.lifetime.map(|lifetime| lifetime.as_secs()),
    )
}

/// Covers: story:8-89, story:8-90, setting:proxy.provider, setting:proxy.providers, setting:proxy.kind, setting:proxy.country, setting:proxy.sticky_lifetime
#[rstest]
#[case::bright_data_by_default(
    "bright_data",
    json!({}),
    wanted(None, None),
    ("http://brd.superproxy.io:33335", ProxyKind::Residential, format!("brd-customer-me-zone-residential-session-{SESSION_ONE}"), 300)
)]
#[case::bright_data_in_a_country(
    "bright_data",
    json!({}),
    wanted(Some(ProxyKind::Isp), Some("DE")),
    ("http://brd.superproxy.io:33335", ProxyKind::Isp, format!("brd-customer-me-zone-isp-country-de-session-{SESSION_ONE}"), 300)
)]
#[case::bright_data_through_the_zone_its_customer_named(
    "bright_data",
    json!({ "zones": { "residential": "residential_proxy1" } }),
    wanted(None, Some("FR")),
    ("http://brd.superproxy.io:33335", ProxyKind::Residential, format!("brd-customer-me-zone-residential_proxy1-country-fr-session-{SESSION_ONE}"), 300)
)]
#[case::bright_data_of_a_kind_its_zones_leave_out(
    "bright_data",
    json!({ "zones": { "residential": "residential_proxy1" } }),
    wanted(Some(ProxyKind::Datacenter), None),
    ("http://brd.superproxy.io:33335", ProxyKind::Datacenter, format!("brd-customer-me-zone-datacenter-session-{SESSION_ONE}"), 300)
)]
#[case::oxylabs_by_default(
    "oxylabs",
    json!({}),
    wanted(None, None),
    ("http://pr.oxylabs.io:7777", ProxyKind::Residential, format!("customer-me-sessid-{SESSION_ONE}-sesstime-1440"), 86_400)
)]
#[case::oxylabs_in_a_country_for_a_shorter_lifetime(
    "oxylabs",
    json!({}),
    Wanted { sticky_lifetime: Some(Duration::from_secs(630)), ..wanted(None, Some("us")) },
    ("http://pr.oxylabs.io:7777", ProxyKind::Residential, format!("customer-me-cc-US-sessid-{SESSION_ONE}-sesstime-11"), 630)
)]
#[case::apify_by_default(
    "apify",
    json!({}),
    wanted(None, None),
    ("http://proxy.apify.com:8000", ProxyKind::Datacenter, format!("session-{SESSION_ONE}"), 26 * 3600)
)]
#[case::apify_residential_in_a_country_through_its_endpoint(
    "apify",
    json!({ "endpoint": "http://127.0.0.1:9000" }),
    wanted(Some(ProxyKind::Residential), Some("de")),
    ("http://127.0.0.1:9000", ProxyKind::Residential, format!("groups-RESIDENTIAL,session-{SESSION_ONE},country-DE"), 1800)
)]
fn each_adapter_builds_its_providers_sticky_username_and_lifetime(
    #[case] name: &str,
    #[case] extra: serde_json::Value,
    #[case] wanted: Wanted,
    #[case] expected: (&str, ProxyKind, String, u64),
) {
    let chosen = adapter(name, &extra)
        .choose(&wanted, "run.session-1", 1)
        .unwrap();

    assert_eq!(
        sticky_given(&chosen),
        (
            expected.0.to_owned(),
            expected.1,
            Some(expected.2),
            Some(expected.3)
        )
    );
}

/// Covers: story:8-89, story:8-106, setting:proxy.kind
#[rstest]
#[case::oxylabs("oxylabs", ProxyKind::Datacenter, vec![ProxyKind::Residential])]
#[case::apify("apify", ProxyKind::Mobile, vec![ProxyKind::Datacenter, ProxyKind::Residential])]
fn an_adapter_refuses_a_kind_its_provider_lacks(
    #[case] name: &str,
    #[case] asked: ProxyKind,
    #[case] offered: Vec<ProxyKind>,
) {
    assert_eq!(
        adapter(name, &json!({})).choose(&wanted(Some(asked), None), "run.session-1", 1),
        Err(Refusal::KindNotOffered {
            provider: name.to_owned(),
            asked,
            offered,
        })
    );
}

/// Covers: story:8-89, setting:proxy.providers
#[rstest]
#[case::bright_data("bright_data")]
#[case::oxylabs("oxylabs")]
fn an_adapter_whose_username_names_the_account_needs_one(#[case] name: &str) {
    let configuration = account(None, &json!({}));
    let refused = match name {
        bright_data::NAME => bright_data::BrightData::new(&configuration).err(),
        _ => oxylabs::Oxylabs::new(&configuration).err(),
    };

    assert!(
        refused.is_some_and(|reason| reason.contains(&format!("proxy.providers.{name}.username"))),
        "{name}"
    );
}

/// A provider's answer of `status` with `headers`.
fn answer(status: u16, headers: &[(&str, &str)]) -> ProviderAnswer {
    ProviderAnswer {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

/// A failure's variant, by name.
fn variant(failure: &ProviderFailure) -> &'static str {
    match failure {
        ProviderFailure::QuotaExhausted => "quota_exhausted",
        ProviderFailure::StickySessionExpired => "sticky_session_expired",
        ProviderFailure::BadCredentials => "bad_credentials",
        ProviderFailure::Unrecognized(_) => "unrecognized",
    }
}

/// The adapter `name`'s translation point.
fn translated(name: &str, answer: &ProviderAnswer) -> ProviderFailure {
    match name {
        bright_data::NAME => bright_data::failure(answer),
        oxylabs::NAME => oxylabs::failure(answer),
        apify::NAME => apify::failure(answer),
        other => panic!("no adapter `{other}`"),
    }
}

/// Text no known failure carries, which must reach no accessor.
const UNKNOWN_TEXT: &str = "zq-unheard-of-refusal";

/// Covers: story:8-89, story:8-108
#[rstest]
#[case::bright_data_wrong_password("bright_data", answer(407, &[("x-brd-err-code", "client_10000"), ("x-brd-error", "Authentication failed")]), "bad_credentials")]
#[case::bright_data_no_credentials("bright_data", answer(407, &[("x-brd-err-code", "client_10010")]), "bad_credentials")]
#[case::bright_data_usage_limit("bright_data", answer(502, &[("x-brd-err-code", "client_10100"), ("x-brd-error", "Zone has reached usage limit")]), "quota_exhausted")]
#[case::bright_data_session_ip_gone("bright_data", answer(502, &[("x-brd-err-code", "client_10064")]), "sticky_session_expired")]
#[case::bright_data_unknown("bright_data", answer(502, &[("x-brd-err-code", "client_19999"), ("x-brd-error", UNKNOWN_TEXT)]), "unrecognized")]
#[case::bright_data_usage_limit_in_proxy_status("bright_data", answer(502, &[("proxy-status", "brd.superproxy.io; received-status=502; error=http_request_denied; details=\"client_10100: Zone has reached usage limit\"")]), "quota_exhausted")]
#[case::bright_data_session_ip_gone_in_proxy_status("bright_data", answer(502, &[("proxy-status", "brd.superproxy.io; error=destination_unavailable; details=\"client_10064: The IP assigned to this session is unavailable\"")]), "sticky_session_expired")]
#[case::bright_data_suspended_account_in_proxy_status("bright_data", answer(407, &[("proxy-status", "brd.superproxy.io; error=http_request_denied; details=\"client_10020: Account is suspended\"")]), "quota_exhausted")]
#[case::bright_data_zone_not_found_in_proxy_status("bright_data", answer(407, &[("proxy-status", "brd.superproxy.io; error=http_request_denied; details=\"client_10002: Zone not found\"")]), "bad_credentials")]
#[case::bright_data_unknown_in_proxy_status("bright_data", answer(502, &[("proxy-status", "brd.superproxy.io; error=http_request_error; details=\"client_19999: zq\"")]), "unrecognized")]
#[case::oxylabs_authentication("oxylabs", answer(407, &[("x-error-description", "Authentication failed")]), "bad_credentials")]
#[case::oxylabs_traffic_limit("oxylabs", answer(407, &[("x-error-description", "Traffic limit exhausted")]), "quota_exhausted")]
#[case::oxylabs_session_ended("oxylabs", answer(502, &[("x-error-description", "Exit node not found: session ended")]), "sticky_session_expired")]
#[case::oxylabs_unknown("oxylabs", answer(500, &[("x-error-description", UNKNOWN_TEXT)]), "unrecognized")]
#[case::apify_authentication("apify", answer(407, &[]), "bad_credentials")]
#[case::apify_payment_required("apify", answer(402, &[]), "quota_exhausted")]
#[case::apify_unknown("apify", answer(599, &[("x-apify-proxy-error", UNKNOWN_TEXT)]), "unrecognized")]
fn each_adapter_reads_its_providers_failures(
    #[case] name: &str,
    #[case] answer: ProviderAnswer,
    #[case] expected: &str,
) {
    assert_eq!(variant(&translated(name, &answer)), expected);
}

/// Covers: story:8-108, error:proxy_provider_unavailable
#[rstest]
#[case::bright_data("bright_data", answer(502, &[("x-brd-err-code", "client_19999"), ("x-brd-error", UNKNOWN_TEXT)]))]
#[case::oxylabs("oxylabs", answer(500, &[("x-error-description", UNKNOWN_TEXT)]))]
#[case::apify("apify", answer(599, &[("x-apify-proxy-error", UNKNOWN_TEXT)]))]
fn an_unknown_failure_keeps_its_text_out_and_is_proxy_provider_unavailable(
    #[case] name: &str,
    #[case] answer: ProviderAnswer,
) {
    let failure = translated(name, &answer);
    let proxy = server("http://proxy.example:8000");

    let error = failure
        .unavailable(name, &proxy, Some("https://site.example/"))
        .unwrap();

    let shown = format!(
        "{failure:?} {} {}",
        error.message(),
        serde_json::to_string(&error.details()).unwrap()
    );
    assert_eq!(
        (
            error.code(),
            error.retryable(),
            error.details().map(|details| details["failure"].clone()),
            error.details().map(|details| details["provider"].clone()),
            shown.contains(UNKNOWN_TEXT),
        ),
        (
            "proxy_provider_unavailable",
            true,
            Some(json!("unrecognized")),
            Some(json!(name)),
            false
        )
    );
}

/// Covers: story:8-108, error:proxy_provider_unavailable
#[rstest]
#[case::quota_exhausted(ProviderFailure::QuotaExhausted, Some("quota_exhausted"))]
#[case::sticky_session_expired(
    ProviderFailure::StickySessionExpired,
    Some("sticky_session_expired")
)]
#[case::bad_credentials(ProviderFailure::BadCredentials, None)]
fn a_known_failure_is_proxy_provider_unavailable_naming_it_but_bad_credentials(
    #[case] failure: ProviderFailure,
    #[case] named: Option<&str>,
) {
    let error = failure.unavailable("oxylabs", &server("http://proxy.example:8000"), None);

    assert_eq!(
        error.map(|error| (error.code(), error.details().unwrap()["failure"].clone())),
        named.map(|named| ("proxy_provider_unavailable", json!(named)))
    );
}
