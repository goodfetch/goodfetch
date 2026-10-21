use std::net::SocketAddr;

use rstest::rstest;

use super::default_address;
use crate::connection::ProductVersion;

/// Covers: story:12-3, setting:console.address
#[rstest]
#[case::major_1("1.4.2", "127.0.0.1:7801")]
#[case::major_2("2.0.0", "127.0.0.1:7802")]
#[case::major_2_pre_release("2.1.0-rc.1", "127.0.0.1:7802")]
#[case::preview_0_3("0.3.9", "127.0.0.1:7703")]
#[case::preview_0_4("0.4.0", "127.0.0.1:7704")]
#[case::preview_0_0("0.0.0", "127.0.0.1:7700")]
fn each_major_has_its_own_console_port(#[case] version: &str, #[case] expected: &str) {
    let version = ProductVersion::parse(version).unwrap();

    let address = default_address(&version);

    assert_eq!(address, expected.parse::<SocketAddr>().unwrap());
}
