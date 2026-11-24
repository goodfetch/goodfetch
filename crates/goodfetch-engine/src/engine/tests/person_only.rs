//! Who gives a person's answer (ADR 0149): every person-only operation, on
//! each carriage and under each surface claim. On a local carriage, a
//! machine's Daemon's socket, a `cli` or `kennel` claim is believed, as ADR
//! 0141 has it. On an Exposed Daemon's address, over lines or WebSocket, and
//! through a gateway, no claim is: the call answers `person_required`, whose
//! `fix` names `needs-you answer`.

use std::collections::BTreeSet;

use goodfetch_protocol::catalog::{Product, Reach};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::entries::operations_of;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::daemon::{TOKEN, connect, daemon, the_pick};
use super::websocket::{WebSocketClient, exposed, same_origin};
use super::{Client, engine};
use crate::daemon::DaemonOptions;
use crate::transport::InMemoryDialer;

/// How a Connection reaches the Daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Carriage {
    /// A machine's Daemon's socket, with its token: the one local carriage.
    Socket,
    /// A gateway's Connection on that socket, relaying its Client's claim.
    Relayed,
    /// An Exposed Daemon's address, in newline-delimited JSON-RPC.
    ExposedLines,
    /// An Exposed Daemon's address, the protocol over WebSocket.
    ExposedWebSocket,
}

impl Carriage {
    const ALL: [Self; 4] = [
        Self::Socket,
        Self::Relayed,
        Self::ExposedLines,
        Self::ExposedWebSocket,
    ];

    fn options(self) -> DaemonOptions {
        match self {
            Self::Socket | Self::Relayed => the_pick(),
            Self::ExposedLines | Self::ExposedWebSocket => exposed(),
        }
    }

    /// The surfaces a Connection of this carriage can claim: a gateway
    /// relays no gateway's and no Browser host's.
    fn claims(self) -> impl Iterator<Item = Surface> {
        Surface::ALL.into_iter().filter(move |surface| {
            self != Self::Relayed || !matches!(surface, Surface::Cloud | Surface::BrowserHost)
        })
    }

    /// The `hello` that claims `surface` on this carriage.
    fn hello(self, surface: Surface) -> Value {
        let mut params = json!({
            "protocol_version": "0.1",
            "surface": surface.as_str(),
            "needs": "collie",
            "product_version": "1.4.0",
            "token": TOKEN,
        });
        if self == Self::Relayed {
            params["surface"] = json!("cloud");
            params["client_surface"] = json!(surface.as_str());
        }
        json!({ "jsonrpc": "2.0", "id": 0, "method": "hello", "params": params })
    }
}

/// A Client's end of either kind of Connection.
enum Opened {
    Lines(Client),
    WebSocket(WebSocketClient),
}

impl Opened {
    async fn of(carriage: Carriage, dialer: &InMemoryDialer) -> Self {
        match carriage {
            Carriage::ExposedWebSocket => Self::WebSocket(same_origin(dialer).await),
            Carriage::Socket | Carriage::Relayed | Carriage::ExposedLines => {
                Self::Lines(connect(dialer).await)
            }
        }
    }

    async fn send(&mut self, messages: &[Value]) {
        match self {
            Self::Lines(client) => client.send(messages).await,
            Self::WebSocket(client) => client.send(messages).await,
        }
    }

    async fn next(&mut self) -> Value {
        match self {
            Self::Lines(client) => client.next().await,
            Self::WebSocket(client) => client.next().await,
        }
        .expect("the engine closed before replying")
    }

    /// The reply to the `hello`, id 0.
    async fn replies_to_hello(&mut self) -> Value {
        loop {
            let message = self.next().await;
            if message["id"] == json!(0) {
                return message;
            }
        }
    }

    /// The replies to the calls `1..=count`, by id, read past every other
    /// message.
    async fn replies(&mut self, count: usize) -> Vec<Value> {
        let mut replies = vec![Value::Null; count];
        let mut left = count;
        while left > 0 {
            let message = self.next().await;
            let Some(id) = message["id"].as_u64().filter(|id| *id > 0) else {
                continue;
            };
            replies[usize::try_from(id).unwrap() - 1] = message;
            left -= 1;
        }
        replies
    }
}

/// One person-only call's answer: on which carriage, under which claim.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Asked {
    carriage: Carriage,
    claim: &'static str,
    operation: &'static str,
}

/// Covers: error:person_required, operation:certs.accept, operation:certs.forget,
/// operation:needs_you.allow_consent_domain, operation:needs_you.remove_refetch_domain,
/// operation:needs_you.deny_wall_domain, operation:usage.raise, operation:config.policy_read,
/// operation:config.policy_write, operation:config.user_write
#[test]
fn only_a_person_surface_on_a_local_carriage_gives_a_persons_answer() {
    let operations: Vec<&'static str> = operations_of(Product::Collie)
        .filter(|operation| operation.reach == Reach::PersonOnly)
        .map(|operation| operation.name)
        .collect();
    let mut asked = BTreeSet::new();
    let mut refused = BTreeSet::new();
    let mut fixes = BTreeSet::new();
    for carriage in Carriage::ALL {
        let env = TestEnv::new();
        let calls = operations.clone();
        let (_, answers) = daemon(
            &engine(Product::Collie),
            &env,
            carriage.options(),
            |dialer| async move {
                let mut answers = Vec::new();
                for claim in carriage.claims() {
                    let mut client = Opened::of(carriage, &dialer).await;
                    client.send(&[carriage.hello(claim)]).await;
                    let accepted = client.replies_to_hello().await;
                    assert_eq!(
                        accepted["result"]["product"],
                        json!("collie"),
                        "{carriage:?} {claim:?}: {accepted}"
                    );
                    let requests: Vec<Value> = calls
                        .iter()
                        .zip(1_u64..)
                        .map(|(method, id)| {
                            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": {} })
                        })
                        .collect();
                    client.send(&requests).await;
                    let replies = client.replies(requests.len()).await;
                    answers.extend(
                        calls
                            .iter()
                            .zip(replies)
                            .map(|(operation, reply)| (claim, *operation, reply)),
                    );
                }
                answers
            },
        );
        for (claim, operation, reply) in answers {
            let one = Asked {
                carriage,
                claim: claim.as_str(),
                operation,
            };
            if reply["error"]["data"]["code"] == json!("person_required") {
                fixes.insert(reply["error"]["data"]["fix"].as_str().map(str::to_owned));
                refused.insert(one);
            } else {
                asked.insert(one);
            }
        }
    }

    let believed: BTreeSet<Asked> = operations
        .iter()
        .flat_map(|operation| {
            ["cli", "kennel"].map(|claim| Asked {
                carriage: Carriage::Socket,
                claim,
                operation,
            })
        })
        .collect();
    assert_eq!(
        (asked, refused.len(), fixes),
        (
            believed,
            // Every other claim on the socket, every claim on the other
            // carriages: nine on each, seven relayed.
            operations.len() * (7 + 7 + 9 + 9),
            BTreeSet::from([Some(
                "Give it as a person, at the engine's machine: in a terminal, `goodfetch-collie needs-you answer` takes you through what waits for you, and the Operation's own command prompts where it has one; or in the Kennel there."
                    .to_owned()
            )]),
        )
    );
}
