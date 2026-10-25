//! The SSRF case table in process, through `egress`'s interface (ADR 0067;
//! `TESTING.md`'s in-process layer, every OS): each case's URL judged as a
//! request, then the addresses its host stands for (its literal address, or
//! the `ScriptedResolver`'s answers in turn) judged as the connection that
//! would try them. The addresses a judgment let through are those a
//! connection would reach; nothing connects. A redirect's location is
//! judged as the next hop's request, and an upstream proxy's case as the
//! request the engine judges before the proxy sees it. Rebinding is a name
//! whose answer changes between two fetches. The cases each runner takes
//! are its entry's in the table's layers (`classification`).
//!
//! Covers: story:4-8, story:4-9, story:4-89

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use goodfetch_protocol::egress::EgressRule;
use goodfetch_test_support::{
    CaseDomains, Outcome, Redirect, Refusal, ResolverHandle, Runner, ScriptedResolver, SsrfCase,
    SsrfTable,
};
use url::Host;

use crate::egress::{Judgment, Policy, Refused, Request, RequestKind, judge, judge_addresses};

/// The public stand-in: a globally reachable address, which no test
/// connects to.
const PUBLIC: IpAddr = IpAddr::V4(Ipv4Addr::new(93, 184, 215, 14));

/// The table's layer this module runs.
const LAYER: &str = "classification";

/// The case's domain policy, every host allowed when it sets none.
fn session(case: &SsrfCase) -> Policy {
    let domains = case.domains.clone().unwrap_or(CaseDomains {
        allowed: vec!["*".to_owned()],
        blocked: Vec::new(),
    });
    Policy::new(&domains.allowed, &domains.blocked).unwrap()
}

/// One fetch of `url`: the addresses a connection would try, or the
/// refusal.
fn fetch_once(
    url: &str,
    session: &Policy,
    resolver: &ResolverHandle,
) -> Result<Vec<IpAddr>, Refused> {
    let request = Request {
        url,
        kind: RequestKind::Fetch,
    };
    let parsed = match judge(&request, session) {
        Judgment::Allowed(parsed) => parsed,
        Judgment::Refused(refused) => return Err(refused),
    };
    let addresses = match parsed.host() {
        Some(Host::Ipv4(address)) => vec![IpAddr::V4(address)],
        Some(Host::Ipv6(address)) => vec![IpAddr::V6(address)],
        Some(Host::Domain(name)) => resolver.lookup(name),
        None => Vec::new(),
    };
    let port = parsed.port_or_known_default().unwrap_or(80);
    let sockets: Vec<SocketAddr> = addresses
        .iter()
        .map(|address| SocketAddr::new(*address, port))
        .collect();
    judge_addresses(url, &sockets, session)?;
    Ok(addresses)
}

/// Where the case's redirect sends its next hop, if it has one.
fn location(case: &SsrfCase) -> Option<&str> {
    case.redirect.as_ref().map(|redirect| match redirect {
        Redirect::Status { location, .. }
        | Redirect::RefreshHeader { location }
        | Redirect::MetaRefresh { location }
        | Redirect::Script { location } => location.as_str(),
    })
}

/// Each of the case's fetches in turn, a redirect's hop after its fetch,
/// up to the first refusal.
fn fetches(case: &SsrfCase) -> Vec<Result<Vec<IpAddr>, Refused>> {
    let resolver = ScriptedResolver::new(case.script(PUBLIC));
    let handle = resolver.handle();
    let session = session(case);
    let mut fetched = Vec::new();
    let hops = std::iter::once(case.url.as_str()).chain(location(case));
    for url in std::iter::repeat_n(hops, case.fetches as usize).flatten() {
        let result = fetch_once(url, &session, &handle);
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
fn every_case_of_the_table_is_refused_before_a_connection_would_try_its_target() {
    let runner = Runner::committed(
        LAYER,
        file!(),
        "every_case_of_the_table_is_refused_before_a_connection_would_try_its_target",
    )
    .unwrap();
    // The positive controls: a name whose only answer is public is let
    // through to it, and so is a redirect's hop to it, which the runner
    // sees.
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "public_only"
        family = "mixed_answers"
        url = "http://public.ssrf.test/"
        lookups = [{ name = "public.ssrf.test", answers = ["public"] }]
        target = "public"
        refusal = { code = "egress_refused", rule = "private_address" }

        [[cases]]
        name = "redirect_to_a_public_name"
        family = "redirect"
        url = "http://redirect.ssrf.test/"
        lookups = [
            { name = "redirect.ssrf.test", answers = ["public"] },
            { name = "public.ssrf.test", answers = ["public"] },
        ]
        redirect = { kind = "status", status = 302, location = "http://public.ssrf.test/" }
        target = "public"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();

    let result = runner.run(&controls.cases, PUBLIC, run_case);

    assert!(
        runner.cases().len() > 100,
        "only {} cases",
        runner.cases().len()
    );
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
    let runner = Runner::committed(
        LAYER,
        file!(),
        "a_rebinding_name_is_refused_on_the_connection_after_its_answer_changes",
    )
    .unwrap();
    let cases = runner.cases();

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
