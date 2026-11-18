//! The engine's requests to the program, such as the Crawl requests a
//! crawl's handlers answer (ADR 0093): a Connection hands out each
//! method's requests as a stream, the program answers each once, with a
//! result or an error object, and one it drops unanswered goes back to the
//! engine as `call_cancelled` by the program (ADRs 0093 and 0102). Over a
//! peer the test plays on the in-memory network, and over stdio with the
//! Scripted engine as the Attached engine.
//! Covers: story:11-60, error:call_cancelled
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_collie_client::transport::{AttachedEngine, Endpoint, Network};
use goodfetch_collie_client::{Connection, ConnectionRuntime, Error};
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

/// The answer a dropped request gets (ADR 0102).
fn dropped(id: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32006,
            "message": "the program dropped the engine's request without answering it",
            "data": {"code": "call_cancelled", "retryable": false, "by": "program"},
        },
    })
}

/// The answer a request of a method no stream serves gets.
fn unserved(id: &Value, method: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32601,
            "message": format!("this Client serves no `{method}`"),
            "data": {"code": "method_not_found", "retryable": false},
        },
    })
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
async fn open(network: &Network, endpoint: &Endpoint) -> Connection {
    let (connection, _said) = Connection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        network,
        endpoint,
        hello(),
    )
    .await
    .unwrap();
    connection
}

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
            let given_back: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(given_back, dropped(&json!(7)));
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

#[test]
fn a_method_no_stream_serves_is_answered_with_the_method_error_and_one_stream_serves_each_method() {
    let (network, mut lines, mut writer) = scripted();
    let (serving, served) = oneshot::channel();
    let (dropped_stream, stream_dropped) = oneshot::channel();
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let peer = async {
            let hello = lines.next_line().await.unwrap().unwrap();
            writer.write_all(&reply(&hello, &said())).await.unwrap();
            served.await.unwrap();
            writer
                .write_all(&request(&json!(1), "nobody.serves", &json!({})))
                .await
                .unwrap();
            let refused: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(refused, unserved(&json!(1), "nobody.serves"));
            stream_dropped.await.unwrap();
            writer
                .write_all(&request(&json!(2), HANDLE, &json!({})))
                .await
                .unwrap();
            let refused: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(refused, unserved(&json!(2), HANDLE));
        };
        let program = async {
            let connection = open(&network, &Endpoint::InMemory).await;
            let requests = connection.handler_requests(HANDLE).unwrap();
            assert!(matches!(
                connection.handler_requests(HANDLE),
                Err(Error::AlreadyServed { method }) if method == HANDLE
            ));
            serving.send(()).unwrap();
            drop(requests);
            dropped_stream.send(()).unwrap();
            connection
        };
        let ((), connection) = tokio::join!(peer, program);
        let again = connection.handler_requests(HANDLE);
        assert!(again.is_ok(), "a dropped stream frees its method");
        connection.close().await.unwrap();
        assert!(matches!(
            connection.handler_requests("other.method"),
            Err(Error::Closed)
        ));
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
/// each answer, a dropped one's included, before it sends the next.
fn attached_script() -> Value {
    let request =
        |method: &str, n: u32| json!({"send": {"request": {"method": method, "params": {"n": n}}}});
    json!({
        "description": "An Attached engine sends the program requests of a method it serves and of one it does not, and reads each answer: a result, the handler's error object, the method error, and call_cancelled for the request it dropped.",
        "steps": [
            {"expect": {"method": "hello", "params": serde_json::to_value(hello()).unwrap()}},
            {"reply": {"result": said()}},
            request(HANDLE, 1),
            {"await_reply": {"result": {"handled": 1}}},
            request(HANDLE, 2),
            {"await_reply": {"error": serde_json::to_value(failure()).unwrap()}},
            request("nobody.serves", 3),
            {"await_reply": {"error": unserved(&json!(0), "nobody.serves")["error"]}},
            request(HANDLE, 4),
            {"await_reply": {"error": dropped(&json!(0))["error"]}},
            request(HANDLE, 5),
            {"await_reply": {"result": {"handled": 5}}},
        ],
    })
}

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
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let connection = open(&Network::Os, &engine).await;
        let mut requests = connection.handler_requests(HANDLE).unwrap();
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
