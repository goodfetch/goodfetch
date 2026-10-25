#![expect(
    clippy::disallowed_types,
    reason = "the test host's interfaces change between Sessions behind a lock no loom test reaches"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Mutex;

use goodfetch_protocol::egress::LocalAddressFailure;
use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;

use super::host::routed_locally;
use super::{
    BlockSizes, Host, Ipv6Prefix, LocalAddresses, Rotation, SessionAddresses, Unavailable,
    block_of, in_prefix, spread,
};

/// A host as a table: the addresses it holds, its interfaces, and the
/// prefixes it binds any address of.
#[derive(Default)]
struct Table {
    held: BTreeSet<IpAddr>,
    interfaces: Mutex<BTreeMap<String, Vec<IpAddr>>>,
    any_of: Vec<Ipv6Prefix>,
}

impl Table {
    fn holding(addresses: &[&str]) -> Self {
        Self {
            held: addresses.iter().map(|text| text.parse().unwrap()).collect(),
            ..Self::default()
        }
    }

    fn with_interface(self, name: &str, addresses: &[&str]) -> Self {
        self.set_interface(name, addresses);
        self
    }

    fn set_interface(&self, name: &str, addresses: &[&str]) {
        self.interfaces.lock().unwrap().insert(
            name.to_owned(),
            addresses.iter().map(|text| text.parse().unwrap()).collect(),
        );
    }
}

impl Host for Table {
    fn holds(&self, address: IpAddr) -> bool {
        self.held.contains(&address)
            || self
                .interfaces
                .lock()
                .unwrap()
                .values()
                .flatten()
                .any(|held| *held == address)
    }

    fn interface(&self, name: &str) -> Option<Vec<IpAddr>> {
        self.interfaces.lock().unwrap().get(name).cloned()
    }

    fn addresses(&self) -> Vec<IpAddr> {
        let interfaces = self.interfaces.lock().unwrap();
        self.held
            .iter()
            .copied()
            .chain(interfaces.values().flatten().copied())
            .collect()
    }

    fn binds_any_of(&self, prefix: Ipv6Prefix) -> bool {
        self.any_of
            .iter()
            .any(|bound| bound.length <= prefix.length && bound.contains(prefix.network))
    }
}

fn rotation(value: &serde_json::Value) -> Rotation {
    Rotation::new(LocalAddresses::parse(value).unwrap(), BlockSizes::default())
}

fn prefix(text: &str) -> Ipv6Prefix {
    match LocalAddresses::parse(&json!(text)).unwrap() {
        LocalAddresses::Prefix(prefix) => prefix,
        other => panic!("a prefix, found {other:?}"),
    }
}

/// `text`'s address, none when it is none.
fn v4(text: &str) -> Option<Ipv4Addr> {
    text.parse().ok()
}

/// `text`'s address, none when it is none.
fn v6(text: &str) -> Option<Ipv6Addr> {
    text.parse().ok()
}

/// Each Session binds one address per family for its life; the next one
/// takes the next block.
/// Covers: story:4-44, setting:egress.local_addresses
#[test]
fn each_session_holds_one_address_of_each_family_listed() {
    let host = Table::holding(&["192.0.2.10", "198.51.100.10", "2001:db8:1::10"]);
    let rotation = rotation(&json!(["192.0.2.10", "198.51.100.10", "2001:db8:1::10"]));

    rotation.check(&host).unwrap();
    let first = rotation.next_session(&host).unwrap();
    let second = rotation.next_session(&host).unwrap();

    assert_eq!(
        (first, second),
        (
            SessionAddresses {
                ipv4: v4("192.0.2.10"),
                ipv6: v6("2001:db8:1::10")
            },
            SessionAddresses {
                ipv4: v4("198.51.100.10"),
                ipv6: v6("2001:db8:1::10")
            },
        )
    );
}

/// Covers: story:4-46, error:local_address_unavailable
#[rstest]
#[case::an_address_the_host_does_not_hold(
    json!(["192.0.2.10", "192.0.2.99"]),
    "192.0.2.99",
    LocalAddressFailure::NotHeld
)]
#[case::an_interface_the_host_does_not_have(json!("wlan9"), "wlan9", LocalAddressFailure::UnknownInterface)]
#[case::an_interface_with_only_a_link_local_address(
    json!("bare0"),
    "bare0",
    LocalAddressFailure::InterfaceWithoutAddress
)]
#[case::a_prefix_with_no_address_of_the_host(
    json!("2001:db8:ffff::/64"),
    "2001:db8:ffff::/64",
    LocalAddressFailure::PrefixUnusable
)]
fn a_value_that_yields_no_usable_address_refuses_the_run(
    #[case] value: serde_json::Value,
    #[case] entry: &str,
    #[case] failure: LocalAddressFailure,
) {
    let host =
        Table::holding(&["192.0.2.10", "2001:db8:1::10"]).with_interface("bare0", &["fe80::1"]);

    assert_eq!(
        rotation(&value).check(&host),
        Err(Unavailable {
            entry: entry.to_owned(),
            failure
        })
    );
}

/// A prefix covers the host's own addresses inside it where the host binds
/// no other, and any of its addresses where it does.
/// Covers: story:4-44, story:4-47
#[test]
fn a_prefix_gives_the_hosts_addresses_inside_it_or_any_of_them_where_bindable() {
    let assigned = Table::holding(&["2001:db8:1:5::7", "2001:db8:2::1", "192.0.2.10"]);
    let bindable = Table {
        any_of: vec![prefix("2001:db8:1::/48")],
        ..Table::default()
    };
    let on_assigned = rotation(&json!("2001:db8:1::/48"));
    let on_bindable = rotation(&json!("2001:db8:1::/48"));

    let assigned_sessions = [
        on_assigned.next_session(&assigned).unwrap(),
        on_assigned.next_session(&assigned).unwrap(),
    ];
    let bindable_sessions = [
        on_bindable.next_session(&bindable).unwrap(),
        on_bindable.next_session(&bindable).unwrap(),
    ];

    assert_eq!(
        (assigned_sessions, bindable_sessions),
        (
            [
                SessionAddresses {
                    ipv4: None,
                    ipv6: v6("2001:db8:1:5::7")
                },
                SessionAddresses {
                    ipv4: None,
                    ipv6: v6("2001:db8:1:5::7")
                },
            ],
            [
                SessionAddresses {
                    ipv4: None,
                    ipv6: v6("2001:db8:1::1")
                },
                SessionAddresses {
                    ipv4: None,
                    ipv6: v6("2001:db8:1:1::1")
                },
            ],
        )
    );
}

/// An interface's addresses are read when each Session starts, not when
/// the Run did.
/// Covers: story:4-44
#[test]
fn an_interface_resolves_to_its_current_addresses_at_each_session_start() {
    let host = Table::default().with_interface("eth1", &["192.0.2.10", "fe80::1"]);
    let rotation = rotation(&json!("eth1"));
    rotation.check(&host).unwrap();

    let before = rotation.next_session(&host).unwrap();
    host.set_interface("eth1", &["198.51.100.20", "2001:db8::20"]);
    let after = rotation.next_session(&host).unwrap();
    host.set_interface("eth1", &[]);
    let emptied = rotation.next_session(&host);

    assert_eq!(
        (before, after, emptied),
        (
            SessionAddresses {
                ipv4: v4("192.0.2.10"),
                ipv6: None
            },
            SessionAddresses {
                ipv4: v4("198.51.100.20"),
                ipv6: v6("2001:db8::20")
            },
            Err(Unavailable {
                entry: "eth1".to_owned(),
                failure: LocalAddressFailure::InterfaceWithoutAddress,
            }),
        )
    );
}

/// The real loopback interface, read through the OS.
/// Covers: story:4-44
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn the_loopback_interface_resolves_to_its_addresses_through_the_os() {
    let name = if cfg!(target_os = "linux") {
        "lo"
    } else {
        "lo0"
    };
    let rotation = rotation(&json!(name));

    let session = rotation.next_session(&super::System).unwrap();

    assert_eq!(session.ipv4, v4("127.0.0.1"));
}

#[rstest]
#[case::a_local_route_covering_the_prefix(
    "20010db8000100000000000000000000 30 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000001 00000000 80200001       lo",
    true
)]
#[case::a_route_that_is_not_local(
    "20010db8000100000000000000000000 30 00000000000000000000000000000000 00 00000000000000000000000000000000 00000400 00000001 00000000 00000001     eth0",
    false
)]
#[case::a_local_route_of_another_prefix(
    "20010db8000200000000000000000000 30 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000001 00000000 80200001       lo",
    false
)]
#[case::a_local_route_inside_the_prefix(
    "20010db8000100010000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000000 00000001 00000000 80200001       lo",
    false
)]
fn linux_reads_a_local_route_covering_the_prefix(#[case] line: &str, #[case] routed: bool) {
    assert_eq!(routed_locally(line, prefix("2001:db8:1::/48")), routed);
}

/// Addresses and a block size, the default or another, for the property
/// below.
fn addresses_and_block() -> impl Strategy<Value = (Vec<IpAddr>, u8)> {
    prop_oneof![
        (
            prop::collection::vec(any::<u32>().prop_map(|bits| IpAddr::V4(bits.into())), 1..40),
            prop_oneof![Just(24_u8), 0_u8..=32],
        ),
        (
            prop::collection::vec(
                (0_u128..64, any::<u16>()).prop_map(|(block, host)| {
                    IpAddr::V6(Ipv6Addr::from(
                        (0x2001_0db8_u128 << 96) | (block << 64) | u128::from(host),
                    ))
                }),
                1..40,
            ),
            prop_oneof![Just(64_u8), 48_u8..=128],
        ),
    ]
    .prop_map(|(mut addresses, block)| {
        addresses.sort_unstable();
        addresses.dedup();
        (addresses, block)
    })
}

proptest! {
    /// New Sessions take the blocks round-robin: any run of as many
    /// consecutive Sessions as there are blocks visits every block once,
    /// with the built-in block size and with a set one.
    /// Covers: story:4-47, setting:egress.rotation_block.ipv4, setting:egress.rotation_block.ipv6
    #[test]
    fn new_sessions_spread_across_every_block_round_robin(
        (addresses, block) in addresses_and_block(),
        first in 0_u64..1_000,
    ) {
        let blocks: BTreeSet<u128> = addresses.iter().map(|address| block_of(*address, block)).collect();
        let count = u64::try_from(blocks.len()).unwrap();

        let taken: Vec<IpAddr> = (first..first + count)
            .map(|session| spread(&addresses, block, session).unwrap())
            .collect();
        let visited: BTreeSet<u128> = taken.iter().map(|address| block_of(*address, block)).collect();

        prop_assert!(taken.iter().all(|address| addresses.contains(address)));
        prop_assert_eq!(visited, blocks);
    }

    /// Any address of a bindable prefix: consecutive Sessions take distinct
    /// blocks of it, each address inside the prefix and never a block's
    /// own first address.
    /// Covers: story:4-47, setting:egress.rotation_block.ipv6
    #[test]
    fn sessions_on_a_bindable_prefix_take_its_blocks_in_turn(
        length in 16_u8..=64,
        extra in prop_oneof![Just(None), (0_u8..=48).prop_map(Some)],
        network in any::<u128>(),
        first in 0_u64..1_000,
    ) {
        let block = extra.map_or(64, |extra| length + extra);
        let prefix = Ipv6Prefix {
            network: Ipv6Addr::from(network & !(u128::MAX >> length)),
            length,
        };
        let distinct = (1_u64 << (u32::from(block - length)).min(10)).min(256);

        let taken: Vec<Ipv6Addr> = (first..first + distinct)
            .map(|session| in_prefix(prefix, block, session))
            .collect();
        let blocks: BTreeSet<u128> =
            taken.iter().map(|address| block_of(IpAddr::V6(*address), block)).collect();

        prop_assert!(taken.iter().all(|address| prefix.contains(*address)));
        prop_assert_eq!(u64::try_from(blocks.len()).unwrap(), distinct);
        let past_each_blocks_own = taken
            .iter()
            .all(|address| block == 128 || u128::from(*address) & (u128::MAX >> block) != 0);
        prop_assert!(past_each_blocks_own);
    }
}

/// A Session drawn by its ordinal takes what the rotation gave it in turn,
/// so a Session reopened from run state keeps its addresses, whichever
/// Sessions were drawn since.
/// Covers: story:4-44
#[test]
fn a_session_drawn_again_by_its_ordinal_keeps_its_addresses() {
    let host = Table::holding(&["192.0.2.10", "198.51.100.10", "203.0.113.10"]);
    let rotation = rotation(&json!(["192.0.2.10", "198.51.100.10", "203.0.113.10"]));
    let in_turn: Vec<SessionAddresses> = (0..3)
        .map(|_| rotation.next_session(&host).unwrap())
        .collect();

    let again: Vec<SessionAddresses> = [2, 0, 1]
        .into_iter()
        .map(|ordinal| rotation.session(&host, ordinal).unwrap())
        .collect();

    assert_eq!(again, vec![in_turn[2], in_turn[0], in_turn[1]]);
}
