use std::net::IpAddr;

use super::{EgressRule, Family, Refusal, SsrfTable, SsrfTableError};
use crate::resolver::ScriptedResolver;

fn address(text: &str) -> IpAddr {
    text.parse().unwrap()
}

#[test]
fn a_case_reads_its_input_and_expected_refusal() {
    let table = SsrfTable::parse(
        r#"
        [[cases]]
        name = "decimal_integer"
        family = "encoding"
        url = "http://2130706433/"
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();

    let case = &table.cases[0];

    assert_eq!(
        (
            case.name.as_str(),
            case.family,
            case.url.as_str(),
            case.target,
            &case.refusal
        ),
        (
            "decimal_integer",
            Family::Encoding,
            "http://2130706433/",
            Some(address("127.0.0.1")),
            &Refusal::EgressRefused {
                rule: EgressRule::PrivateAddress
            }
        )
    );
}

#[test]
fn a_cases_lookups_script_the_resolver_with_the_layers_public_stand_in() {
    let table = SsrfTable::parse(
        r#"
        [[cases]]
        name = "ttl_zero_alternation"
        family = "rebinding"
        url = "http://rebind.ssrf.test/"
        lookups = [
            { name = "rebind.ssrf.test", answers = ["public"] },
            { name = "rebind.ssrf.test", answers = ["127.0.0.1", "public"] },
        ]
        fetches = 2
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();
    let resolver = ScriptedResolver::new(table.cases[0].script(address("192.31.196.1")));
    let handle = resolver.handle();

    let answers = [
        handle.lookup("rebind.ssrf.test"),
        handle.lookup("rebind.ssrf.test"),
    ];

    assert_eq!(
        answers,
        [
            vec![address("192.31.196.1")],
            vec![address("127.0.0.1"), address("192.31.196.1")]
        ]
    );
}

#[test]
fn a_case_naming_an_unknown_family_is_refused() {
    let parsed = SsrfTable::parse(
        r#"
        [[cases]]
        name = "decimal_integer"
        family = "spelling"
        url = "http://2130706433/"
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    );

    assert!(matches!(parsed, Err(SsrfTableError::Malformed(_))));
}
