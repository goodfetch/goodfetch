//! Certificate verification on the engine's own HTTP client, against the
//! Proving ground (ADRs 0016 and 0042, pitfall H03): every certificate is
//! verified, a domain's `tls.trusted_cas` trusts its authority for that
//! domain and nowhere else, and a certificate that fails verification ends
//! the request with `certificate_invalid` before anything is sent. The
//! `resolve` setting maps every host name to the Proving ground's loopback
//! address: no test reaches the internet.

use std::net::IpAddr;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::http_client::{FetchError, PageTraffic, Resolve, Waiter};
use goodfetch_engine::tls::Trust;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use serde_json::{Value, json};

/// Any seed: the families used here are served on every one.
const SEED: u64 = 625;

/// What a run may reach when the Proving ground's authority is trusted for
/// each of `domains`: every host name it serves, at its address.
fn traffic(proving_ground: &ProvingGround, domains: &[&str]) -> PageTraffic {
    let description = proving_ground.description();
    let resolve = description
        .resolve
        .iter()
        .fold(Resolve::new(), |resolve, (host, address)| {
            resolve.with(host.clone(), vec![address.parse::<IpAddr>().unwrap()])
        });
    let authority = CertificateDer::from_pem_file(&description.ca).unwrap();
    let trust = domains
        .iter()
        .fold(Trust::default_roots(), |trust, domain| {
            trust
                .with_trusted_cas(domain, vec![authority.clone()])
                .unwrap()
        });
    PageTraffic {
        egress: Policy::new(&["*"], &[] as &[&str]).unwrap(),
        resolve,
        trust,
        clock: Clock::system(),
        delays: Delays::BUILT_IN,
    }
}

/// `path` on `host` over HTTPS, at the Proving ground's site.
fn site_url(proving_ground: &ProvingGround, host: &str, path: &str) -> String {
    format!("https://{host}:{}{path}", proving_ground.address().port())
}

/// The hosts and paths of the page requests the Proving ground received.
fn requests(observations: &Value) -> Vec<Value> {
    observations["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|request| json!([request["host"], request["target"]]))
        .collect()
}

/// Story 4-33: the Proving ground's private authority, listed for
/// `scope.test`, makes that domain's certificates pass, its subdomains'
/// too, and leaves another domain's certificate from the same authority
/// refused, nothing sent to it.
/// Covers: story:4-33, setting:tls.trusted_cas
#[test]
fn a_domains_trusted_authority_passes_its_certificates_and_no_other_domains() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let scope_only = traffic(&proving_ground, &["scope.test"]);
    let runtime = goodfetch_test_support::runtime();

    let (listed, subdomain, other) = runtime.block_on(async {
        let listed = scope_only
            .get(&site_url(&proving_ground, "scope.test", "/"), Waiter::Call)
            .await
            .map(|response| response.status());
        let subdomain = scope_only
            .get(
                &site_url(&proving_ground, "www.scope.test", "/news"),
                Waiter::Call,
            )
            .await
            .map(|response| response.status());
        let other = scope_only
            .get(
                &site_url(&proving_ground, "catalog.test", "/catalog/1"),
                Waiter::Call,
            )
            .await
            .map(|response| response.status());
        (listed, subdomain, other)
    });

    assert_eq!(listed.unwrap(), 200);
    assert_eq!(subdomain.unwrap(), 200);
    assert!(
        matches!(other, Err(FetchError::Failed(_))),
        "another domain's certificate is refused: {other:?}"
    );
    assert_eq!(
        requests(&proving_ground.observations().unwrap()),
        [
            json!(["scope.test", "/"]),
            json!(["www.scope.test", "/news"])
        ]
    );
}
