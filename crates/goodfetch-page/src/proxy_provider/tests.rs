//! The Proxy-provider driver's cases in process (ADR 0129; spec #599's
//! Driver cases), with a fake provider that fills the two primitives, and
//! the `list` provider read from configuration.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_protocol::catalog::PriceValue;
use goodfetch_protocol::page::proxy::{
    CountryCode, ProxyKind, ProxyProviderConfiguration, ProxyServer,
};
use rstest::rstest;
use serde_json::json;

use super::{
    Chosen, Endpoint, Offer, ProxyDriver, ProxyProvider, Refusal, Sticky, Tally, Wanted, list,
    priced,
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
                }],
                longest_lifetime: Some(Duration::from_mins(30)),
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
    ProxyDriver::new("fake", Box::new(Fake::new()))
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

/// Covers: story:8-106, warning:proxy_exit_may_change
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
    ProxyDriver::new("list", Box::new(list::List::new(&configuration)))
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

/// Covers: story:8-92
#[test]
fn a_tally_charges_bytes_and_their_price_rounded_up_never_twice() {
    let mut tally = Tally::new(PriceValue::parse("5 USD"));

    let added: Vec<_> = [100_000_000, 100_000_000, 1, 1, 199_999_998]
        .into_iter()
        .map(|bytes| tally.add(bytes))
        .collect();

    assert_eq!(
        added,
        vec![
            (100_000_000, Some(500_000)),
            (100_000_000, Some(500_000)),
            (1, Some(1)),
            (1, Some(0)),
            (199_999_998, Some(999_999)),
        ]
    );
}
