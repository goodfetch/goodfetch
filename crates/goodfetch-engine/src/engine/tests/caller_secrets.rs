//! The caller as a Secret provider through the engine's public interface
//! over the in-memory network (ADRs 0015 and 0092): a Run's `caller:`
//! reference is resolved by sending `secrets.resolve` to the Client of the
//! Connection that opened the Run, at the moment of use and within
//! `timeout.secret`; its answer is used and masked, and never reaches the
//! Goodfetch home. No resolver, one that fails and one that does not answer
//! in time each raise `secret_unavailable` with `resolver_failed` and a fix
//! naming `resolveSecret`; another Connection is never asked. A test
//! operation stands for a page that uses the value and reflects it.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, Product, Reach, Stability, schema_of,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::daemon::{cli_hello, daemon, opened as daemon_opened, sdk_hello, the_pick};
use super::{Client, attached_on, engine_with, opened};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler};
use crate::secrets::Purpose;

/// The name the test operation resolves its reference for.
const PLACEHOLDER: &str = "test.secret";

/// Resolves a Secret reference for a Run and returns the value.
const USE_SECRET: Operation = Operation {
    name: "test.use_secret",
    description: "Resolves a Secret reference for a Run and returns its value, as a page reflecting it would.",
    class: OperationClass::Effect,
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
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

async fn use_secret(params: Value, context: CallContext) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let reference = SecretReference::parse(params["reference"].as_str().unwrap()).unwrap();
    let run = context.runs().live(run_id.clone()).await.unwrap();
    let purpose = Purpose {
        placeholder: PLACEHOLDER,
        domain: params["domain"].as_str(),
        domains: &[],
        layer: None,
        run_id: Some(&run_id),
    };
    let value = run
        .secrets()
        .resolve(&reference, &purpose)
        .await
        .map_err(|error| crate::run::secret_error(&error))?;
    Ok(json!({ "text": value.expose_secret() }))
}

fn engine() -> super::Engine {
    engine_with(
        Product::Collie,
        Some(
            Contribution::new("test-use-secret")
                .with_handler(Handler::new(&USE_SECRET, use_secret)),
        ),
    )
}

/// Sends the call `id`, then reads until its answer, handing each request
/// the engine sends meanwhile to `asked`, which may reply to it; every
/// message read is kept in `received`.
async fn calling(
    client: &mut Client,
    id: i64,
    method: &str,
    params: Value,
    received: &mut Vec<Value>,
    mut asked: impl AsyncFnMut(&mut Client, &Value),
) -> Value {
    client
        .send(&[json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })])
        .await;
    loop {
        let message = super::owed(client)
            .await
            .expect("the engine closed before replying");
        received.push(message.clone());
        if message.get("method").is_none() && message["id"] == json!(id) {
            return message;
        }
        if message.get("method").is_some() && message.get("id").is_some() {
            asked(client, &message).await;
        }
    }
}

/// A reply to the engine's `secrets.resolve` request `request` with `value`.
fn reply(request: &Value, value: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "value": value } })
}

/// A failure of the engine's request `request` with the Error code `code`.
fn failure(request: &Value, class: i64, code: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request["id"],
        "error": {
            "code": class,
            "message": "the program's resolver said so",
            "data": { "code": code, "retryable": false, "details": {} },
        },
    })
}

/// A Client whose resolver answers `caller:db` with a canary: the engine
/// asks it with the reference's name, the Run and the receiving host, uses
/// the value, masks it in the call's result, and writes it nowhere in the
/// Goodfetch home.
///
/// Covers: story:4-59, error:secret_unavailable
#[test]
fn a_callers_resolver_answers_a_caller_reference_which_is_used_masked_and_never_kept() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let value = canaries.plant(SecretKind::CallerAnswer).value().to_owned();

    let received = attached_on(&engine(), &env, |mut client: Client| {
        let value = value.clone();
        async move {
            let mut received = Vec::new();
            opened(&mut client).await;
            let start = json!({ "run_id": "caller" });
            calling(
                &mut client,
                1,
                "run.start",
                start,
                &mut received,
                async |_, _| {},
            )
            .await;
            let used =
                json!({ "run_id": "caller", "reference": "caller:db", "domain": "example.com" });
            calling(
                &mut client,
                2,
                "test.use_secret",
                used,
                &mut received,
                async |client: &mut Client, request: &Value| {
                    client.send(&[reply(request, &value)]).await;
                },
            )
            .await;
            let finish = json!({ "run_id": "caller" });
            calling(
                &mut client,
                3,
                "run.finish",
                finish,
                &mut received,
                async |_, _| {},
            )
            .await;
            client.close().await;
            while let Some(message) = client.next().await {
                received.push(message);
            }
            received
        }
    });

    let asked: Vec<&Value> = received
        .iter()
        .filter(|message| message["method"] == json!("secrets.resolve"))
        .map(|message| &message["params"])
        .collect();
    let used = received
        .iter()
        .find(|message| message.get("method").is_none() && message["id"] == json!(2))
        .map(|message| message["result"]["text"].clone());
    let bytes = serde_json::to_vec(&received).unwrap();
    let leaks: Vec<String> = canaries
        .leaks(&[
            Output::Bytes {
                name: "received",
                bytes: &bytes,
            },
            Output::Path(env.folder()),
        ])
        .unwrap()
        .into_iter()
        .map(|hit| format!("{:?} in {}", hit.encoding, hit.output))
        .collect();
    assert_eq!(
        (
            asked
                .iter()
                .map(|params| (
                    params["name"].clone(),
                    params["run_id"].clone(),
                    params["domain"].clone()
                ))
                .collect::<Vec<_>>(),
            used,
            leaks
        ),
        (
            vec![(json!("db"), json!("caller"), json!("example.com"))],
            Some(json!(format!("%{PLACEHOLDER}%"))),
            Vec::<String>::new()
        )
    );
}

/// How the opening Connection's Client fails to give a value.
#[derive(Clone, Copy, Debug)]
enum NoValue {
    /// It has no resolver, so it answers that it has no such method.
    NoResolver,
    /// Its resolver threw.
    Threw,
    /// Its resolver gives no answer within `timeout.secret`.
    NoAnswer,
}

/// No resolver, a resolver that throws and one that does not answer within
/// `timeout.secret` each fail the use with `secret_unavailable`,
/// `failure: "resolver_failed"`, and a fix naming `resolveSecret`; the
/// request left unanswered is cancelled.
///
/// Covers: story:4-59, error:secret_unavailable, setting:timeout.secret
#[rstest]
#[case::no_resolver(NoValue::NoResolver)]
#[case::threw(NoValue::Threw)]
#[case::no_answer(NoValue::NoAnswer)]
fn a_caller_reference_no_resolver_answers_is_resolver_failed_naming_resolve_secret(
    #[case] no_value: NoValue,
) {
    let env = TestEnv::new();

    let received = attached_on(&engine(), &env, |mut client: Client| async move {
        let mut received = Vec::new();
        opened(&mut client).await;
        let start =
            json!({ "run_id": "unresolved", "settings": { "timeout": { "secret": 1_000 } } });
        calling(
            &mut client,
            1,
            "run.start",
            start,
            &mut received,
            async |_, _| {},
        )
        .await;
        let used = json!({ "run_id": "unresolved", "reference": "caller:db" });
        calling(
            &mut client,
            2,
            "test.use_secret",
            used,
            &mut received,
            async |client: &mut Client, request: &Value| match no_value {
                NoValue::NoResolver => {
                    client
                        .send(&[failure(request, -32601, "method_not_found")])
                        .await;
                }
                NoValue::Threw => {
                    client
                        .send(&[failure(request, -32000, "handler_failed")])
                        .await;
                }
                NoValue::NoAnswer => {}
            },
        )
        .await;
        client.close().await;
        while let Some(message) = client.next().await {
            received.push(message);
        }
        received
    });

    let request_id = received
        .iter()
        .find(|message| message["method"] == json!("secrets.resolve"))
        .map(|message| message["id"].clone());
    let cancelled = received
        .iter()
        .any(|message| message["method"] == json!("$/cancel"));
    let error = received
        .iter()
        .find(|message| message.get("method").is_none() && message["id"] == json!(2))
        .map(|message| message["error"]["data"].clone())
        .unwrap_or_default();
    let fix = error["fix"].as_str().unwrap_or_default();
    assert_eq!(
        (
            request_id.is_some(),
            error["code"].clone(),
            error["failure"].clone(),
            fix.contains("resolveSecret") && fix.contains("resolve_secret"),
            cancelled,
        ),
        (
            true,
            json!("secret_unavailable"),
            json!("resolver_failed"),
            true,
            matches!(no_value, NoValue::NoAnswer),
        ),
        "{error}"
    );
}

/// On a Daemon, a Connection that did not open the Run is never asked: its
/// use of the Run's `caller:` reference asks only the opening Connection's
/// Client, which has no resolver, and fails with `secret_unavailable`.
///
/// Covers: story:4-59, error:secret_unavailable
#[test]
fn a_connection_that_did_not_open_the_run_is_never_asked() {
    let env = TestEnv::new();

    let (_, (opener, other)) = daemon(&engine(), &env, the_pick(), |dialer| async move {
        let (mut opener_received, mut other_received) = (Vec::new(), Vec::new());
        let mut opener = daemon_opened(&dialer, sdk_hello()).await;
        let mut other = daemon_opened(&dialer, cli_hello()).await;
        let start = json!({ "run_id": "opened" });
        calling(
            &mut opener,
            1,
            "run.start",
            start,
            &mut opener_received,
            async |_, _| {},
        )
        .await;
        let used = json!({ "run_id": "opened", "reference": "caller:db" });
        other
            .send(&[
                json!({ "jsonrpc": "2.0", "id": 1, "method": "test.use_secret", "params": used }),
            ])
            .await;
        loop {
            let message = super::owed(&mut opener).await.expect("the opener is asked");
            opener_received.push(message.clone());
            if message["method"] == json!("secrets.resolve") {
                opener
                    .send(&[failure(&message, -32601, "method_not_found")])
                    .await;
                break;
            }
        }
        loop {
            let message = super::owed(&mut other).await.expect("the engine replies");
            other_received.push(message.clone());
            if message.get("method").is_none() && message["id"] == json!(1) {
                break;
            }
        }
        let finish = json!({ "run_id": "opened" });
        calling(
            &mut opener,
            2,
            "run.finish",
            finish,
            &mut opener_received,
            async |_, _| {},
        )
        .await;
        opener.close().await;
        other.close().await;
        (opener_received, other_received)
    });

    let asked = |received: &[Value]| {
        received
            .iter()
            .filter(|message| message["method"] == json!("secrets.resolve"))
            .count()
    };
    let error = other
        .last()
        .map(|message| message["error"]["data"].clone())
        .unwrap_or_default();
    assert_eq!(
        (
            asked(&opener),
            asked(&other),
            error["code"].clone(),
            error["failure"].clone()
        ),
        (1, 0, json!("secret_unavailable"), json!("resolver_failed")),
        "{error}"
    );
}
