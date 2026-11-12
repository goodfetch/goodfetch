//! The headers a hop leaves with on a transport whose Session presents
//! some (ADR 0021; issue 2315): the Session's, in their order, each name
//! the hop sets carrying every value the hop gave it, in the Session's
//! place for that name, and the hop's other headers after them.

use url::Url;

use crate::driver::{Hop, Reuse, Sending, Sent, Transport, asked};

/// A transport that presents `0` on every request and sends none.
struct Presenting(&'static [(&'static str, &'static str)]);

impl Transport for Presenting {
    fn presents(&self, _url: &Url) -> Vec<(String, String)> {
        owned(self.0)
    }

    fn send<'a>(&'a self, _hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(std::future::pending())
    }
}

/// `headers`, owned.
fn owned(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// The headers a `GET` setting `own` leaves with on a transport presenting
/// `presented`.
fn leaves_with(presented: &'static [(&str, &str)], own: &[(&str, &str)]) -> Vec<(String, String)> {
    let hop = Hop {
        headers: owned(own),
        ..Hop::get(Url::parse("http://site.test/").unwrap())
    };
    asked(&Presenting(presented), &hop).headers
}

#[test]
fn a_hops_own_header_takes_the_sessions_place_and_its_others_follow() {
    let headers = leaves_with(
        &[
            ("user-agent", "Session"),
            ("accept", "text/html"),
            ("accept-encoding", "gzip, deflate"),
        ],
        &[("x-own", "1"), ("Accept", "application/json")],
    );

    assert_eq!(
        headers,
        owned(&[
            ("user-agent", "Session"),
            ("Accept", "application/json"),
            ("accept-encoding", "gzip, deflate"),
            ("x-own", "1"),
        ])
    );
}

#[test]
fn a_hop_keeps_every_value_of_a_header_it_repeats() {
    let headers = leaves_with(
        &[("user-agent", "Session"), ("accept", "text/html")],
        &[
            ("accept", "application/json"),
            ("x-own", "1"),
            ("x-own", "2"),
            ("Accept", "text/csv"),
        ],
    );

    assert_eq!(
        headers[..5],
        owned(&[
            ("user-agent", "Session"),
            ("accept", "application/json"),
            ("Accept", "text/csv"),
            ("x-own", "1"),
            ("x-own", "2"),
        ])
    );
}

#[test]
fn a_transport_that_presents_nothing_asks_for_chromes_codings() {
    assert_eq!(
        leaves_with(&[], &[("x-own", "1")]),
        owned(&[
            ("x-own", "1"),
            ("accept-encoding", "gzip, deflate, br, zstd")
        ])
    );
}
