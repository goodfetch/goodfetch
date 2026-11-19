//! The engine's requests to the program, such as the Crawl requests a
//! crawl's handlers answer (ADR 0093): a Connection hands out each
//! method's requests as a stream, the program answers each once, with a
//! result or an error object, and one it drops unanswered goes back to the
//! engine as `call_cancelled` by the program (ADRs 0093 and 0102). Over a
//! peer the test plays on the in-memory network, and over stdio with the
//! Scripted engine as the Attached engine.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::future::Future;

use goodfetch_collie_client::transport::{AttachedEngine, Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error};
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfoParams};
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject};
use goodfetch_test_support::{ATTACHED_SCRIPT, TestEnv, Tool, tools};
use serde_json::{Map, Value, json};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::oneshot;

use crate::connection::{hello, reply, scripted};

/// The method the tests serve.
const HANDLE: &str = "crawl_request.handle";

/// What the played engine answers `hello` with.
fn said() -> Value {
    json!({
        "protocol_version": "0.1",
        "product": "collie",
        "product_version": "0.0.0",
        "engine_instance": "01J0000000000000000000000",
    })
}

/// The engine's request `id` of `method` with `params`, as one line.
fn request(id: &Value, method: &str, params: &Value) -> Vec<u8> {
    let mut line = serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    )
    .unwrap();
    line.push(b'\n');
    line
}

/// The error a dropped request is answered with (ADR 0102), its message
/// aside: a failure is matched on its Error code.
fn dropped() -> Value {
    json!({
        "code": -32006,
        "data": {"code": "call_cancelled", "retryable": false, "by": "program"},
    })
}

/// The error a request of a method no stream serves is answered with, its
/// message aside.
fn unserved() -> Value {
    json!({
        "code": -32601,
        "data": {"code": "method_not_found", "retryable": false},
    })
}

/// The error of the Client's answer `line` to the request `id`, its message
/// taken out.
fn error_of(line: &str, id: &Value) -> Value {
    let mut answer: Value = serde_json::from_str(line).unwrap();
    assert_eq!(answer["jsonrpc"], "2.0");
    assert_eq!(&answer["id"], id);
    let mut error = answer["error"].take();
    let message = error.as_object_mut().unwrap().remove("message").unwrap();
    assert!(message.is_string(), "an error has a message");
    error
}

/// `error` with `message`, as the Scripted engine reads an answer: whole.
fn with_message(mut error: Value, message: &str) -> Value {
    error["message"] = json!(message);
    error
}

/// The handler's own failure the tests answer with.
fn failure() -> ErrorObject {
    let mut details = Map::new();
    details.insert("fix".to_owned(), json!("retry later"));
    ErrorObject {
        code: -32009,
        message: "the handler gave up".to_owned(),
        data: ErrorData {
            code: "handler_failed".to_owned(),
            retryable: true,
            details,
        },
    }
}

/// Opens a Connection over `network` on the runtime driving the test.
async fn open(network: &Network, endpoint: &Endpoint) -> ClientConnection {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        network,
        endpoint,
        hello(),
    )
    .await
    .unwrap();
    connection
}

/// Covers: story:11-60
#[test]
fn the_program_answers_the_engines_requests_with_a_result_and_with_an_error_object() {
    let (network, mut lines, mut writer) = scripted();
    let (serving, served) = oneshot::channel();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            served.await.unwrap();
            writer
                .write_all(&request(&json!(1), HANDLE, &json!({"n": 1})))
                .await
                .unwrap();
            writer
                .write_all(&request(&json!("two"), HANDLE, &json!({"n": 2})))
                .await
                .unwrap();
            let answered: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(
                answered,
                json!({"jsonrpc": "2.0", "id": 1, "result": {"handled": 1}})
            );
            let failed: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(
                failed,
                json!({"jsonrpc": "2.0", "id": "two", "error": serde_json::to_value(failure()).unwrap()})
            );
            assert_eq!(lines.next_line().await.unwrap(), None, "the Client closed");
        };
        let program = async {
            let connection = open(&network, &Endpoint::InMemory).await;
            let mut requests = connection.handler_requests(HANDLE).unwrap();
            serving.send(()).unwrap();
            let first = requests.next().await.unwrap();
            assert_eq!(first.method(), HANDLE);
            assert_eq!(first.params(), &json!({"n": 1}));
            first.answer(json!({"handled": 1})).unwrap();
            let second = requests.next().await.unwrap();
            assert_eq!(second.params(), &json!({"n": 2}));
            second.fail(failure()).unwrap();
            connection.close().await.unwrap();
            assert!(requests.next().await.is_none(), "a closed Connection ends its streams");
        };
        tokio::join!(peer, program);
    });
}

/// Covers: story:11-60, error:call_cancelled
#[test]
fn a_dropped_request_is_answered_as_cancelled_by_the_program_and_the_connection_keeps_serving() {
    let (network, mut lines, mut writer) = scripted();
    let (serving, served) = oneshot::channel();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            served.await.unwrap();
            writer
                .write_all(&request(&json!(7), HANDLE, &json!({})))
                .await
                .unwrap();
            let given_back = lines.next_line().await.unwrap().unwrap();
            assert_eq!(error_of(&given_back, &json!(7)), dropped());
            writer
                .write_all(&request(&json!(8), HANDLE, &json!({})))
                .await
                .unwrap();
            let answered: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(answered, json!({"jsonrpc": "2.0", "id": 8, "result": {}}));
        };
        let program = async {
            let connection = open(&network, &Endpoint::InMemory).await;
            let mut requests = connection.handler_requests(HANDLE).unwrap();
            serving.send(()).unwrap();
            drop(requests.next().await.unwrap());
            requests.next().await.unwrap().answer(json!({})).unwrap();
            connection
        };
        let ((), connection) = tokio::join!(peer, program);
        connection.close().await.unwrap();
    });
}

/// Runs `program` on a Connection whose played engine answers `hello` and
/// then sends nothing until the Client closes, which `program` does.
fn on_a_quiet_connection<F, Fut>(program: F)
where
    F: FnOnce(ClientConnection) -> Fut,
    Fut: Future<Output = ()>,
{
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            assert_eq!(lines.next_line().await.unwrap(), None, "the Client closed");
        };
        let program = async { program(open(&network, &Endpoint::InMemory).await).await };
        tokio::join!(peer, program);
    });
}

#[test]
fn a_request_of_a_method_no_stream_serves_is_answered_with_the_method_error() {
    let (network, mut lines, mut writer) = scripted();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            writer
                .write_all(&request(&json!(1), "nobody.serves", &json!({})))
                .await
                .unwrap();
            let refused = lines.next_line().await.unwrap().unwrap();
            assert_eq!(error_of(&refused, &json!(1)), unserved());
        };
        let program = async { open(&network, &Endpoint::InMemory).await };
        let ((), connection) = tokio::join!(peer, program);
        connection.close().await.unwrap();
    });
}

#[test]
fn a_second_stream_of_a_served_method_is_refused() {
    on_a_quiet_connection(|connection| async move {
        let _serving = connection.handler_requests(HANDLE).unwrap();
        assert!(matches!(
            connection.handler_requests(HANDLE),
            Err(Error::AlreadyServed { method }) if method == HANDLE
        ));
        connection.close().await.unwrap();
    });
}

#[test]
fn a_dropped_stream_frees_its_method() {
    on_a_quiet_connection(|connection| async move {
        drop(connection.handler_requests(HANDLE).unwrap());
        assert!(connection.handler_requests(HANDLE).is_ok());
        connection.close().await.unwrap();
    });
}

#[test]
fn a_closed_connection_opens_no_stream() {
    on_a_quiet_connection(|connection| async move {
        connection.close().await.unwrap();
        assert!(matches!(
            connection.handler_requests(HANDLE),
            Err(Error::Closed)
        ));
    });
}

#[test]
fn a_request_after_its_stream_is_dropped_is_answered_with_the_method_error() {
    let (network, mut lines, mut writer) = scripted();
    let (dropped_stream, stream_dropped) = oneshot::channel();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            stream_dropped.await.unwrap();
            writer
                .write_all(&request(&json!(1), HANDLE, &json!({})))
                .await
                .unwrap();
            let refused = lines.next_line().await.unwrap().unwrap();
            assert_eq!(error_of(&refused, &json!(1)), unserved());
        };
        let program = async {
            let connection = open(&network, &Endpoint::InMemory).await;
            drop(connection.handler_requests(HANDLE).unwrap());
            dropped_stream.send(()).unwrap();
            connection
        };
        let ((), connection) = tokio::join!(peer, program);
        connection.close().await.unwrap();
    });
}

/// Covers: error:call_cancelled
#[test]
fn a_dropped_stream_answers_the_requests_it_held_as_dropped() {
    let (network, mut lines, mut writer) = scripted();
    let (serving, served) = oneshot::channel();
    let (held, holding) = oneshot::channel();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            served.await.unwrap();
            for id in [1, 2] {
                writer
                    .write_all(&request(&json!(id), HANDLE, &json!({})))
                    .await
                    .unwrap();
            }
            // The Client reads its input in order, so once the third
            // request's answer comes, the stream holds the first two.
            writer
                .write_all(&request(&json!(3), "nobody.serves", &json!({})))
                .await
                .unwrap();
            let refused = lines.next_line().await.unwrap().unwrap();
            assert_eq!(error_of(&refused, &json!(3)), unserved());
            held.send(()).unwrap();
            let mut given_back = Vec::new();
            for _ in 0..2 {
                let answer = lines.next_line().await.unwrap().unwrap();
                let id: Value = serde_json::from_str::<Value>(&answer).unwrap()["id"].take();
                assert_eq!(error_of(&answer, &id), dropped());
                given_back.push(id.as_i64().unwrap());
            }
            given_back.sort_unstable();
            assert_eq!(given_back, [1, 2]);
        };
        let program = async {
            let connection = open(&network, &Endpoint::InMemory).await;
            let requests = connection.handler_requests(HANDLE).unwrap();
            serving.send(()).unwrap();
            holding.await.unwrap();
            drop(requests);
            connection
        };
        let ((), connection) = tokio::join!(peer, program);
        connection.close().await.unwrap();
    });
}

#[test]
fn a_request_held_past_close_cannot_be_answered() {
    let (network, mut lines, mut writer) = scripted();
    let (serving, served) = oneshot::channel();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            served.await.unwrap();
            writer
                .write_all(&request(&json!(1), HANDLE, &json!({})))
                .await
                .unwrap();
            assert_eq!(lines.next_line().await.unwrap(), None, "the Client closed");
        };
        let program = async {
            let connection = open(&network, &Endpoint::InMemory).await;
            let mut requests = connection.handler_requests(HANDLE).unwrap();
            serving.send(()).unwrap();
            let held = requests.next().await.unwrap();
            connection.close().await.unwrap();
            assert!(matches!(held.answer(json!({})), Err(Error::Closed)));
        };
        tokio::join!(peer, program);
    });
}

/// The Scripted engine's play: it sends requests over stdio and checks
/// each answer, a dropped one's included, before it sends the next. Its
/// player matches an answer whole, so the script names the messages too.
fn attached_script() -> Value {
    let request =
        |method: &str, n: u32| json!({"send": {"request": {"method": method, "params": {"n": n}}}});
    json!({
        "description": "An Attached engine sends the program requests of a method it serves and of one it does not, and reads each answer: a result, the handler's error object, the method error, and call_cancelled for the request it dropped.",
        "steps": [
            {"expect": {"method": "hello", "params": serde_json::to_value(hello()).unwrap()}},
            {"reply": {"result": said()}},
            // The program's call comes once it serves the method, as a
            // crawl's start would: a request sent before would find no stream.
            {"expect": {"method": ENGINE_INFO.name, "params": serde_json::to_value(EngineInfoParams::default()).unwrap()}},
            {"reply": {"result": {}}},
            request(HANDLE, 1),
            {"await_reply": {"result": {"handled": 1}}},
            request(HANDLE, 2),
            {"await_reply": {"error": serde_json::to_value(failure()).unwrap()}},
            request("nobody.serves", 3),
            {"await_reply": {"error": with_message(unserved(), "this Client serves no `nobody.serves`")}},
            request(HANDLE, 4),
            {"await_reply": {"error": with_message(dropped(), "the program dropped the engine's request without answering it")}},
            request(HANDLE, 5),
            {"await_reply": {"result": {"handled": 5}}},
        ],
    })
}

/// Covers: story:11-60, error:call_cancelled
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes the Scripted engine's script before any async worker runs"
)]
fn an_attached_engine_reaches_the_program_over_stdio() {
    let env = TestEnv::new();
    let installed = env.home().join(ATTACHED_SCRIPT);
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::write(&installed, serde_json::to_vec(&attached_script()).unwrap()).unwrap();
    let engine = Endpoint::Attached(AttachedEngine {
        executable: tools()
            .unwrap()
            .path(Tool::ScriptedEngine)
            .unwrap()
            .to_path_buf(),
        environment: env
            .env()
            .iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect(),
    });
    let runtime = goodfetch_test_support::real_time_runtime();
    runtime.block_on(async {
        let connection = open(&Network::Os, &engine).await;
        let mut requests = connection.handler_requests(HANDLE).unwrap();
        let _info: Value = connection
            .call(ENGINE_INFO.name, &EngineInfoParams::default())
            .await
            .unwrap();
        let first = requests.next().await.unwrap();
        first.answer(json!({"handled": 1})).unwrap();
        requests.next().await.unwrap().fail(failure()).unwrap();
        drop(requests.next().await.unwrap());
        let last = requests.next().await.unwrap();
        assert_eq!(
            last.params(),
            &json!({"n": 5}),
            "the engine read every answer before"
        );
        last.answer(json!({"handled": 5})).unwrap();
        connection.close().await.unwrap();
    });
}
