//! The DNS fixture `serve` starts on its own address (ADR 0067: rebinding
//! is tested against it, never a public rebinding service): every answer
//! has a TTL of 0; the host names the Proving ground serves resolve to its
//! address; a name under `alternating.dns.test` answers the addresses its
//! labels declare in turn, one per query, and one under `mixed.dns.test`
//! answers them all at once. The forward proxies resolve through the same
//! answers.
//! Covers: story:2-82

use std::net::IpAddr;

use serde_json::json;

use crate::clients::{self, DnsAnswer, RecordType};
use crate::proving_ground::ProvingGround;

const SEED: u64 = 311;

/// A public-looking address and a private one, as labels.
const ALTERNATING: &str = "93-184-215-14.127-0-0-1.alternating.dns.test";
const MIXED: &str = "93-184-215-14.127-0-0-1.mixed.dns.test";

fn address(text: &str) -> IpAddr {
    text.parse().expect("an address")
}

#[test]
fn a_host_name_the_proving_ground_serves_resolves_to_its_address() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = clients::dns_query(&proving_ground.dns(), "1.catalog.test", RecordType::A);

    assert_eq!(
        answer,
        DnsAnswer {
            rcode: 0,
            answers: vec![(0, address("127.0.0.1"))],
        }
    );
}

#[test]
fn a_name_nobody_declared_does_not_exist() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = clients::dns_query(&proving_ground.dns(), "nowhere.test", RecordType::A);

    assert_eq!(
        answer,
        DnsAnswer {
            rcode: 3,
            answers: Vec::new(),
        }
    );
}

#[test]
fn an_alternating_name_answers_its_addresses_in_turn() {
    let proving_ground = ProvingGround::start(SEED);
    let dns = proving_ground.dns();

    let answers: Vec<_> = (0..4)
        .map(|_| clients::dns_query(&dns, ALTERNATING, RecordType::A).answers)
        .collect();

    assert_eq!(
        answers,
        [
            vec![(0, address("93.184.215.14"))],
            vec![(0, address("127.0.0.1"))],
            vec![(0, address("93.184.215.14"))],
            vec![(0, address("127.0.0.1"))],
        ]
    );
}

#[test]
fn a_mixed_name_answers_all_its_addresses_at_once() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = clients::dns_query(&proving_ground.dns(), MIXED, RecordType::A);

    assert_eq!(
        answer.answers,
        [(0, address("93.184.215.14")), (0, address("127.0.0.1"))]
    );
}

#[test]
fn an_ipv6_label_answers_only_aaaa_queries() {
    let proving_ground = ProvingGround::start(SEED);
    let dns = proving_ground.dns();
    let name = "00000000000000000000ffff7f000001.127-0-0-1.mixed.dns.test";

    let answers = [
        clients::dns_query(&dns, name, RecordType::Aaaa).answers,
        clients::dns_query(&dns, name, RecordType::A).answers,
    ];

    assert_eq!(
        answers,
        [
            vec![(0, address("::ffff:127.0.0.1"))],
            vec![(0, address("127.0.0.1"))],
        ]
    );
}

#[test]
fn a_proxy_resolves_an_alternating_name_anew_for_each_relay() {
    let proving_ground = ProvingGround::start(SEED);
    let target = proving_ground.target();
    let port = target.rsplit_once(':').expect("a port").1;
    let request =
        format!("GET http://{ALTERNATING}:{port}/ HTTP/1.1\r\nHost: {ALTERNATING}:{port}\r\n\r\n");

    let statuses = [
        clients::raw(&proving_ground.proxy("open"), request.as_bytes()).status(),
        clients::raw(&proving_ground.proxy("open"), request.as_bytes()).status(),
    ];

    let observations = proving_ground.observations();
    let outcomes: Vec<_> = observations["relays"]
        .as_array()
        .expect("relays")
        .iter()
        .map(|relay| json!([relay["destination"], relay["outcome"]]))
        .collect();
    assert_eq!(statuses, [403, 200]);
    assert_eq!(
        outcomes,
        [
            json!([format!("93.184.215.14:{port}"), "destination_prohibited"]),
            json!([target, "relayed"]),
        ]
    );
}
