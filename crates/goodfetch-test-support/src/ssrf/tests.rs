use std::net::IpAddr;

use rstest::rstest;

use super::{
    Answer, CaseDomains, CoverageGap, CoverageGaps, EgressRule, Family, LetThrough, Miss, Outcome,
    Refusal, SsrfCase, SsrfTable, SsrfTableError, run_cases,
};
use crate::resolver::ScriptedResolver;
use crate::special_purpose::SpecialPurposeRegistries;

fn address(text: &str) -> IpAddr {
    text.parse().unwrap()
}

/// The fixture layers' public stand-in.
const PUBLIC: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 7));

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
            Some(Answer::Address(address("127.0.0.1"))),
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
        Outcome::connected(case.target_address(PUBLIC))
    }
}

#[test]
fn a_runner_reports_every_case_its_layer_lets_through() {
    let table = SsrfTable::parse(LOOPBACK_CASES).unwrap();

    let result = run_cases(&table.cases, PUBLIC, dotted_text_filter);

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

    let result = run_cases(&table.cases[..1], PUBLIC, |case| Outcome {
        refusal: Some(PRIVATE_ADDRESS),
        connections: case.target_address(PUBLIC).into_iter().collect(),
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

    let result = run_cases(&table.cases[..1], PUBLIC, |_| {
        Outcome::refused(scheme.clone())
    });

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

    let result = run_cases(&table.cases, PUBLIC, |_| Outcome {
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

    let result = run_cases(&table.cases, PUBLIC, |_| Outcome::refused(PRIVATE_ADDRESS));

    assert_eq!(result, Ok(()));
}

/// A registry of two blocks that are not globally reachable and one that
/// is.
const REGISTRY: &str = "Address Block,Name,RFC,Allocation Date,Termination Date,Source,Destination,Forwardable,Globally Reachable,Reserved-by-Protocol\r
10.0.0.0/8,Private-Use,[RFC 1918],1996-02,N/A,True,True,True,False,False\r
fe80::/10,Link-Local Unicast,[RFC 4291],2006-02,N/A,True,True,False,False,True\r
192.31.196.0/24,AS112-v4,[RFC 7535],2014-12,N/A,True,True,True,True,False\r
";

/// A special-purpose case of `block` aimed at `target`.
fn block_case(name: &str, block: &str, target: &str) -> String {
    format!(
        r#"
        [[cases]]
        name = "{name}"
        family = "special_purpose_block"
        url = "http://example.ssrf.test/"
        target = "{target}"
        block = "{block}"
        refusal = {{ code = "egress_refused", rule = "private_address" }}
        "#
    )
}

fn coverage(table: &str) -> Result<(), CoverageGaps> {
    let registries = SpecialPurposeRegistries::parse_csv(REGISTRY).unwrap();
    SsrfTable::parse(table).unwrap().coverage(&registries)
}

#[test]
fn a_table_with_a_case_for_every_block_not_globally_reachable_covers_the_registries() {
    let table = block_case("private_use", "10.0.0.0/8", "10.0.0.1")
        + &block_case("link_local", "fe80::/10", "fe80::1");

    let result = coverage(&table);

    assert_eq!(result, Ok(()));
}

#[test]
fn a_block_not_globally_reachable_without_a_case_is_a_gap() {
    let table = block_case("private_use", "10.0.0.0/8", "10.0.0.1");

    let result = coverage(&table);

    assert_eq!(
        result,
        Err(CoverageGaps {
            gaps: vec![CoverageGap::Uncovered {
                block: "fe80::/10".to_owned()
            }]
        })
    );
}

#[test]
fn a_case_naming_a_block_the_registries_do_not_list_is_a_gap() {
    let table = block_case("private_use", "10.0.0.0/8", "10.0.0.1")
        + &block_case("link_local", "fe80::/10", "fe80::1")
        + &block_case("site_local", "fec0::/10", "fec0::1");

    let result = coverage(&table);

    assert_eq!(
        result,
        Err(CoverageGaps {
            gaps: vec![CoverageGap::UnknownBlock {
                case: "site_local".to_owned(),
                block: "fec0::/10".to_owned()
            }]
        })
    );
}

#[test]
fn a_case_whose_target_lies_outside_its_block_is_a_gap() {
    let table = block_case("private_use", "10.0.0.0/8", "11.0.0.1")
        + &block_case("link_local", "fe80::/10", "fe80::1");

    let result = coverage(&table);

    assert_eq!(
        result,
        Err(CoverageGaps {
            gaps: vec![CoverageGap::OutsideBlock {
                case: "private_use".to_owned(),
                block: "10.0.0.0/8".to_owned()
            }]
        })
    );
}

#[test]
fn two_cases_of_one_name_are_a_gap() {
    let table = block_case("private_use", "10.0.0.0/8", "10.0.0.1")
        + &block_case("private_use", "fe80::/10", "fe80::1");

    let result = coverage(&table);

    assert_eq!(
        result,
        Err(CoverageGaps {
            gaps: vec![CoverageGap::DuplicateName {
                case: "private_use".to_owned()
            }]
        })
    );
}

#[test]
fn the_committed_table_covers_the_committed_iana_registries() {
    let registries = SpecialPurposeRegistries::committed().unwrap();
    let table = SsrfTable::committed().unwrap();

    let result = table.coverage(&registries);

    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}

#[rstest]
#[case::encoding(Family::Encoding)]
#[case::ipv6_form(Family::Ipv6Form)]
#[case::special_purpose_block(Family::SpecialPurposeBlock)]
#[case::mixed_answers(Family::MixedAnswers)]
#[case::redirect(Family::Redirect)]
#[case::rebinding(Family::Rebinding)]
#[case::upstream_proxy(Family::UpstreamProxy)]
#[case::plugin_traffic(Family::PluginTraffic)]
#[case::lent_page_client(Family::LentPageClient)]
#[case::operator_endpoint(Family::OperatorEndpoint)]
#[case::domain_policy(Family::DomainPolicy)]
#[case::borrowed_browser(Family::BorrowedBrowser)]
fn the_committed_table_has_cases_of_every_family(#[case] family: Family) {
    let table = SsrfTable::committed().unwrap();

    let cases = table.cases_of(family).count();

    assert_ne!(cases, 0);
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

#[test]
fn a_domain_policy_case_reads_the_policy_it_runs_under() {
    let table = SsrfTable::parse(
        r#"
        [[cases]]
        name = "blocked_subdomain"
        family = "domain_policy"
        url = "http://shop.scope.test/cart"
        domains = { allowed = ["scope.test"], blocked = ["shop.scope.test"] }
        lookups = [{ name = "shop.scope.test", answers = ["public"] }]
        target = "public"
        refusal = { code = "egress_refused", rule = "domain_policy" }

        [[cases]]
        name = "blocked_host"
        family = "domain_policy"
        url = "http://elsewhere.test/"
        domains = { blocked = ["elsewhere.test"] }
        target = "public"
        refusal = { code = "egress_refused", rule = "domain_policy" }
        "#,
    )
    .unwrap();
    let domain_policy = Refusal::EgressRefused {
        rule: EgressRule::DomainPolicy,
    };

    assert_eq!(
        table
            .cases
            .iter()
            .map(|case| (
                case.domains.clone(),
                case.target_address(PUBLIC),
                &case.refusal
            ))
            .collect::<Vec<_>>(),
        [
            (
                Some(CaseDomains {
                    allowed: vec!["scope.test".to_owned()],
                    blocked: vec!["shop.scope.test".to_owned()],
                }),
                Some(PUBLIC),
                &domain_policy,
            ),
            (
                Some(CaseDomains {
                    allowed: vec!["*".to_owned()],
                    blocked: vec!["elsewhere.test".to_owned()],
                }),
                Some(PUBLIC),
                &domain_policy,
            ),
        ]
    );
}

#[test]
fn a_connection_to_the_public_stand_in_reached_a_target_written_public() {
    let table = SsrfTable::parse(
        r#"
        [[cases]]
        name = "host_not_allowed"
        family = "domain_policy"
        url = "http://elsewhere.test/"
        domains = { allowed = ["scope.test"] }
        target = "public"
        refusal = { code = "egress_refused", rule = "domain_policy" }
        "#,
    )
    .unwrap();
    let domain_policy = Refusal::EgressRefused {
        rule: EgressRule::DomainPolicy,
    };

    let result = run_cases(&table.cases, PUBLIC, |_| Outcome {
        refusal: Some(domain_policy.clone()),
        connections: vec![PUBLIC],
    });

    assert_eq!(
        result,
        Err(LetThrough {
            misses: vec![Miss {
                case: "host_not_allowed".to_owned(),
                expected: domain_policy.clone(),
                refusal: Some(domain_policy.clone()),
                reached_target: true,
            }]
        })
    );
}
