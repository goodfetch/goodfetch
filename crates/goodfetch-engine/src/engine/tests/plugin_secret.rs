//! Secret provider Plugin kind (ADRs 0015, 0033, 0092 and 0129, issue 996):
//! a toy Secret provider resolves `plugin:<scheme>:<rest>` references through
//! `secret.resolve`, the resolved value is used and masked by the engine and
//! appears in no Effect journal, Run trace, Usage, Audit log or log output;
//! handler failure, malformed answer or unreachable Plugin fails with
//! `secret_unavailable` naming the reference without leaking raw Plugin text.

use std::sync::Arc;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, ProtocolVersion, Reach, Stability, schema_of,
};
use goodfetch_protocol::home::RUN_TRACE_PLUGIN_CALLS;
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::plugin::secret::SECRET_RESOLVE;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::SECRET_UNAVAILABLE;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv, capturing};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tracing::Level;

use super::run::call;
use super::{attached_on, engine_with};
use crate::composition::{Contribution, Handler};
use crate::plugin_gate::PluginFetch;
use crate::plugin_host::instance::{InMemoryPeer, InMemoryPlugin, PluginInstance, Scope};
use crate::plugin_kinds::secret::{ProcessSecretProvider, SecretProviderDriver, secret_deadline};
use crate::run::accounts::RunAccounts;
use crate::runs::RunParts;
use crate::secrets::{ConfigProblem, Purpose, Resolving, SecretProvider};
use crate::settings::{Layers, resolve};

/// Scripted reply for one `secret.resolve` call.
#[derive(Clone)]
enum ScriptedReply {
    Value(Value),
    Error { code: i64, message: String },
    DropConnection,
}

async fn send_json(to: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, msg: Value) {
    let mut bytes = serde_json::to_vec(&msg).unwrap();
    bytes.push(b'\n');
    to.write_all(&bytes).await.unwrap();
    to.flush().await.unwrap();
}

/// Spawns an in-memory Secret provider peer that completes `hello` and serves
/// each `secret.resolve` with the scripted reply.
fn spawn_secret_peer(
    owner: &crate::tasks::TaskOwner,
    peer: InMemoryPeer,
    manifest: PluginManifest,
    reply: ScriptedReply,
) {
    owner.spawn(async move {
        let (from_engine, _) = tokio::io::split(peer.from_engine);
        let (_, mut to_engine) = tokio::io::split(peer.to_engine);
        let mut lines = BufReader::new(from_engine).lines();

        let Some(Ok(hello_line)) = lines.next_line().await.transpose() else {
            return;
        };
        let hello: Value = serde_json::from_str(&hello_line).unwrap();
        send_json(
            &mut to_engine,
            json!({
                "jsonrpc": "2.0",
                "id": hello["id"],
                "result": {
                    "protocol_version": "0.1",
                    "name": manifest.name,
                    "version": manifest.version,
                    "kinds": [{ "kind": "secret", "capabilities": ["resolve"] }],
                    "network_level": manifest.network_level.as_str(),
                }
            }),
        )
        .await;

        while let Some(Ok(line)) = lines.next_line().await.transpose() {
            let msg: Value = serde_json::from_str(&line).unwrap();
            let Some(method) = msg.get("method").and_then(Value::as_str) else {
                continue;
            };
            if method != SECRET_RESOLVE {
                continue;
            }
            let id = msg["id"].clone();
            match &reply {
                ScriptedReply::Value(result) => {
                    send_json(
                        &mut to_engine,
                        json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                    )
                    .await;
                }
                ScriptedReply::Error { code, message } => {
                    send_json(
                        &mut to_engine,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": code, "message": message },
                        }),
                    )
                    .await;
                }
                ScriptedReply::DropConnection => {
                    drop(to_engine);
                    return;
                }
            }
        }
    });
}

fn secret_manifest(name: &str, scheme: &str) -> PluginManifest {
    PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": name,
            "version": "1.0.0",
            "kinds": ["secret"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": "none",
            "scheme": scheme,
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}

/// Bridges an in-memory `ProcessSecretProvider` + `SecretProviderDriver` into
/// `RunSecrets` for testing the full `RunSecrets` + `SecretProviderDriver` +
/// `RunAccounts` + `RunTrace` + `AuditLog` path in process.
struct InMemorySecretBridge {
    provider: ProcessSecretProvider,
    parts: RunParts,
}

impl SecretProvider for InMemorySecretBridge {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            let accounts = RunAccounts {
                identity: &self.parts.identity,
                charging: &self.parts.charging,
                trace: &self.parts.trace,
            };
            let clock = self.parts.identity.clock();
            let deadline = secret_deadline(None);
            SecretProviderDriver::new(&self.provider, &accounts, &clock, &deadline)
                .resolve(reference)
                .await
        })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}

fn secret_test_contribution(
    op_name: &'static str,
    plugin_name: &'static str,
    reference_str: &'static str,
    reply: ScriptedReply,
    expected: Option<String>,
) -> Contribution {
    let op: &'static Operation = Box::leak(Box::new(Operation {
        name: op_name,
        description: "Resolves a Secret reference via a Secret provider Plugin.",
        class: OperationClass::Read,
        kind: OperationKind::RoundTrip,
        reach: Reach::AllSurfaces,
        params: schema_of::<Value>,
        result: schema_of::<Value>,
        events: &[],
        cancellable: false,
        input: None,
        bound: None,
        stability: Stability::Experimental,
        deprecated: None,
        arrived_in: ProtocolVersion { major: 0, minor: 1 },
    }));

    Contribution::new("test-secret-provider").with_handler(Handler::new(
        op,
        move |params: Value, context| {
            let reply = reply.clone();
            let expected = expected.clone();
            async move {
                let run_id: RunId = serde_json::from_value(params["run_id"].clone()).unwrap();
                let parts = context.runs().live(run_id.clone()).await.unwrap();
                let manifest = secret_manifest(plugin_name, plugin_name);
                let clock = context.engine().clock();
                let owner = context.engine().owner();
                let resolved_settings =
                    resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
                let fetch = PluginFetch::of(
                    &resolved_settings,
                    plugin_name,
                    clock,
                    owner.blocking_threads(),
                )
                .unwrap();
                let (instance, peer) = PluginInstance::start_in_memory(
                    owner,
                    InMemoryPlugin {
                        manifest: manifest.clone(),
                        product: Product::Collie,
                        fetch: Arc::new(fetch),
                        clock,
                        spend: None,
                    },
                    Scope::Run(run_id.clone()),
                );
                spawn_secret_peer(owner, peer, manifest, reply);
                let folder = parts.identity.folder().display().to_string();
                let provider = ProcessSecretProvider::new(plugin_name.to_owned(), instance.clone());
                let run_secrets = parts.keys.secrets().without_plugin_provider();
                run_secrets.set_plugin_provider(Arc::new(InMemorySecretBridge { provider, parts }));

                let reference = SecretReference::parse(reference_str).unwrap();
                let resolved = run_secrets
                    .resolve(
                        &reference,
                        &Purpose {
                            placeholder: "db_password",
                            domain: Some("shop.test"),
                            domains: &["shop.test".to_owned()],
                            layer: None,
                            run_id: Some(&run_id),
                        },
                    )
                    .await;
                instance.stop().await;
                let value = resolved.map_err(|error| crate::secrets::secret_error(&error))?;
                let matched = expected
                    .as_deref()
                    .is_some_and(|want| value.expose_secret() == want);
                let masked = run_secrets
                    .mask()
                    .masks()
                    .apply(
                        &format!("token={}", value.expose_secret()),
                        crate::mask::Scope::Kept,
                    )
                    .into_owned();
                Ok::<_, crate::CallError>(json!({
                    "matched": matched,
                    "masked": masked,
                    "folder": folder,
                }))
            }
        },
    ))
}

async fn send_hello(client: &mut super::Client) {
    client
        .send(&[json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "hello",
            "params": {
                "protocol_version": "0.1",
                "surface": "cli",
                "needs": "collie",
                "product_version": "1.4.0",
            },
        })])
        .await;
    client.next().await.unwrap();
}

/// A toy Secret provider with a declared scheme resolves a planted canary
/// reference; the engine receives and masks the value, records only metadata
/// in the Audit log and Run trace, writes no Effect journal entry, and leaves
/// the canary in no file, report or log (ADRs 0015, 0033, 0092).
///
/// Covers: story:7-90, story:7-40
#[test]
fn a_secret_provider_resolves_a_canary_with_zero_leaks_and_no_effect_journal() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let planted = canaries.plant(SecretKind::HeaderValue).value().to_owned();

    let (subscriber, captured_logs) = capturing(&[("goodfetch_engine", Level::DEBUG)]);
    let _guard = tracing::subscriber::set_default(subscriber);

    let contribution = secret_test_contribution(
        "test.resolve_secret",
        "toy-vault",
        "plugin:toy-vault:db/password",
        ScriptedReply::Value(json!({ "value": planted })),
        Some(planted.clone()),
    );

    let engine = engine_with(Product::Collie, Some(contribution));
    let (answer, report, usage, audit_entries) =
        attached_on(&engine, &env, |mut client| async move {
            send_hello(&mut client).await;
            let _ = call(
                &mut client,
                1,
                "run.start",
                json!({ "run_id": "secret-run" }),
            )
            .await;
            let answer = call(
                &mut client,
                2,
                "test.resolve_secret",
                json!({ "run_id": "secret-run" }),
            )
            .await;
            let usage = call(
                &mut client,
                3,
                "run.usage",
                json!({ "run_id": "secret-run" }),
            )
            .await;
            let report = call(
                &mut client,
                4,
                "run.finish",
                json!({ "run_id": "secret-run" }),
            )
            .await;
            let audit_entries = call(&mut client, 5, "audit.export", json!({})).await;
            (answer, report, usage, audit_entries)
        });

    assert_eq!(answer["result"]["matched"], json!(true), "{answer}");
    assert_eq!(
        answer["result"]["masked"],
        json!("token=%db_password%"),
        "{answer}"
    );

    let run_folder = std::path::PathBuf::from(answer["result"]["folder"].as_str().unwrap());
    assert_secret_trace_and_no_leaks(
        &env,
        &run_folder,
        &canaries,
        &report,
        &usage,
        &audit_entries,
        &captured_logs.written(),
    );
}

fn assert_secret_trace_and_no_leaks(
    env: &TestEnv,
    run_folder: &std::path::Path,
    canaries: &Canaries,
    report: &Value,
    usage: &Value,
    audit_entries: &Value,
    logs: &[u8],
) {
    let trace_text = std::fs::read_to_string(run_folder.join(RUN_TRACE_PLUGIN_CALLS)).unwrap();
    let trace_lines: Vec<Value> = trace_text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(trace_lines.len(), 1, "{trace_text}");
    assert_eq!(
        (
            trace_lines[0]["plugin"].as_str(),
            trace_lines[0]["kind"].as_str(),
            trace_lines[0]["method"].as_str(),
            trace_lines[0]["class"].as_str(),
            trace_lines[0]["failed"].is_null(),
        ),
        (
            Some("toy-vault"),
            Some("secret"),
            Some("secret.resolve"),
            Some("read"),
            true,
        )
    );
    let connection = rusqlite::Connection::open(run_folder.join("run.sqlite")).unwrap();
    let effect_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM effect_journal", [], |row| row.get(0))
        .unwrap();
    assert_eq!(effect_count, 0);
    assert_eq!(
        usage["result"]["plugins"],
        json!([{ "plugin": "toy-vault", "calls": 1, "time": 0 }]),
        "{usage}"
    );
    let report_json = report.to_string();
    let usage_json = usage.to_string();
    let audit_json = audit_entries.to_string();
    let hits = canaries
        .search(&[
            Output::Path(env.home()),
            Output::Bytes {
                name: "report",
                bytes: report_json.as_bytes(),
            },
            Output::Bytes {
                name: "usage",
                bytes: usage_json.as_bytes(),
            },
            Output::Bytes {
                name: "audit",
                bytes: audit_json.as_bytes(),
            },
            Output::Bytes {
                name: "logs",
                bytes: logs,
            },
        ])
        .unwrap();
    assert!(hits.is_empty(), "canary leaked: {hits:?}");
}

/// Handler failure, malformed answer and an unreachable Secret provider each
/// fail with `secret_unavailable` naming the reference and never leak raw
/// Plugin text into the error or logs (ADRs 0033 and 0092).
///
/// Covers: story:7-90
#[test]
fn secret_provider_failures_answer_secret_unavailable_without_leaking_plugin_text() {
    let raw_secret_leak = "SENSITIVE_PLUGIN_ECHO_996_DO_NOT_LEAK";
    let cases = [
        (
            "handler-error",
            ScriptedReply::Error {
                code: -32000,
                message: format!("vault error echoing {raw_secret_leak}"),
            },
        ),
        (
            "extra-field",
            ScriptedReply::Value(json!({ "value": raw_secret_leak, "extra": true })),
        ),
        (
            "missing-value",
            ScriptedReply::Value(json!({ "wrong": raw_secret_leak })),
        ),
        ("unreachable", ScriptedReply::DropConnection),
    ];

    for (case_name, reply) in cases {
        let env = TestEnv::new();
        let (subscriber, captured_logs) = capturing(&[("goodfetch_engine", Level::DEBUG)]);
        let _guard = tracing::subscriber::set_default(subscriber);

        let contribution = secret_test_contribution(
            "test.resolve_fail",
            "broken-vault",
            "plugin:broken-vault:api/token",
            reply,
            None,
        );
        let engine = engine_with(Product::Collie, Some(contribution));
        let answer = attached_on(&engine, &env, |mut client| async move {
            send_hello(&mut client).await;
            let _ = call(&mut client, 1, "run.start", json!({ "run_id": "fail-run" })).await;
            let answer = call(
                &mut client,
                2,
                "test.resolve_fail",
                json!({ "run_id": "fail-run" }),
            )
            .await;
            let _ = call(
                &mut client,
                3,
                "run.finish",
                json!({ "run_id": "fail-run" }),
            )
            .await;
            answer
        });

        assert_eq!(
            answer["error"]["data"]["code"],
            json!(SECRET_UNAVAILABLE.name),
            "case {case_name}: {answer}"
        );
        assert_eq!(
            answer["error"]["data"]["reference"],
            json!("plugin:broken-vault:api/token"),
            "case {case_name}: {answer}"
        );
        assert!(
            !answer.to_string().contains(raw_secret_leak),
            "case {case_name} leaked raw text in error: {answer}"
        );
        let logs = String::from_utf8_lossy(&captured_logs.written()).into_owned();
        assert!(
            !logs.contains(raw_secret_leak),
            "case {case_name} leaked raw text in logs: {logs}"
        );
    }
}
