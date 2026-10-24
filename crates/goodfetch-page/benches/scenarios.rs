//! The page crate's benchmark scenarios (ADR 0071), shared by its Callgrind
//! and DHAT targets. The guard's check-and-arm round trip as the engine
//! does it, a fixed number of times. Each round trip builds the guard's
//! very command (its script, its argument and the call into its world, the
//! crate's own modules compiled in here), sends it over a real CDP
//! connection to a browser end in the same process, which answers every
//! command with a ready target, and reads the answer as the guard reads
//! it. Only the browser's own work is left out. And the interception's
//! per-request pipeline: a browser end in the same process pauses a fixed
//! list of requests, and each is read as the pipeline reads it, matched
//! against a page's routes, none of which answers it, judged by steps 3 to
//! 7 under a domain policy of a hundred entries (the pipeline's `steps`,
//! compiled in here) and sent on, or failed when refused, with the
//! browser's answer read. Inputs are literals; no network, no sleep, no
//! work whose amount depends on timing.

#[path = "../src/guard/check.rs"]
#[expect(
    dead_code,
    reason = "the scenario reads only a ready target's answer, never a waiting one's details"
)]
mod check;
#[path = "../src/interception/steps.rs"]
#[expect(
    dead_code,
    reason = "no route of the scenario answers, so no override is merged"
)]
pub(crate) mod steps;
#[path = "../src/driver/world.rs"]
mod world;

use std::sync::Arc;
use std::time::Duration;

use goodfetch_cdp::generated::{fetch, network};
use goodfetch_cdp::{Connection, Framer};
use goodfetch_clean_up_rules::matching::Choices;
use goodfetch_clean_up_rules::never_block::NeverBlock;
use goodfetch_clean_up_rules::rules::FilterLists;
use goodfetch_engine::egress::{Policy, RequestKind};
use goodfetch_page::url_pattern::UrlPattern;
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
                        check::argument(check::Target::Selector("#target"), "pointer", SLICE),
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
                            y: Some(_),
                            ..
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

/// The page routes of the interception scenario: globs a program arms,
/// none matching the scenario's requests.
const ROUTES: [&str; 8] = [
    "**/api/**",
    "**/*.{png,jpg,webp}",
    "**/analytics.js",
    "https://cdn.example.test/**",
    "**/login*",
    "**/graphql",
    "**/*.woff2",
    "**/ads/**",
];

/// The `index`th request the browser end pauses, as a page's requests
/// mix: of every ten, one document to a host the policy does not list,
/// two of the site's documents, a tracker script and an ad image the
/// embedded Filter lists block, and the site's own scripts, style sheets
/// and images, which go on.
fn paused(index: usize) -> Vec<u8> {
    let site = format!("site-{}.example.test", index % 100);
    let (url, resource_type) = match index % 10 {
        0 => (
            format!("https://tracker.example.test/shelf/{index}/items?page=2"),
            "Document",
        ),
        1 | 2 => (
            format!("https://{site}/shelf/{index}/items?page=2"),
            "Document",
        ),
        3 => (format!("https://{site}/js/adobe-analytics.js"), "Script"),
        4 => (format!("https://{site}/img/top-ad-sidebar.gif"), "Image"),
        5 | 6 => (format!("https://{site}/static/{index}/app.js"), "Script"),
        7 => (format!("https://{site}/static/site.css"), "Stylesheet"),
        _ => (format!("https://{site}/media/{index}/photo.avif"), "Image"),
    };
    let mut bytes = serde_json::to_vec(&json!({
        "method": "Fetch.requestPaused",
        "params": {
            "requestId": format!("interception-job-{index}.0"),
            "request": {
                "url": url,
                "method": "GET",
                "headers": {
                    "Accept": "text/html,application/xhtml+xml",
                    "Referer": format!("https://{site}/"),
                    "User-Agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/156.0.0.0",
                    "Upgrade-Insecure-Requests": "1",
                },
                "initialPriority": "VeryHigh",
                "referrerPolicy": "strict-origin-when-cross-origin",
            },
            "frameId": "8D5B4C2A9E1F",
            "resourceType": resource_type,
        },
    }))
    .expect("invariant: an event encodes");
    bytes.push(0);
    bytes
}

/// A browser end that answers every command with an empty result and,
/// after each, pauses the next of `count` requests, as a page's requests
/// come while the engine answers the earlier ones, until the client closes
/// its end.
async fn pausing_browser(mut stream: DuplexStream, count: usize) {
    let mut paused_so_far = 0;
    let mut framer = Framer::default();
    let mut buffer = vec![0; 1 << 16];
    loop {
        while let Some(frame) = framer.next_frame().expect("invariant: the client frames") {
            let command: Value =
                serde_json::from_slice(&frame).expect("invariant: a command is JSON");
            let mut answer = serde_json::to_vec(&json!({ "id": command["id"], "result": {} }))
                .expect("invariant: an answer encodes");
            answer.push(0);
            if stream.write_all(&answer).await.is_err() {
                return;
            }
            if paused_so_far < count {
                if stream.write_all(&paused(paused_so_far)).await.is_err() {
                    return;
                }
                paused_so_far += 1;
            }
        }
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(count) => framer.push(&buffer[..count]),
        }
    }
}

/// The domain policy of the interception scenario: a hundred hosts
/// allowed.
fn policy() -> Policy {
    let allowed: Vec<String> = (0..100)
        .map(|entry| format!("site-{entry}.example.test"))
        .collect();
    Policy::new(&allowed, &[] as &[&str]).expect("invariant: every entry is a host")
}

/// Sends `command` and reads the browser end's answer.
async fn sent<C: goodfetch_cdp::Command>(connection: &Connection, command: &C) {
    connection
        .send(None, command, Duration::from_secs(30))
        .await
        .expect("invariant: the browser end answers every command");
}

/// The interception scenario's step 4, its setup, outside the measured
/// work: the embedded Filter lists loaded once and every list on, as by
/// default, with nothing in `browser.block_resources`.
pub(crate) fn filter_lists(count: usize) -> (usize, steps::Blocking) {
    let lists = FilterLists::embedded().expect("invariant: the embedded Filter lists load");
    let blocking = steps::Blocking {
        never_block: Arc::new(NeverBlock::new()),
        lists: Some(Arc::new(lists)),
        choices: Choices {
            ads: true,
            trackers: true,
            annoyances: true,
            ..Choices::default()
        },
    };
    (count, blocking)
}

/// `count` paused requests through the pipeline under `blocking`'s step
/// 4; returns how many it sent on.
pub(crate) fn intercept(count: usize, blocking: &steps::Blocking) -> usize {
    let routes: Vec<UrlPattern> = ROUTES
        .iter()
        .map(|glob| UrlPattern::glob(glob).expect("invariant: the globs read"))
        .collect();
    let policy = policy();
    runtime().block_on(async {
        let (client, end) = tokio::io::duplex(1 << 16);
        let (reader, writer) = tokio::io::split(client);
        let (connection, driver) = Connection::new(reader, writer);
        let (_, sent_on, ()) = tokio::join!(
            driver.run(),
            async {
                let mut events = connection.subscribe(None);
                sent(&connection, &fetch::EnableParams::default()).await;
                let mut sent_on = 0;
                for _ in 0..count {
                    let event = events
                        .next()
                        .await
                        .expect("invariant: the browser end pauses every request");
                    let paused = event
                        .read::<fetch::RequestPausedEvent>()
                        .expect("invariant: the browser end pauses requests");
                    let url = &paused.request.url;
                    if routes.iter().any(|route| route.matches(url)) {
                        continue;
                    }
                    let overrides = steps::Overrides::default();
                    let source = paused
                        .request
                        .headers
                        .get("Referer")
                        .and_then(Value::as_str)
                        .unwrap_or(url);
                    let request = steps::Paused {
                        id: &paused.request_id,
                        url,
                        method: &paused.request.method,
                        overrides: &overrides,
                        kind: steps::kind(paused.resource_type, true),
                        source_url: source,
                        egress: RequestKind::Page,
                    };
                    match steps::send_on(&request, &policy, blocking, false) {
                        Ok(steps::Sent::Continue(command)) => {
                            sent(&connection, &command).await;
                            sent_on += 1;
                        }
                        Ok(steps::Sent::StandIn(stand_in)) => {
                            sent(&connection, &stand_in).await;
                        }
                        Err(_) => {
                            let failed = fetch::FailRequestParams {
                                request_id: paused.request_id.clone(),
                                error_reason: network::ErrorReason::BlockedByClient,
                            };
                            sent(&connection, &failed).await;
                        }
                    }
                }
                drop(connection);
                sent_on
            },
            pausing_browser(end, count)
        );
        sent_on
    })
}
