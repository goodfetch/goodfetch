//! A name server on loopback for the engine's own resolver: it answers each
//! name from its table over UDP and TCP and records every name it was asked,
//! so a test sees whether a lookup reached it.
#![expect(
    clippy::disallowed_types,
    reason = "the test's name server binds its own socket on loopback, as a name server outside the engine does (ADR 0060 covers the engine's sockets)"
)]

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr};

use hickory_resolver::proto::op::{Message, ResponseCode};
use hickory_resolver::proto::rr::rdata::A;
use hickory_resolver::proto::rr::{RData, Record, RecordType};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

/// What the name server answers for a name.
#[derive(Clone, Debug)]
pub(crate) enum Answer {
    /// The name's IPv4 addresses, and no IPv6 address.
    Addresses(Vec<Ipv4Addr>),
    /// The name does not exist (`NXDOMAIN`).
    NoSuchName,
    /// The server failed (`SERVFAIL`).
    ServerFailure,
}

/// The name server: its sockets, its table and the names it was asked.
pub(crate) struct NameServer {
    udp: UdpSocket,
    tcp: TcpListener,
    answers: BTreeMap<String, Answer>,
    asked: Vec<String>,
}

impl NameServer {
    /// A name server on a free loopback port answering `answers`, keyed by
    /// lowercase names without their trailing dot. Any other name does not
    /// exist.
    pub(crate) async fn start<'name>(
        answers: impl IntoIterator<Item = (&'name str, Answer)>,
    ) -> Self {
        let (tcp, udp) = loop {
            let tcp = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let address = tcp.local_addr().unwrap();
            match UdpSocket::bind(address).await {
                Ok(udp) => break (tcp, udp),
                Err(error) if error.kind() == ErrorKind::AddrInUse => {}
                Err(error) => panic!("binding test name server UDP socket on {address}: {error}"),
            }
        };
        Self {
            udp,
            tcp,
            answers: answers
                .into_iter()
                .map(|(name, answer)| (name.to_owned(), answer))
                .collect(),
            asked: Vec::new(),
        }
    }

    /// Where it listens.
    pub(crate) fn address(&self) -> SocketAddr {
        self.udp.local_addr().unwrap()
    }

    /// The names it was asked, in order, each query once.
    pub(crate) fn asked(&self) -> &[String] {
        &self.asked
    }

    /// Answers UDP and TCP queries until the future is dropped.
    pub(crate) async fn serve(&mut self) {
        let mut buffer = [0_u8; 4096];
        let (queries_tx, mut queries_rx) =
            mpsc::unbounded_channel::<(Vec<u8>, oneshot::Sender<Vec<u8>>)>();
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                received = self.udp.recv_from(&mut buffer) => {
                    let (length, from) = received.unwrap();
                    let query = Message::from_vec(&buffer[..length]).unwrap();
                    let response = self.respond(&query);
                    self.udp
                        .send_to(&response.to_vec().unwrap(), from)
                        .await
                        .unwrap();
                }
                accepted = self.tcp.accept() => {
                    let (stream, _) = accepted.unwrap();
                    let _ = stream.set_nodelay(true);
                    connections.spawn(serve_tcp_client(stream, queries_tx.clone()));
                }
                Some((raw, reply_tx)) = queries_rx.recv() => {
                    let query = Message::from_vec(&raw).unwrap();
                    let response = self.respond(&query).to_vec().unwrap();
                    let _ = reply_tx.send(response);
                }
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    }

    fn respond(&mut self, query: &Message) -> Message {
        let mut response = Message::response(query.metadata.id, query.metadata.op_code);
        response.metadata.recursion_desired = query.metadata.recursion_desired;
        response.metadata.recursion_available = true;
        response.add_queries(query.queries.clone());
        for question in &query.queries {
            let name = question.name().to_ascii();
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            self.asked.push(name.clone());
            match self.answers.get(&name) {
                Some(Answer::Addresses(addresses)) => {
                    if question.query_type() == RecordType::A {
                        for address in addresses {
                            response.add_answer(Record::from_rdata(
                                question.name().clone(),
                                60,
                                RData::A(A(*address)),
                            ));
                        }
                    }
                }
                Some(Answer::NoSuchName) | None => {
                    response.metadata.response_code = ResponseCode::NXDomain;
                }
                Some(Answer::ServerFailure) => {
                    response.metadata.response_code = ResponseCode::ServFail;
                }
            }
        }
        response
    }
}

async fn serve_tcp_client(
    mut stream: TcpStream,
    queries: mpsc::UnboundedSender<(Vec<u8>, oneshot::Sender<Vec<u8>>)>,
) {
    let mut length_bytes = [0_u8; 2];
    loop {
        if stream.read_exact(&mut length_bytes).await.is_err() {
            return;
        }
        let length = usize::from(u16::from_be_bytes(length_bytes));
        let mut query = vec![0_u8; length];
        if stream.read_exact(&mut query).await.is_err() {
            return;
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        if queries.send((query, reply_tx)).is_err() {
            return;
        }
        let Ok(reply) = reply_rx.await else {
            return;
        };
        let reply_len = u16::try_from(reply.len()).unwrap();
        let mut framed = Vec::with_capacity(2 + reply.len());
        framed.extend_from_slice(&reply_len.to_be_bytes());
        framed.extend_from_slice(&reply);
        if stream.write_all(&framed).await.is_err() {
            return;
        }
    }
}
