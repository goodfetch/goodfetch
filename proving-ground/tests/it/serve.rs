//! `serve`'s first output line: one JSON object with the bound address, the
//! version, the seed, the network profile with its calibration (the
//! `network` tests), and the address of each network fixture with the
//! proxies' credentials, so every test and Journey variant starts its own
//! Proving ground on port 0 (ADRs 0062 and 0063).
//! Covers: story:2-80

use regex::Regex;
use serde_json::Value;

use crate::proving_ground::ProvingGround;

#[test]
fn serve_on_port_zero_first_prints_its_address_version_and_seed() {
    let proving_ground = ProvingGround::start(u64::MAX);

    let line: Value = serde_json::from_str(proving_ground.line()).expect("the line is JSON");

    let object = line.as_object().expect("the line is one JSON object");
    assert_eq!(
        object.keys().collect::<Vec<_>>(),
        [
            "address",
            "certificate_authority",
            "dns",
            "hosts",
            "network",
            "proxies",
            "proxy_credentials",
            "seed",
            "target",
            "udp",
            "version"
        ]
    );
    assert!(
        Regex::new(r"^127\.0\.0\.1:[1-9][0-9]*$")
            .unwrap()
            .is_match(line["address"].as_str().unwrap()),
        "{line}"
    );
    assert_eq!(line["seed"].as_u64(), Some(u64::MAX));
    assert!(
        Regex::new(r"^0\.0\.0\+[0-9a-f]{40}$")
            .unwrap()
            .is_match(line["version"].as_str().unwrap()),
        "{line}"
    );
}

/// Covers: story:2-82
#[test]
fn serve_first_prints_the_address_of_each_network_fixture() {
    let proving_ground = ProvingGround::start(311);

    let line: Value = serde_json::from_str(proving_ground.line()).expect("the line is JSON");

    let address = Regex::new(r"^127\.0\.0\.1:[1-9][0-9]*$").unwrap();
    let proxies = line["proxies"].as_object().expect("the proxies by name");
    assert_eq!(
        proxies.keys().collect::<Vec<_>>(),
        [
            "datacenter",
            "down",
            "isp",
            "limited",
            "mobile",
            "open",
            "open_tls",
            "refusing_tunnels",
            "residential",
            "socks5"
        ]
    );
    assert!(
        proxies
            .values()
            .chain([&line["dns"], &line["target"], &line["udp"]])
            .all(|fixture| fixture.as_str().is_some_and(|text| address.is_match(text))),
        "{line}"
    );
}

/// Covers: story:2-82
#[test]
fn network_fixtures_serves_the_fixtures_the_first_line_reported() {
    let proving_ground = ProvingGround::start(311);
    let line: Value = serde_json::from_str(proving_ground.line()).expect("the line is JSON");

    let served = proving_ground.control("/network-fixtures");

    assert_eq!(served.status, 200, "{}", served.body);
    assert_eq!(
        (&served.json()["proxies"], &served.json()["dns"]),
        (&line["proxies"], &line["dns"])
    );
}

/// Covers: story:2-82
#[test]
fn the_proxy_credentials_are_derived_from_the_seed() {
    let first = ProvingGround::start(311);
    let again = ProvingGround::start(311);
    let other = ProvingGround::start(312);

    let credentials = [&first, &again, &other].map(|proving_ground| {
        let line: Value = serde_json::from_str(proving_ground.line()).expect("the line is JSON");
        line["proxy_credentials"].clone()
    });

    assert_eq!(credentials[0], credentials[1]);
    assert_ne!(credentials[0], credentials[2]);
    assert!(credentials[0]["username"].is_string() && credentials[0]["password"].is_string());
}

#[test]
fn serve_first_prints_every_host_name_it_serves() {
    let proving_ground = ProvingGround::start(341);

    let line: Value = serde_json::from_str(proving_ground.line()).expect("the line is JSON");

    let hosts: Vec<&str> = line["hosts"]
        .as_array()
        .expect("the host names are a list")
        .iter()
        .map(|host| host.as_str().expect("a host name is a string"))
        .collect();
    assert!(hosts.contains(&"catalog.test"), "{line}");
    assert!(hosts.is_sorted(), "{line}");
}

#[test]
fn serve_first_prints_the_certificate_authority_it_serves_under() {
    let proving_ground = ProvingGround::start(341);

    let line: Value = serde_json::from_str(proving_ground.line()).expect("the line is JSON");

    assert_eq!(
        line["certificate_authority"].as_str(),
        Some(proving_ground.certificate_authority().as_str())
    );
}
