use std::time::Duration;

use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream};
use tokio::sync::mpsc;

use super::{HELLO_DEADLINE, Line, Relayed, hello};

/// Longer than any answer these tests wait for, so a call that never ends
/// fails the test instead of holding it (the runtime's clock is paused).
const HANG: Duration = Duration::from_secs(3600);

/// A Line over an in-memory pipe, its tasks `owner`'s, and the engine's
/// end of the pipe; lines that answer no call of the gateway's go to
/// `client`.
fn line(owner: &TaskOwner, client: Option<mpsc::Sender<String>>) -> (Line, DuplexStream) {
    let (gateway, engine) = tokio::io::duplex(4096);
    let (reader, writer) = tokio::io::split(gateway);
    (
        Line::open(BufReader::new(reader), writer, client, owner),
        engine,
    )
}

/// Covers: story:12-232
#[rstest]
#[case::collies_engine(Product::Collie)]
#[case::foxhounds_engine(Product::Foxhound)]
fn the_gateways_hello_says_it_is_the_gateway_and_announces_the_engines_product(
    #[case] product: Product,
) {
    let sent = hello(product, None, None);

    assert_eq!((sent.surface, sent.needs), (Surface::Cloud, Some(product)));
}

#[test]
fn the_gateways_hello_for_a_client_names_the_clients_surface() {
    let relayed = Relayed {
        surface: Surface::Mcp,
        form: None,
        log_level: None,
    };

    let sent = hello(Product::Collie, None, Some(relayed));

    assert_eq!(
        (sent.surface, sent.client_surface),
        (Surface::Cloud, Some(Surface::Mcp))
    );
}

#[test]
fn a_call_on_an_engine_that_exits_before_it_answers_fails_instead_of_waiting() {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let (line, engine) = line(&owner, None);
        owner.spawn(async move {
            // The engine reads the request, then exits without answering.
            let mut lines = BufReader::new(engine).lines();
            let _ = lines.next_line().await;
        });

        let answered = tokio::time::timeout(HANG, line.call("run.start", json!({}))).await;

        assert!(matches!(answered, Ok(Err(_))), "{answered:?}");
    });
}

#[test]
fn a_hello_an_engine_never_answers_fails_at_its_deadline() {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let (line, _engine) = line(&owner, None);

        let answered =
            tokio::time::timeout(HANG, line.call_within("hello", json!({}), HELLO_DEADLINE)).await;

        assert!(
            matches!(&answered, Ok(Err(error)) if error.kind() == std::io::ErrorKind::TimedOut),
            "{answered:?}"
        );
    });
}

#[test]
fn an_engines_line_that_answers_no_call_of_the_gateways_reaches_the_client() {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let (client, mut received) = mpsc::channel(4);
        let (line, engine) = line(&owner, Some(client));
        owner.spawn(async move {
            let (reader, mut writer) = tokio::io::split(engine);
            let mut lines = BufReader::new(reader).lines();
            let Ok(Some(request)) = lines.next_line().await else {
                return;
            };
            let request: Value = serde_json::from_str(&request).unwrap();
            let mut answers = String::new();
            answers.push_str(&json!({"jsonrpc": "2.0", "id": 7, "result": {}}).to_string());
            answers.push('\n');
            answers.push_str(
                &json!({"jsonrpc": "2.0", "id": request["id"], "result": {"own": true}})
                    .to_string(),
            );
            answers.push('\n');
            let _ = writer.write_all(answers.as_bytes()).await;
            let _ = lines.next_line().await;
        });

        let answered = line.call("engine.info", json!({})).await.unwrap();

        assert_eq!(
            (answered["result"].clone(), received.recv().await),
            (
                json!({"own": true}),
                Some(json!({"jsonrpc": "2.0", "id": 7, "result": {}}).to_string())
            )
        );
    });
}
