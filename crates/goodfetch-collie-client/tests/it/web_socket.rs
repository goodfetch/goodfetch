//! A page's WebSockets (ADRs 0081 and 0085), played by the Scripted engine
//! as the Attached engine: `page.events(events::WebSocket)` hands one
//! `WebSocket` per socket the page opens, and each socket's own events,
//! which the same `page.on` call carries, reach that socket's own stream
//! alone, in order, its close last. A socket whose stream is dropped gets
//! nothing more, while the page's stream and the other sockets go on. The
//! script grants credits again once 8 of the call's events are settled:
//! read, or passed over for a socket that closed or whose stream is gone.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_collie_client::transport::Endpoint;
use goodfetch_collie_client::transport::Network;
use goodfetch_collie_client::{
    ClientConnection, ConnectionRuntime, EventStream, WebSocketEvent, blocking, events,
};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::connection::hello;
use crate::session_events::scripted_engine_changed;

/// The script both forms play, which the TypeScript Client plays too: two
/// interleaved sockets, a frame of the first after its close.
const SCRIPT: &str = "page/web-socket.json";

/// The Scripted engine playing [`SCRIPT`] with this Client's call ids: the
/// TypeScript Client numbers its calls from 0, this one from 1, so each
/// `id` the script's messages name is one more.
fn scripted_engine(env: &TestEnv) -> Endpoint {
    scripted_engine_with(env, |_events| {})
}

/// The Scripted engine as [`scripted_engine`]'s, the events of its batch
/// changed by `change`, then numbered again in order.
fn scripted_engine_with(env: &TestEnv, change: impl FnOnce(&mut Vec<Value>)) -> Endpoint {
    scripted_engine_changed(env, SCRIPT, |steps| {
        for step in steps.iter_mut() {
            for named in ["/expect/params/id", "/send/notification/params/id"] {
                if let Some(Value::Number(id)) = step.pointer_mut(named) {
                    *id = (id.as_u64().unwrap() + 1).into();
                }
            }
        }
        let events = steps
            .iter_mut()
            .find_map(|step| step.pointer_mut("/send/notification/params/events"))
            .and_then(Value::as_array_mut)
            .unwrap();
        change(events);
        for (position, event) in (1_u64..).zip(events.iter_mut()) {
            event["position"] = position.into();
        }
    })
}

/// What a socket's events read as: the frame's payload, or `closed`.
fn heard(event: &WebSocketEvent) -> String {
    match event {
        WebSocketEvent::FrameSent(frame) => format!("sent {}", frame.payload),
        WebSocketEvent::FrameReceived(frame) => format!("received {}", frame.payload),
        WebSocketEvent::Close(closed) => format!("closed {}", closed.web_socket_id),
        other => format!("other {other:?}"),
    }
}

/// Every event `stream` hands until it ends.
async fn read_all(mut stream: EventStream<WebSocketEvent>) -> Vec<String> {
    let mut read = Vec::new();
    while let Some(event) = stream.next().await {
        read.push(heard(&event.unwrap()));
    }
    read
}

/// What the script's first socket, `ws-a`, hands: its frames and its
/// close, not the frame after it.
fn first_socket() -> Vec<String> {
    ["sent hello a", "received a1", "closed ws-a"]
        .map(str::to_owned)
        .to_vec()
}

/// What the script's second socket, `ws-b`, hands.
fn second_socket() -> Vec<String> {
    ["received b1", "received b2", "closed ws-b"]
        .map(str::to_owned)
        .to_vec()
}

/// Each socket's stream hands its own frames in order and its close once,
/// then ends; the frame the engine sent after the first socket's close
/// reaches no one.
/// Covers: story:5-48, story:11-62, story:11-63
#[test]
fn each_socket_hands_its_own_frames_in_order_and_its_close_once() {
    let env = TestEnv::new();
    let engine = scripted_engine(&env);
    let read = goodfetch_test_support::real_time_runtime().block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &engine,
            hello(),
        )
        .await
        .unwrap();
        let run = connection.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let mut sockets = page.events(events::WebSocket);
        let first = sockets.next().await.unwrap().unwrap();
        let second = sockets.next().await.unwrap().unwrap();
        let opened = [
            (first.web_socket_id.clone(), first.url.clone()),
            (second.web_socket_id.clone(), second.url.clone()),
        ];
        let firsts = read_all(first.events()).await;
        let seconds = read_all(second.events()).await;
        drop(sockets);

        run.finish().await.unwrap();
        connection.close().await.unwrap();
        (opened, firsts, seconds)
    });

    assert_eq!(
        read,
        (
            [
                (
                    "ws-a".to_owned(),
                    "wss://20.actionability.test/a".to_owned()
                ),
                (
                    "ws-b".to_owned(),
                    "wss://20.actionability.test/b".to_owned()
                ),
            ],
            first_socket(),
            second_socket(),
        )
    );
}

/// A socket whose stream is dropped gets nothing more: the page's stream
/// still hands the next socket, which hands its own events.
/// Covers: story:5-48, story:11-62, story:11-63
#[test]
fn dropping_a_sockets_stream_stops_its_events_alone() {
    let env = TestEnv::new();
    let engine = scripted_engine(&env);
    let read = goodfetch_test_support::real_time_runtime().block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &engine,
            hello(),
        )
        .await
        .unwrap();
        let run = connection.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let mut sockets = page.events(events::WebSocket);
        let first = sockets.next().await.unwrap().unwrap();
        drop(first.events());
        let second = sockets.next().await.unwrap().unwrap();
        let seconds = read_all(second.events()).await;
        drop(sockets);

        run.finish().await.unwrap();
        connection.close().await.unwrap();
        seconds
    });

    assert_eq!(read, second_socket());
}

/// A socket's event a newer engine adds is passed over, as a stream passes
/// over the events it does not read (ADR 0079): the socket's stream hands
/// its known events around it, its close last.
/// Covers: story:5-48, story:11-63
#[test]
fn a_sockets_event_this_client_does_not_know_is_passed_over() {
    let env = TestEnv::new();
    let engine = scripted_engine_with(&env, |events| {
        let added = json!({
            "position": 0,
            "event": "web_socket.error",
            "data": { "page_id": "page-1", "web_socket_id": "ws-a", "message": "newer" },
        });
        events.insert(4, added);
    });
    let read = goodfetch_test_support::real_time_runtime().block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            &engine,
            hello(),
        )
        .await
        .unwrap();
        let run = connection.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let mut sockets = page.events(events::WebSocket);
        let first = sockets.next().await.unwrap().unwrap();
        let second = sockets.next().await.unwrap().unwrap();
        let firsts = read_all(first.events()).await;
        let seconds = read_all(second.events()).await;
        drop(sockets);

        run.finish().await.unwrap();
        connection.close().await.unwrap();
        (firsts, seconds)
    });

    assert_eq!(read, (first_socket(), second_socket()));
}

/// The same script through `blocking`: each socket's `events` iterates
/// its own frames and its close.
/// Covers: story:5-48, story:11-62, story:11-63
#[test]
fn each_socket_iterates_its_own_frames_through_blocking() {
    let env = TestEnv::new();
    let engine = scripted_engine(&env);
    let one = std::num::NonZeroUsize::MIN;
    let owned = goodfetch_runtime::named(goodfetch_runtime::Threads {
        workers: Some(one),
        blocking: one,
    })
    .unwrap();
    let (opened, _said) = goodfetch_test_support::real_time_runtime()
        .block_on(ClientConnection::open(
            ConnectionRuntime::owned(owned),
            &Network::Os,
            &engine,
            hello(),
        ))
        .unwrap();
    let connection = blocking::ClientConnection::from(opened);
    let run = connection.start().send().unwrap();
    let page = run.new_page().send().unwrap();

    let mut sockets = page.events(events::WebSocket);
    let first: blocking::WebSocket = sockets.next().unwrap().unwrap();
    let second = sockets.next().unwrap().unwrap();
    let urls = [first.url.clone(), second.url.clone()];
    let read = |socket: blocking::WebSocket| {
        socket
            .events()
            .map(|event| heard(&event.unwrap()))
            .collect::<Vec<_>>()
    };
    let firsts = read(first);
    let seconds = read(second);
    drop(sockets);

    run.finish().send().unwrap();
    connection.close().unwrap();

    assert_eq!(
        (urls, firsts, seconds),
        (
            [
                "wss://20.actionability.test/a".to_owned(),
                "wss://20.actionability.test/b".to_owned(),
            ],
            first_socket(),
            second_socket(),
        )
    );
}
