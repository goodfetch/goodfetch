use std::net::IpAddr;

use rstest::rstest;

use super::{Cidr, Reachability, RegistryError, SpecialPurposeBlock, SpecialPurposeRegistries};

/// IANA's header row, then rows of the shapes IANA writes: a plain one, a
/// footnoted block and reachability, two blocks in one cell, an `N/A`
/// reachability and a terminated block whose columns are empty.
const REGISTRY: &str = "Address Block,Name,RFC,Allocation Date,Termination Date,Source,Destination,Forwardable,Globally Reachable,Reserved-by-Protocol\r
10.0.0.0/8,Private-Use,[RFC 1918],1996-02,N/A,True,True,True,False,False\r
192.0.0.0/24 [2],IETF Protocol Assignments,\"[RFC 6890, Section 2.1]\",2010-01,N/A,False,False,False,False [1],False\r
\"192.0.0.170/32, 192.0.0.171/32\",NAT64/DNS64 Discovery,\"[RFC 8880]; [RFC 7050, Section 2.2]\",2013-02,N/A,False,False,False,False,True\r
2001::/32,TEREDO,[RFC 4380]; [RFC 8190],2006-01,N/A,True,True,True,N/A [2],False\r
192.88.99.0/24,Deprecated (6to4 Relay Anycast),[RFC 7526],2001-06,2015-03,,,,,\r
192.31.196.0/24,AS112-v4,[RFC 7535],2014-12,N/A,True,True,True,True,False\r
";

fn block(block: &str, name: &str, globally_reachable: Reachability) -> SpecialPurposeBlock {
    SpecialPurposeBlock {
        block: block.to_owned(),
        name: name.to_owned(),
        globally_reachable,
    }
}

#[test]
fn a_registry_reads_each_block_and_whether_it_is_globally_reachable() {
    let registries = SpecialPurposeRegistries::parse_csv(REGISTRY).unwrap();

    assert_eq!(
        registries.blocks,
        [
            block("10.0.0.0/8", "Private-Use", Reachability::NotGlobal),
            block(
                "192.0.0.0/24",
                "IETF Protocol Assignments",
                Reachability::NotGlobal
            ),
            block(
                "192.0.0.170/32",
                "NAT64/DNS64 Discovery",
                Reachability::NotGlobal
            ),
            block(
                "192.0.0.171/32",
                "NAT64/DNS64 Discovery",
                Reachability::NotGlobal
            ),
            block("2001::/32", "TEREDO", Reachability::Unstated),
            block(
                "192.88.99.0/24",
                "Deprecated (6to4 Relay Anycast)",
                Reachability::Unstated
            ),
            block("192.31.196.0/24", "AS112-v4", Reachability::Global),
        ]
    );
}

#[test]
fn a_registry_row_without_its_columns_is_refused() {
    let parsed = SpecialPurposeRegistries::parse_csv(
        "Address Block,Name,RFC,Allocation Date,Termination Date,Source,Destination,Forwardable,Globally Reachable,Reserved-by-Protocol\r\n10.0.0.0/8,Private-Use\r\n",
    );

    assert!(matches!(parsed, Err(RegistryError::Row { line: 2 })));
}

#[test]
fn a_file_without_iana_s_header_is_refused() {
    let parsed = SpecialPurposeRegistries::parse_csv("Block,Name\r\n10.0.0.0/8,Private-Use\r\n");

    assert!(matches!(parsed, Err(RegistryError::Header)));
}

#[rstest]
#[case::first_address("10.0.0.0/8", "10.0.0.0", true)]
#[case::last_address("10.0.0.0/8", "10.255.255.255", true)]
#[case::next_block("10.0.0.0/8", "11.0.0.0", false)]
#[case::one_address("255.255.255.255/32", "255.255.255.255", true)]
#[case::ipv6_inside("2001::/23", "2001:1ff::1", true)]
#[case::ipv6_outside("2001::/23", "2001:200::", false)]
#[case::other_family("::ffff:0:0/96", "10.0.0.1", false)]
fn a_block_holds_the_addresses_its_prefix_covers(
    #[case] block: &str,
    #[case] address: &str,
    #[case] holds: bool,
) {
    let cidr = Cidr::parse(block).unwrap();

    let contains = cidr.contains(address.parse::<IpAddr>().unwrap());

    assert_eq!(contains, holds);
}

#[rstest]
#[case::no_prefix("10.0.0.0")]
#[case::prefix_too_long("10.0.0.0/33")]
#[case::not_an_address("ten/8")]
fn a_malformed_block_is_refused(#[case] block: &str) {
    let parsed = Cidr::parse(block);

    assert_eq!(parsed, None);
}
