//! `ProvingGround::start`: the commit's Proving ground started from a seed on
//! port 0 under the test's environment, with the `resolve` mapping of its
//! host names, its certificate authority and its network fixtures, so a
//! test reaches its pages, its forward proxies and its connection record
//! with no setup of its own (ADRs 0059 and 0063).
#![expect(
    clippy::disallowed_types,
    reason = "these tests reach the Proving ground over a plain socket, as any client of it does"
)]

use std::net::TcpStream;
use std::time::Duration;

use goodfetch_test_support::{ProvingGround, ProvingGroundError, ReportedAddressError, TestEnv};
use serde_json::Value;

use crate::{get, make_executable};

/// The Proving ground's own task list, which names the seed it serves.
fn tasks(proving_ground: &ProvingGround) -> Value {
    let address = proving_ground.address().to_string();
    let answer = get(&address, &address, "/tasks");
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()
}

#[test]
fn a_started_proving_ground_answers_on_its_address_with_the_seed_asked_for() {
    let env = TestEnv::new();

    let proving_ground = ProvingGround::start(&env, 341).unwrap();

    assert_eq!(proving_ground.description().seed, 341);
    assert_eq!(tasks(&proving_ground)["proving_ground"]["seed"], 341);
}

#[test]
fn two_proving_grounds_answer_on_different_ports_each_with_its_own_seed() {
    let env = TestEnv::new();

    let first = ProvingGround::start(&env, 1).unwrap();
    let second = ProvingGround::start(&env, 2).unwrap();

    assert_ne!(first.address().port(), second.address().port());
    assert_eq!(tasks(&first)["proving_ground"]["seed"], 1);
    assert_eq!(tasks(&second)["proving_ground"]["seed"], 2);
}

#[test]
fn a_dropped_proving_ground_is_gone() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 341).unwrap();
    let address = proving_ground.address();

    drop(proving_ground);

    let connected = TcpStream::connect(address);
    assert_eq!(
        connected.map_err(|error| error.kind()).err(),
        Some(std::io::ErrorKind::ConnectionRefused)
    );
}

/// Stopped, it is interrupted as Ctrl-C does, so it ends its connections
/// and undoes any network shaping before it exits, where a kill would
/// leave the shaping in place.
#[cfg(unix)]
#[test]
fn a_stopped_proving_ground_exits_of_its_own_once_interrupted() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 341).unwrap();

    let status = proving_ground.stop().unwrap();

    assert!(status.success(), "{status}");
}

#[test]
fn a_program_that_never_prints_its_line_fails_at_the_deadline() {
    let env = TestEnv::new();
    let program = silent_program(&env);

    let started = ProvingGround::start_program(&env, &program, 341, Duration::from_millis(200));

    assert!(
        matches!(
            started,
            Err(ProvingGroundError::Line(ReportedAddressError::Deadline(_)))
        ),
        "{:?}",
        started.err()
    );
}

/// A program that prints nothing and waits for its standard input, in the
/// test's folder: a shell script on Unix, a batch file on Windows.
fn silent_program(env: &TestEnv) -> std::path::PathBuf {
    if cfg!(windows) {
        let path = env.folder().join("silent.cmd");
        std::fs::write(&path, "@set /p line=\r\n").unwrap();
        path
    } else {
        let path = env.folder().join("silent");
        std::fs::write(&path, "#!/bin/sh\nread line\n").unwrap();
        make_executable(&path);
        path
    }
}

#[test]
fn the_resolve_mapping_reaches_catalog_test() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 341).unwrap();
    let description = proving_ground.description();
    let host = format!("catalog.test:{}", proving_ground.address().port());

    let answer = get(
        &format!(
            "{}:{}",
            description.resolve["catalog.test"],
            proving_ground.address().port()
        ),
        &host,
        "/catalog/1",
    );

    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(answer.body.contains("/products/P-0001"), "{}", answer.body);
}

#[test]
fn the_open_forward_proxy_reaches_catalog_test_and_the_connection_record_holds_its_relay() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 341).unwrap();
    let fixtures = &proving_ground.description().fixtures;
    let url = proving_ground.url("catalog.test", "/catalog/1");
    let host = url.trim_start_matches("http://").split('/').next().unwrap();

    let answer = get(&fixtures.proxies["open"], host, &url);

    assert_eq!(answer.status, 200, "{}", answer.body);
    let record = connection_record(&fixtures.connections);
    assert_eq!(record["relays"][0]["proxy"], "open");
    assert_eq!(record["relays"][0]["target"], url.as_str());
    assert_eq!(record["relays"][0]["outcome"], "relayed");
}

/// The connection record at its URL, `http://<address>/observations`.
fn connection_record(url: &str) -> Value {
    let rest = url.strip_prefix("http://").unwrap();
    let (address, path) = rest.split_at(rest.find('/').unwrap());
    let answer = get(address, address, path);
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()
}

#[test]
fn the_certificate_authority_file_holds_the_one_the_proving_ground_serves_under() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 341).unwrap();
    let address = proving_ground.address().to_string();

    let file = std::fs::read_to_string(&proving_ground.description().ca).unwrap();

    let served = get(&address, &address, "/certificate-authority");
    assert!(file.starts_with("-----BEGIN CERTIFICATE-----"), "{file}");
    assert_eq!(file, served.body);
}
