//! Certificate verification on the engine's own HTTP client, against the
//! Proving ground (ADRs 0016 and 0042, pitfall H03): every certificate is
//! verified, a domain's `tls.trusted_cas` trusts its authority for that
//! domain and nowhere else, and a certificate that fails verification ends
//! the request with `certificate_invalid` before anything is sent. The
//! `resolve` setting maps every host name to the Proving ground's loopback
//! address, which the Session's private ranges list: no test reaches the
//! internet.

use std::net::IpAddr;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::http_client::{Failure, FetchError, PageTraffic, Resolve, Waiter};
use goodfetch_engine::tls::{CertificateExceptions, ClientCertificates, Trust};
use goodfetch_protocol::tls::{
    CertificateFailure, CertificateInvalidDetails, KnownCertificateFailure,
};
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
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
        egress: Policy::new(&["*"], &[] as &[&str])
            .unwrap()
            .with_private_ranges(&[proving_ground.address().ip().to_string()])
            .unwrap(),
        resolve,
        local: None,
        proxy: None,
        trust,
        exceptions: CertificateExceptions::default(),
        client_certificates: ClientCertificates::default(),
        cookies: Vec::new(),
        clock: Clock::system(),
        delays: Delays::default(),
    }
}

/// `path` on `host` over HTTPS, at the Proving ground's site.
fn site_url(proving_ground: &ProvingGround, host: &str, path: &str) -> String {
    format!("https://{host}:{}{path}", proving_ground.address().port())
}

/// The hosts and paths of the page requests the Proving ground received.
fn requests(observations: &Value) -> Vec<Value> {
    ReceivedRequests::of(observations)
        .every()
        .map(|request| json!([request.host, request.target]))
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
    let Err(FetchError::Failed(Failure::CertificateInvalid(other))) = other else {
        panic!("another domain's certificate is refused: {other:?}");
    };
    assert_eq!(
        other.details(),
        h03_refusal("catalog.test", KnownCertificateFailure::UnknownIssuer)
    );
    assert_eq!(
        requests(&proving_ground.observations().unwrap()),
        [
            json!(["scope.test", "/"]),
            json!(["www.scope.test", "/news"])
        ]
    );
}

/// What a certificate that fails in each of pitfall H03's ways ends a
/// request with: `certificate_invalid`, naming the host, the failure and
/// the setting that would make an exception.
fn h03_refusal(host: &str, failure: KnownCertificateFailure) -> CertificateInvalidDetails {
    let fix = match failure {
        KnownCertificateFailure::UnknownIssuer => format!(
            "Trust the certificate authority of {host} for its domain under `tls.trusted_cas` in user or project configuration or the Operator policy, or list its domain under `tls.accept_invalid_certs` to pin its certificate on first use."
        ),
        _ => format!(
            "List the domain of {host} under `tls.accept_invalid_certs` to pin its certificate on first use, or give its pin there in user or project configuration or the Operator policy."
        ),
    };
    CertificateInvalidDetails {
        host: host.to_owned(),
        failure: CertificateFailure::Known(failure),
        fix,
    }
}

/// Pitfall H03 on the engine's HTTP client (stories 4-31 and 4-32): an
/// expired, a not yet valid, an unknown issuer's, a name-mismatched and a
/// self-signed certificate are each refused with `certificate_invalid`,
/// though the Proving ground's authority is trusted for their domain, and
/// no request reaches them; a valid certificate under the same trust passes.
/// Covers: story:4-31, story:4-32, error:certificate_invalid, setting:tls.trusted_cas
#[test]
fn each_invalid_certificate_is_refused_with_certificate_invalid_naming_the_failure_and_fix() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let trusting = traffic(&proving_ground, &["certificates.test", "scope.test"]);
    let cases = [
        (
            "expired.certificates.test",
            KnownCertificateFailure::Expired,
        ),
        (
            "not-yet-valid.certificates.test",
            KnownCertificateFailure::NotYetValid,
        ),
        (
            "unknown-issuer.certificates.test",
            KnownCertificateFailure::UnknownIssuer,
        ),
        (
            "name-mismatch.certificates.test",
            KnownCertificateFailure::NameMismatch,
        ),
        (
            "self-signed.certificates.test",
            KnownCertificateFailure::SelfSigned,
        ),
    ];
    let runtime = goodfetch_test_support::runtime();

    let (valid, refused) = runtime.block_on(async {
        let valid = trusting
            .get(&site_url(&proving_ground, "scope.test", "/"), Waiter::Call)
            .await
            .map(|response| response.status());
        let mut refused = Vec::new();
        for (host, _) in cases {
            let fetched = trusting
                .get(&site_url(&proving_ground, host, "/"), Waiter::Call)
                .await;
            refused.push(match fetched {
                Err(FetchError::Failed(Failure::CertificateInvalid(invalid))) => {
                    Ok((invalid.error().code(), invalid.details()))
                }
                other => Err(format!("{other:?}")),
            });
        }
        (valid, refused)
    });

    assert_eq!(valid.unwrap(), 200);
    assert_eq!(
        refused,
        cases
            .iter()
            .map(|(host, failure)| Ok(("certificate_invalid", h03_refusal(host, *failure))))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        requests(&proving_ground.observations().unwrap()),
        [json!(["scope.test", "/"])]
    );
}
