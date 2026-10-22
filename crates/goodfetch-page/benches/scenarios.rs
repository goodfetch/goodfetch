//! The page crate's benchmark scenario (ADR 0071), shared by its Callgrind
//! and DHAT targets: the guard's check-and-arm round trip as the engine
//! does it, a fixed number of times. Each round trip builds the guard's
//! very command (its script, its argument and the call into its world, the
//! crate's own modules compiled in here), sends it over a real CDP
//! connection to a browser end in the same process, which answers every
//! command with a ready target, and reads the answer as the guard reads
//! it. Only the browser's own work is left out. Inputs are literals; no
//! network, no sleep, no work whose amount depends on timing.

#[path = "../src/guard/check.rs"]
#[expect(
    dead_code,
    reason = "the scenario reads only a ready target's answer, never a waiting one's details"
)]
mod check;
#[path = "../src/driver/world.rs"]
mod world;

use std::time::Duration;

use goodfetch_cdp::{Connection, Framer};
use goodfetch_test_support::runtime;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};

/// How long the guard's check may wait in the page.
const SLICE: Duration = Duration::from_secs(1);

/// What the browser end answers each check with: a ready, armed target.
fn ready(id: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&json!({
        "id": id,
        "result": { "result": { "type": "object", "value": { "state": "ready", "x": 180.5, "y": 120.0 } } },
    }))
    .expect("invariant: an answer encodes");
    bytes.push(0);
    bytes
}

/// A browser end that answers every command it reads with [`ready`],
/// until the client closes its end.
async fn browser(mut stream: DuplexStream) {
    let mut framer = Framer::default();
    let mut buffer = vec![0; 1 << 16];
    loop {
        while let Some(frame) = framer.next_frame().expect("invariant: the client frames") {
            let command: Value =
                serde_json::from_slice(&frame).expect("invariant: a command is JSON");
            if stream.write_all(&ready(&command["id"])).await.is_err() {
                return;
            }
        }
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(count) => framer.push(&buffer[..count]),
        }
    }
}

/// `count` check-and-arm round trips; returns how many found the target
/// ready.
pub(crate) fn check_and_arm(count: usize) -> usize {
    runtime().block_on(async {
        let (client, end) = tokio::io::duplex(1 << 16);
        let (reader, writer) = tokio::io::split(client);
        let (connection, driver) = Connection::new(reader, writer);
        let (_, ready, ()) = tokio::join!(
            driver.run(),
            async {
                let mut ready = 0;
                for _ in 0..count {
                    let command = world::call(
                        check::CHECK,
                        check::argument("#target", "pointer", SLICE),
                        1,
                    );
                    let returned = connection
                        .send(None, &command, Duration::from_secs(30))
                        .await
                        .expect("invariant: the browser end answers every check");
                    let seen = check::Seen::read(&returned.result.value.unwrap_or_default())
                        .expect("invariant: the browser end answers a ready target");
                    if matches!(
                        seen,
                        check::Seen::Ready {
                            x: Some(_),
                            y: Some(_)
                        }
                    ) {
                        ready += 1;
                    }
                }
                drop(connection);
                ready
            },
            browser(end)
        );
        ready
    })
}
