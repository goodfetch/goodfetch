//! The `identity` family on `identity.test`, the lab's identity site ported:
//! a three-page shelf crawled under an identity. Every request of the
//! identified task must carry a Web Bot Auth signature that the key
//! directory its `Signature-Agent` names verifies as the request arrives
//! (checked with Cloudflare's `web-bot-auth`); no request of the standard
//! task may be signed. Both are judged from the Proving ground's own record
//! of each request's check.

use super::shelf::{self, Shelf};
use crate::raw::{Answer, Page, Request};
use crate::task::{Expected, Kind, Task};

/// The host name the family is served on.
pub(super) const HOST: &str = "identity.test";

const FAMILY: &str = "identity";
const PLACE: u64 = 1622;

const TASKS: [(bool, &str); 2] = [
    (
        true,
        "Crawl this shelf from its first page through its `rel=\"next\"` links as an \
         identified agent, signing every request with Web Bot Auth (HTTP Message Signatures, \
         RFC 9421, with a `Signature-Agent` naming your key directory), and return every \
         Record with its `id` and `price`.",
    ),
    (
        false,
        "Crawl this shelf from its first page through its `rel=\"next\"` links as a standard \
         browser, signing no request, and return every Record with its `id` and `price`.",
    ),
];

fn shelf(ordinal: u64) -> Shelf {
    Shelf {
        prefix: "I",
        place: PLACE,
        ordinal,
    }
}

/// The family's tasks.
pub(super) fn tasks(seed: u64, port: u16) -> Vec<Task> {
    (1..)
        .zip(TASKS)
        .map(|(ordinal, (signed, description))| {
            super::task(
                (FAMILY, HOST),
                ordinal,
                Kind::Crawl,
                "/catalog/1",
                description,
                Expected {
                    records: shelf(ordinal).records(seed),
                    signed: Some(signed),
                    ..Expected::default()
                },
                port,
            )
        })
        .collect()
}

/// The answer to `request` on `host`: the shelf of its task, or of the
/// first task on the family's own host.
pub(super) fn answer(seed: u64, host: &str, request: &Request) -> Answer {
    let ordinal = super::ordinal(host).unwrap_or(1);
    match shelf::page_number(request.path(), "") {
        Some(page) => Answer::Page(Page::html(200, shelf(ordinal).plain_page(seed, page, ""))),
        None => Answer::Page(Page::text(404, "not found")),
    }
}
