//! The JSON-RPC protocol's generated refusal cases (ADR 0066) against a
//! running engine, each Product's executable started as an Attached engine
//! (Pre-merge): the harness sends each fabricated `hello`, announcing the
//! Product the case needs (ADR 0095), with a call behind it. A refusal is
//! the first reply, and the Connection then closes before any other.
//!
//! `browser-host-outdated` is not run here: no Session reaches a Browser
//! host yet, and the Browser host's own handshake runs it once it does.

use goodfetch_conformance::{
    EngineMode, EngineTarget, FabricatedHelloOutcome, send_fabricated_hello,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::refusals::{RefusalCase, Reply};
use goodfetch_test_support::{ATTACHED_SCRIPT, TestEnv, Tool, tools};
use rstest::rstest;

/// The generated case of a Client's `hello` of a newer preview, which counts as
/// a newer major (ADR 0030).
const CLIENT_NEWER_PREVIEW: &str = include_str!(
    "../../../goodfetch-protocol/generated/refusals/protocol/client-newer-preview.json"
);

/// The Scripted engine's scripts these tests play.
const SCRIPTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/it/fixtures/refusals");

#[rstest]
fn each_fabricated_hello_gets_its_generated_reply_from_a_running_engine(
    #[files("../goodfetch-protocol/generated/refusals/protocol/*.json")]
    #[exclude("browser-host-outdated")]
    #[mode = str]
    case: &str,
) {
    let case: RefusalCase = serde_json::from_str(case).unwrap();
    let hello = case.hello.clone().unwrap();
    let env = TestEnv::new();
    let tool = match hello.engine {
        Product::Collie => Tool::Collie,
        Product::Foxhound => Tool::Foxhound,
    };
    let target = EngineTarget {
        executable: tools().unwrap().path(tool).unwrap().to_path_buf(),
        mode: EngineMode::Attached,
        environment: env.env().clone(),
        folder: env.folder().to_path_buf(),
    };

    let outcome = send_fabricated_hello(&target, &case).unwrap();

    assert_eq!(
        outcome,
        match hello.reply {
            Reply::Accepted { protocol_version } => FabricatedHelloOutcome::Accepted {
                protocol_version,
                product: hello.engine,
            },
            Reply::Refused { error_code } => FabricatedHelloOutcome::Refused {
                error_code,
                closed: true,
            },
        }
    );
}

/// The harness on a broken target, the Scripted engine playing an engine
/// that refuses the `hello` and then answers the call behind it (ADR 0069).
#[test]
fn an_engine_that_refuses_but_keeps_answering_is_seen_not_to_close() {
    let env = TestEnv::new();
    let installed = env.home().join(ATTACHED_SCRIPT);
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::copy(
        std::path::Path::new(SCRIPTS).join("refuses-and-keeps-answering-attached.json"),
        &installed,
    )
    .unwrap();
    let target = EngineTarget {
        executable: tools()
            .unwrap()
            .path(Tool::ScriptedEngine)
            .unwrap()
            .to_path_buf(),
        mode: EngineMode::Attached,
        environment: env.env().clone(),
        folder: env.folder().to_path_buf(),
    };
    let case: RefusalCase = serde_json::from_str(CLIENT_NEWER_PREVIEW).unwrap();

    let outcome = send_fabricated_hello(&target, &case).unwrap();

    assert_eq!(
        outcome,
        FabricatedHelloOutcome::Refused {
            error_code: "protocol_version_unsupported".to_owned(),
            closed: false,
        }
    );
}
