//! ADR 0056's address rules for a `cdp_url` through `egress`'s interface
//! (ADR 0115; spec #564's `egress` seam): one set in Operator policy or
//! configuration is an Operator-configured endpoint, one from the
//! program's own code or the CLI may also use loopback, and any other is
//! an address a page can reach, so a model never points the engine at an
//! internal debugging port.
//!
//! Covers: story:4-15, error:cdp_url_not_allowed, setting:egress.no_private_endpoints
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::egress::{CdpUrlRule, EgressRule};
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;

use super::{policy, sockets};
use crate::egress::{
    BrowserLocation, CdpUrlOrigin, CdpUrlRefused, EndpointPolicy, Refused, cdp_url_address,
    judge_cdp_url, judge_unresolved,
};

const URL: &str = "ws://browser.test:9222/devtools/browser/1";

fn own(layer: SettingsLayer) -> CdpUrlOrigin {
    CdpUrlOrigin {
        layer,
        acting_for_model: false,
    }
}

fn from_a_model(layer: SettingsLayer) -> CdpUrlOrigin {
    CdpUrlOrigin {
        layer,
        acting_for_model: true,
    }
}

const FROM_THE_PROGRAM: CdpUrlOrigin = CdpUrlOrigin {
    layer: SettingsLayer::Run,
    acting_for_model: false,
};

const FROM_A_MODEL: CdpUrlOrigin = CdpUrlOrigin {
    layer: SettingsLayer::Run,
    acting_for_model: true,
};

fn judged(
    addresses: &[&str],
    origin: CdpUrlOrigin,
    endpoints: EndpointPolicy,
) -> Result<(), CdpUrlRefused> {
    judge_cdp_url(
        URL,
        &sockets(addresses),
        origin,
        &policy(&["*"], &[]),
        endpoints,
    )
}

fn refused(
    origin: CdpUrlOrigin,
    rule: CdpUrlRule,
    address: &str,
    setting: Option<&'static str>,
) -> CdpUrlRefused {
    CdpUrlRefused {
        url: URL.to_owned(),
        layer: origin.layer,
        rule,
        address: address.parse().unwrap(),
        setting,
    }
}

#[rstest]
#[case::loopback(&["127.0.0.1"])]
#[case::another_loopback_address(&["127.0.0.2"])]
#[case::ipv6_loopback(&["::1"])]
#[case::ipv4_mapped_loopback(&["::ffff:127.0.0.1"])]
#[case::a_public_address(&["93.184.215.14"])]
fn the_programs_own_cdp_url_may_use_loopback(#[case] addresses: &[&str]) {
    assert_eq!(
        judged(addresses, FROM_THE_PROGRAM, EndpointPolicy::default()),
        Ok(())
    );
}

#[rstest]
#[case::a_private_network(&["10.0.0.1"], "10.0.0.1")]
#[case::cloud_metadata(&["169.254.169.254"], "169.254.169.254")]
#[case::ipv6_unique_local(&["fd00::1"], "fd00::1")]
#[case::loopback_and_a_private_network(&["127.0.0.1", "192.168.1.20"], "192.168.1.20")]
fn the_programs_own_cdp_url_is_refused_any_other_private_address(
    #[case] addresses: &[&str],
    #[case] address: &str,
) {
    assert_eq!(
        judged(addresses, FROM_THE_PROGRAM, EndpointPolicy::default()),
        Err(refused(
            FROM_THE_PROGRAM,
            CdpUrlRule::PrivateAddress,
            address,
            Some("egress.private_ranges")
        ))
    );
}

#[rstest]
#[case::from_a_client_acting_for_a_model(FROM_A_MODEL, &["127.0.0.1"], "127.0.0.1")]
#[case::ipv6_from_a_model(FROM_A_MODEL, &["::1"], "::1")]
#[case::from_a_recipe(own(SettingsLayer::Recipe), &["127.0.0.1"], "127.0.0.1")]
fn a_cdp_url_from_a_model_or_a_recipe_is_refused_loopback(
    #[case] origin: CdpUrlOrigin,
    #[case] addresses: &[&str],
    #[case] address: &str,
) {
    assert_eq!(
        judged(addresses, origin, EndpointPolicy::default()),
        Err(refused(origin, CdpUrlRule::Loopback, address, None))
    );
}

#[test]
fn a_cdp_url_from_a_model_is_refused_a_private_network() {
    assert_eq!(
        judged(&["10.0.0.1"], FROM_A_MODEL, EndpointPolicy::default()),
        Err(refused(
            FROM_A_MODEL,
            CdpUrlRule::PrivateAddress,
            "10.0.0.1",
            Some("egress.private_ranges")
        ))
    );
}

#[test]
fn a_cdp_url_from_a_model_at_a_public_address_is_allowed() {
    assert_eq!(
        judged(&["93.184.215.14"], FROM_A_MODEL, EndpointPolicy::default()),
        Ok(())
    );
}

#[rstest]
#[case::operator_policy(SettingsLayer::OperatorPolicy, &["10.0.0.1"])]
#[case::user_configuration(SettingsLayer::UserConfiguration, &["192.168.1.20"])]
#[case::project_configuration(SettingsLayer::ProjectConfiguration, &["127.0.0.1"])]
#[case::cloud_metadata_from_operator_policy(SettingsLayer::OperatorPolicy, &["169.254.169.254"])]
fn a_configured_cdp_url_is_an_operator_configured_endpoint_on_any_address(
    #[case] layer: SettingsLayer,
    #[case] addresses: &[&str],
) {
    assert_eq!(
        judged(addresses, own(layer), EndpointPolicy::default()),
        Ok(())
    );
}

/// O8 of the deep review (accepted by the owner on 2026-10-28): a Client
/// acting for a model sends the project configuration itself, so a
/// `cdp_url` there is the model's, refused on loopback and on a private
/// network; the same address in the Operator policy or user configuration
/// stays an Operator-configured endpoint (positive controls).
#[rstest]
#[case::loopback(
    from_a_model(SettingsLayer::ProjectConfiguration),
    &["127.0.0.1"],
    Err((CdpUrlRule::Loopback, "127.0.0.1", None))
)]
#[case::a_private_network(
    from_a_model(SettingsLayer::ProjectConfiguration),
    &["10.0.0.1"],
    Err((CdpUrlRule::PrivateAddress, "10.0.0.1", Some("egress.private_ranges")))
)]
#[case::a_public_address(
    from_a_model(SettingsLayer::ProjectConfiguration),
    &["93.184.215.14"],
    Ok(())
)]
#[case::user_configuration(from_a_model(SettingsLayer::UserConfiguration), &["127.0.0.1"], Ok(()))]
#[case::operator_policy(from_a_model(SettingsLayer::OperatorPolicy), &["10.0.0.1"], Ok(()))]
fn a_cdp_url_in_a_models_project_configuration_is_the_models(
    #[case] origin: CdpUrlOrigin,
    #[case] addresses: &[&str],
    #[case] expected: Result<(), (CdpUrlRule, &str, Option<&'static str>)>,
) {
    assert_eq!(
        judged(addresses, origin, EndpointPolicy::default()),
        expected.map_err(|(rule, address, setting)| refused(origin, rule, address, setting))
    );
}

#[rstest]
#[case::a_private_network(SettingsLayer::OperatorPolicy, &["10.0.0.1"], "10.0.0.1")]
#[case::loopback(SettingsLayer::UserConfiguration, &["127.0.0.1"], "127.0.0.1")]
fn a_configured_cdp_url_is_refused_private_addresses_under_no_private_endpoints(
    #[case] layer: SettingsLayer,
    #[case] addresses: &[&str],
    #[case] address: &str,
) {
    assert_eq!(
        judged(addresses, own(layer), EndpointPolicy::new(true)),
        Err(refused(
            own(layer),
            CdpUrlRule::PrivateAddress,
            address,
            Some("egress.no_private_endpoints")
        ))
    );
}

#[test]
fn a_refusal_names_the_rule_the_layer_and_the_fix_on_the_wire() {
    let error = refused(FROM_A_MODEL, CdpUrlRule::Loopback, "127.0.0.1", None).error();

    assert_eq!(
        (
            error.code(),
            error.details().cloned().map(serde_json::Value::Object)
        ),
        (
            "cdp_url_not_allowed",
            Some(serde_json::json!({
                "url": URL,
                "layer": "run",
                "rule": "loopback",
                "address": "127.0.0.1",
                "fix": "set the address in Operator policy or in user configuration, or give a public address",
            }))
        )
    );
}

#[rstest]
#[case::as_given(
    "ws://127.0.0.1:9222/devtools/browser/1",
    "ws://127.0.0.1:9222/devtools/browser/1"
)]
#[case::its_query_left_out(
    "wss://cloud.test/devtools/browser/1?token=abc&x=1",
    "wss://cloud.test/devtools/browser/1"
)]
#[case::its_credentials_left_out("ws://name:word@127.0.0.1:9222/", "ws://127.0.0.1:9222/")]
#[case::its_fragment_left_out("http://127.0.0.1:9222/#key", "http://127.0.0.1:9222/")]
#[case::not_a_url("not a url?token=abc", "not a url")]
fn a_cdp_url_is_named_without_its_query_or_credentials(#[case] url: &str, #[case] named: &str) {
    assert_eq!(cdp_url_address(url), named);
}

/// `security.resolved_ip_check` switches off the Run's check of the
/// addresses its pages reach, never ADR 0056's rules for a `cdp_url`: a
/// model's still reaches no private address with the switch off.
/// Covers: story:4-15, story:4-17, setting:security.resolved_ip_check
#[test]
fn the_resolved_address_check_switched_off_leaves_a_cdp_urls_rules_on() {
    let off = super::of_the_run(serde_json::json!({ "security": { "resolved_ip_check": false } }));

    assert_eq!(
        judge_cdp_url(
            URL,
            &sockets(&["10.0.0.1"]),
            FROM_A_MODEL,
            &off,
            EndpointPolicy::default(),
        ),
        Err(refused(
            FROM_A_MODEL,
            CdpUrlRule::PrivateAddress,
            "10.0.0.1",
            Some("egress.private_ranges")
        ))
    );
}

/// O9 of the deep review (accepted by the owner on 2026-10-28): a
/// Borrowed browser is on the engine's machine only when every address its
/// `cdp_url` resolves to is loopback; any other address, among them one
/// that only connects to this machine, such as `0.0.0.0`, puts it
/// elsewhere, and so does no address at all.
#[rstest]
#[case::loopback(&["127.0.0.1"], BrowserLocation::EnginesMachine)]
#[case::ipv6_loopback(&["::1", "127.0.0.1"], BrowserLocation::EnginesMachine)]
#[case::ipv4_mapped_loopback(&["::ffff:127.0.0.1"], BrowserLocation::EnginesMachine)]
#[case::a_private_network(&["10.0.0.5"], BrowserLocation::Elsewhere)]
#[case::a_public_address(&["93.184.215.14"], BrowserLocation::Elsewhere)]
#[case::loopback_and_a_private_network(&["127.0.0.1", "10.0.0.5"], BrowserLocation::Elsewhere)]
#[case::the_unspecified_address(&["0.0.0.0"], BrowserLocation::Elsewhere)]
#[case::no_address(&[], BrowserLocation::Elsewhere)]
fn a_borrowed_browser_is_on_the_engines_machine_only_at_loopback(
    #[case] addresses: &[&str],
    #[case] location: BrowserLocation,
) {
    assert_eq!(BrowserLocation::of(&sockets(addresses)), location);
}

/// O9 of the deep review (accepted by the owner on 2026-10-28): a name the
/// engine cannot resolve is refused on a Borrowed browser elsewhere, whose
/// network may resolve it to an intranet host or a cloud's metadata
/// service, naming `network.resolve`, which lets the engine judge it; on a
/// browser on the engine's machine, which resolves it on the network the
/// engine sees, it is left to the browser (positive control), and so it is
/// with `security.resolved_ip_check` off, which judges no address.
/// Covers: story:4-14, error:egress_refused, setting:network.resolve
#[test]
fn a_name_the_engine_cannot_resolve_is_refused_on_a_borrowed_browser_elsewhere() {
    const UNRESOLVED: &str = "http://metadata.google.internal/computeMetadata/v1/";
    let on = policy(&["*"], &[]);
    let off = super::of_the_run(serde_json::json!({ "security": { "resolved_ip_check": false } }));

    assert_eq!(
        (
            judge_unresolved(UNRESOLVED, &on, BrowserLocation::Elsewhere),
            judge_unresolved(UNRESOLVED, &on, BrowserLocation::EnginesMachine),
            judge_unresolved(UNRESOLVED, &off, BrowserLocation::Elsewhere),
        ),
        (
            Err(Refused {
                url: UNRESOLVED.to_owned(),
                rule: EgressRule::DomainPolicy,
                address: None,
                setting: Some("network.resolve"),
            }),
            Ok(()),
            Ok(()),
        )
    );
}
