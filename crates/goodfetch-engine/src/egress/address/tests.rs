//! The resolved-address classification through `egress`'s interface (spec
//! #564's `egress` seam): `classify` against the reference table read from
//! IANA's committed registries by `goodfetch-test-support`, a parser apart
//! from the one `xtask codegen` writes the engine's table with (ADR 0067's
//! differential oracle), and properties of the forms that embed an IPv4
//! address and of zone IDs.
//!
//! Covers: story:4-8, story:4-9

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

use crate::egress::special_purpose::BLOCKS;
use goodfetch_test_support::{Cidr, Reachability, SpecialPurposeBlock, SpecialPurposeRegistries};
use proptest::prelude::*;
use rstest::rstest;

use crate::egress::{AddressClass, classify, classify_socket};

/// The registries as `goodfetch-test-support` reads them.
fn reference() -> Vec<SpecialPurposeBlock> {
    SpecialPurposeRegistries::committed().unwrap().blocks
}

/// A reference block's CIDR text read as its first address and prefix.
fn bounds(block: &SpecialPurposeBlock) -> (IpAddr, u8) {
    let (address, prefix) = block.block.split_once('/').unwrap();
    (address.parse().unwrap(), prefix.parse().unwrap())
}

/// The most specific reference block holding `address`.
fn most_specific(blocks: &[SpecialPurposeBlock], address: IpAddr) -> Option<&SpecialPurposeBlock> {
    blocks
        .iter()
        .filter(|block| Cidr::parse(&block.block).unwrap().contains(address))
        .max_by_key(|block| bounds(block).1)
}

/// The blocks a classification judges by the IPv4 address they embed
/// rather than by their own row (spec #564).
const EMBEDDING: [&str; 5] = [
    "::ffff:0:0/96",
    "64:ff9b::/96",
    "64:ff9b:1::/48",
    "2002::/16",
    "2001::/32",
];

/// The cloud-metadata addresses, refused whatever block holds them.
const METADATA: [&str; 3] = ["169.254.169.254", "100.100.100.200", "fd00:ec2::254"];

/// The first, second, middle and last addresses of `block`.
fn samples(block: &SpecialPurposeBlock) -> Vec<IpAddr> {
    let (first, prefix) = bounds(block);
    match first {
        IpAddr::V4(first) => {
            let size = 1u64 << (32 - u32::from(prefix));
            let base = u64::from(first.to_bits());
            [0, 1, size / 2, size - 1]
                .into_iter()
                .filter(|offset| *offset < size)
                .map(|offset| {
                    IpAddr::V4(Ipv4Addr::from_bits(u32::try_from(base + offset).unwrap()))
                })
                .collect()
        }
        IpAddr::V6(first) => {
            let host_bits = 128 - u32::from(prefix);
            let last = if host_bits == 128 {
                u128::MAX
            } else {
                (1u128 << host_bits) - 1
            };
            [0, 1, last / 2 + 1, last]
                .into_iter()
                .filter(|offset| *offset <= last)
                .map(|offset| IpAddr::V6(Ipv6Addr::from_bits(first.to_bits() + offset)))
                .collect()
        }
    }
}

#[test]
fn the_engines_table_is_the_committed_registries() {
    let table: Vec<(String, &str, Reachability)> = BLOCKS
        .iter()
        .map(|block| {
            (
                format!("{}/{}", block.address, block.prefix),
                block.name,
                match block.globally_reachable {
                    Some(true) => Reachability::Global,
                    Some(false) => Reachability::NotGlobal,
                    None => Reachability::Unstated,
                },
            )
        })
        .collect();
    let blocks = reference();
    let reference: Vec<(String, &str, Reachability)> = blocks
        .iter()
        .map(|block| {
            let (address, prefix) = bounds(block);
            (
                format!("{address}/{prefix}"),
                block.name.as_str(),
                block.globally_reachable,
            )
        })
        .collect();

    assert_eq!(table, reference);
}

/// Every block of the registries but those judged by the address they
/// embed: an address whose most specific block it is classifies as global
/// exactly when the registry says the block is globally reachable, a
/// terminated block's or an unstated one's being refused (fail closed).
#[test]
fn every_registry_block_classifies_as_the_registry_says() {
    let blocks = reference();
    let mut judged = 0;
    let mut wrong = Vec::new();
    for block in blocks
        .iter()
        .filter(|block| !EMBEDDING.contains(&block.block.as_str()))
    {
        for address in samples(block) {
            if most_specific(&blocks, address) != Some(block)
                || METADATA.contains(&address.to_string().as_str())
            {
                continue;
            }
            judged += 1;
            let global = classify(address).is_global();
            if global != (block.globally_reachable == Reachability::Global) {
                wrong.push(format!(
                    "{address} in {}: {:?}",
                    block.block,
                    classify(address)
                ));
            }
        }
    }

    assert_eq!(wrong, Vec::<String>::new());
    assert!(judged > 150, "only {judged} addresses judged");
}

#[rstest]
#[case::aws_gcp_azure_ipv4("169.254.169.254")]
#[case::alibaba("100.100.100.200")]
#[case::aws_ipv6("fd00:ec2::254")]
#[case::ipv4_mapped("::ffff:169.254.169.254")]
#[case::nat64("64:ff9b::a9fe:a9fe")]
#[case::six_to_four("2002:a9fe:a9fe::1")]
fn a_cloud_metadata_address_is_metadata_in_any_form(#[case] address: IpAddr) {
    assert_eq!(classify(address), AddressClass::Metadata);
}

#[rstest]
#[case::public_ipv4("93.184.215.14")]
#[case::public_ipv6("2606:2800:21f:cb07:6820:80da:af6b:8b2c")]
#[case::a_global_block_inside_a_refused_one("192.0.0.9")]
#[case::ipv4_mapped_public("::ffff:93.184.215.14")]
#[case::nat64_public("64:ff9b::5db8:d70e")]
fn a_globally_reachable_address_is_global(#[case] address: IpAddr) {
    assert_eq!(classify(address), AddressClass::Global);
}

#[rstest]
#[case::ipv4_multicast("224.0.0.251")]
#[case::ipv6_multicast("ff02::1")]
#[case::site_local("fec0::1")]
#[case::ipv4_compatible("::7f00:1")]
#[case::reserved_by_the_ietf("4000::1")]
fn an_address_outside_global_unicast_is_refused(#[case] address: IpAddr) {
    assert_eq!(classify(address), AddressClass::OutsideGlobalUnicast);
}

/// Whether the reference says an IPv4 address may be reached: not a
/// metadata address, not multicast, and its most specific block, if any,
/// globally reachable.
fn reference_global_ipv4(blocks: &[SpecialPurposeBlock], address: Ipv4Addr) -> bool {
    !METADATA.contains(&address.to_string().as_str())
        && !address.is_multicast()
        && most_specific(blocks, IpAddr::V4(address))
            .is_none_or(|block| block.globally_reachable == Reachability::Global)
}

/// An IPv4 address, often from a special-purpose block.
fn ipv4() -> impl Strategy<Value = Ipv4Addr> {
    prop_oneof![
        any::<u32>().prop_map(Ipv4Addr::from_bits),
        (
            prop::sample::select(vec![0u8, 10, 100, 127, 169, 172, 192, 198, 203, 240, 255]),
            any::<[u8; 3]>()
        )
            .prop_map(|(first, [b, c, d])| Ipv4Addr::new(first, b, c, d)),
        prop::sample::select(vec![
            Ipv4Addr::new(169, 254, 169, 254),
            Ipv4Addr::new(100, 100, 100, 200),
            Ipv4Addr::new(192, 0, 0, 9),
            Ipv4Addr::new(192, 88, 99, 2),
        ]),
    ]
}

/// `address` after the RFC 6052 prefix `prefix` of length `length`, with
/// the octet at bits 64 to 71 left zero.
fn rfc6052(prefix: Ipv6Addr, length: u32, address: Ipv4Addr) -> Ipv6Addr {
    let mut octets = prefix.octets();
    let mut position = usize::try_from(length / 8).unwrap();
    for octet in address.octets() {
        if position == 8 {
            position += 1;
        }
        octets[position] = octet;
        position += 1;
    }
    Ipv6Addr::from(octets)
}

/// The Teredo address of a client behind `client`, whose server is
/// `server` (RFC 4380: the client's address and port are written inverted).
fn teredo(server: Ipv4Addr, client: Ipv4Addr, flags: u16, port: u16) -> Ipv6Addr {
    Ipv6Addr::from_bits(
        (0x2001 << 112)
            | (u128::from(server.to_bits()) << 64)
            | (u128::from(flags) << 48)
            | (u128::from(!port) << 32)
            | u128::from(!client.to_bits()),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Any IPv4 address classifies as the registries, the metadata
    /// addresses and multicast say.
    #[test]
    fn an_ipv4_address_classifies_as_the_reference_says(address in ipv4()) {
        let blocks = reference();

        prop_assert_eq!(
            classify(IpAddr::V4(address)).is_global(),
            reference_global_ipv4(&blocks, address)
        );
    }

    /// An IPv4-mapped, NAT64 (well-known prefix) or 6to4 address is the
    /// IPv4 address it embeds, whatever follows it.
    #[test]
    fn an_address_embedding_ipv4_classifies_as_the_address_it_embeds(
        address in ipv4(),
        six_to_four_rest in any::<u64>(),
        subnet in any::<u16>(),
    ) {
        let embedded = classify(IpAddr::V4(address));
        let [a, b, c, d] = address.octets();
        let six_to_four = Ipv6Addr::from_bits(
            (0x2002u128 << 112)
                | (u128::from(address.to_bits()) << 80)
                | (u128::from(subnet) << 64)
                | u128::from(six_to_four_rest),
        );

        prop_assert_eq!(classify(IpAddr::V6(address.to_ipv6_mapped())), embedded);
        prop_assert_eq!(
            classify(IpAddr::V6(Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, u16::from_be_bytes([a, b]), u16::from_be_bytes([c, d])))),
            embedded
        );
        prop_assert_eq!(classify(IpAddr::V6(six_to_four)), embedded);
    }

    /// A Teredo address is refused when the client address it hides is,
    /// and classifies as that address when its server is public.
    #[test]
    fn a_teredo_address_classifies_as_the_client_it_hides(
        client in ipv4(),
        flags in any::<u16>(),
        port in any::<u16>(),
    ) {
        let server = Ipv4Addr::new(65, 54, 227, 120);

        prop_assert_eq!(
            classify(IpAddr::V6(teredo(server, client, flags, port))),
            classify(IpAddr::V4(client))
        );
    }

    /// Under the local-use NAT64 prefix, an embedded address that may not
    /// be reached is refused at every prefix length RFC 6052 allows
    /// within it.
    #[test]
    fn a_local_use_nat64_address_embedding_a_refused_address_is_refused(
        address in ipv4().prop_filter("refused", |address| !classify(IpAddr::V4(*address)).is_global()),
        length in prop::sample::select(vec![48u32, 56, 64, 96]),
    ) {
        let prefix = Ipv6Addr::new(0x64, 0xff9b, 1, 0, 0, 0, 0, 0);

        prop_assert!(!classify(IpAddr::V6(rfc6052(prefix, length, address))).is_global());
    }

    /// An IPv6 address with a zone ID names a link, so it is refused
    /// whatever the address; without one it classifies as its address.
    #[test]
    fn an_address_with_a_zone_id_is_refused(
        bits in any::<u128>(),
        zone in 1u32..,
        port in any::<u16>(),
    ) {
        let address = Ipv6Addr::from_bits(bits);

        prop_assert_eq!(
            classify_socket(SocketAddr::V6(SocketAddrV6::new(address, port, 0, zone))),
            AddressClass::ZoneId
        );
        prop_assert_eq!(
            classify_socket(SocketAddr::V6(SocketAddrV6::new(address, port, 0, 0))),
            classify(IpAddr::V6(address))
        );
    }
}
