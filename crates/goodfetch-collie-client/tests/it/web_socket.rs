//! A page's WebSockets (ADRs 0081 and 0085), played by the Scripted engine
//! as the Attached engine: `page.events(events::WebSocket)` hands one
//! `WebSocket` per socket the page opens, and each socket's own events,
//! which the same `page.on` call carries, reach that socket's own stream
//! alone, in order, its close last. A socket whose stream is dropped gets
//! nothing more, while the page's stream and the other sockets go on. The
//! script grants credits again once 8 of the call's events are settled:
//! read, or passed over for a socket that closed or whose stream is gone.
//! The call is lossless (ADR 0024): a socket's events left unread hold
//! their credits, so the engine waits and every stream of the call waits
//! with it. Dropping the page's stream cancels the call, and a failed call
//! ends the page's stream with its error; either way each socket's stream
//! hands what it holds, then ends.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_collie_client::transport::Endpoint;
use goodfetch_collie_client::transport::Network;
use goodfetch_collie_client::{
    ClientConnection, ConnectionRuntime, EventStream, Page, Run, WebSocketEvent, blocking, events,
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
    scripted_engine_steps(env, |steps| {
        let events = steps
            .iter_mut()
            .find_map(|step| step.pointer_mut("/send/notification/params/events"))
            .and_then(Value::as_array_mut)
            .unwrap();
        change(events);
    })
}

/// The Scripted engine as [`scripted_engine`]'s, its steps changed by
/// `change`, which names the `page.on` call by the script's own id, 3;
/// then the events of every batch numbered again in order.
fn scripted_engine_steps(env: &TestEnv, change: impl FnOnce(&mut Vec<Value>)) -> Endpoint {
    scripted_engine_changed(env, SCRIPT, |steps| {
        change(steps);
        for step in steps.iter_mut() {
            for named in ["/expect/params/id", "/send/notification/params/id"] {
                if let Some(Value::Number(id)) = step.pointer_mut(named) {
                    *id = (id.as_u64().unwrap() + 1).into();
                }
            }
        }
        let events = steps
            .iter_mut()
            .filter_map(|step| step.pointer_mut("/send/notification/params/events"))
            .filter_map(Value::as_array_mut)
            .flatten();
        for (position, event) in (1_u64..).zip(events) {
            event["position"] = position.into();
        }
    })
}

/// Plays `played` once the `page.on` call got its first credits, in place
/// of the script's batch and what follows it, then the Run's finish.
fn played_once_credited(steps: &mut Vec<Value>, played: Vec<Value>) {
    let credited = steps
        .iter()
        .position(|step| step.pointer("/expect/method") == Some(&json!("$/credit")))
        .unwrap();
    let finish = steps.split_off(steps.len() - 2);
    steps.truncate(credited + 1);
    steps.extend(played);
    steps.extend(finish);
}

/// The engine sends `events` on the `page.on` call, numbered later.
fn batch(events: &[Value]) -> Value {
    json!({"send": {"notification": {
        "method": "$/events",
        "params": {"id": 3, "events": events},
    }}})
}

/// The engine waits for `credits` more on the `page.on` call.
fn credited(credits: u32) -> Value {
    json!({"expect": {"method": "$/credit", "params": {"id": 3, "credits": credits}}})
}

/// The engine waits for the `page.on` call's cancel.
fn cancelled() -> Value {
    json!({"expect": {"method": "$/cancel", "params": {"id": 3}}})
}

/// The page opened the socket `id`.
fn socket_opened(id: &str) -> Value {
    json!({
        "position": 0,
        "event": "page.web_socket",
        "data": {"page_id": "page-1", "web_socket_id": id, "url": format!("wss://20.actionability.test/{id}")},
    })
}

/// The socket `id` received `payload`.
fn frame(id: &str, payload: &str) -> Value {
    json!({
        "position": 0,
        "event": "web_socket.frame_received",
        "data": {"page_id": "page-1", "web_socket_id": id, "payload": payload, "binary": false},
    })
}

/// What `frame` of `payloads` read as.
fn received(payloads: impl IntoIterator<Item = String>) -> Vec<String> {
    payloads
        .into_iter()
        .map(|payload| format!("received {payload}"))
        .collect()
}

/// Opens a Connection to `engine`, starts a Run and opens a page, then
/// plays `program` on the page, finishes the Run and closes the
/// Connection. The page outlives the Run, so that it is closed with it.
fn on_a_page<T>(engine: &Endpoint, program: impl AsyncFnOnce(&Page) -> T) -> T {
    goodfetch_test_support::real_time_runtime().block_on(async {
        let (connection, _said) = ClientConnection::open(
            ConnectionRuntime::driven(tokio::runtime::Handle::current()),
            &Network::Os,
            engine,
            hello(),
        )
        .await
        .unwrap();
        let run: Run = connection.start().await.unwrap();
        let page = run.new_page().await.unwrap();
        let read = program(&page).await;
        run.finish().await.unwrap();
        connection.close().await.unwrap();
        read
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

/// Dropping the page's stream while a socket's stream is read cancels the
/// call with `$/cancel`: that read ends, and the other socket's stream
/// hands every event it holds, then ends. What they settle after the
/// cancel grants no credits to a call that is over.
/// Covers: story:5-48, story:11-63
#[test]
fn dropping_the_pages_stream_cancels_the_call_and_each_socket_hands_what_it_holds() {
    let env = TestEnv::new();
    let engine = scripted_engine_steps(&env, |steps| {
        let mut events = vec![socket_opened("ws-a"), socket_opened("ws-b")];
        events.extend((1..=7).map(|held| frame("ws-a", &format!("a{held}"))));
        // Last, so the read that finds it has put the others in place.
        events.push(frame("ws-b", "b1"));
        played_once_credited(steps, vec![batch(&events), cancelled()]);
    });
    let read = on_a_page(&engine, async |page| {
        let mut sockets = page.events(events::WebSocket);
        let first = sockets.next().await.unwrap().unwrap();
        let second = sockets.next().await.unwrap().unwrap();
        let (read_one, one_read) = tokio::sync::oneshot::channel();
        let reading = async {
            let mut events = second.events();
            let mut read = Vec::new();
            let mut read_one = Some(read_one);
            while let Some(event) = events.next().await {
                read.push(heard(&event.unwrap()));
                if let Some(read_one) = read_one.take() {
                    read_one.send(()).unwrap();
                }
            }
            read
        };
        let cancelling = async move {
            one_read.await.unwrap();
            drop(sockets);
        };
        let (seconds, ()) = tokio::join!(reading, cancelling);
        (seconds, read_all(first.events()).await)
    });

    let held = ["a1", "a2", "a3", "a4", "a5", "a6", "a7"];
    assert_eq!(
        read,
        (
            received(["b1".to_owned()]),
            received(held.map(str::to_owned))
        )
    );
}

/// A `page.on` call the engine fails ends the page's stream with that
/// error, once, and each socket's stream after what it held, here when a
/// socket's stream heard the failure first.
/// Covers: story:5-48, story:11-63
#[test]
fn a_failed_call_ends_the_pages_stream_with_its_error_once_and_every_sockets_stream() {
    let env = TestEnv::new();
    let engine = scripted_engine_steps(&env, |steps| {
        let events = vec![
            socket_opened("ws-a"),
            frame("ws-a", "a1"),
            socket_opened("ws-b"),
        ];
        let failed = json!({"reply": {"error": {
            "code": -32000,
            "message": "events lost",
            "data": {"code": "page_events_lost", "group": "unavailable", "retryable": false},
        }}});
        played_once_credited(steps, vec![batch(&events), failed]);
    });
    let read = on_a_page(&engine, async |page| {
        let mut sockets = page.events(events::WebSocket);
        let first = sockets.next().await.unwrap().unwrap();
        let second = sockets.next().await.unwrap().unwrap();
        let seconds = read_all(second.events()).await;
        let failure = sockets.next().await.map(|item| {
            item.map(|_| ())
                .map_err(|error| error.code().as_str().to_owned())
        });
        let ended = sockets.next().await.is_none();
        (seconds, failure, ended, read_all(first.events()).await)
    });

    assert_eq!(
        read,
        (
            Vec::new(),
            Some(Err("page_events_lost".to_owned())),
            true,
            received(["a1".to_owned()])
        )
    );
}

/// A socket's stream left unread past the call's credits holds them, so
/// the engine sends nothing more and the page's stream and the other
/// socket's wait (ADR 0024); reading it settles its events, and once
/// enough are, the call's next credits go and the others resume. The
/// strict script proves the wait: it sends the others' next events only
/// after those credits.
/// Covers: story:5-48, story:11-63
#[test]
fn a_socket_left_unread_past_the_credits_stalls_the_others_until_it_is_read() {
    let env = TestEnv::new();
    let engine = scripted_engine_steps(&env, |steps| {
        let mut events = vec![socket_opened("ws-a"), socket_opened("ws-b")];
        events.extend((1..=14).map(|held| frame("ws-a", &format!("a{held}"))));
        assert_eq!(events.len(), 16, "the call's first credits");
        let resumed = batch(&[frame("ws-b", "b1"), socket_opened("ws-c")]);
        played_once_credited(
            steps,
            vec![batch(&events), credited(8), resumed, cancelled()],
        );
    });
    let read = on_a_page(&engine, async |page| {
        let mut sockets = page.events(events::WebSocket);
        let first = sockets.next().await.unwrap().unwrap();
        let second = sockets.next().await.unwrap().unwrap();
        let mut firsts = first.events();
        let mut read = Vec::new();
        for _ in 0..6 {
            read.push(heard(&firsts.next().await.unwrap().unwrap()));
        }
        let resumed = heard(&second.events().next().await.unwrap().unwrap());
        let third = sockets.next().await.unwrap().unwrap();
        // Cancelled before the first socket's stream settles what it holds.
        drop(sockets);
        (read, resumed, third.web_socket_id.clone())
    });

    assert_eq!(
        read,
        (
            received(["a1", "a2", "a3", "a4", "a5", "a6"].map(str::to_owned)),
            "received b1".to_owned(),
            "ws-c".to_owned()
        )
    );
}
