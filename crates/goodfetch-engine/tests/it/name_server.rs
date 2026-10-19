//! A name server on loopback for the engine's own resolver: it answers each
//! name from its table over UDP and records every name it was asked, so a
//! test sees whether a lookup reached it.
#![expect(
    clippy::disallowed_types,
    reason = "the test's name server binds its own socket on loopback, as a name server outside the engine does (ADR 0060 covers the engine's sockets)"
)]

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};

use hickory_resolver::proto::op::{Message, ResponseCode};
use hickory_resolver::proto::rr::rdata::A;
use hickory_resolver::proto::rr::{RData, Record, RecordType};
use tokio::net::UdpSocket;

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

/// The name server: its socket, its table and the names it was asked.
pub(crate) struct NameServer {
    socket: UdpSocket,
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
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        Self {
            socket,
            answers: answers
                .into_iter()
                .map(|(name, answer)| (name.to_owned(), answer))
                .collect(),
            asked: Vec::new(),
        }
    }

    /// Where it listens.
    pub(crate) fn address(&self) -> SocketAddr {
        self.socket.local_addr().unwrap()
    }

    /// The names it was asked, in order, each query once.
    pub(crate) fn asked(&self) -> &[String] {
        &self.asked
    }

    /// Answers queries until the future is dropped.
    pub(crate) async fn serve(&mut self) {
        let mut buffer = [0_u8; 4096];
        loop {
            let (length, from) = self.socket.recv_from(&mut buffer).await.unwrap();
            let query = Message::from_vec(&buffer[..length]).unwrap();
            let response = self.respond(&query);
            self.socket
                .send_to(&response.to_vec().unwrap(), from)
                .await
                .unwrap();
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
