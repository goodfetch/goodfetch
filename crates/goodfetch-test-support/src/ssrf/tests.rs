use std::net::IpAddr;

use super::{
    EgressRule, Family, LetThrough, Miss, Outcome, Refusal, SsrfCase, SsrfTable, SsrfTableError,
    run_cases,
};
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

/// Three loopback cases: the dotted form, and two encodings a filter that
/// looks for the dotted text lets through.
const LOOPBACK_CASES: &str = r#"
    [[cases]]
    name = "dotted"
    family = "encoding"
    url = "http://127.0.0.1/"
    target = "127.0.0.1"
    refusal = { code = "egress_refused", rule = "private_address" }

    [[cases]]
    name = "decimal_integer"
    family = "encoding"
    url = "http://2130706433/"
    target = "127.0.0.1"
    refusal = { code = "egress_refused", rule = "private_address" }

    [[cases]]
    name = "hexadecimal_integer"
    family = "encoding"
    url = "http://0x7f000001/"
    target = "127.0.0.1"
    refusal = { code = "egress_refused", rule = "private_address" }
"#;

const PRIVATE_ADDRESS: Refusal = Refusal::EgressRefused {
    rule: EgressRule::PrivateAddress,
};

/// A fixture layer that refuses a URL holding the dotted loopback text and
/// connects to the target of any other.
fn dotted_text_filter(case: &SsrfCase) -> Outcome {
    if case.url.contains("127.0.0.1") {
        Outcome::refused(PRIVATE_ADDRESS)
    } else {
        Outcome::connected(case.target)
    }
}

#[test]
fn a_runner_reports_every_case_its_layer_lets_through() {
    let table = SsrfTable::parse(LOOPBACK_CASES).unwrap();

    let result = run_cases(&table.cases, dotted_text_filter);

    assert_eq!(
        result,
        Err(LetThrough {
            misses: vec![
                Miss {
                    case: "decimal_integer".to_owned(),
                    expected: PRIVATE_ADDRESS,
                    refusal: None,
                    reached_target: true,
                },
                Miss {
                    case: "hexadecimal_integer".to_owned(),
                    expected: PRIVATE_ADDRESS,
                    refusal: None,
                    reached_target: true,
                },
            ]
        })
    );
}

#[test]
fn a_runner_reports_a_refusal_that_came_after_a_connection_reached_the_target() {
    let table = SsrfTable::parse(LOOPBACK_CASES).unwrap();

    let result = run_cases(&table.cases[..1], |case| Outcome {
        refusal: Some(PRIVATE_ADDRESS),
        connections: case.target.into_iter().collect(),
    });

    assert_eq!(
        result,
        Err(LetThrough {
            misses: vec![Miss {
                case: "dotted".to_owned(),
                expected: PRIVATE_ADDRESS,
                refusal: Some(PRIVATE_ADDRESS),
                reached_target: true,
            }]
        })
    );
}

#[test]
fn a_runner_reports_a_case_refused_under_another_rule() {
    let table = SsrfTable::parse(LOOPBACK_CASES).unwrap();
    let scheme = Refusal::EgressRefused {
        rule: EgressRule::Scheme,
    };

    let result = run_cases(&table.cases[..1], |_| Outcome::refused(scheme.clone()));

    assert_eq!(
        result,
        Err(LetThrough {
            misses: vec![Miss {
                case: "dotted".to_owned(),
                expected: PRIVATE_ADDRESS,
                refusal: Some(scheme.clone()),
                reached_target: false,
            }]
        })
    );
}

#[test]
fn a_connection_to_the_ipv4_address_an_ipv4_mapped_target_embeds_reached_the_target() {
    let table = SsrfTable::parse(
        r#"
        [[cases]]
        name = "ipv4_mapped"
        family = "ipv6_form"
        url = "http://[::ffff:127.0.0.1]/"
        target = "::ffff:127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();

    let result = run_cases(&table.cases, |_| Outcome {
        refusal: Some(PRIVATE_ADDRESS),
        connections: vec![address("127.0.0.1")],
    });

    assert_eq!(
        result,
        Err(LetThrough {
            misses: vec![Miss {
                case: "ipv4_mapped".to_owned(),
                expected: PRIVATE_ADDRESS,
                refusal: Some(PRIVATE_ADDRESS),
                reached_target: true,
            }]
        })
    );
}

#[test]
fn a_runner_passes_a_layer_that_refuses_every_case_before_any_connection() {
    let table = SsrfTable::parse(LOOPBACK_CASES).unwrap();

    let result = run_cases(&table.cases, |_| Outcome::refused(PRIVATE_ADDRESS));

    assert_eq!(result, Ok(()));
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
