//! The plugin protocol's writer for the Compatibility corpus (ADR 0066):
//! one fixed scenario between the engine and a `sample_store` Plugin, the
//! messages each side sends built from the protocol's own types, written
//! into the folder given as the only argument as `engine.jsonl` and
//! `plugin.jsonl`, one message per line. The engine opens with `hello`,
//! pings, makes a call the Plugin answers, one it does not have, and one it
//! stops with `$/cancel` past its deadline, whose late result the Plugin
//! still sends. `xtask compat write` runs it.
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous corpus writer that writes files (ADR 0066)"
)]

#[path = "support/transcript.rs"]
mod transcript;

use std::collections::BTreeSet;
use std::path::PathBuf;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::jsonrpc::{
    ErrorData, ErrorObject, ErrorResponse, Message, Notification, ProtocolFailure, RequestId,
    Version,
};
use goodfetch_protocol::plugin::{
    CANCEL, DeclaredKind, HELLO, NetworkLevel, PING, PLUGIN_PROTOCOL, PLUGIN_PROTOCOL_VERSION,
    PluginCancelParams, PluginHelloParams, PluginHelloResult, PluginKindName, PluginPingParams,
    PluginPingResult,
};
use goodfetch_protocol::run::RunId;
use serde_json::{Map, json};
use transcript::{call, result, to_value, write};

/// The conformance harness's sample kind, which the toy Plugin implements.
const SAMPLE_STORE: PluginKindName = PluginKindName::from_static("sample_store");

/// The method of the sample kind that keeps a value.
const PUT: &str = "sample_store.put";

/// A method of the sample kind the Plugin does not have.
const DELETE: &str = "sample_store.delete";

fn main() {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .expect("invariant: xtask compat passes the output folder"),
    );
    let late = RequestId::Text("put-late".to_owned());
    let engine = [
        call(
            RequestId::Number(0),
            HELLO,
            &PluginHelloParams {
                protocol: PLUGIN_PROTOCOL,
                product: Product::Collie,
                run_id: Some(
                    RunId::parse("books-2026-01-05").expect("invariant: the Run ID is well formed"),
                ),
                network_level: NetworkLevel::None,
            },
        ),
        call(RequestId::Number(1), PING, &PluginPingParams {}),
        call(
            RequestId::Number(2),
            PUT,
            &json!({ "key": "title", "value": "Dune" }),
        ),
        call(RequestId::Number(3), DELETE, &json!({ "key": "title" })),
        call(
            late.clone(),
            PUT,
            &json!({ "key": "author", "value": "Frank Herbert" }),
        ),
        Message::Notification(Notification {
            jsonrpc: Version,
            method: CANCEL.to_owned(),
            params: Some(to_value(&PluginCancelParams {
                call_id: late.clone(),
                deadline: true,
            })),
        }),
    ];
    let plugin = [
        result(
            RequestId::Number(0),
            &PluginHelloResult {
                protocol_version: PLUGIN_PROTOCOL_VERSION,
                name: "sample-store".to_owned(),
                version: "1.0.0".to_owned(),
                kinds: vec![DeclaredKind {
                    kind: SAMPLE_STORE,
                    capabilities: vec!["put".to_owned()],
                }],
                hook_stages: BTreeSet::new(),
                network_level: NetworkLevel::None,
            },
        ),
        result(RequestId::Number(1), &PluginPingResult {}),
        result(RequestId::Number(2), &json!({ "replaced": false })),
        Message::Error(ErrorResponse {
            jsonrpc: Version,
            id: Some(RequestId::Number(3)),
            error: ErrorObject {
                code: ProtocolFailure::MethodNotFound.code(),
                message: "the Plugin has no such method".to_owned(),
                data: ErrorData {
                    code: ProtocolFailure::MethodNotFound.name().to_owned(),
                    retryable: false,
                    details: Map::new(),
                },
            },
        }),
        result(late, &json!({ "replaced": false })),
    ];
    write(&output.join("engine.jsonl"), &engine);
    write(&output.join("plugin.jsonl"), &plugin);
}
