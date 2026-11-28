//! J-016, "Be told once each at 50 %, 80 % and 100 % of a Limit", on the `cli`
//! surface (ADRs 0027, 0041, 0063 and 0075; issue 233): a person sets a
//! monthly install Limit on requests and configures a signed `webhook`
//! Notification channel, then runs two concurrent runs — one on the Daemon and
//! one on an Attached engine — each extracting the Proving ground catalog's
//! product pages one after another until both stop at `limit_reached`. The
//! webhook receiver verifies every Standard Webhooks delivery and confirms
//! exactly one Notification arrived at 50 %, 80 % and 100 % in order, with
//! 50 % and 80 % arriving while both runs were still going on, and
//! `needs-you list` holds one `limit_reached` item for the install Limit.
#![expect(
    clippy::disallowed_methods,
    reason = "the Journey drives a live webhook receiver, Daemon and Attached engine"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the Journey's local webhook receiver binds a loopback TcpListener"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_collie_client::verify_webhook;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::notifications::{NotificationData, NotificationEvent, NotificationPayload};
use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_client::{Attached, DaemonClient, Hello};
use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, JourneyRun, browser_cache};
use serde_json::{Value, json};

use crate::daemon_run::{ANY_PORT, Foreground, opened};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// Monthly install Limit on requests: 10 requests, well below the catalog's
/// 100 product pages, with Slice size 1 so 50 % (5), 80 % (8) and 100 % (10)
/// are each crossed cleanly by alternating runs on both engines.
const LIMIT_AMOUNT: &str = "10";

const MONTHLY: &str = r#"{"every":"month","time_zone":"UTC"}"#;

/// One verified Notification delivery observed by the local webhook receiver.
struct Received {
    payload: NotificationPayload,
    active_runs: usize,
}

/// Local loopback HTTP webhook receiver that verifies each Standard Webhooks
/// delivery and records how many runs were active when it arrived.
struct WebhookReceiver {
    url: String,
    stop: Arc<AtomicBool>,
    received: Arc<Mutex<Vec<Received>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl WebhookReceiver {
    fn start(secret: String, active_runs: Arc<AtomicUsize>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback webhook listener binds");
        listener
            .set_nonblocking(true)
            .expect("webhook listener nonblocking");
        let address = listener.local_addr().expect("webhook listener local addr");
        let url = format!("http://{address}/notifications");
        let stop = Arc::new(AtomicBool::new(false));
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop_thread = Arc::clone(&stop);
        let received_thread = Arc::clone(&received);
        let thread = thread::spawn(move || {
            while !stop_thread.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        if let Some((headers, body)) = read_http_request(&mut stream) {
                            let verified = verify_webhook(
                                &body,
                                headers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                                &[secret.as_str()],
                            )
                            .expect("webhook delivery verifies with Standard Webhooks");
                            let active = active_runs.load(Ordering::Acquire);
                            received_thread.lock().unwrap().push(Received {
                                payload: verified,
                                active_runs: active,
                            });
                            let _ = stream.write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                            );
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            stop,
            received,
            thread: Some(thread),
        }
    }

    fn finish(mut self) -> Vec<Received> {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        std::mem::take(&mut *self.received.lock().unwrap())
    }
}

impl Drop for WebhookReceiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// Reads a single HTTP/1.1 request with `Content-Length` from `stream`.
fn read_http_request(
    stream: &mut std::net::TcpStream,
) -> Option<(BTreeMap<String, String>, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]);
    let mut headers = BTreeMap::new();
    for line in head.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let content_length: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    Some((
        headers,
        buffer[header_end..header_end + content_length].to_vec(),
    ))
}

/// Extracts the percentage number from a `limit_threshold` Notification title.
fn percent_of(payload: &NotificationPayload) -> u64 {
    let NotificationData::Full(full) = &payload.data else {
        panic!("expected full Notification payload, got {payload:?}");
    };
    assert_eq!(full.event, NotificationEvent::LIMIT_THRESHOLD);
    if full.title.contains("50 %") {
        50
    } else if full.title.contains("80 %") {
        80
    } else if full.title.contains("100 %") {
        100
    } else {
        panic!("unexpected limit_threshold title: {}", full.title);
    }
}

/// Navigates `page_id` to `url` and extracts its product heading, returning
/// `true` on success or `false` once `limit_reached` stops the run.
fn extract_one_on_daemon(
    client: &mut DaemonClient,
    next_id: &mut i64,
    page_id: &str,
    url: &str,
) -> bool {
    *next_id += 1;
    let went = client.call(
        *next_id,
        "page.goto",
        &json!({ "page_id": page_id, "url": url }),
    );
    if went["error"]["data"]["code"] == json!("limit_reached") {
        return false;
    }
    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    *next_id += 1;
    let read = client.call(
        *next_id,
        "page.read",
        &json!({ "page_id": page_id, "fields": { "name": { "locator": { "kind": "css", "selector": "h1" } } } }),
    );
    assert!(read.get("result").is_some(), "{read}");
    true
}

/// Navigates `page_id` to `url` on an Attached engine and extracts its product
/// heading, returning `true` on success or `false` once `limit_reached` stops
/// the run.
fn extract_one_on_attached(
    client: &mut Attached,
    next_id: &mut i64,
    page_id: &str,
    url: &str,
) -> bool {
    *next_id += 1;
    let went = client.ask(
        *next_id,
        "page.goto",
        &json!({ "page_id": page_id, "url": url }),
    );
    if went["error"]["data"]["code"] == json!("limit_reached") {
        return false;
    }
    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    *next_id += 1;
    let read = client.ask(
        *next_id,
        "page.read",
        &json!({ "page_id": page_id, "fields": { "name": { "locator": { "kind": "css", "selector": "h1" } } } }),
    );
    assert!(read.get("result").is_some(), "{read}");
    true
}

/// Covers: story:3-73
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn cli() {
    let mut run = JourneyRun::start("J-016", "cli", Path::new(COLLIE), ResultDocument::journey)
        .expect("J-016's cli variant starts");
    let cache = browser_cache(run.env())
        .expect("Playwright's Chrome for Testing is installed")
        .to_string_lossy()
        .into_owned();
    let hook_secret = format!("whsec_{}", BASE64.encode(&[0x41_u8; 32]));
    let active_runs = Arc::new(AtomicUsize::new(0));
    let receiver = WebhookReceiver::start(hook_secret.clone(), Arc::clone(&active_runs));
    let hook_url = receiver.url.clone();
    let mut deliveries: Vec<Received> = Vec::new();
    let mut needs_you_items: Vec<Value> = Vec::new();

    run.step(
        "Set a monthly install Limit below what extracting every product page of the catalog uses, and a Notification channel the person reads",
        |run| {
            let proving_ground = run.proving_ground().expect("J-016 has a Proving ground");
            let reaching = proving_ground.description().reaching();
            run.env().user_configuration(&format!(
                "{reaching}\n\
                 daemon.address = \"{ANY_PORT}\"\n\
                 daemon.idle_timeout = 60000\n\
                 [settings.notifications.channels.ops]\n\
                 type = \"webhook\"\n\
                 url = \"env:J016_HOOK_URL\"\n\
                 [settings.notifications.channels.ops.webhook]\n\
                 secret = \"env:J016_HOOK_SECRET\"\n"
            ));
            let set = run
                .command()
                .args([
                    "usage",
                    "set",
                    "--scope",
                    "install",
                    "--usage-kind",
                    "requests",
                    "--period",
                    MONTHLY,
                    "--amount",
                    LIMIT_AMOUNT,
                ])
                .output()
                .unwrap();
            assert!(set.status.success(), "{set:?}");
        },
    );

    run.step(
        "Start a run on the Daemon and a run on an Attached engine at once, each extracting the catalog's product pages one after another, until both stop",
        |run| {
            let proving_ground = run.proving_ground().expect("J-016 has a Proving ground");
            let variables = [
                (BROWSERS_PATH_VARIABLE, cache.as_str()),
                ("J016_HOOK_URL", hook_url.as_str()),
                ("J016_HOOK_SECRET", hook_secret.as_str()),
            ];
            let mut daemon =
                Foreground::start(run.env(), run.engine().to_str().unwrap(), &variables);
            let endpoint = daemon.serving();
            let mut daemon_client = opened(run.env(), &endpoint);
            let mut attached = Attached::start_with(
                run.env(),
                run.engine(),
                Duration::from_secs(60),
                |command| {
                    command.env(BROWSERS_PATH_VARIABLE, &cache);
                    command.env("J016_HOOK_URL", &hook_url);
                    command.env("J016_HOOK_SECRET", &hook_secret);
                },
            );
            let hello = attached.open(
                &Hello::needing(Product::Collie)
                    .id("hello")
                    .surface(Surface::Cli),
            );
            assert!(hello.get("result").is_some(), "{hello}");

            let mut daemon_id = 1_i64;
            let mut attached_id = 1_i64;
            let started_daemon =
                daemon_client.call(daemon_id, "run.start", &json!({ "run_id": "daemon-run" }));
            assert!(started_daemon.get("result").is_some(), "{started_daemon}");
            let started_attached =
                attached.ask(attached_id, "run.start", &json!({ "run_id": "attached-run" }));
            assert!(
                started_attached.get("result").is_some(),
                "{started_attached}"
            );

            daemon_id += 1;
            let daemon_page = daemon_client.call(
                daemon_id,
                "run.new_page",
                &json!({ "run_id": "daemon-run" }),
            );
            let daemon_page_id = daemon_page["result"]["page_id"]
                .as_str()
                .unwrap_or_else(|| panic!("{daemon_page}"))
                .to_owned();
            attached_id += 1;
            let attached_page = attached.ask(
                attached_id,
                "run.new_page",
                &json!({ "run_id": "attached-run" }),
            );
            let attached_page_id = attached_page["result"]["page_id"]
                .as_str()
                .unwrap_or_else(|| panic!("{attached_page}"))
                .to_owned();

            active_runs.store(2, Ordering::Release);
            let mut daemon_going = true;
            let mut attached_going = true;
            let mut page_num = 1_u32;

            while daemon_going || attached_going {
                if daemon_going {
                    let url = proving_ground.url("1.catalog.test", &format!("/products/P-{page_num:04}"));
                    page_num += 1;
                    if !extract_one_on_daemon(&mut daemon_client, &mut daemon_id, &daemon_page_id, &url) {
                        daemon_going = false;
                        active_runs.fetch_sub(1, Ordering::AcqRel);
                    }
                }
                if attached_going {
                    let url = proving_ground.url("1.catalog.test", &format!("/products/P-{page_num:04}"));
                    page_num += 1;
                    if !extract_one_on_attached(&mut attached, &mut attached_id, &attached_page_id, &url) {
                        attached_going = false;
                        active_runs.fetch_sub(1, Ordering::AcqRel);
                    }
                }
            }

            daemon_id += 1;
            let _ = daemon_client.call(daemon_id, "run.finish", &json!({ "run_id": "daemon-run" }));
            attached_id += 1;
            let _ = attached.ask(attached_id, "run.finish", &json!({ "run_id": "attached-run" }));
            drop(daemon_client);
            let _ = attached.finish();
            daemon.signal();
            assert!(daemon.exited(), "the Daemon exited cleanly after SIGTERM");
        },
    );

    run.step("Read the Notifications the channel received", |_| {
        deliveries = receiver.finish();
    });

    run.step("Read \"Needs you\"", |run| {
        let listed = run
            .command()
            .args(["needs-you", "list", "--json"])
            .output()
            .unwrap();
        assert!(listed.status.success(), "{listed:?}");
        let doc: Value =
            serde_json::from_slice(&listed.stdout).unwrap_or_else(|_| panic!("{listed:?}"));
        needs_you_items = doc["items"]
            .as_array()
            .unwrap_or_else(|| panic!("{doc}"))
            .clone();
    });

    run.check(
        "The channel received exactly one Notification at each of 50 %, 80 % and 100 % of the install Limit, in that order, though runs on both engines crossed them",
        |_| {
            let percentages: Vec<u64> = deliveries
                .iter()
                .map(|entry| percent_of(&entry.payload))
                .collect();
            assert_eq!(percentages, vec![50, 80, 100]);
        },
    );

    run.check(
        "The 50 % and 80 % Notifications arrived while both runs were still going on",
        |_| {
            assert_eq!(
                (deliveries[0].active_runs, deliveries[1].active_runs),
                (2, 2),
                "expected both runs active when 50 % and 80 % Notifications arrived"
            );
        },
    );

    run.check(
        "\"Needs you\" holds one item for the install Limit reached at 100 %",
        |_| {
            let limit_items: Vec<&Value> = needs_you_items
                .iter()
                .filter(|item| item["kind"] == json!("limit_reached"))
                .collect();
            assert_eq!(limit_items.len(), 1, "{needs_you_items:?}");
            assert_eq!(
                limit_items[0]["details"]["limit"]["scope"]["kind"],
                json!("install")
            );
            assert_eq!(
                limit_items[0]["details"]["limit"]["usage_kind"],
                json!("requests")
            );
            assert_eq!(limit_items[0]["details"]["amount"], json!(10));
        },
    );

    run.finish()
        .expect("J-016's document is written")
        .assert_proved();
}
