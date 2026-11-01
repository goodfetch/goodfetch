//! A Session's proxy settings as the catalog reads them: `proxy.country`'s
//! codes, and a Proxy provider's entry.

use rstest::rstest;
use serde_json::json;

use super::{PROXY_COUNTRY, ProxyProviderConfiguration};

/// `proxy.country` takes only a code ISO 3166-1 assigns, in either case,
/// so a code that names no country is refused when the setting is read,
/// whatever the provider offers (deep review finding 599-3): `ZZ` and
/// `QQ` were never assigned, `EU` and `UK` are reserved without being
/// assigned.
///
/// Covers: setting:proxy.country
#[rstest]
#[case::us("US", true)]
#[case::lower_case_gb("gb", true)]
#[case::aland("AX", true)]
#[case::never_assigned("ZZ", false)]
#[case::user_assigned("QQ", false)]
#[case::reserved_eu("EU", false)]
#[case::reserved_uk("UK", false)]
#[case::three_letters("USA", false)]
fn proxy_country_takes_only_a_code_iso_3166_1_assigns(#[case] code: &str, #[case] taken: bool) {
    let checked = PROXY_COUNTRY.kind.check(&json!(code));

    assert_eq!(checked.is_ok(), taken, "{checked:?}");
}

/// A `zones` table names a Bright Data zone by proxy kind, as its customer
/// named it (O1 of the deep review, accepted by the owner on 2026-10-28;
/// finding 599-5): a zone name is letters, digits and `_`, since `-` parts
/// Bright Data's username, and a key names a proxy kind.
///
/// Covers: setting:proxy.providers
#[rstest]
#[case::by_kind(json!({ "residential": "residential_proxy1", "isp": "Isp2" }), None)]
#[case::an_unknown_kind(json!({ "cable": "cable1" }), Some("`cable`"))]
#[case::a_dash_in_its_name(json!({ "residential": "my-zone" }), Some("`my-zone`"))]
#[case::an_empty_name(json!({ "residential": "" }), Some("``"))]
#[case::not_a_table(json!("residential_proxy1"), Some("`zones`"))]
fn a_providers_zones_name_a_zone_by_proxy_kind(
    #[case] zones: serde_json::Value,
    #[case] refused: Option<&str>,
) {
    let entry = json!({ "username": "me", "password": "env:PROXY_PASSWORD", "zones": zones });

    let read = ProxyProviderConfiguration::read(&entry);

    assert_eq!(
        read.err()
            .map(|reason| refused.is_some_and(|named| reason.contains(named))),
        refused.map(|_| true)
    );
}
