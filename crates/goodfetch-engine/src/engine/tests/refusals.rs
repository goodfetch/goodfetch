//! The JSON-RPC protocol's generated refusal cases (ADR 0066) in process,
//! over the in-memory network: each fabricated `hello`, a Client's with the
//! Product check of every pair of Products (ADR 0095) or a Browser host's
//! settling on the lower minor (ADR 0096), sent with a call behind it. A
//! refusal is the only reply, before the call's, and the Connection then
//! closes; its Error code is the case's, and the error is snapshotted.
//!
//! `browser-host-outdated` is not run here: no Session reaches a Browser
//! host yet, and the Browser host's own handshake runs it once it does.

use std::path::PathBuf;

use goodfetch_protocol::formats::HelloFrom;
use goodfetch_protocol::refusals::{FabricatedHello, RefusalCase, Reply};
use rstest::rstest;
use serde_json::{Value, json};

use super::{engine, exchange, pretty, request};

/// The `hello` a case fabricates.
fn fabricated(case: &RefusalCase, hello: &FabricatedHello) -> Value {
    let mut params = json!({
        "protocol_version": case.found,
        "surface": match hello.from {
            HelloFrom::Client => "rust",
            HelloFrom::BrowserHost => "browser_host",
            HelloFrom::GroupMember => panic!("a Group member opens the peer protocol"),
        },
        "product_version": "1.4.0",
    });
    if let Some(needs) = hello.needs {
        params["needs"] = json!(needs);
    }
    json!({ "jsonrpc": "2.0", "id": 0, "method": "hello", "params": params })
}

#[rstest]
fn each_fabricated_hello_gets_its_generated_reply(
    #[files("../goodfetch-protocol/generated/refusals/protocol/*.json")]
    #[exclude("browser-host-outdated")]
    path: PathBuf,
) {
    let case: RefusalCase = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let hello = case.hello.clone().unwrap();

    let mut received = exchange(
        &engine(hello.engine),
        &[fabricated(&case, &hello), request(1, "engine.info")],
    );

    match hello.reply {
        Reply::Accepted { protocol_version } => assert_eq!(
            (
                received.len(),
                super::takes_engine_instance(&mut received[0]),
                super::takes_timeouts(&mut received[0]),
                &received[0]["result"]
            ),
            (
                2,
                true,
                true,
                &json!({
                    "protocol_version": protocol_version,
                    "product": hello.engine,
                    "product_version": "1.4.0",
                    "max_calls_in_flight": super::IN_FLIGHT_CAP,
                })
            ),
            "{received:#?}"
        ),
        Reply::Refused { error_code } => {
            assert_eq!(
                (received.len(), &received[0]["error"]["data"]["code"]),
                (1, &json!(error_code)),
                "{received:#?}"
            );
            let name = path.file_stem().unwrap().to_string_lossy();
            insta::assert_snapshot!(format!("refusal_{name}"), pretty(&received[0]));
        }
    }
}
