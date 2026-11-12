//! The plain hosts, which belong to no task: every path of each is one
//! small page that names it, whatever the method, and none has a
//! robots.txt. They are the site of a crawl whose pages do not matter to
//! what its test proves, such as how its requests are admitted, ordered,
//! paced or committed, on host names of several registrable domains:
//! `shop.test` and `www.shop.test`; `heavy.test`, `shop.heavy.test` and
//! `www.heavy.test`; `a.test`, `b.test`, `api.test`, `other.test`,
//! `many.test` and `capped.test`, each alone on its own; and
//! `alice.github.io` and `bob.github.io`, two sites under one public
//! suffix, reached only at the address a test maps them to.
//!
//! A page under `/held/<key>` is held until some request asks a plain host
//! for `/release/<key>`, so a test keeps a crawl's request in flight for as
//! long as it needs, with no clock: a release that came first holds
//! nothing.

use std::collections::BTreeSet;

use crate::raw::{Answer, Page, Request};

/// The plain hosts.
pub(super) const NAMED_HOSTS: [&str; 13] = [
    "shop.test",
    "www.shop.test",
    "heavy.test",
    "shop.heavy.test",
    "www.heavy.test",
    "a.test",
    "b.test",
    "api.test",
    "other.test",
    "many.test",
    "capped.test",
    "alice.github.io",
    "bob.github.io",
];

/// Whether `host` is a plain host.
pub(super) fn serves(host: &str) -> bool {
    NAMED_HOSTS.contains(&host)
}

/// The keys released so far, which the held pages wait for.
#[derive(Default)]
pub(super) struct Released(tokio::sync::watch::Sender<BTreeSet<String>>);

/// The page at the request's path, or `404` for a robots.txt. A page under
/// `/held/<key>` is sent only once `/release/<key>` was asked for, on any
/// plain host.
pub(super) async fn answer(released: &Released, request: &Request) -> Answer {
    let path = request.path();
    if let Some(key) = path.strip_prefix("/release/") {
        released.0.send_modify(|keys| {
            keys.insert(key.to_owned());
        });
        return Answer::Page(Page::text(200, "released"));
    }
    if let Some(key) = path.strip_prefix("/held/") {
        // The sender lives as long as the site, so the wait ends only on
        // the key.
        let _ = released
            .0
            .subscribe()
            .wait_for(|keys| keys.contains(key))
            .await;
    }
    match path {
        "/robots.txt" => Answer::Page(Page::text(404, "not found")),
        path => Answer::Page(Page::html(
            200,
            format!("<!doctype html><title>{path}</title><p>A page at {path}.</p>"),
        )),
    }
}
