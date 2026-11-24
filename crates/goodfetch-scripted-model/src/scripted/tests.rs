//! The Scripted model at its seam (spec #327's Seam B): each call within a
//! live Run of an in-memory engine, as the driver makes it. Call *n* gets
//! reply *n*, each kind with its token usage, and `requests()` lists the
//! requests in order; a call beyond the script fails, and so does a reply
//! left unused, at `assert_done()` and on drop.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Entropy (ADR 0059)"
)]

use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

use futures_util::StreamExt as _;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::ledger::Reservation;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{CallContext as EngineCallContext, Handler, Identity};
use goodfetch_engine::{CallError, Channel, Composition, Contribution, Dependencies, Engine};
use goodfetch_model_provider::{CallContext, ModelProvider as _};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::model::{
    ModelContent, ModelMessage, ModelRequest, ModelResponse, ModelResponsePart, ModelTokens,
    ModelToolCall, ModelToolChoice,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::{ScriptExhausted, ScriptedModel};
use crate::reply::Reply;

/// What one call's stream yields.
type Answer = Vec<Result<ModelResponsePart, Box<dyn Error + Send + Sync>>>;

/// The operation whose handler makes the test's model calls within the Run
/// its params name.
const CALLS: Operation = Operation {
    name: "test.scripted_model_calls",
    description: "Calls the Scripted model with the test's requests, in order.",
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
    arrived_in: PROTOCOL_VERSION,
};

/// The Run the calls are for.
const RUN: &str = "scripted-model-run";

/// What the handler shares with the test: the model, the requests to send
/// it and each call's answer.
struct Calls {
    model: ScriptedModel,
    requests: Vec<ModelRequest>,
    answers: Mutex<Vec<Answer>>,
}

fn failed(error: impl std::fmt::Display) -> CallError {
    CallError::protocol(ProtocolFailure::InternalError, error.to_string())
}

fn dependencies(env: &TestEnv) -> Dependencies {
    Dependencies {
        environment: BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(env.home().to_owned()),
        clock: Clock::paused(env.start_time()),
        entropy: Entropy::seeded(env.seed()),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

/// Calls `model` with each of `requests` in order, within a live Run of an
/// in-memory Collie engine: the model back, then each call's answer.
fn called(model: ScriptedModel, requests: Vec<ModelRequest>) -> (ScriptedModel, Vec<Answer>) {
    let calls = Arc::new(Calls {
        model,
        requests,
        answers: Mutex::new(Vec::new()),
    });
    let handled = Arc::clone(&calls);
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(
            Contribution::new("test-scripted-model").with_handler(Handler::new(
                &CALLS,
                move |params: Value, context: EngineCallContext| {
                    let calls = Arc::clone(&handled);
                    async move {
                        let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
                        let run = context.runs().live(run_id).await.map_err(failed)?;
                        let reservation = Reservation::default();
                        let cancellation = CancellationToken::new();
                        let mut answers = Vec::new();
                        for request in &calls.requests {
                            let context = CallContext {
                                run: &run,
                                reservation: &reservation,
                                deadline: None,
                                cancellation: &cancellation,
                            };
                            answers.push(calls.model.call(request, context).collect().await);
                        }
                        *calls.answers.lock().await = answers;
                        Ok::<Value, CallError>(json!({ "called": true }))
                    }
                },
            )),
        );
    let identity = Identity {
        product: Product::Collie,
        product_version: goodfetch_test_client::product_version(),
        channel: Channel::CratesIo,
    };
    let engine = Engine::new(identity, &composition).expect("invariant: the composition composes");
    let env = TestEnv::new();
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let (reader, mut writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut lines = BufReader::new(reader).lines();
            for message in [
                goodfetch_test_client::hello(Product::Collie),
                json!({ "jsonrpc": "2.0", "id": 1, "method": "run.start", "params": { "run_id": RUN } }),
                json!({ "jsonrpc": "2.0", "id": 2, "method": CALLS.name, "params": { "run_id": RUN } }),
            ] {
                let mut line = serde_json::to_vec(&message).expect("invariant: JSON");
                line.push(b'\n');
                writer.write_all(&line).await.expect("invariant: in memory");
                let reply = loop {
                    let line = lines
                        .next_line()
                        .await
                        .expect("invariant: in memory")
                        .expect("invariant: the engine answers");
                    let reply: Value =
                        serde_json::from_str(&line).expect("invariant: the engine writes JSON");
                    if reply["id"] == message["id"] {
                        break reply;
                    }
                };
                assert!(reply.get("result").is_some(), "{reply}");
            }
            writer.shutdown().await.expect("invariant: in memory");
        };
        let (served, ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(&env)),
            client
        );
        served.expect("invariant: the engine got its Connection");
    });
    drop(engine);
    drop(composition);
    let calls = Arc::into_inner(calls).expect("invariant: the engine let go of its handler");
    (calls.model, calls.answers.into_inner())
}

/// A request asking the model about `topic`.
fn request(topic: &str) -> ModelRequest {
    ModelRequest {
        model: "scripted".to_owned(),
        messages: vec![ModelMessage::User {
            content: vec![ModelContent::Text {
                text: topic.to_owned(),
            }],
        }],
        tools: Vec::new(),
        tool_choice: ModelToolChoice::Auto,
        response_schema: None,
        max_output_tokens: None,
    }
}

fn usage(input_tokens: u64, output_tokens: u64) -> ModelTokens {
    ModelTokens {
        input_tokens,
        output_tokens,
        ..ModelTokens::default()
    }
}

/// The answer's parts, each `Ok`.
fn parts(answer: Answer) -> Vec<ModelResponsePart> {
    answer
        .into_iter()
        .map(|part| part.expect("the call succeeds"))
        .collect()
}

fn text(text: &str) -> ModelResponsePart {
    ModelResponsePart::Text {
        text: text.to_owned(),
    }
}

fn done(text: &str, tool_calls: Vec<ModelToolCall>, finish_reason: &str) -> ModelResponsePart {
    ModelResponsePart::Done {
        response: ModelResponse {
            text: text.to_owned(),
            tool_calls,
            finish_reason: finish_reason.to_owned(),
            refusal: None,
            usage: usage(12, 3),
        },
    }
}

fn tool_call() -> ModelToolCall {
    ModelToolCall {
        id: "call-1".to_owned(),
        name: "click".to_owned(),
        arguments: r##"{"selector":"#buy"}"##.to_owned(),
    }
}

/// Each kind of reply answers its call with its parts and its token usage,
/// the whole response last.
#[rstest]
#[case::text(Reply::text("Dune.", usage(12, 3)), vec![text("Dune."), done("Dune.", Vec::new(), "stop")])]
#[case::tool_calls(
    Reply::tool_calls(vec![tool_call()], usage(12, 3)),
    vec![done("", vec![tool_call()], "tool_calls")]
)]
#[case::structured_output(
    Reply::structured(&json!({ "title": "Dune" }), usage(12, 3)),
    vec![text(r#"{"title":"Dune"}"#), done(r#"{"title":"Dune"}"#, Vec::new(), "stop")]
)]
#[case::stream_of_chunks(
    Reply::stream(["Du", "ne", "."], usage(12, 3)),
    vec![text("Du"), text("ne"), text("."), done("Dune.", Vec::new(), "stop")]
)]
fn each_reply_kind_answers_with_its_usage(
    #[case] reply: Reply,
    #[case] expected: Vec<ModelResponsePart>,
) {
    let (model, mut answers) = called(ScriptedModel::new([reply]), vec![request("title")]);

    assert_eq!(parts(answers.remove(0)), expected);
    model.assert_done();
}

/// A provider error ends its call with that error, which the driver
/// classifies.
#[test]
fn a_provider_error_ends_its_call_with_it() {
    let (model, mut answers) = called(
        ScriptedModel::new([Reply::error(std::io::Error::other("rate limited"))]),
        vec![request("title")],
    );

    let answer = answers.remove(0);
    assert_eq!(answer.len(), 1);
    let Some(Err(error)) = answer.into_iter().next() else {
        unreachable!("one part");
    };
    let error = error
        .downcast::<std::io::Error>()
        .expect("the scripted error");
    assert_eq!(error.to_string(), "rate limited");
    model.assert_done();
}

/// Call *n* gets reply *n*, and `requests()` lists every request in the
/// order it came: the script answered in full passes `assert_done()` and
/// its drop.
#[test]
fn call_n_gets_reply_n_and_requests_are_listed_in_order() {
    let requests = vec![request("first"), request("second"), request("third")];
    let (model, answers) = called(
        ScriptedModel::new([
            Reply::text("one", usage(12, 3)),
            Reply::text("two", usage(12, 3)),
            Reply::text("three", usage(12, 3)),
        ]),
        requests.clone(),
    );

    let texts: Vec<Vec<ModelResponsePart>> = answers.into_iter().map(parts).collect();
    assert_eq!(
        texts,
        vec![
            vec![text("one"), done("one", Vec::new(), "stop")],
            vec![text("two"), done("two", Vec::new(), "stop")],
            vec![text("three"), done("three", Vec::new(), "stop")],
        ]
    );
    assert_eq!(model.requests(), requests);
    model.assert_done();
}

/// A call beyond the script ends with [`ScriptExhausted`], which names it,
/// and fails the test at `assert_done()`, even when the code under test
/// swallowed that error. The panic's text is the overrun's, so a failed
/// assertion before it fails the test.
#[test]
#[should_panic(expected = "1 call beyond its script of 1 reply")]
fn a_call_beyond_the_script_ends_with_script_exhausted_and_fails_at_assert_done() {
    let (model, mut answers) = called(
        ScriptedModel::new([Reply::text("one", usage(12, 3))]),
        vec![request("first"), request("second")],
    );

    let answer = answers.remove(1);
    assert_eq!(answer.len(), 1);
    let Some(Err(error)) = answer.into_iter().next() else {
        unreachable!("one part");
    };
    let error = error
        .downcast::<ScriptExhausted>()
        .expect("the script's end");
    assert_eq!(
        *error,
        ScriptExhausted {
            call: 2,
            replies: 1
        }
    );
    assert_eq!(model.requests(), vec![request("first"), request("second")]);
    model.assert_done();
}

/// A call beyond the script fails the test on drop too.
#[test]
#[should_panic(expected = "1 call beyond its script of 1 reply")]
fn a_call_beyond_the_script_fails_on_drop() {
    let (model, _) = called(
        ScriptedModel::new([Reply::text("one", usage(12, 3))]),
        vec![request("first"), request("second")],
    );

    drop(model);
}

/// A reply left unused fails the test at `assert_done()`.
#[test]
#[should_panic(expected = "1 of its 2 replies unused")]
fn an_unused_reply_fails_at_assert_done() {
    let (model, _) = called(
        ScriptedModel::new([
            Reply::text("one", usage(12, 3)),
            Reply::text("two", usage(12, 3)),
        ]),
        vec![request("first")],
    );

    model.assert_done();
}

/// A reply left unused fails the test on drop, with no `assert_done()`.
#[test]
#[should_panic(expected = "1 of its 2 replies unused")]
fn an_unused_reply_fails_on_drop() {
    let (model, _) = called(
        ScriptedModel::new([
            Reply::text("one", usage(12, 3)),
            Reply::text("two", usage(12, 3)),
        ]),
        vec![request("first")],
    );

    drop(model);
}
