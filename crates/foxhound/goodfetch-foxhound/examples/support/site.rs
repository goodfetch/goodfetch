//! The site the corpus writers' crawls fetch, served from memory through
//! the fetch tiers' test seam, since no fetch tier reaches a network here:
//! with nothing set, `auto` serves every request on a tier (issue 1602).

use std::time::Duration;

use bytes::Bytes;
use goodfetch_fetch_tier::{Body, Hop, Reuse, Sending, Sent, Transport};
use serde_json::{Value, json};

/// The address the crawls' hosts are mapped to: a public one, which the
/// egress policy lets through, and which no connection reaches, since the
/// engine's fetch tiers serve [`Site`] from memory.
const SITE_ADDRESS: &str = "93.184.216.34";

/// The `fetch` and `network` settings of a crawl of [`Site`]: the `http`
/// tier forced, the one tier an engine with no browser serves, and each
/// host of `hosts` mapped to [`SITE_ADDRESS`], so no name is looked up.
pub(crate) fn settings(hosts: &[&str]) -> (Value, Value) {
    let resolve: serde_json::Map<String, Value> = hosts
        .iter()
        .map(|host| ((*host).to_owned(), json!([SITE_ADDRESS])))
        .collect();
    (json!({ "tier": "http" }), json!({ "resolve": resolve }))
}

/// The site: every URL answers one small document that names it without
/// its `ref` parameter, so a page reads alike with or without `ref` and
/// unlike any other, and a host has a robots.txt only when the site was
/// given one for it.
pub(crate) struct Site {
    /// How long each answer takes, on the paused clock.
    latency: Duration,
    /// The robots.txt of each host that has one.
    robots_txt: Vec<(&'static str, &'static str)>,
}

impl Site {
    /// The site, each answer `latency` after its request, on the paused
    /// clock, which passes once the engine is idle, and each host of
    /// `robots_txt` answering its robots.txt file.
    pub(crate) fn new(latency: Duration, robots_txt: &[(&'static str, &'static str)]) -> Self {
        Self {
            latency,
            robots_txt: robots_txt.to_vec(),
        }
    }
}

impl Transport for Site {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        let (status, body) = if hop.url.path() == "/robots.txt" {
            let file = self
                .robots_txt
                .iter()
                .find(|(host, _)| hop.url.host_str() == Some(host));
            match file {
                Some((_, file)) => (200, (*file).to_owned()),
                None => (404, "not found".to_owned()),
            }
        } else {
            let url = hop.url.as_str();
            let named = url.split_once("&ref=").map_or(url, |(named, _)| named);
            (
                200,
                format!(
                    "<!doctype html><html><head><title>{named}</title></head><body><h1>{named}</h1></body></html>"
                ),
            )
        };
        let content_type = if hop.url.path() == "/robots.txt" {
            "text/plain; charset=utf-8"
        } else {
            "text/html; charset=utf-8"
        };
        Box::pin(async move {
            tokio::time::sleep(self.latency).await;
            Ok(Sent {
                status,
                headers: vec![
                    ("content-type".to_owned(), content_type.to_owned()),
                    ("content-length".to_owned(), body.len().to_string()),
                ],
                body: Box::new(Whole(Some(Bytes::from(body)))),
                reused: false,
                decoded: false,
                certificate_exception: None,
            })
        })
    }
}

/// A page's body, sent in one chunk.
struct Whole(Option<Bytes>);

impl Body for Whole {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        let chunk = self.0.take().filter(|chunk| !chunk.is_empty());
        Box::pin(std::future::ready(Ok(chunk)))
    }
}
