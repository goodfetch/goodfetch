//! `Canaries::plant_through` and `sightings`: a Secret canary planted
//! through the Proving ground's `canaries` login flows (ADR 0067), whose
//! record of every request that carried it tells the copy its flow sends
//! from a leak. A tool that uses the canary where its flow needs it is
//! played by a plain HTTP client; a leaking one also sends it elsewhere.
#![expect(
    clippy::disallowed_types,
    reason = "these tests reach the Proving ground over a plain socket, as any client of it does"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use goodfetch_test_support::{
    Canaries, CanaryError, Placement, ProvingGround, SecretKind, Sighting, TestEnv,
};
use rstest::rstest;

/// Sends one HTTP/1.1 request to the Proving ground and returns the status.
fn send(proving_ground: &ProvingGround, request: &str) -> u16 {
    let mut stream = TcpStream::connect(proving_ground.address()).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let response = String::from_utf8_lossy(&response);
    response.split(' ').nth(1).unwrap().parse().unwrap()
}

#[rstest]
#[case::typed_password(SecretKind::TypedPassword)]
#[case::totp_seed(SecretKind::TotpSeed)]
#[case::session_cookie(SecretKind::SessionCookie)]
#[case::endpoint_token(SecretKind::EndpointToken)]
#[case::header_value(SecretKind::HeaderValue)]
fn a_proving_ground_plant_matches_its_kinds_gitleaks_pattern(#[case] kind: SecretKind) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 379).unwrap();
    let mut canaries = Canaries::new(env.seed());

    let canary = canaries.plant_through(&proving_ground, kind).unwrap();

    assert!(
        regex::Regex::new(&kind.pattern())
            .unwrap()
            .is_match(canary.value()),
        "{} does not match {}",
        canary.id(),
        kind.pattern()
    );
}

#[test]
fn sightings_tell_the_copy_a_login_flow_sends_from_a_leak() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 379).unwrap();
    let mut canaries = Canaries::new(env.seed());
    let canary = canaries
        .plant_through(&proving_ground, SecretKind::TypedPassword)
        .unwrap()
        .clone();
    let account = canary.account().unwrap();
    let body = format!("username={}&password={}", account.username, canary.value());
    let logged_in = send(
        &proving_ground,
        &format!(
            "POST /login HTTP/1.1\r\nHost: 1.canaries.test\r\nContent-Type: \
             application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: \
             close\r\n\r\n{body}",
            body.len()
        ),
    );
    assert_eq!(logged_in, 303);
    send(
        &proving_ground,
        &format!(
            "GET /account HTTP/1.1\r\nHost: 1.canaries.test\r\nX-Note: {}\r\nConnection: \
             close\r\n\r\n",
            canary.value()
        ),
    );

    let sightings = canaries.sightings(&proving_ground).unwrap();

    assert_eq!(
        sightings,
        vec![
            Sighting {
                canary: canary.id().to_owned(),
                kind: SecretKind::TypedPassword,
                host: "1.canaries.test".to_owned(),
                target: "/login".to_owned(),
                part: "body".to_owned(),
                placement: Placement::Planted,
            },
            Sighting {
                canary: canary.id().to_owned(),
                kind: SecretKind::TypedPassword,
                host: "1.canaries.test".to_owned(),
                target: "/account".to_owned(),
                part: "header:x-note".to_owned(),
                placement: Placement::Leaked,
            },
        ]
    );
}

#[test]
fn plant_through_refuses_a_kind_without_a_login_flow() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 379).unwrap();
    let mut canaries = Canaries::new(env.seed());

    let error = canaries
        .plant_through(&proving_ground, SecretKind::ProxyPassword)
        .unwrap_err();

    assert!(matches!(
        error,
        CanaryError::NoLoginFlow(SecretKind::ProxyPassword)
    ));
}
