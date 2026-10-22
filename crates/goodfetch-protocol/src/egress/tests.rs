use serde_json::json;

use super::{EgressRefusedDetails, EgressRule};

#[test]
fn egress_refused_names_the_url_the_rule_and_the_setting_on_the_wire() {
    let details = EgressRefusedDetails {
        url: "https://elsewhere.test/offsite".to_owned(),
        rule: EgressRule::DomainPolicy,
        address: None,
        setting: Some("egress.allowed_domains".to_owned()),
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({
            "url": "https://elsewhere.test/offsite",
            "rule": "domain_policy",
            "setting": "egress.allowed_domains",
        })
    );
}

#[test]
fn egress_refused_names_the_address_a_private_address_rule_judged() {
    let details = EgressRefusedDetails {
        url: "http://2130706433/".to_owned(),
        rule: EgressRule::PrivateAddress,
        address: Some("127.0.0.1".to_owned()),
        setting: Some("egress.private_ranges".to_owned()),
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({
            "url": "http://2130706433/",
            "rule": "private_address",
            "address": "127.0.0.1",
            "setting": "egress.private_ranges",
        })
    );
}
