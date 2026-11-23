//! Tests that need the extra loopback addresses `127.0.0.2` to `127.0.0.9`
//! (TESTING.md, Storage, network and OS services): a host without them skips
//! this module.
//!
//! Two Toxiproxy proxies, each on a loopback address of its own, as two
//! engines' links are (ADR 0060): a fault on one leaves the other as it was.

use std::net::{IpAddr, Ipv4Addr};

use goodfetch_test_support::{TestEnv, Toxiproxy};

use crate::toxiproxy::{ANSWER_DEADLINE, Reach, SILENCE, echo, reach};

const FIRST: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
const SECOND: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 3));

#[test]
fn two_proxies_on_two_loopback_addresses_act_independently() {
    let env = TestEnv::new();
    let toxiproxy = Toxiproxy::start(&env).unwrap();
    let first = toxiproxy.proxy("first", echo(), FIRST).unwrap();
    let second = toxiproxy.proxy("second", echo(), SECOND).unwrap();
    assert_eq!((first.ip(), second.ip()), (FIRST, SECOND));

    toxiproxy.cut("first").unwrap();
    assert_eq!(reach(first, ANSWER_DEADLINE), Reach::Refused);
    assert_eq!(reach(second, ANSWER_DEADLINE), Reach::Answered);

    toxiproxy.heal("first").unwrap();
    toxiproxy.partition("second").unwrap();
    assert_eq!(reach(first, ANSWER_DEADLINE), Reach::Answered);
    assert_eq!(reach(second, SILENCE), Reach::Silent);

    toxiproxy.heal("second").unwrap();
    assert_eq!(reach(second, ANSWER_DEADLINE), Reach::Answered);
}
