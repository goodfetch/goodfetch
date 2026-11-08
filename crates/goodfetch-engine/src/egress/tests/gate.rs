//! A connection's whole egress check through the [`Gate`] (ADR 0017): its
//! URL before any lookup, then every address its host resolves to before
//! any connection, in that order, for each egress class.
//!
//! Covers: error:egress_refused, setting:egress.private_ranges, setting:egress.no_private_endpoints
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::cell::Cell;
use std::net::SocketAddr;

use goodfetch_protocol::egress::EgressRule;

use super::{policy, refused, sockets};
use crate::egress::{EndpointPolicy, Gate, Policy, Refused, RequestKind, Stopped};

/// A Session's gate for its own HTTP client's requests.
fn session(policy: &Policy) -> Gate<'_> {
    Gate::Session {
        policy,
        kind: RequestKind::Fetch,
    }
}

/// What passing a URL through a gate came to: the addresses it may try,
/// or where it stopped.
type Outcome = Result<Vec<SocketAddr>, Stopped<()>>;

/// The hosts and ports the resolver was asked for.
type Asked = Vec<(String, u16)>;

/// What passing `url` through `gate` came to, the resolver answering
/// `answers`, and the hosts and ports it was asked for.
fn passed(gate: Gate<'_>, url: &str, answers: &[&str]) -> (Outcome, Asked) {
    let asked = Cell::new(Vec::new());
    let answers = sockets(answers);
    let passed = goodfetch_test_support::runtime().block_on(gate.pass(url, async |host, port| {
        let mut so_far = asked.take();
        so_far.push((host.to_owned(), port));
        asked.set(so_far);
        Ok(answers.clone())
    }));
    (passed.map(|passed| passed.addresses), asked.take())
}

#[test]
fn a_url_the_domain_policy_refuses_is_never_looked_up() {
    let session_policy = policy(&["example.test"], &[]);

    let (passed, asked) = passed(
        session(&session_policy),
        "https://other.test/",
        &["93.184.215.14"],
    );

    assert!(
        matches!(&passed, Err(Stopped::Refused(refused)) if refused.setting == Some("egress.allowed_domains")),
        "{passed:?}"
    );
    assert_eq!(asked, Vec::new());
}

#[test]
fn an_allowed_url_is_looked_up_by_its_host_and_port_and_answers_its_judged_addresses() {
    let session_policy = policy(&["example.test"], &[]);

    let (passed, asked) = passed(
        session(&session_policy),
        "https://WWW.Example.test.:8443/path",
        &["93.184.215.14"],
    );

    assert_eq!(
        (passed.unwrap(), asked),
        (
            sockets(&["93.184.215.14"]),
            vec![("www.example.test".to_owned(), 8443)]
        )
    );
}

#[test]
fn an_answer_a_page_may_not_reach_refuses_the_connection() {
    let session_policy = policy(&["*"], &[]);

    let (passed, _) = passed(
        session(&session_policy),
        "https://example.test/",
        &["93.184.215.14", "10.0.0.1"],
    );

    let Err(Stopped::AddressRefused(refused)) = passed else {
        panic!("the private answer refuses the connection: {passed:?}");
    };
    assert_eq!(
        refused,
        Refused {
            address: Some("10.0.0.1".parse().unwrap()),
            ..super::refused(
                "https://example.test/",
                EgressRule::PrivateAddress,
                Some("egress.private_ranges")
            )
        }
    );
}

#[test]
fn a_host_the_private_ranges_name_reaches_its_private_answers_whatever_its_spelling() {
    let session_policy = policy(&["*"], &[])
        .with_private_ranges(&["intranet.test"])
        .unwrap();
    let gate = session(&session_policy);

    let by_url = passed(gate, "http://Intranet.Test./", &["10.0.0.1"]).0;
    let by_host = gate
        .judge_host("INTRANET.test", "http://intranet.test/")
        .and_then(|judged| judged.addresses(&sockets(&["10.0.0.1"])));

    assert_eq!((by_url.is_ok(), by_host), (true, Ok(())));
}

#[test]
fn a_host_a_connector_holds_alone_is_judged_by_the_domain_policy_first() {
    let session_policy = policy(&["*"], &["blocked.test"]);

    let judged =
        session(&session_policy).judge_host("www.blocked.test", "http://www.blocked.test/");

    assert_eq!(
        judged.map(|_| ()),
        Err(refused(
            "http://www.blocked.test/",
            EgressRule::DomainPolicy,
            Some("egress.blocked_domains")
        ))
    );
}

#[test]
fn an_endpoint_reaches_a_private_address_unless_no_private_endpoints_refuses_it() {
    let open = passed(
        Gate::Endpoint(EndpointPolicy::new(false)),
        "http://collector.test:4318/",
        &["127.0.0.1"],
    )
    .0;
    let closed = passed(
        Gate::Endpoint(EndpointPolicy::new(true)),
        "http://collector.test:4318/",
        &["127.0.0.1"],
    )
    .0;

    assert!(open.is_ok(), "{open:?}");
    assert!(
        matches!(&closed, Err(Stopped::AddressRefused(refused)) if refused.setting == Some("egress.no_private_endpoints")),
        "{closed:?}"
    );
}

#[test]
fn a_resolver_that_finds_nothing_stops_the_connection_with_its_failure() {
    let session_policy = policy(&["*"], &[]);

    let stopped = goodfetch_test_support::runtime().block_on(
        session(&session_policy).pass("https://example.test/", async |_, _| {
            Err::<Vec<SocketAddr>, _>("no address")
        }),
    );

    assert!(
        matches!(stopped, Err(Stopped::Unresolved("no address"))),
        "{stopped:?}"
    );
}
