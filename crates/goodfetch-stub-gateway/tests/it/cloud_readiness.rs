//! The Cloud-readiness conformance suite's gateway target kind (spec #586,
//! ADRs 0068 and 0096) against the Stub gateway, reached by its address as
//! Goodfetch Cloud's gateway is from its own repository: every gateway case
//! passes, and the document it writes is a valid `conformance` result
//! document, kept where `just test` gathers them so the Feature report
//! reads its cases (ADR 0072). A gateway that runs no Foxhound engine
//! refuses a Foxhound Client at `hello` with `not_in_product`.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous test reads TESTING.md and keeps its result document"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a test opens its own WebSocket to the gateway, outside the engine (ADR 0060)"
)]

use std::path::{Path, PathBuf};

use goodfetch_conformance::{GatewaySuite, GatewayTarget, ProvingGround, run_gateway_suite};
use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::result_document::{
    CapabilityResult, CaseOutcome, LoadedDocument, Outcome, ResultDocument, read,
};
use goodfetch_test_support::{
    ProvingGround as LocalProvingGround, RESULT_DOCUMENTS_VARIABLE, TestEnv, Variables,
};
use serde_json::{Value, json};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest as _;

use crate::gateway::{Gateway, TOKEN};

/// This commit's Product version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The Stub gateway as the gateway target.
fn target(gateway: &Gateway) -> GatewayTarget {
    GatewayTarget {
        name: "goodfetch-stub-gateway".to_owned(),
        address: gateway.console.clone(),
        token: TOKEN.to_owned(),
    }
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
#[test]
fn every_gateway_case_passes_on_the_stub_gateway_reached_by_its_address() {
    let env = TestEnv::new();
    let local = LocalProvingGround::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&local.address().to_string());
    let gateway = Gateway::start(&env);

    let document = run_gateway_suite(
        &GatewaySuite::embedded(),
        &target(&gateway),
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
    keep(&written);
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
    let section = testing
        .split("### The Cloud-readiness conformance suite")
        .nth(1)
        .and_then(|rest| rest.split("\n## ").next())
        .and_then(|rest| rest.split("\n### ").next())
        .expect("TESTING.md has the subsection");
    let lines: Vec<&str> = section
        .lines()
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
    let cookie = gateway.signed_in();
    let address = gateway.console.trim_start_matches("http://");
    let mut request = format!("ws://{address}/protocol")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Origin", gateway.console.parse().unwrap());
    request
        .headers_mut()
        .insert("Cookie", cookie.parse().unwrap());
    let (mut socket, _) =
        tungstenite::client(request, std::net::TcpStream::connect(address).unwrap()).unwrap();
    socket
        .send(Message::text(
            json!({"jsonrpc": "2.0", "id": 1, "method": "hello", "params": {
                "protocol_version": PROTOCOL_VERSION.to_string(),
                "surface": "rust",
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
