//! The SSRF case table in process, through `egress`'s interface (ADR 0067;
//! `TESTING.md`'s in-process layer, every OS): each case's URL judged as a
//! request, then the addresses its host stands for (its literal address, or
//! the `ScriptedResolver`'s answers in turn) judged as the connection that
//! would try them. The addresses a judgment let through are those a
//! connection would reach; nothing connects. Rebinding is a name whose
//! answer changes between two fetches.
//!
//! Covers: story:4-8, story:4-9

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use goodfetch_protocol::egress::EgressRule;
use goodfetch_test_support::{
    CaseDomains, Family, Outcome, Refusal, ResolverHandle, ScriptedResolver, SsrfCase, SsrfTable,
    run_cases,
};
use url::Host;

use crate::egress::{Judgment, Policy, Refused, Request, RequestKind, judge, judge_addresses};

/// The public stand-in: a globally reachable address, which no test
/// connects to.
const PUBLIC: IpAddr = IpAddr::V4(Ipv4Addr::new(93, 184, 215, 14));

/// The families whose cases a request of the run meets directly, with no
/// redirect, proxy, Plugin or Lent page.
const DIRECT: [Family; 6] = [
    Family::Encoding,
    Family::Ipv6Form,
    Family::SpecialPurposeBlock,
    Family::MixedAnswers,
    Family::Rebinding,
    Family::DomainPolicy,
];

/// The case's domain policy, every host allowed when it sets none.
fn session(case: &SsrfCase) -> Policy {
    let domains = case.domains.clone().unwrap_or(CaseDomains {
        allowed: vec!["*".to_owned()],
        blocked: Vec::new(),
    });
    Policy::new(&domains.allowed, &domains.blocked).unwrap()
}

/// One fetch of the case's URL: the addresses a connection would try, or
/// the refusal.
fn fetch_once(
    case: &SsrfCase,
    session: &Policy,
    resolver: &ResolverHandle,
) -> Result<Vec<IpAddr>, Refused> {
    let request = Request {
        url: &case.url,
        kind: RequestKind::Fetch,
    };
    let url = match judge(&request, session) {
        Judgment::Allowed(url) => url,
        Judgment::Refused(refused) => return Err(refused),
    };
    let addresses = match url.host() {
        Some(Host::Ipv4(address)) => vec![IpAddr::V4(address)],
        Some(Host::Ipv6(address)) => vec![IpAddr::V6(address)],
        Some(Host::Domain(name)) => resolver.lookup(name),
        None => Vec::new(),
    };
    let port = url.port_or_known_default().unwrap_or(80);
    let sockets: Vec<SocketAddr> = addresses
        .iter()
        .map(|address| SocketAddr::new(*address, port))
        .collect();
    judge_addresses(&case.url, &sockets, session)?;
    Ok(addresses)
}

/// Each of the case's fetches in turn, up to the first refusal.
fn fetches(case: &SsrfCase) -> Vec<Result<Vec<IpAddr>, Refused>> {
    let resolver = ScriptedResolver::new(case.script(PUBLIC));
    let handle = resolver.handle();
    let session = session(case);
    let mut fetched = Vec::new();
    for _ in 0..case.fetches {
        let result = fetch_once(case, &session, &handle);
        let refused = result.is_err();
        fetched.push(result);
        if refused {
            break;
        }
    }
    fetched
}

/// The table's form of the rule a refusal names.
fn table_rule(rule: EgressRule) -> goodfetch_test_support::EgressRule {
    match rule {
        EgressRule::DomainPolicy => goodfetch_test_support::EgressRule::DomainPolicy,
        EgressRule::PrivateAddress => goodfetch_test_support::EgressRule::PrivateAddress,
        EgressRule::Scheme => goodfetch_test_support::EgressRule::Scheme,
    }
}

/// The in-process layer's runner: the last fetch's refusal, and every
/// address a fetch let a connection try.
fn run_case(case: &SsrfCase) -> Outcome {
    let fetched = fetches(case);
    Outcome {
        refusal: match fetched.last() {
            Some(Err(refused)) => Some(Refusal::EgressRefused {
                rule: table_rule(refused.rule),
            }),
            _ => None,
        },
        connections: fetched.into_iter().flatten().flatten().collect(),
    }
}

#[test]
fn every_direct_case_of_the_table_is_refused_before_a_connection_would_try_its_target() {
    let table = SsrfTable::committed().unwrap();
    // The positive control: a name whose only answer is public is let
    // through to it, which the runner sees.
    let control = SsrfTable::parse(
        r#"
        [[cases]]
        name = "public_only"
        family = "mixed_answers"
        url = "http://public.ssrf.test/"
        lookups = [{ name = "public.ssrf.test", answers = ["public"] }]
        target = "public"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();
    let cases: Vec<&SsrfCase> = DIRECT
        .iter()
        .flat_map(|family| table.cases_of(*family))
        .collect();

    let controlled = run_cases(&control.cases, PUBLIC, run_case);
    let result = run_cases(cases.iter().copied(), PUBLIC, run_case);

    assert_eq!(
        controlled.map_err(|let_through| {
            let miss = &let_through.misses[0];
            (miss.refusal.clone(), miss.reached_target)
        }),
        Err((None, true))
    );
    assert!(cases.len() > 80, "only {} direct cases", cases.len());
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}

/// One fetch as the rebinding test sees it: the addresses a connection
/// would try, or the address its refusal names.
type Turn = Result<Vec<IpAddr>, Option<IpAddr>>;

/// A name whose answer turns private after its first lookup reaches its
/// public answer once, and its second connection is refused, naming the
/// private address: the check runs on every connection's own lookup.
#[test]
fn a_rebinding_name_is_refused_on_the_connection_after_its_answer_changes() {
    let table = SsrfTable::committed().unwrap();
    let cases: Vec<&SsrfCase> = table.cases_of(Family::Rebinding).collect();

    let fetched: Vec<(String, Vec<Turn>)> = cases
        .iter()
        .map(|case| {
            let fetched = fetches(case)
                .into_iter()
                .map(|fetch| fetch.map_err(|refused| refused.address))
                .collect();
            (case.name.clone(), fetched)
        })
        .collect();

    assert_eq!(cases.len(), 3);
    assert_eq!(
        fetched,
        cases
            .iter()
            .map(|case| (
                case.name.clone(),
                vec![Ok(vec![PUBLIC]), Err(case.target_address(PUBLIC))]
            ))
            .collect::<Vec<_>>()
    );
}
