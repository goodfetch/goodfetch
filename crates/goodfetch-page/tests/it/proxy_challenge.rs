//! How the pinned Chromium hands a proxy's `407` to a CDP client (#2337,
//! which settles #1895's premise and ADR 0140's sentence on the egress
//! relay's 407): whether `Fetch.enable` takes authentication requests with no
//! request pattern, which patterns raise `Fetch.authRequired`, and whether
//! credentials given once in a browser context let that context's other
//! pages through. The browser is the pinned build, driven over CDP by the
//! test itself, each context's proxy the Proving ground's authenticated one,
//! which challenges as the relay does (`407` with `Proxy-Authenticate:
//! Basic`). A Chromium roll that changes an answer fails a test here.
#![expect(
    clippy::disallowed_types,
    reason = "the test dials its own browser's debugging socket (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test builds its own runtime with real time, since the browser answers over a real socket, and starts that browser and reads its port (ADR 0060)"
)]

use std::process::Stdio;
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_test_support::{
    ProvingGround, TestEnv, browser_cache, chrome_for_testing, reported_debugging_port,
};
use rstest::rstest;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// Any seed: the hosts below are served on every one.
const SEED: u64 = 2337;

/// The Proving ground's proxy that takes Basic credentials.
const PROXY: &str = "datacenter";

/// Three hosts of the Proving ground: three origins, a tunnel each.
const HOSTS: [&str; 3] = ["1.access.test", "2.navigation.test", "fetch-edge.test"];

/// How long a command, or an event the test waits for, may take.
const WAIT: Duration = Duration::from_secs(20);

/// How long the browser has to exit before it's killed.
const BROWSER_STOP: Duration = Duration::from_secs(10);

/// What the test answers a paused request's authentication challenge with.
#[derive(Clone)]
struct Credentials {
    username: String,
    password: String,
}

/// The test's CDP client of the browser: commands answered in turn, every
/// event kept, and each paused request continued and each challenge answered
/// with the credentials as they come.
struct Cdp {
    socket: WebSocketStream<TcpStream>,
    next: u64,
    events: Vec<Value>,
    credentials: Credentials,
}

impl Cdp {
    /// Sends `method` with `params` on the target `session` (the browser's
    /// own when `None`), and answers its result or the browser's message.
    async fn call(
        &mut self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        let mut command = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            command["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::text(command.to_string()))
            .await
            .expect("invariant: the browser reads its socket");
        let answer = tokio::time::timeout(WAIT, async {
            loop {
                let message = self.read().await;
                if message["id"] == json!(id) {
                    return message;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{method} got no answer"));
        match answer.get("error") {
            Some(error) => Err(error["message"].as_str().unwrap_or_default().to_owned()),
            None => Ok(answer["result"].clone()),
        }
    }

    /// The next message, an event kept and answered first.
    async fn read(&mut self) -> Value {
        loop {
            let message = self
                .socket
                .next()
                .await
                .expect("invariant: the browser keeps its socket open")
                .expect("invariant: the socket reads");
            let Message::Text(text) = message else {
                continue;
            };
            let message: Value = serde_json::from_str(&text).expect("invariant: CDP is JSON");
            if message.get("id").is_none() {
                self.answer(&message).await;
                self.events.push(message.clone());
            }
            return message;
        }
    }

    /// Continues a paused request, as it is, and answers a challenge with
    /// the credentials.
    async fn answer(&mut self, event: &Value) {
        let session = event["sessionId"].as_str();
        let params = &event["params"];
        let request_id = params["requestId"].clone();
        let answer = match event["method"].as_str() {
            Some("Fetch.requestPaused") if params.get("responseStatusCode").is_some() => {
                ("Fetch.continueResponse", json!({ "requestId": request_id }))
            }
            Some("Fetch.requestPaused") => {
                ("Fetch.continueRequest", json!({ "requestId": request_id }))
            }
            Some("Fetch.authRequired") => (
                "Fetch.continueWithAuth",
                json!({
                    "requestId": request_id,
                    "authChallengeResponse": {
                        "response": "ProvideCredentials",
                        "username": self.credentials.username,
                        "password": self.credentials.password,
                    },
                }),
            ),
            _ => return,
        };
        self.next += 1;
        let mut command = json!({ "id": self.next, "method": answer.0, "params": answer.1 });
        if let Some(session) = session {
            command["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::text(command.to_string()))
            .await
            .expect("invariant: the browser reads its socket");
    }

    /// Waits for the event `method` on `session`, answering what comes
    /// meanwhile; whether it came within [`WAIT`].
    async fn until(&mut self, session: &str, method: &str) -> bool {
        let seen = |event: &Value| {
            event["sessionId"] == json!(session) && event["method"] == json!(method)
        };
        if self.events.iter().any(seen) {
            return true;
        }
        tokio::time::timeout(WAIT, async {
            loop {
                let message = self.read().await;
                if seen(&message) {
                    return;
                }
            }
        })
        .await
        .is_ok()
    }

    /// A new browser context whose every connection goes through `proxy`.
    async fn context(&mut self, proxy: &str) -> String {
        let created = self
            .call(
                None,
                "Target.createBrowserContext",
                json!({ "proxyServer": format!("http://{proxy}") }),
            )
            .await
            .expect("the browser makes a context");
        created["browserContextId"]
            .as_str()
            .expect("invariant: a context has an ID")
            .to_owned()
    }

    /// A blank page in `context`, attached, its page and network events on:
    /// its session.
    async fn page(&mut self, context: &str) -> String {
        let created = self
            .call(
                None,
                "Target.createTarget",
                json!({ "url": "about:blank", "browserContextId": context }),
            )
            .await
            .expect("the browser makes a page");
        let attached = self
            .call(
                None,
                "Target.attachToTarget",
                json!({ "targetId": created["targetId"], "flatten": true }),
            )
            .await
            .expect("the test attaches to its page");
        let session = attached["sessionId"]
            .as_str()
            .expect("invariant: an attachment has a session")
            .to_owned();
        for method in ["Page.enable", "Network.enable"] {
            self.call(Some(&session), method, json!({}))
                .await
                .expect("the page turns its domain on");
        }
        session
    }

    /// Navigates the page of `session` to `url`: the browser's error text if
    /// it failed, and whether its load event came.
    async fn navigate(&mut self, session: &str, url: &str) -> (Option<String>, bool) {
        let navigated = self
            .call(Some(session), "Page.navigate", json!({ "url": url }))
            .await
            .expect("the page takes the navigation");
        let failed = navigated["errorText"].as_str().map(str::to_owned);
        let loaded = failed.is_none() && self.until(session, "Page.loadEventFired").await;
        (failed, loaded)
    }

    /// The events `method` the page of `session` raised.
    fn raised(&self, session: &str, method: &str) -> Vec<&Value> {
        self.events
            .iter()
            .filter(|event| {
                event["sessionId"] == json!(session) && event["method"] == json!(method)
            })
            .collect()
    }
}

/// A runtime for one test, on real time, as the browser answers.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("invariant: the runtime starts")
}

/// Runs `program` on the pinned build, headless and stock but for its
/// background traffic, no certificate checked (the Proving ground's sites
/// chain to its own authority), driven by the test over CDP, beside the
/// Proving ground: the HTTPS URL of each of [`HOSTS`] and the authenticated
/// proxy's address go to the program.
fn on_the_pinned_build<T>(
    program: impl AsyncFnOnce(&mut Cdp, [String; 3], String, &ProvingGround) -> T,
) -> T {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).expect("the Proving ground starts");
    let fixtures = &proving_ground.description().fixtures;
    let proxy = fixtures.proxies[PROXY].clone();
    let credentials = Credentials {
        username: fixtures.proxy_credentials.username.clone(),
        password: fixtures.proxy_credentials.password.clone(),
    };
    let port = proving_ground.address().port();
    let urls = HOSTS.map(|host| format!("https://{host}:{port}/"));
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let profile = env.folder().join("proxy-challenge-profile");
    let mut command = env.process_command(chrome_for_testing(&cache));
    command
        .args(chromium_background_traffic_off())
        .args([
            "--headless",
            "--no-first-run",
            "--use-mock-keychain",
            "--password-store=basic",
            "--ignore-certificate-errors",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stderr(Stdio::null());
    let process = env.spawn(&mut command).expect("the browser starts");
    let debugging = reported_debugging_port(&profile);
    let reported = std::fs::read_to_string(profile.join("DevToolsActivePort"))
        .expect("invariant: it reported its port there");
    let path = reported
        .lines()
        .nth(1)
        .expect("invariant: its second line is the browser's path");
    let found = runtime().block_on(async {
        let dialed = TcpStream::connect(("127.0.0.1", debugging))
            .await
            .expect("the browser listens");
        let browser = format!("ws://127.0.0.1:{debugging}{path}");
        let (socket, _) = tokio_tungstenite::client_async(browser.as_str(), dialed)
            .await
            .expect("the browser speaks WebSocket");
        let mut cdp = Cdp {
            socket,
            next: 0,
            events: Vec::new(),
            credentials,
        };
        program(&mut cdp, urls, proxy, &proving_ground).await
    });
    let _ = process.interrupt_and_wait(BROWSER_STOP);
    found
}

/// The relays the Proving ground's proxies carried, in arrival order: each
/// one's target and outcome.
fn relays(proving_ground: &ProvingGround) -> Vec<(String, String)> {
    proving_ground
        .observations()
        .expect("the Proving ground answers")["relays"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|relay| {
            let field = |name: &str| relay[name].as_str().unwrap_or_default().to_owned();
            (field("target"), field("outcome"))
        })
        .collect()
}

/// The host of each of `relays` whose outcome is `outcome`, once each, in
/// order.
fn hosts(relays: &[(String, String)], outcome: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for (target, found) in relays {
        let host = target
            .rsplit_once(':')
            .map_or(target.as_str(), |(host, _)| host);
        if found == outcome && !hosts.iter().any(|seen| seen == host) {
            hosts.push(host.to_owned());
        }
    }
    hosts
}

/// The browser's message when it refuses a navigation whose proxy asks for
/// credentials no one gave.
const NO_CREDENTIALS: &str = "net::ERR_INVALID_AUTH_CREDENTIALS";

/// Case 1, and so 2: `Fetch.enable` with `handleAuthRequests` and no request
/// pattern, which #1895 would answer the relay's 407 with, is refused, and
/// the page meets the 407 with no challenge raised.
#[test]
fn fetch_refuses_authentication_requests_with_no_request_pattern() {
    let found = on_the_pinned_build(async |cdp, urls, proxy, _| {
        let context = cdp.context(&proxy).await;
        let page = cdp.page(&context).await;
        let enabled = cdp
            .call(
                Some(&page),
                "Fetch.enable",
                json!({ "patterns": [], "handleAuthRequests": true }),
            )
            .await;
        let navigated = cdp.navigate(&page, &urls[0]).await;
        let challenges = cdp.raised(&page, "Fetch.authRequired").len();
        (enabled, navigated, challenges)
    });

    assert_eq!(
        found,
        (
            Err("Can't specify empty patterns with handleAuth set".to_owned()),
            (Some(NO_CREDENTIALS.to_owned()), false),
            0
        )
    );
}

/// Case 3: a request raises `Fetch.authRequired` only when a pattern
/// pauses it at the request stage: the document's request alone does, and
/// a pattern the page's requests never meet, or one for responses only,
/// leaves the 407 to fail the navigation. What each pauses is the
/// page's document (the page has no other request).
#[rstest]
#[case::every_request(json!([{ "urlPattern": "*" }]), (None, true), 1, &["Document"])]
#[case::documents_alone(json!([{ "resourceType": "Document" }]), (None, true), 1, &["Document"])]
#[case::responses_alone(
    json!([{ "urlPattern": "*", "requestStage": "Response" }]),
    (Some(NO_CREDENTIALS), false),
    0,
    &["Document"]
)]
#[case::nothing_it_meets(
    json!([{ "urlPattern": "https://never.invalid/*" }]),
    (Some(NO_CREDENTIALS), false),
    0,
    &[]
)]
fn a_request_paused_at_its_request_stage_raises_the_proxy_challenge(
    #[case] patterns: Value,
    #[case] navigated: (Option<&str>, bool),
    #[case] challenges: usize,
    #[case] paused: &[&str],
) {
    let found = on_the_pinned_build(async |cdp, urls, proxy, _| {
        let context = cdp.context(&proxy).await;
        let page = cdp.page(&context).await;
        let enabled = cdp
            .call(
                Some(&page),
                "Fetch.enable",
                json!({ "patterns": patterns, "handleAuthRequests": true }),
            )
            .await;
        let navigated = cdp.navigate(&page, &urls[0]).await;
        let challenges = cdp.raised(&page, "Fetch.authRequired").len();
        let paused: Vec<String> = cdp
            .raised(&page, "Fetch.requestPaused")
            .iter()
            .map(|event| {
                event["params"]["resourceType"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
        (enabled, navigated, challenges, paused)
    });

    let navigated = (navigated.0.map(str::to_owned), navigated.1);
    let paused: Vec<String> = paused.iter().map(|kind| (*kind).to_owned()).collect();
    assert_eq!(
        found,
        (Ok(json!({})), navigated, challenges, paused),
        "{patterns}"
    );
}

/// Case 4: credentials given once per Session. A page of the engine's own
/// in the context answers the challenge of its document's request; a second
/// page of the context, with no `Fetch`, then reaches three origins, a
/// tunnel each, with no challenge and no failed request; another context,
/// given no credentials, still meets the 407.
#[test]
fn credentials_given_once_let_the_contexts_other_pages_through() {
    let found = on_the_pinned_build(async |cdp, urls, proxy, proving_ground| {
        let session = cdp.context(&proxy).await;
        let engine_page = cdp.page(&session).await;
        cdp.call(
            Some(&engine_page),
            "Fetch.enable",
            json!({ "patterns": [{ "resourceType": "Document" }], "handleAuthRequests": true }),
        )
        .await
        .expect("the engine's page takes Fetch");
        let given = cdp.navigate(&engine_page, &urls[0]).await;
        let challenges = cdp.raised(&engine_page, "Fetch.authRequired").len();
        let before = relays(proving_ground).len();

        let page = cdp.page(&session).await;
        let mut loads = Vec::new();
        for url in &urls {
            loads.push(cdp.navigate(&page, url).await);
        }
        let failed = cdp.raised(&page, "Network.loadingFailed").len();
        let asked_again = cdp
            .raised(&page, "Network.responseReceived")
            .iter()
            .any(|event| event["params"]["response"]["status"] == json!(407));
        let after = relays(proving_ground);
        let others = after[before..].to_vec();

        let other = cdp.context(&proxy).await;
        let stranger = cdp.page(&other).await;
        let refused = cdp.navigate(&stranger, &urls[1]).await;
        let strangers = relays(proving_ground)[after.len()..].to_vec();
        (
            (given, challenges),
            (loads, failed, asked_again),
            (
                hosts(&others, "proxy_authentication_required"),
                hosts(&others, "relayed"),
            ),
            (
                refused,
                hosts(&strangers, "proxy_authentication_required"),
                hosts(&strangers, "relayed"),
            ),
        )
    });

    let (given, others, other_relays, stranger) = found;
    assert_eq!(given, ((None, true), 1), "the engine's page");
    assert_eq!(
        others,
        (vec![(None, true); 3], 0, false),
        "the context's other page"
    );
    // The first origin's tunnel may be the engine page's, still open.
    assert!(other_relays.0.is_empty(), "{other_relays:?}");
    assert!(
        other_relays
            .1
            .iter()
            .all(|host| HOSTS.contains(&host.as_str()))
            && other_relays.1.iter().any(|host| host == HOSTS[1])
            && other_relays.1.iter().any(|host| host == HOSTS[2]),
        "{other_relays:?}"
    );
    assert_eq!(
        stranger,
        (
            (Some(NO_CREDENTIALS.to_owned()), false),
            vec![HOSTS[1].to_owned()],
            Vec::<String>::new()
        ),
        "another context"
    );
}
