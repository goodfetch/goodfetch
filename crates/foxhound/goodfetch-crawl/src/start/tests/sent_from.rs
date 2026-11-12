//! The tier Provenance names for an attempt in an engine that composed no
//! browser pages (ADR 0023; issue 2409): under `fetch.tier: auto`, its
//! document leaves on the Session's own HTTP transport, fed here from
//! memory through the fetch tiers' test seam, so it is recorded on `http`.

use std::sync::Arc;

use bytes::Bytes;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_fetch_tier::{Body, FetchTiers, Hop, Reuse, Sending, Sent, Transport};
use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Reply, crawl_on, dependencies, engine_on, run_folder};

/// The site's host, mapped to an address the egress policy lets through,
/// which no connection reaches.
const HOST: &str = "shop.test";

/// A site that answers every page one small document, and no robots.txt.
struct Site;

/// A body sent whole in one chunk.
struct Whole(Option<Bytes>);

impl Body for Whole {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(std::future::ready(Ok(self.0.take())))
    }
}

impl Transport for Site {
    fn send<'a>(&'a self, hop: &'a Hop, _reuse: Reuse) -> Sending<'a, Sent> {
        let (status, body) = if hop.url.path() == "/robots.txt" {
            (404, "")
        } else {
            (
                200,
                "<!doctype html><html><head><title>Page</title></head><body><h1>Page</h1></body></html>",
            )
        };
        Box::pin(std::future::ready(Ok(Sent {
            status,
            headers: vec![
                (
                    "content-type".to_owned(),
                    "text/html; charset=utf-8".to_owned(),
                ),
                ("content-length".to_owned(), body.len().to_string()),
            ],
            body: Box::new(Whole(Some(Bytes::from_static(body.as_bytes())))),
            reused: false,
            decoded: false,
            certificate_exception: None,
        })))
    }
}

/// Each `fetch_tier` entry of the Provenance log of `run_id`: its tier and
/// how it was chosen.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's Provenance log once its engine stopped, on no async worker"
)]
fn tiers(env: &TestEnv, run_id: &str) -> Vec<Value> {
    std::fs::read_to_string(run_folder(env, run_id).join(RUN_TRACE_PROVENANCE))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| entry["action"] == "fetch_tier")
        .map(|entry| json!([entry["tier"], entry["chosen"]]))
        .collect()
}

/// Covers: story:5-116
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn a_document_the_http_transport_sent_under_auto_is_recorded_on_http() {
    let env = TestEnv::new();

    crawl_on(
        &engine_on(FetchTiers::serving(Arc::new(Site))),
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "shop",
                        "seeds": [{ "url": format!("http://{HOST}/page/1"), "label": "page" }],
                        "settings": {
                            "fetch": { "tier": "auto" },
                            "politeness": { "domain": { "delay": 0 } },
                            "network": { "resolve": { HOST: ["127.0.0.1"] } },
                            "egress": { "private_ranges": [HOST] },
                        },
                    }),
                    &mut |_| Reply::Silent,
                )
                .await;
            assert!(started.get("result").is_some(), "{started}");
            client
                .call("run.finish", json!({ "run_id": "shop" }), &mut |_| {
                    Reply::Records(Vec::new())
                })
                .await
        },
    );

    // The sample's first run, on the document tier, went out on the HTTP
    // transport; its browser run has no browser to open a page in.
    assert_eq!(
        tiers(&env, "shop").first(),
        Some(&json!(["http", "sample"]))
    );
}
