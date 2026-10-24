//! The second egress class through `egress`'s interface (ADR 0017; spec
//! #564's `egress` seam): an Operator-configured endpoint reaches private
//! addresses, a model on localhost among them, unless
//! `egress.no_private_endpoints` holds it to globally reachable ones, and
//! no run's domain policy judges it.
//!
//! Covers: story:4-16, error:egress_refused, setting:egress.no_private_endpoints
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::egress::EgressRule;
use goodfetch_protocol::settings::SettingsLayer;
use rstest::rstest;
use serde_json::json;

use super::{private, sockets, verdict};
use crate::egress::{EndpointPolicy, Refused, judge_endpoint, judge_endpoint_addresses};
use crate::settings::{Layers, Policy, Values, resolve};

const NO_PRIVATE_ENDPOINTS: Option<&str> = Some("egress.no_private_endpoints");

#[rstest]
#[case::loopback(&["127.0.0.1"])]
#[case::ipv6_loopback(&["::1"])]
#[case::a_private_network(&["10.0.0.1"])]
#[case::ipv6_unique_local(&["fd00::1"])]
#[case::a_public_and_a_private_answer(&["93.184.215.14", "192.168.1.20"])]
fn an_endpoint_reaches_private_addresses_by_default(#[case] addresses: &[&str]) {
    assert_eq!(
        judge_endpoint_addresses(
            "http://collector.test:4318/v1/traces",
            &sockets(addresses),
            EndpointPolicy::default()
        ),
        Ok(())
    );
}

#[rstest]
#[case::loopback(&["127.0.0.1"], "127.0.0.1")]
#[case::ipv6_loopback(&["::1"], "::1")]
#[case::a_private_network(&["10.0.0.1"], "10.0.0.1")]
#[case::cloud_metadata(&["169.254.169.254"], "169.254.169.254")]
#[case::ipv4_mapped_loopback(&["::ffff:127.0.0.1"], "::ffff:127.0.0.1")]
#[case::a_private_answer_after_a_public_one(&["93.184.215.14", "10.0.0.1"], "10.0.0.1")]
fn with_no_private_endpoints_an_endpoint_is_refused_every_private_address(
    #[case] addresses: &[&str],
    #[case] refused: &str,
) {
    let url = "http://collector.test:4318/v1/traces";

    assert_eq!(
        judge_endpoint_addresses(url, &sockets(addresses), EndpointPolicy::new(true)),
        Err(private(url, refused, NO_PRIVATE_ENDPOINTS))
    );
}

#[test]
fn with_no_private_endpoints_an_endpoint_at_public_addresses_is_reached() {
    assert_eq!(
        judge_endpoint_addresses(
            "https://collector.test/v1/traces",
            &sockets(&["93.184.215.14", "2606:2800:21f:cb07:6820:80da:af6b:8b2c"]),
            EndpointPolicy::new(true)
        ),
        Ok(())
    );
}

#[test]
fn with_no_private_endpoints_a_zone_id_is_refused() {
    let url = "http://collector.test/";
    let scoped = std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
        "fe80::1".parse().unwrap(),
        80,
        0,
        2,
    ));

    assert_eq!(
        judge_endpoint_addresses(url, &[scoped], EndpointPolicy::new(true)),
        Err(private(url, "fe80::1", NO_PRIVATE_ENDPOINTS))
    );
}

#[rstest]
#[case::http("http://127.0.0.1:4318/v1/traces")]
#[case::https("https://collector.test/v1/traces")]
fn an_endpoint_of_the_web_is_allowed_whatever_a_runs_domain_policy(#[case] url: &str) {
    assert_eq!(verdict(judge_endpoint(url)), Ok(url.to_owned()));
}

#[rstest]
#[case::a_file("file:///etc/passwd")]
#[case::gopher("gopher://collector.test:70/")]
#[case::data("data:text/plain,hello")]
fn an_endpoint_of_another_scheme_is_refused(#[case] url: &str) {
    assert_eq!(
        verdict(judge_endpoint(url)),
        Err(Refused {
            url: url.to_owned(),
            rule: EgressRule::Scheme,
            address: None,
            setting: None,
        })
    );
}

#[rstest]
#[case::set_in_the_operator_policy(true, EndpointPolicy::new(true))]
#[case::unset(false, EndpointPolicy::new(false))]
fn the_endpoint_policy_is_the_setting_of_the_operator_policy(
    #[case] set: bool,
    #[case] expected: EndpointPolicy,
) {
    let mut layers = Layers::default();
    if set {
        let Some(settings) = json!({ "egress": { "no_private_endpoints": true } })
            .as_object()
            .cloned()
        else {
            panic!("the policy's settings are a table");
        };
        layers.operator_policy = Policy {
            settings: Values::read(SettingsLayer::OperatorPolicy, &settings, Product::Collie)
                .unwrap(),
            ..Policy::default()
        };
    }
    let settings = resolve(&layers, Product::Collie, Product::Collie).unwrap();

    assert_eq!(EndpointPolicy::of(&settings), expected);
}
