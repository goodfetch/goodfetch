//! The opened call under loom (ADR 0060): two values' streams read on two
//! threads, each reading the call's batches for both, so that whichever
//! reads a batch puts the other's event in the other's queue. Neither loses
//! a wakeup: a lost one shows as loom's deadlock, a reader waiting on an
//! event the other queued while the model's thread waits for both.

use std::collections::VecDeque;
use std::future::poll_fn;
use std::task::{Context, Poll, Waker};

use goodfetch_protocol::events::EventEnvelope;
use loom::sync::{Arc, Mutex};
use serde_json::{Value, json};

use super::{CallFlow, Opened, Opening, Opens};
use crate::error::Error;

/// What the call's `page.web_socket` events open, as the Client reads them.
const WEB_SOCKETS: Opens = Opens {
    opener: "page.web_socket",
    namespace: "web_socket",
    id: "web_socket_id",
    events: &["web_socket.close", "web_socket.frame_received"],
};

/// The call's batches the engine sent and its streams have not read, and
/// the waker of the stream that found none.
#[derive(Default)]
struct Batches {
    sent: VecDeque<EventEnvelope>,
    ended: bool,
    reader: Option<Waker>,
}

/// The call's batches, which the model's thread sends as the engine would.
#[derive(Clone)]
struct Sent(Arc<Mutex<Batches>>);

impl Sent {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Batches::default())))
    }

    /// Sends a batch of `events`, waking the stream that found none.
    fn batch(&self, events: impl IntoIterator<Item = EventEnvelope>) {
        self.change(|batches| batches.sent.extend(events));
    }

    /// Ends the call, as its reply does.
    fn end(&self) {
        self.change(|batches| batches.ended = true);
    }

    fn change(&self, change: impl FnOnce(&mut Batches)) {
        let reader = {
            let mut batches = self.0.lock().expect("no holder panics");
            change(&mut batches);
            batches.reader.take()
        };
        if let Some(reader) = reader {
            reader.wake();
        }
    }
}

impl CallFlow for Sent {
    fn poll_envelope(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<EventEnvelope, Error>>> {
        let mut batches = self.0.lock().expect("no holder panics");
        if let Some(envelope) = batches.sent.pop_front() {
            return Poll::Ready(Some(Ok(envelope)));
        }
        if batches.ended {
            return Poll::Ready(None);
        }
        batches.reader = Some(context.waker().clone());
        Poll::Pending
    }

    fn settle(&mut self) {}

    fn end(&mut self) {}
}

/// The event `event` of the socket `id`, with its `data`.
fn envelope(event: &str, id: &str, data: Value) -> EventEnvelope {
    let mut data = data;
    data["web_socket_id"] = json!(id);
    serde_json::from_value(json!({"position": 1, "event": event, "data": data}))
        .expect("an event envelope")
}

/// The socket `id` opened.
fn opened(id: &str) -> EventEnvelope {
    envelope("page.web_socket", id, json!({"url": "wss://loom.test/"}))
}

/// The socket `id` received `payload`.
fn frame(id: &str, payload: &str) -> EventEnvelope {
    envelope(
        "web_socket.frame_received",
        id,
        json!({"payload": payload, "binary": false}),
    )
}

/// The next value `opening` hands.
fn next_opened(opening: &Opening<Sent>) -> Opened<Sent> {
    let (_, opened) = loom::future::block_on(poll_fn(|context| opening.poll_opened(context)))
        .expect("a value opened")
        .expect("the call goes on");
    opened
}

/// The payloads `opened` hands until it ends.
fn payloads(opened: &Opened<Sent>) -> Vec<Value> {
    loom::future::block_on(async {
        let mut read = Vec::new();
        while let Some(event) = poll_fn(|context| opened.poll_event(context)).await {
            read.push(event["data"]["payload"].clone());
        }
        read
    })
}

#[test]
fn two_readers_routing_into_each_others_queues_never_lose_a_wakeup() {
    loom::model(|| {
        let sent = Sent::new();
        sent.batch([opened("ws-a"), opened("ws-b")]);
        let opening = Opening::new(sent.clone(), WEB_SOCKETS);
        let readers = [next_opened(&opening), next_opened(&opening)]
            .map(|opened| loom::thread::spawn(move || payloads(&opened)));
        // Each event is the other reader's to find first.
        sent.batch([frame("ws-b", "b1"), frame("ws-a", "a1")]);
        sent.end();
        let read = readers.map(|reader| reader.join().expect("a reader does not panic"));
        assert_eq!(read, [[json!("a1")], [json!("b1")]]);
        drop(opening);
    });
}

/// Dropping the page's stream cancels the call while both values' streams
/// wait on two threads: no batch wakes them, so the drop does. Each hands
/// the event a read routed to it before the drop, if any, then ends.
#[test]
fn dropping_the_opening_wakes_the_values_streams_that_wait() {
    loom::model(|| {
        let sent = Sent::new();
        sent.batch([opened("ws-a"), opened("ws-b")]);
        let opening = Opening::new(sent.clone(), WEB_SOCKETS);
        let readers = [next_opened(&opening), next_opened(&opening)]
            .map(|opened| loom::thread::spawn(move || payloads(&opened)));
        sent.batch([frame("ws-b", "b1"), frame("ws-a", "a1")]);
        drop(opening);
        let read = readers.map(|reader| reader.join().expect("a reader does not panic"));
        let [a, b] = &read;
        assert!(a.is_empty() || a == &[json!("a1")], "{read:?}");
        assert!(b.is_empty() || b == &[json!("b1")], "{read:?}");
    });
}
