//! The Cloud-readiness conformance suite's gateway target kind (spec #586,
//! ADRs 0068 and 0096) against the Stub gateway, reached by its address as
//! Goodfetch Cloud's gateway is from its own repository, over `http` and
//! over `https` through a TLS terminator in front of it: every gateway case
//! passes, and the document it writes is a valid `conformance` result
//! document, kept where `just test` gathers them so the Feature report
//! reads its cases (ADR 0072). A gateway that runs no Foxhound engine
//! refuses a Foxhound Client at `hello` with `not_in_product`.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous test reads TESTING.md, keeps its result document and builds the runtime its TLS terminator runs on"
)]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_conformance::{
    GatewayAddress, GatewaySuite, GatewayTarget, ProvingGround, run_gateway_suite,
};
use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::result_document::{
    CapabilityResult, CaseOutcome, LoadedDocument, Outcome, ResultDocument, read,
};
use goodfetch_test_support::{
    ProvingGround as LocalProvingGround, RESULT_DOCUMENTS_VARIABLE, TestEnv, Variables,
};
use rstest::rstest;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::SecretString;
use serde_json::{Value, json};
use tungstenite::Message;

use crate::gateway::{Gateway, TOKEN};

/// This commit's Product version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How the suite reaches the Stub gateway.
#[derive(Clone, Copy)]
enum Reached {
    /// At its own `http` address.
    Plain,
    /// At `https://localhost`, through a TLS terminator in front of it.
    BehindTls,
}

/// The Stub gateway as the gateway target, reached as `reached` says; the
/// TLS terminator runs on `runtime`.
fn target(gateway: &Gateway, reached: Reached, runtime: &tokio::runtime::Runtime) -> GatewayTarget {
    let (address, trusted_certificates) = match reached {
        Reached::Plain => (gateway.console.clone(), Vec::new()),
        Reached::BehindTls => {
            let upstream = gateway.console.trim_start_matches("http://").to_owned();
            let (port, authority) = terminate_tls(runtime, upstream);
            (format!("https://localhost:{port}/"), vec![authority])
        }
    };
    GatewayTarget {
        name: "goodfetch-stub-gateway".to_owned(),
        address: GatewayAddress::parse(&address).unwrap(),
        token: SecretString::from(TOKEN),
        trusted_certificates,
    }
}

/// A TLS terminator on loopback, as an upstream terminator stands in front
/// of a gateway: it takes TLS for `localhost` under a certificate its own
/// authority signs and relays the bytes to `upstream`. Its port and its
/// authority's certificate (DER).
#[expect(
    clippy::disallowed_methods,
    reason = "the stand-in terminator, outside the gateway, runs its accept loop and connections on the test's runtime (ADR 0062)"
)]
#[expect(
    clippy::disallowed_types,
    reason = "the stand-in terminator opens its own sockets, outside the gateway (ADR 0060)"
)]
fn terminate_tls(runtime: &tokio::runtime::Runtime, upstream: String) -> (u16, Vec<u8>) {
    let authority_key = rcgen::KeyPair::generate().unwrap();
    let mut authority_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    authority_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let authority = authority_params.self_signed(&authority_key).unwrap();
    let issuer = rcgen::Issuer::new(authority_params, authority_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![leaf.der().clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            0,
        )))
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    runtime.spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (acceptor, upstream) = (acceptor.clone(), upstream.clone());
            tokio::spawn(async move {
                let (Ok(mut client), Ok(mut gateway)) = (
                    acceptor.accept(tcp).await,
                    tokio::net::TcpStream::connect(upstream.as_str()).await,
                ) else {
                    return;
                };
                let _ = tokio::io::copy_bidirectional(&mut client, &mut gateway).await;
            });
        }
    });
    (port, authority.der().to_vec())
}

/// Keeps `document` as `conformance-cloud-readiness-gateway.json` where
/// `just test` gathers the result documents, when it runs the test.
fn keep(document: &ResultDocument) {
    let Some(folder) = Variables::from_process()
        .get(RESULT_DOCUMENTS_VARIABLE)
        .map(PathBuf::from)
    else {
        return;
    };
    std::fs::create_dir_all(&folder).unwrap();
    let mut bytes = serde_json::to_vec_pretty(document).unwrap();
    bytes.push(b'\n');
    std::fs::write(
        folder.join("conformance-cloud-readiness-gateway.json"),
        bytes,
    )
    .unwrap();
}

/// Covers: story:14-73
#[rstest]
#[case::at_its_own_address(Reached::Plain)]
#[case::behind_a_tls_terminator(Reached::BehindTls)]
fn every_gateway_case_passes_on_the_stub_gateway_reached_by_its_address(#[case] reached: Reached) {
    let env = TestEnv::new();
    let local = LocalProvingGround::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&local.address().to_string());
    let gateway = Gateway::start(&env);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();

    let document = run_gateway_suite(
        &GatewaySuite::embedded(),
        &target(&gateway, reached, &runtime),
        &proving_ground,
    )
    .unwrap();

    let failed: Vec<(String, Vec<String>)> = document
        .capabilities()
        .iter()
        .flat_map(CapabilityResult::cases)
        .filter(|case| case.outcome() != CaseOutcome::Passed)
        .map(|case| {
            let checks = case
                .facts()
                .entries
                .iter()
                .flat_map(|entry| &entry.checks)
                .filter(|check| !check.passed)
                .map(|check| check.name.clone())
                .collect();
            (case.facts().id.clone(), checks)
        })
        .collect();
    assert_eq!(failed, Vec::<(String, Vec<String>)>::new());
    let written = ResultDocument::Conformance(document);
    if matches!(reached, Reached::Plain) {
        keep(&written);
    }
    let reread = read(&serde_json::to_vec(&written).unwrap()).unwrap();
    let LoadedDocument::Conformance(reread) = reread else {
        panic!("the gateway suite wrote no `conformance` document");
    };
    assert!(
        reread
            .capabilities()
            .iter()
            .all(|capability| capability.outcome() == Outcome::Proved)
    );
}

/// Covers: story:14-13, story:14-75
#[test]
fn testing_md_lists_every_cloud_readiness_case_with_its_covers() {
    let testing =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../TESTING.md"))
            .unwrap();
    // Found by its heading line, the part `xtask affected` reads it as.
    let mut section = testing.lines();
    assert!(
        section.any(|line| line.trim_end() == "### The Cloud-readiness conformance suite"),
        "TESTING.md has the subsection"
    );
    let lines: Vec<&str> = section
        .take_while(|line| !line.starts_with("## ") && !line.starts_with("### "))
        .filter(|line| line.starts_with("- `"))
        .collect();

    let missing: Vec<String> = GatewaySuite::embedded()
        .case_list()
        .capabilities
        .iter()
        .flat_map(|capability| &capability.cases)
        .filter(|case| {
            !lines.iter().any(|line| {
                line.starts_with(&format!("- `{}`", case.id))
                    && case
                        .covers
                        .iter()
                        .all(|feature| line.contains(&format!("`{}`", feature.as_str())))
            })
        })
        .map(|case| case.id.clone())
        .collect();

    assert_eq!(missing, Vec::<String>::new());
    assert_eq!(
        lines.len(),
        GatewaySuite::embedded()
            .case_list()
            .capabilities
            .iter()
            .map(|capability| capability.cases.len())
            .sum::<usize>(),
        "the subsection lists a case the case list does not declare"
    );
}

/// Covers: story:14-15
#[test]
fn a_gateway_that_runs_no_foxhound_engine_refuses_a_foxhound_client_at_hello() {
    let env = TestEnv::new();
    let gateway = Gateway::start_without_foxhound(&env);
    let mut socket = gateway.socket(false);
    socket
        .send(Message::text(
            json!({"jsonrpc": "2.0", "id": 1, "method": "hello", "params": {
                "protocol_version": PROTOCOL_VERSION.to_string(),
                "surface": "rust",
                "token": TOKEN,
                "needs": "foxhound",
                "product_version": VERSION,
            }})
            .to_string(),
        ))
        .unwrap();

    let Message::Text(answer) = socket.read().unwrap() else {
        panic!("the gateway answered no text");
    };
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(
        (
            &answer["error"]["data"]["code"],
            &answer["error"]["data"]["needs"]
        ),
        (&json!("not_in_product"), &json!("foxhound")),
        "{answer}"
    );
}
