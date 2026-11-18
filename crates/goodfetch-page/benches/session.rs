//! B2 (ADR 0140): a Collie Session the whole engine drives, through the
//! protocol, against a scripted browser that answers each command at once. The
//! engine's Borrowed browser path reaches it, so every line of a Session's
//! code runs, its CDP connection and its command log included, with no
//! test-only way in: the connection runs over the in-memory pipe that
//! `check_and_arm` uses, which the engine bridges to the browser's WebSocket
//! on loopback.
//!
//! Shared by the counts cell (`examples/counts.rs`) and its test
//! (`tests/it/session_counts.rs`). The browser counts each command by its ID,
//! from outside the connection, so a command's unit does not depend on what
//! the connection counts: the same command written twice is one unit and two
//! sends. Every count but one repeats exactly. The exception is `C1.file_opens`:
//! the command log's writer opens per batch, and how many commands a batch holds
//! follows the socket's timing, so its figure moves below the 2 per command
//! that one batch per command would cost, which is its limit.
//!
//! Each new outcome of a family's rows (a read, an action, a request) is a
//! step of [`program`]'s and, where it needs one, an answer of [`answered`].
//! The browser counts each guard's check, as a call of `check.js`'s source,
//! and the program each action it asked for and the engine answered, by
//! kind: C3's units. The actions on a page come after its reads, so no
//! gesture falls in a read's window.
//!
//! C2's reads are locator waits the browser answers as ready at once, each one
//! engine read, done once the Session's work after each navigation ended: the
//! rows of [`READ_ROWS`] are what those reads counted, never the reads a
//! navigation judges its document with, which are C4's outcome. After them the
//! program reads each document's formats once, a serialization of the page
//! (`DOM.getDocument`, `DOM.getOuterHTML`), its own outcome: the browser counts
//! each `DOM.getOuterHTML` and the commands sent meanwhile, and
//! [`SERIALIZATION_ROWS`] are what that read counted. Then it reads the
//! document's fields with `page.read` (ADR 0147), once at each of
//! [`FIELD_COUNTS`], its own outcome: the browser answers [`READ_FIELDS`]
//! with an element for each field, and [`FIELD_READ_ROWS`] are what each
//! of those reads counted, by its number of fields.
//!
//! Every request a page sends passes the interception (C5): the browser
//! pauses each navigation's document as it leaves and again at its
//! response, then, once it loaded, an image the egress policy refuses, at a
//! documentation address no range of the Run lists. It counts the requests
//! it paused, its pauses and the requests it was told to fail, each the unit
//! of a C5 row. A document answers `304`, a status the Collie page's
//! robots.txt reading leaves alone: that reading fetches through a page of
//! its own, which this browser does not play.
//!
//! Each navigation reaches its document with a 200 response, in its `Network`
//! events (its interception's response pause keeps its `304`), so the Session
//! judges it as it judges a real one (C4): its Block markers, its Page check
//! when the Run declares one, and the Consent prompt runner, which the last
//! page shows a prompt to. The browser plays the runner's bundle in the guard's world:
//! started in that page's document, it reports the prompt found, hidden and
//! its first pass settled, as the bundle reports through its binding.
#![expect(
    clippy::disallowed_types,
    reason = "a benchmark's browser end listens on its own socket, and counts its commands on an atomic no behaviour reads (ADR 0071)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::stream::FuturesUnordered;
use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{
    CallContext, CallError, Channel, Composition, Contribution, Dependencies, Engine, Handler,
    Identity,
};
use goodfetch_page::Launcher;
use goodfetch_page::blocks::{Block, BlockClassifier};
use goodfetch_page::page_value::{
    BrowserPages, HandlerPages, Observer, OwnFetch, PageValue, PooledSession,
};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::page::{PageId, PageResponse, SessionId};
use goodfetch_protocol::run::RunId;
use serde_json::{Map, Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream, ReadHalf, WriteHalf,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// One extra unit of a row's work, done once per command the browser reads.
pub(crate) type Plant = Arc<dyn Fn() + Send + Sync>;

/// A plant of one extra unit of `row` on `work`: through the counted
/// function of a compilation, and for a row with no counted function a
/// benchmark could call, counted at the seam that counts.
pub(crate) fn counting(work: &CountedWork, row: Row) -> Plant {
    let work = work.clone();
    match row {
        Row::InterceptedCompilations => Arc::new(move || {
            drop(work.compile_regex(row, "planted"));
        }),
        _ => Arc::new(move || work.count(row)),
    }
}

/// The Run the scenario starts.
pub(crate) const RUN: &str = "session-counts";

/// The browser's one page's target and frame, and the session its events
/// are written on: each attachment gets a session of its own, as Chromium
/// gives it, numbered after it (see [`Attachments`]).
const SESSION: &str = "page-session";
const TARGET: &str = "page-target";
const FRAME: &str = TARGET;

/// How long the counts must hold still for the Session's work to have ended:
/// what the engine sends on its own after a call answered.
const QUIET: Duration = Duration::from_millis(100);

/// The browser a Session's pages open in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Browsers {
    /// A Borrowed browser the program names, the scripted browser on a
    /// WebSocket: one page, navigated to each of [`PAGES`] in turn.
    Borrowed,
    /// The Run's browser pool, whose launcher starts the scripted browser on
    /// an in-memory pipe (C7): one page per document of [`PAGES`], each
    /// opened, navigated and closed through `BrowserPages` in one Session
    /// of the pool, as Foxhound's crawl opens its handlers' pages.
    Pool,
}

/// The pages the scenario navigates to, a navigation in the middle of a
/// Session's life, the last one showing a Consent prompt. Literal addresses
/// with no network: the browser answers.
const PAGES: [&str; 3] = [
    "http://192.0.2.1/first",
    "http://192.0.2.1/second",
    CONSENT_PAGE,
];

/// The page whose document shows a Consent prompt the runner hides.
const CONSENT_PAGE: &str = "http://192.0.2.1/consent";

/// The binding the Consent prompt runner reports through (`consent/messages.rs`).
const CONSENT_BINDING: &str = "__goodfetch_consent__";

/// The Run's settings of a Session with a Page check: its login page by a
/// glob on the URL, which no page matches, and its expected page by an
/// element, which every page shows.
pub(crate) fn page_check() -> Value {
    json!({ "session": { "page_check": {
        "login_page": { "kind": "glob", "pattern": "**/login" },
        "expected_page": { "kind": "css", "selector": "main" },
    } } })
}

/// The reads the program makes in each document: more than one, so a
/// document's later reads are seen apart from its first.
const READS: usize = 2;

/// The rows C2's reads count, read over the reads alone.
pub(crate) const READ_ROWS: [Row; 3] = [
    Row::PageReadCommands,
    Row::PageReadWorldCreations,
    Row::PageReadDocumentWorlds,
];

/// The rows a page's serialization for its formats counts, read over those
/// reads alone.
pub(crate) const SERIALIZATION_ROWS: [Row; 1] = [Row::PageSerializationCommands];

/// The rows a page's `page.read` counts, read over those reads alone.
pub(crate) const FIELD_READ_ROWS: [Row; 1] = [Row::PageFieldReadCommands];

/// The numbers of fields the program reads in each document, one
/// `page.read` each: C2's `read_commands` is held at one field and at
/// twenty (ADR 0147).
pub(crate) const FIELD_COUNTS: [usize; 2] = [1, 20];

/// The page function `page.read` finds a browser page's fields with, as
/// the engine sends it: the browser answers it by its source.
const READ_FIELDS: &str = include_str!("../src/fields/read.js");

/// The rows C3's actions count, read over the actions alone: each of the
/// program's reads is a wait, whose checks count on these rows too, and are
/// C2's outcome, not an action's.
const ACTION_ROWS: [Row; 5] = [
    Row::ActionScriptSends,
    Row::ActionCheckCommands,
    Row::ActionClickCommands,
    Row::ActionFillCommands,
    Row::ActionPressCommands,
];

/// The range the pages' address is in, which the Run lists in
/// `egress.private_ranges`, as a program against a local site does.
const PAGES_RANGE: &str = "192.0.2.0/24";

/// The image each page loads, at an address no range of the Run lists: the
/// egress policy refuses it.
const REFUSED: &str = "http://198.51.100.7/pixel.png";

/// The guard's check and its disarm, as the engine sends them: the browser
/// answers each by its source.
const CHECK: &str = include_str!("../src/guard/check.js");
const DISARM: &str = include_str!("../src/guard/disarm.js");

/// The element each action of the program acts on, which the browser's
/// answers make ready at once.
const FIELD: &str = "#field";

/// Each guarded action of the program on each page, by its kind: C3's.
const ACTIONS: [(&str, &str); 3] = [
    ("click", "locator.click"),
    ("fill", "locator.fill"),
    ("press", "locator.press"),
];

/// What one run of the scenario counted.
pub(crate) struct Counted {
    /// The commands the browser read, each once.
    pub(crate) commands: u64,
    /// The navigations the browser was asked for, each once.
    pub(crate) navigations: u64,
    /// The guard's checks the browser was asked for outside the program's
    /// reads, each once.
    pub(crate) checks: u64,
    /// The guarded actions the engine answered, by kind.
    pub(crate) attempts: BTreeMap<&'static str, u64>,
    /// The engine reads in a frame the browser answered while the program
    /// read, each one `Runtime.callFunctionOn`.
    pub(crate) reads: u64,
    /// The documents and frames those reads ran in.
    pub(crate) documents: u64,
    /// The commands the browser read while the program read.
    pub(crate) read_commands: u64,
    /// The serializations of the page the browser answered while the
    /// program read its formats, each one `DOM.getOuterHTML`.
    pub(crate) serializations: u64,
    /// The commands the browser read while the program read its formats.
    pub(crate) serialization_commands: u64,
    /// The `page.read` calls the browser answered, each one
    /// `Runtime.callFunctionOn` of [`READ_FIELDS`].
    pub(crate) field_reads: u64,
    /// The commands the browser read while the program read fields, by the
    /// number of fields each read held, over all documents.
    pub(crate) field_read_commands: BTreeMap<usize, u64>,
    /// What [`FIELD_READ_ROWS`] counted over the reads of each number of
    /// fields, over all documents.
    pub(crate) field_rows: BTreeMap<usize, u64>,
    /// The pages the browser opened (`Target.createTarget`), each once.
    pub(crate) pages_opened: u64,
    /// The requests the browser paused, each once.
    pub(crate) requests: u64,
    /// The browser's `Fetch.requestPaused` events: a request's, and a
    /// document's response's.
    pub(crate) paused: u64,
    /// The requests the browser was told to fail, each once.
    pub(crate) refused: u64,
    /// Its Counted work.
    pub(crate) work: CountedWork,
    /// What [`READ_ROWS`], [`SERIALIZATION_ROWS`], [`FIELD_READ_ROWS`] and
    /// [`ACTION_ROWS`] counted while the program read.
    during_reads: BTreeMap<&'static str, u64>,
}

impl Counted {
    /// What `row` counted over the outcome it is held per: over the program's
    /// reads for [`READ_ROWS`], over its formats reads for
    /// [`SERIALIZATION_ROWS`], over its `page.read` calls for
    /// [`FIELD_READ_ROWS`], over the whole Session but its reads for
    /// [`ACTION_ROWS`], over the whole Session for every other row.
    pub(crate) fn count(&self, row: Row) -> u64 {
        let during = self.during_reads.get(row.name()).copied();
        if ACTION_ROWS.contains(&row) {
            return self.work.read(row) - during.unwrap_or(0);
        }
        during.unwrap_or_else(|| self.work.read(row))
    }
}

/// What the browser saw of the program's reads, while it reads.
#[derive(Default)]
struct Reading {
    /// Whether the program reads now.
    now: AtomicBool,
    /// The calls answered while it read.
    calls: AtomicU64,
    /// The commands read while it read, each once.
    commands: AtomicU64,
    /// The document and frame of each world a call ran in.
    documents: Mutex<BTreeSet<(String, String)>>,
    /// Whether the program reads a page's formats now.
    serializing: AtomicBool,
    /// The serializations answered while it read formats.
    serializations: AtomicU64,
    /// The commands read while it read formats, each once.
    serialization_commands: AtomicU64,
    /// Whether the program reads a page's fields now.
    fields: AtomicBool,
    /// The fields' reads answered while it read fields.
    field_reads: AtomicU64,
    /// The commands read while it read fields, each once.
    field_read_commands: AtomicU64,
}

impl Reading {
    /// Counts the command `method` with `params` while the program reads
    /// formats or fields, and each serialization or field read it is.
    fn formats_and_fields(&self, method: &str, params: &Value) {
        if self.serializing.load(Ordering::Relaxed) {
            self.serialization_commands.fetch_add(1, Ordering::Relaxed);
            if method == "DOM.getOuterHTML" {
                self.serializations.fetch_add(1, Ordering::Relaxed);
            }
        }
        if self.fields.load(Ordering::Relaxed) {
            self.field_read_commands.fetch_add(1, Ordering::Relaxed);
            if method == "Runtime.callFunctionOn"
                && params["functionDeclaration"].as_str() == Some(READ_FIELDS)
            {
                self.field_reads.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// The browser's one page: its frame's document, by its loader, and each
/// isolated world it made, by its context, with the document it was made in.
#[derive(Default)]
struct Page {
    loader: String,
    worlds: BTreeMap<u64, (String, String)>,
}

impl Page {
    /// A page just opened, on its blank document.
    fn blank() -> Self {
        Self {
            loader: "loader".to_owned(),
            ..Self::default()
        }
    }
}

/// The scheme of the build's own pages, which a headless browser's
/// identity read loads one of: no request leaves for it, and its
/// navigation names no loader to wait on.
const BUILDS_OWN: &str = "chrome://";

/// What a headless browser's identity read finds its client hints to be.
fn client_hints() -> Value {
    json!({
        "brands": [],
        "fullVersionList": [],
        "platform": "Linux",
        "platformVersion": "",
        "architecture": "",
        "model": "",
        "mobile": false,
    })
}

/// The document the browser serializes each page as: the program reads its
/// links.
const SERIALIZED: &str = r#"<!doctype html><html><head><title>Page</title></head><body><main><a href="/next">Next</a></main></body></html>"#;

/// The states a locator wait waits for, as its check's argument names them.
const WAITED: [&str; 4] = ["attached", "detached", "visible", "hidden"];

/// The browser's counts of what it read and sent, each the unit of a row.
#[derive(Default)]
struct Units {
    commands: AtomicU64,
    requests: AtomicU64,
    paused: AtomicU64,
    refused: AtomicU64,
}

impl Units {
    fn reading(&self) -> [u64; 4] {
        [&self.commands, &self.requests, &self.paused, &self.refused]
            .map(|unit| unit.load(Ordering::Relaxed))
    }
}

/// A request the browser paused, by its interception ID.
struct Request {
    url: String,
    network_id: String,
    kind: &'static str,
    /// Whether its response was paused.
    responded: bool,
    /// Whether it was failed.
    failed: bool,
}

/// The requests the browser paused, and the next ID it gives one.
#[derive(Default)]
struct Requests {
    paused: BTreeMap<String, Request>,
    next: u64,
}

impl Requests {
    /// Pauses a request of `kind` to `url`, sent from the document at
    /// `document`: its events, as the browser sends them, its
    /// `Network.requestWillBeSent` first for one a page's events follow.
    fn pause(
        &mut self,
        url: &str,
        kind: &'static str,
        document: &str,
        units: &Units,
    ) -> Vec<Value> {
        self.next += 1;
        let (id, network_id) = (
            format!("interception-{}", self.next),
            format!("network-{}", self.next),
        );
        let headers = if kind == "Document" {
            json!({})
        } else {
            json!({ "Referer": document })
        };
        let request = json!({
            "url": url,
            "method": "GET",
            "headers": headers,
            "initialPriority": "VeryHigh",
            "referrerPolicy": "strict-origin-when-cross-origin",
        });
        let mut events = Vec::new();
        if kind != "Document" {
            events.push(json!({
                "method": "Network.requestWillBeSent",
                "sessionId": SESSION,
                "params": {
                    "requestId": network_id,
                    "loaderId": document,
                    "documentURL": document,
                    "request": request,
                    "timestamp": 0.0,
                    "wallTime": 0.0,
                    "initiator": { "type": "parser" },
                    "redirectHasExtraInfo": false,
                    "type": kind,
                    "frameId": FRAME,
                },
            }));
        }
        events.push(json!({
            "method": "Fetch.requestPaused",
            "sessionId": SESSION,
            "params": {
                "requestId": id,
                "request": request,
                "frameId": FRAME,
                "resourceType": kind,
                "networkId": network_id,
            },
        }));
        units.requests.fetch_add(1, Ordering::Relaxed);
        units.paused.fetch_add(1, Ordering::Relaxed);
        self.paused.insert(
            id,
            Request {
                url: url.to_owned(),
                network_id,
                kind,
                responded: false,
                failed: false,
            },
        );
        events
    }

    /// The events of the answer `method` to the request paused at `id`.
    fn answered(&mut self, method: &str, id: &str, units: &Units) -> Vec<Value> {
        let request = self
            .paused
            .get_mut(id)
            .unwrap_or_else(|| panic!("`{method}` answers no paused request: {id}"));
        match method {
            // A document's response pauses too.
            "Fetch.continueRequest" if request.kind == "Document" && !request.responded => {
                request.responded = true;
                units.paused.fetch_add(1, Ordering::Relaxed);
                vec![json!({
                    "method": "Fetch.requestPaused",
                    "sessionId": SESSION,
                    "params": {
                        "requestId": id,
                        "request": {
                            "url": request.url,
                            "method": "GET",
                            "headers": {},
                            "initialPriority": "VeryHigh",
                            "referrerPolicy": "strict-origin-when-cross-origin",
                        },
                        "frameId": FRAME,
                        "resourceType": "Document",
                        "networkId": request.network_id,
                        "responseStatusCode": 304,
                        "responseHeaders": [{ "name": "Content-Type", "value": "text/html" }],
                    },
                })]
            }
            "Fetch.continueResponse" if request.kind == "Document" => {
                let url = request.url.clone();
                let mut events = navigated(&url, &request.network_id);
                // The page's image, before its `load`.
                let image = self.pause(REFUSED, "Image", &url, units);
                let at = events
                    .iter()
                    .position(|event| event["params"]["name"] == "load")
                    .expect("invariant: a navigation loads");
                let load = events.split_off(at);
                events.extend(image);
                events.extend(load);
                events
            }
            "Fetch.failRequest" if !request.failed => {
                request.failed = true;
                units.refused.fetch_add(1, Ordering::Relaxed);
                vec![json!({
                    "method": "Network.loadingFailed",
                    "sessionId": SESSION,
                    "params": {
                        "requestId": request.network_id,
                        "timestamp": 0.0,
                        "type": request.kind,
                        "errorText": "net::ERR_BLOCKED_BY_CLIENT",
                    },
                })]
            }
            _ => Vec::new(),
        }
    }
}

/// A Block classification whose rules read a 200's document for a marker, as
/// one of Collie's does (its `block_detection` rules): every navigation's
/// document is then read for its Block markers. It finds no Block cause.
struct ReadsMarkers;

impl BlockClassifier for ReadsMarkers {
    fn classify(&self, _response: &PageResponse, _markup: &str) -> Option<Block> {
        None
    }

    fn markers(&self, _status: u16) -> Vec<&'static str> {
        vec!["captcha-delivery"]
    }
}

/// The `result` of `method` as the browser answers it on `page`, and the
/// events it sends right after.
fn answered(method: &str, params: &Value, page: &mut Page) -> Result<(Value, Vec<Value>), Value> {
    Ok(match method {
        "Browser.getVersion" => (
            json!({
                "protocolVersion": "1.3",
                "product": "Scripted/1.0",
                "revision": "0",
                "userAgent": "Scripted",
                "jsVersion": "0",
            }),
            Vec::new(),
        ),
        "Target.getBrowserContexts" => (
            json!({ "browserContextIds": [], "defaultBrowserContextId": "context" }),
            Vec::new(),
        ),
        "Target.createBrowserContext" => (json!({ "browserContextId": "context" }), Vec::new()),
        "Target.createTarget" => {
            // The pool's pages open one after another, each on its own
            // blank document; a headless browser's identity read opens one
            // in the background first.
            if params["background"] != json!(true) {
                *page = Page::blank();
            }
            (json!({ "targetId": TARGET }), Vec::new())
        }
        "Target.closeTarget" => (json!({ "success": true }), Vec::new()),
        "Target.attachToTarget" => (json!({ "sessionId": SESSION }), Vec::new()),
        "Page.getFrameTree" => (
            json!({ "frameTree": { "frame": {
                "id": FRAME,
                "loaderId": "loader",
                "url": "about:blank",
                "domainAndRegistry": "",
                "securityOrigin": "null",
                "mimeType": "text/html",
                "adFrameStatus": { "adFrameType": "none" },
                "secureContextType": "InsecureScheme",
                "crossOriginIsolatedContextType": "NotIsolated",
                "gatedAPIFeatures": [],
            } } }),
            Vec::new(),
        ),
        "Page.createIsolatedWorld" => {
            let context = u64::try_from(page.worlds.len()).expect("invariant: a few worlds") + 1;
            let frame = params["frameId"].as_str().unwrap_or_default().to_owned();
            page.worlds.insert(context, (frame, page.loader.clone()));
            (json!({ "executionContextId": context }), Vec::new())
        }
        "Page.addScriptToEvaluateOnNewDocument" => (json!({ "identifier": "script" }), Vec::new()),
        "Runtime.callFunctionOn" => {
            gone(page, &params["executionContextId"])?;
            called(params, &page.loader)
        }
        "Runtime.evaluate" => {
            gone(page, &params["contextId"])?;
            let expression = params["expression"].as_str().unwrap_or_default();
            if expression.contains("navigator.userAgentData") {
                return Ok((
                    json!({ "result": { "type": "object", "value": client_hints() } }),
                    Vec::new(),
                ));
            }
            (json!({ "result": { "type": "undefined" } }), Vec::new())
        }
        "Page.navigate" => {
            let url = params["url"].as_str().unwrap_or_default();
            url.clone_into(&mut page.loader);
            if url.starts_with(BUILDS_OWN) {
                return Ok((json!({ "frameId": FRAME }), Vec::new()));
            }
            (json!({ "frameId": FRAME, "loaderId": url }), Vec::new())
        }
        "DOM.getDocument" => (
            json!({ "root": {
                "nodeId": 1,
                "backendNodeId": 1,
                "nodeType": 9,
                "nodeName": "#document",
                "localName": "",
                "nodeValue": "",
                "documentURL": page.loader,
                "baseURL": page.loader,
            } }),
            Vec::new(),
        ),
        "DOM.getOuterHTML" => (json!({ "outerHTML": SERIALIZED }), Vec::new()),
        _ => (json!({}), Vec::new()),
    })
}

/// The browser's error for a call in `context`, a world of a document `page`
/// no longer shows: an earlier document's world is gone.
fn gone(page: &Page, context: &Value) -> Result<(), Value> {
    let world = context
        .as_u64()
        .and_then(|context| page.worlds.get(&context));
    if world.is_some_and(|(_, loader)| *loader != page.loader) {
        return Err(json!({ "code": -32000, "message": "Cannot find context with specified id" }));
    }
    Ok(())
}

/// What a function called in an isolated world of `document` answers, and
/// the events it sends: the guard's check finds its target ready at once, its
/// disarm says the gesture landed, a locator wait finds its state at once, the
/// Page check's read finds each match, the Consent prompt runner's bundle
/// starts and, on the Consent page, reports its prompt hidden (left while it
/// does not answer) and its first pass settled, `page.read`'s finds an
/// element for each field, and every other function answers nothing.
fn called(params: &Value, document: &str) -> (Value, Vec<Value>) {
    let function = params["functionDeclaration"].as_str().unwrap_or_default();
    let argument = &params["arguments"][0]["value"];
    let value = |value: Value| json!({ "result": { "type": "object", "value": value } });
    if function == CHECK {
        let state = match argument["action"].as_str() {
            Some("pointer") => json!({ "state": "armed", "x": 10.0, "y": 10.0 }),
            Some("fill" | "text") => json!({ "state": "focused" }),
            _ => json!({ "state": "ready" }),
        };
        return (value(state), Vec::new());
    }
    if function == DISARM {
        return (value(json!({ "state": "landed" })), Vec::new());
    }
    if function == READ_FIELDS {
        let element = json!({ "tag": "p", "text": "read", "attributes": [] });
        let found: Vec<Value> = argument["fields"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|field| match field["kind"].as_str() {
                Some("list") => json!([element]),
                Some("rows") => {
                    let row = vec![element.clone(); field["fields"].as_array().map_or(0, Vec::len)];
                    json!([row])
                }
                _ => element.clone(),
            })
            .collect();
        return (
            value(json!({ "baseUrl": document, "found": found })),
            Vec::new(),
        );
    }
    if WAITED.contains(&argument["action"].as_str().unwrap_or_default()) {
        return (value(json!({ "state": "ready" })), Vec::new());
    }
    if function.contains("function pageCheck(") {
        let matches = argument["matches"].as_array().map_or(0, Vec::len);
        let found = vec![true; matches];
        return (
            value(json!({ "found": found, "password": false, "robots": [] })),
            Vec::new(),
        );
    }
    let consent = document == CONSENT_PAGE;
    if function.starts_with("function detecting(") {
        let stands = if consent { "settled" } else { "none" };
        return (value(json!(stands)), Vec::new());
    }
    if function.starts_with("function start(") {
        let context = params["executionContextId"].clone();
        let reports = if consent {
            let action = if argument["answering"] == json!(false) {
                "left"
            } else {
                "hidden"
            };
            [
                json!({ "kind": "detected" }),
                json!({ "kind": "outcome", "url": document, "action": action, "rule": "scripted" }),
                json!({ "kind": "settled" }),
            ]
            .iter()
            .map(|report| {
                json!({
                    "method": "Runtime.bindingCalled",
                    "sessionId": SESSION,
                    "params": {
                        "name": CONSENT_BINDING,
                        "payload": report.to_string(),
                        "executionContextId": context,
                    },
                })
            })
            .collect()
        } else {
            Vec::new()
        };
        return (value(json!("started")), reports);
    }
    (json!({ "result": { "type": "undefined" } }), Vec::new())
}

/// The events the browser sends once it answered `method` with `params`,
/// past [`answered`]'s: a navigation's document pauses as it leaves, and each
/// answer to a paused request moves that request on.
fn moved_on(method: &str, params: &Value, requests: &mut Requests, units: &Units) -> Vec<Value> {
    if method == "Page.navigate" {
        let url = params["url"].as_str().unwrap_or_default();
        if url.starts_with(BUILDS_OWN) {
            return Vec::new();
        }
        return requests.pause(url, "Document", url, units);
    }
    match params["requestId"].as_str() {
        Some(id) if method.starts_with("Fetch.") => requests.answered(method, id, units),
        _ => Vec::new(),
    }
}

/// The events of a navigation to `url`, its document's request `request`, that
/// the network answered with a 200, and that committed and loaded.
fn navigated(url: &str, request: &str) -> Vec<Value> {
    let frame = json!({
        "id": FRAME,
        "loaderId": url,
        "url": url,
        "domainAndRegistry": "",
        "securityOrigin": url,
        "mimeType": "text/html",
        "adFrameStatus": { "adFrameType": "none" },
        "secureContextType": "InsecureScheme",
        "crossOriginIsolatedContextType": "NotIsolated",
        "gatedAPIFeatures": [],
    });
    let lifecycle = |name: &str| {
        json!({
            "method": "Page.lifecycleEvent",
            "sessionId": SESSION,
            "params": { "frameId": FRAME, "loaderId": url, "name": name, "timestamp": 0.0 },
        })
    };
    vec![
        lifecycle("init"),
        json!({
            "method": "Network.requestWillBeSent",
            "sessionId": SESSION,
            "params": {
                "requestId": request,
                "loaderId": url,
                "documentURL": url,
                "request": {
                    "url": url,
                    "method": "GET",
                    "headers": {},
                    "initialPriority": "VeryHigh",
                    "referrerPolicy": "strict-origin-when-cross-origin",
                },
                "timestamp": 0.0,
                "wallTime": 0.0,
                "initiator": { "type": "other" },
                "redirectHasExtraInfo": false,
                "type": "Document",
                "frameId": FRAME,
            },
        }),
        json!({
            "method": "Network.responseReceived",
            "sessionId": SESSION,
            "params": {
                "requestId": request,
                "loaderId": url,
                "timestamp": 0.0,
                "type": "Document",
                "response": {
                    "url": url,
                    "status": 200,
                    "statusText": "OK",
                    "headers": { "Content-Type": "text/html" },
                    "mimeType": "text/html",
                    "charset": "utf-8",
                    "connectionReused": false,
                    "connectionId": 1.0,
                    "encodedDataLength": 100.0,
                    "securityState": "neutral",
                },
                "hasExtraInfo": false,
                "frameId": FRAME,
            },
        }),
        json!({
            "method": "Page.frameNavigated",
            "sessionId": SESSION,
            "params": { "frame": frame, "type": "Navigation" },
        }),
        lifecycle("commit"),
        lifecycle("DOMContentLoaded"),
        lifecycle("load"),
        json!({
            "method": "Page.loadEventFired",
            "sessionId": SESSION,
            "params": { "timestamp": 0.0 },
        }),
    ]
}

/// What the browser end counts, from outside the connection: every command
/// and request, in its units, the pages it opened, and what the program's
/// reads sent.
#[derive(Clone, Default)]
struct Heard {
    units: Arc<Units>,
    /// The pages it opened, each once.
    pages: Arc<AtomicU64>,
    /// The navigations it was asked for, each once.
    navigations: Arc<AtomicU64>,
    /// Each call of the guard's check, by its command's ID.
    checks: Arc<AtomicU64>,
    reading: Arc<Reading>,
}

/// How the browser hears the engine: a Borrowed browser's WebSocket, or a
/// launched one's debugging pipe, each message JSON followed by a NUL byte.
enum Wire {
    Socket(Box<WebSocketStream<TcpStream>>),
    Pipe {
        reader: BufReader<ReadHalf<DuplexStream>>,
        writer: WriteHalf<DuplexStream>,
    },
}

impl Wire {
    /// The browser's end of a launched browser's pipe.
    fn pipe(pipe: DuplexStream) -> Self {
        let (reader, writer) = tokio::io::split(pipe);
        Self::Pipe {
            reader: BufReader::new(reader),
            writer,
        }
    }

    /// The engine's next message; `None` once it closed its end.
    async fn next(&mut self) -> Option<String> {
        match self {
            Self::Socket(socket) => loop {
                match socket.next().await? {
                    Ok(Message::Text(text)) => return Some(text.as_str().to_owned()),
                    Ok(_) => {}
                    Err(_) => return None,
                }
            },
            Self::Pipe { reader, .. } => {
                let mut message = Vec::new();
                reader.read_until(0, &mut message).await.ok()?;
                (message.pop() == Some(0)).then_some(())?;
                String::from_utf8(message).ok()
            }
        }
    }

    /// Sends `message`: whether the engine still reads.
    async fn send(&mut self, message: String) -> bool {
        match self {
            Self::Socket(socket) => socket.send(Message::text(message)).await.is_ok(),
            Self::Pipe { writer, .. } => {
                let mut framed = message.into_bytes();
                framed.push(0);
                writer.write_all(&framed).await.is_ok()
            }
        }
    }
}

/// The sessions a browser's attachments got: one each, as Chromium gives
/// it, so the events of a pool page closed before do not reach its
/// successor's. A headless browser's identity read is a background page's,
/// whose navigation no Session asked for.
#[derive(Default)]
struct Attachments {
    /// The session each `Target.attachToTarget` was answered, by its ID.
    sessions: BTreeMap<u64, String>,
    /// Whether the target created last opened in the background.
    background_next: bool,
    /// The background pages' sessions.
    background: BTreeSet<String>,
}

impl Attachments {
    /// The session of the attachment the command `id` asked for.
    fn attach(&mut self, id: u64) -> String {
        let number = self.sessions.len() + 1;
        let session = self
            .sessions
            .entry(id)
            .or_insert_with(|| format!("{SESSION}-{number}"))
            .clone();
        if self.background_next {
            self.background.insert(session.clone());
        }
        session
    }
}

/// The browser: reads each command of its one connection, does `plant`,
/// answers at once, until the engine closes it or asks it to close. Each
/// event a command on a page's session leads to goes on that session.
async fn browser(mut wire: Wire, counts: Heard, plant: Option<Plant>) {
    let mut seen = BTreeSet::new();
    let mut page = Page::blank();
    let mut requests = Requests::default();
    let mut attachments = Attachments::default();
    while let Some(text) = wire.next().await {
        let command: Value = serde_json::from_str(&text).expect("invariant: a command is JSON");
        let id = command["id"]
            .as_u64()
            .expect("invariant: a command has an ID");
        let method = command["method"].as_str().unwrap_or_default();
        let first = seen.insert(id);
        if first {
            counts.units.commands.fetch_add(1, Ordering::Relaxed);
            // A headless browser's identity read opens a page of its own, no
            // Session's.
            if method == "Target.createTarget" {
                let background = command["params"]["background"] == json!(true);
                attachments.background_next = background;
                if !background {
                    counts.pages.fetch_add(1, Ordering::Relaxed);
                }
            }
            let on_background = command["sessionId"]
                .as_str()
                .is_some_and(|session| attachments.background.contains(session));
            if method == "Page.navigate" && !on_background {
                counts.navigations.fetch_add(1, Ordering::Relaxed);
            }
            let reading = &counts.reading;
            if method == "Runtime.callFunctionOn"
                && command["params"]["functionDeclaration"].as_str() == Some(CHECK)
                && !reading.now.load(Ordering::Relaxed)
            {
                counts.checks.fetch_add(1, Ordering::Relaxed);
            }
            reading.formats_and_fields(method, &command["params"]);
            if reading.now.load(Ordering::Relaxed) {
                reading.commands.fetch_add(1, Ordering::Relaxed);
                if method == "Runtime.callFunctionOn" {
                    reading.calls.fetch_add(1, Ordering::Relaxed);
                    let context = command["params"]["executionContextId"].as_u64();
                    if let Some(document) = context.and_then(|context| page.worlds.get(&context)) {
                        reading
                            .documents
                            .lock()
                            .expect("invariant: no holder panics")
                            .insert(document.clone());
                    }
                }
            }
        }
        if let Some(plant) = &plant {
            plant();
        }
        let (mut answer, mut events) = match answered(method, &command["params"], &mut page) {
            Ok((result, events)) => (json!({ "id": id, "result": result }), events),
            Err(error) => (json!({ "id": id, "error": error }), Vec::new()),
        };
        if first {
            events.extend(moved_on(
                method,
                &command["params"],
                &mut requests,
                &counts.units,
            ));
        }
        if method == "Target.attachToTarget" {
            answer["result"]["sessionId"] = json!(attachments.attach(id));
        }
        if let Some(session) = command.get("sessionId") {
            answer["sessionId"] = session.clone();
            for event in &mut events {
                if event.get("sessionId").is_some() {
                    event["sessionId"] = session.clone();
                }
            }
        }
        let mut messages = vec![answer];
        messages.extend(events);
        for message in messages {
            if !wire.send(message.to_string()).await {
                return;
            }
        }
        if method == "Browser.close" {
            return;
        }
    }
}

/// The engine's answer to one request of the program.
struct Client {
    lines: tokio::io::Lines<BufReader<transport::ConnectionReader>>,
    writer: transport::ConnectionWriter,
    next_id: u64,
}

impl Client {
    /// Calls `method`, failing the scenario on an error: a benchmark whose
    /// work did not happen proves nothing.
    async fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let mut line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
            .to_string()
            .into_bytes();
        line.push(b'\n');
        self.writer
            .write_all(&line)
            .await
            .expect("invariant: the engine reads its Connection");
        loop {
            let line = self
                .lines
                .next_line()
                .await
                .expect("invariant: the engine's lines read")
                .expect("invariant: the engine answers before it closes");
            let message: Value =
                serde_json::from_str(&line).expect("invariant: the engine writes JSON");
            if message["id"] == json!(id) && message.get("method").is_none() {
                return message
                    .get("result")
                    .cloned()
                    .unwrap_or_else(|| panic!("`{method}` failed: {message}"));
            }
        }
    }
}

/// What a program does on a Session: on a Borrowed browser, opens a page and
/// navigates it to each of [`PAGES`], so a navigation sits in the middle; in
/// the browser pool, opens a page at each and closes it after. On each
/// document, it does [`on_page`]'s work. Answers once the browser heard all
/// of it, with the actions the engine answered, by kind.
async fn program(
    client: &mut Client,
    (browsers, cdp_url, settings): (Browsers, &str, Map<String, Value>),
    (heard, work): (&Heard, &CountedWork),
    (during_reads, by_fields): (&mut BTreeMap<&'static str, u64>, &mut ByFields),
) -> BTreeMap<&'static str, u64> {
    client
        .call(
            "hello",
            &json!({ "protocol_version": "0.1", "surface": "rust", "needs": "collie", "product_version": "0.0.0" }),
        )
        .await;
    client
        .call("run.start", &json!({ "run_id": RUN, "settings": settings }))
        .await;
    let mut attempts = BTreeMap::new();
    if browsers == Browsers::Pool {
        for url in PAGES {
            let opened = client
                .call(OPEN_POOL_PAGE.name, &json!({ "run_id": RUN, "url": url }))
                .await;
            quiet(&heard.units, work).await;
            on_page(
                client,
                (&opened["page_id"], url),
                (heard, work),
                (during_reads, by_fields, &mut attempts),
            )
            .await;
            client
                .call(
                    CLOSE_POOL_PAGE.name,
                    &json!({ "page_id": opened["page_id"] }),
                )
                .await;
        }
        return attempts;
    }
    let session = client
        .call(
            "run.new_session",
            &json!({ "run_id": RUN, "source": { "kind": "cdp_url", "url": cdp_url } }),
        )
        .await;
    let opened = client
        .call(
            "session.new_page",
            &json!({ "session_id": session["session_id"] }),
        )
        .await;
    for url in PAGES {
        client
            .call(
                "page.goto",
                &json!({ "page_id": opened["page_id"], "url": url }),
            )
            .await;
        quiet(&heard.units, work).await;
        on_page(
            client,
            (&opened["page_id"], url),
            (heard, work),
            (during_reads, by_fields, &mut attempts),
        )
        .await;
    }
    attempts
}

/// What [`FIELD_READ_ROWS`] counted and the commands the browser read over
/// the `page.read` calls of each number of fields, over all documents.
type ByFields = BTreeMap<usize, (u64, u64)>;

/// What the program does on the page `page_id` once it shows `url`: reads
/// its document [`READS`] times once the navigation's own work ended,
/// adding what [`READ_ROWS`] counted meanwhile to `during_reads`, then its
/// formats once, adding what [`SERIALIZATION_ROWS`] counted, then its
/// fields once at each of [`FIELD_COUNTS`], adding what
/// [`FIELD_READ_ROWS`] counted to `during_reads` and, by the number of
/// fields, to `by_fields`, then, on each
/// page but the Consent prompt's, clicks, fills and presses, adding each
/// to `attempts`: the prompt's page is C4's, and its runner's work no
/// action's.
async fn on_page(
    client: &mut Client,
    (page_id, url): (&Value, &str),
    (heard, work): (&Heard, &CountedWork),
    (during_reads, by_fields, attempts): (
        &mut BTreeMap<&'static str, u64>,
        &mut ByFields,
        &mut BTreeMap<&'static str, u64>,
    ),
) {
    let locator = json!({ "page_id": page_id, "selector": FIELD });
    let before = READ_ROWS.map(|row| work.read(row));
    let actions_before = ACTION_ROWS.map(|row| work.read(row));
    heard.reading.now.store(true, Ordering::Relaxed);
    for _ in 0..READS {
        client
            .call(
                "locator.wait_for",
                &json!({
                    "locator": { "page_id": page_id, "selector": "body" },
                    "state": "attached",
                }),
            )
            .await;
    }
    quiet(&heard.units, work).await;
    heard.reading.now.store(false, Ordering::Relaxed);
    let read_rows = READ_ROWS.into_iter().zip(before);
    for (row, before) in read_rows.chain(ACTION_ROWS.into_iter().zip(actions_before)) {
        *during_reads.entry(row.name()).or_default() += work.read(row) - before;
    }
    let before = SERIALIZATION_ROWS.map(|row| work.read(row));
    let actions_before = ACTION_ROWS.map(|row| work.read(row));
    heard.reading.serializing.store(true, Ordering::Relaxed);
    client
        .call(
            "page.formats",
            &json!({ "page_id": page_id, "formats": { "links": true } }),
        )
        .await;
    quiet(&heard.units, work).await;
    heard.reading.serializing.store(false, Ordering::Relaxed);
    let serialization_rows = SERIALIZATION_ROWS.into_iter().zip(before);
    for (row, before) in serialization_rows.chain(ACTION_ROWS.into_iter().zip(actions_before)) {
        *during_reads.entry(row.name()).or_default() += work.read(row) - before;
    }
    for count in FIELD_COUNTS {
        let before = FIELD_READ_ROWS.map(|row| work.read(row));
        let actions_before = ACTION_ROWS.map(|row| work.read(row));
        let commands_before = heard.reading.field_read_commands.load(Ordering::Relaxed);
        heard.reading.fields.store(true, Ordering::Relaxed);
        client
            .call(
                "page.read",
                &json!({ "page_id": page_id, "fields": declared(count) }),
            )
            .await;
        quiet(&heard.units, work).await;
        heard.reading.fields.store(false, Ordering::Relaxed);
        let commands = heard.reading.field_read_commands.load(Ordering::Relaxed) - commands_before;
        let counted = by_fields.entry(count).or_default();
        for (row, before) in FIELD_READ_ROWS.into_iter().zip(before) {
            let read = work.read(row) - before;
            *during_reads.entry(row.name()).or_default() += read;
            counted.0 += read;
        }
        counted.1 += commands;
        for (row, before) in ACTION_ROWS.into_iter().zip(actions_before) {
            *during_reads.entry(row.name()).or_default() += work.read(row) - before;
        }
    }
    if url == CONSENT_PAGE {
        return;
    }
    for (kind, operation) in ACTIONS {
        let params = match kind {
            "fill" => json!({ "locator": locator, "value": "typed" }),
            "press" => json!({ "locator": locator, "key": "Enter" }),
            _ => json!({ "locator": locator }),
        };
        client.call(operation, &params).await;
        *attempts.entry(kind).or_insert(0) += 1;
    }
}

/// `count` fields as `page.read` takes them: a heading, then for twenty
/// one of each kind past the value field, a list and rows of two fields,
/// and value fields for the rest, by CSS and by `XPath` in turn.
fn declared(count: usize) -> Value {
    let mut fields = Map::new();
    fields.insert(
        "title".to_owned(),
        json!({ "locator": { "kind": "css", "selector": "h1" } }),
    );
    if count > 1 {
        fields.insert(
            "links".to_owned(),
            json!({ "list": { "locator": { "kind": "css", "selector": "a" }, "attribute": "href" } }),
        );
        fields.insert(
            "products".to_owned(),
            json!({
                "rows": { "kind": "css", "selector": "li.product" },
                "fields": {
                    "name": { "locator": { "kind": "css", "selector": "h2" } },
                    "price": { "locator": { "kind": "xpath", "selector": ".//span" } },
                },
            }),
        );
    }
    for index in fields.len()..count {
        let locator = if index % 2 == 0 {
            json!({ "kind": "css", "selector": format!("#field-{index}") })
        } else {
            json!({ "kind": "xpath", "selector": format!("//*[@id='field-{index}']") })
        };
        fields.insert(format!("field_{index:02}"), json!({ "locator": locator }));
    }
    Value::Object(fields)
}

/// Waits until nothing is counted or sent any more, as the engine still sends
/// what no call waits for.
async fn quiet(units: &Units, work: &CountedWork) {
    let reading = || {
        let counts: Vec<u64> = Row::ALL.iter().map(|row| work.read(*row)).collect();
        (units.reading(), counts)
    };
    let mut last = reading();
    loop {
        tokio::time::sleep(QUIET).await;
        let now = reading();
        if now == last {
            return;
        }
        last = now;
    }
}

/// The program's way to a page of the Run's browser pool, as Foxhound's
/// crawl opens one for a handler: `{ run_id, url }`, answered with the page's
/// `page_id`, which the program then drives as its own.
const OPEN_POOL_PAGE: Operation = Operation {
    name: "test.open_pool_page",
    description: "Opens a page of the Run's browser pool through BrowserPages, navigated to a URL.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

/// Closes a page [`OPEN_POOL_PAGE`] opened: `{ page_id }`.
const CLOSE_POOL_PAGE: Operation = Operation {
    name: "test.close_pool_page",
    description: "Closes a page of the Run's browser pool through BrowserPages.",
    ..OPEN_POOL_PAGE
};

/// The pool's one Session, a fresh one, as the crawl's Session pool names it.
fn pooled() -> PooledSession {
    PooledSession {
        id: SessionId("pooled".to_owned()),
        source: Some(None),
        address_ordinal: None,
    }
}

/// The pages a program opens, with an empty value and no observer, as
/// Collie's, and the handle through which the engine opens pool pages,
/// once composed.
#[derive(Clone, Default)]
struct PoolPages(Arc<OnceLock<Arc<dyn BrowserPages>>>);

impl PoolPages {
    fn pages(&self) -> Arc<dyn BrowserPages> {
        Arc::clone(self.0.get().expect("invariant: the engine is composed"))
    }
}

impl HandlerPages for PoolPages {
    fn value(&self, _run: &RunId, _session: &SessionId) -> PageValue {
        PageValue::empty()
    }

    fn observer(&self, _run: &RunId, _session: &SessionId) -> Option<Observer> {
        None
    }

    fn opened(&self, pages: Arc<dyn BrowserPages>) {
        let _ = self.0.set(pages);
    }
}

/// The program's operations on the browser pool's pages.
fn pool_pages(pages: &PoolPages) -> Contribution {
    let (opening, closing) = (pages.clone(), pages.clone());
    Contribution::new("test")
        .with_handler(Handler::new(
            &OPEN_POOL_PAGE,
            move |params: Value, context: CallContext| {
                let pages = opening.pages();
                async move {
                    let run_id = RunId::parse(params["run_id"].as_str().unwrap_or_default())
                        .expect("invariant: the program names its Run");
                    let url = params["url"].as_str().unwrap_or_default();
                    let own = OwnFetch {
                        url,
                        method: "GET",
                        body: None,
                    };
                    let (page_id, _) = pages.open(&run_id, &pooled(), own, &context).await?;
                    Ok::<_, CallError>(json!({ "page_id": page_id }))
                }
            },
        ))
        .with_handler(Handler::new(
            &CLOSE_POOL_PAGE,
            move |params: Value, _context: CallContext| {
                let pages = closing.pages();
                async move {
                    let page_id = PageId(params["page_id"].as_str().unwrap_or_default().to_owned());
                    pages.close(&page_id).await;
                    Ok::<_, CallError>(json!({}))
                }
            },
        ))
}

/// The Collie engine the scenario drives, counting on `metrics`, its pages
/// judged by [`ReadsMarkers`], its Runs' browsers started by `launcher`, and
/// the program's operations on the pool's pages beside its own.
fn engine(metrics: Metrics, launcher: Launcher) -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    let pages = PoolPages::default();
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_page::contribution_with(
            (Vec::new(), launcher),
            pages.clone(),
            Arc::new(ReadsMarkers),
        ))
        .with(pool_pages(&pages));
    Engine::new(identity, &composition)
        .expect("invariant: one handler each")
        .with_metrics(metrics)
}

/// Runs the scenario on `browsers` in the home `home`, with `settings` on its
/// Run, doing `plant` once per command the browser reads, and answers what it
/// counted.
///
/// `metrics` is the engine's, whose [`CountedWork`] counts it all.
#[expect(
    clippy::disallowed_methods,
    reason = "a benchmark builds its own runtime, with real time as the browser's socket needs (ADR 0060); its SQL statements are its own, on no Shared path (ADR 0140)"
)]
pub(crate) fn counted(
    browsers: Browsers,
    home: &Path,
    metrics: Metrics,
    mut settings: Map<String, Value>,
    plant: Option<Plant>,
) -> Counted {
    settings.insert(
        "egress".to_owned(),
        json!({ "private_ranges": [PAGES_RANGE] }),
    );
    let work = metrics.counted_work().clone();
    // The pool reads its scripted browser's memory from this process's.
    let (launcher, started) = Launcher::scripted(Some(std::process::id()));
    let engine = engine(metrics, launcher);
    let heard = Heard::default();
    let mut during_reads = BTreeMap::new();
    let mut by_fields = ByFields::new();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("invariant: the runtime starts");
    let attempts = runtime.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("invariant: loopback binds");
        let address = listener.local_addr().expect("invariant: it is bound");
        let cdp_url = format!("ws://{address}/devtools/browser/scripted");
        let (network, dialer) = transport::in_memory();
        let served: std::pin::Pin<Box<dyn Future<Output = ()>>> = match browsers {
            Browsers::Borrowed => Box::pin(browser_end(listener, heard.clone(), plant)),
            Browsers::Pool => Box::pin(launched_ends(started, heard.clone(), plant)),
        };
        let client = async {
            let (reader, writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
                next_id: 0,
            };
            let attempts = program(
                &mut client,
                (browsers, &cdp_url, settings),
                (&heard, &work),
                (&mut during_reads, &mut by_fields),
            )
            .await;
            quiet(&heard.units, &work).await;
            client
                .writer
                .shutdown()
                .await
                .expect("invariant: in memory");
            while let Ok(Some(_)) = client.lines.next_line().await {}
            attempts
        };
        let dependencies = dependencies(home);
        let ran = async {
            tokio::join!(
                engine.run_attached(Network::InMemory(network), dependencies),
                client,
            )
        };
        tokio::pin!(ran);
        // The pool's browsers are played until the engine is done with them.
        let (engine_ended, attempts) = tokio::select! {
            ran = &mut ran => ran,
            () = served => ran.await,
        };
        engine_ended.expect("invariant: the engine got its Connection");
        attempts
    });
    let reading = &heard.reading;
    let documents = reading
        .documents
        .lock()
        .expect("invariant: no holder panics")
        .len();
    let [commands, requests, paused, refused] = heard.units.reading();
    Counted {
        commands,
        navigations: heard.navigations.load(Ordering::Relaxed),
        checks: heard.checks.load(Ordering::Relaxed),
        attempts,
        reads: reading.calls.load(Ordering::Relaxed),
        documents: u64::try_from(documents).expect("invariant: a few documents"),
        read_commands: reading.commands.load(Ordering::Relaxed),
        serializations: reading.serializations.load(Ordering::Relaxed),
        serialization_commands: reading.serialization_commands.load(Ordering::Relaxed),
        field_reads: reading.field_reads.load(Ordering::Relaxed),
        field_read_commands: by_fields
            .iter()
            .map(|(count, (_, commands))| (*count, *commands))
            .collect(),
        field_rows: by_fields
            .into_iter()
            .map(|(count, (rows, _))| (count, rows))
            .collect(),
        pages_opened: heard.pages.load(Ordering::Relaxed),
        requests,
        paused,
        refused,
        work,
        during_reads,
    }
}

/// What the engine runs on: its home in `home`, the system's clock, seeded
/// entropy and every blocking task inline.
#[expect(
    clippy::disallowed_methods,
    reason = "a benchmark's engine draws seeded entropy, the Entropy's test version (ADR 0059)"
)]
fn dependencies(home: &Path) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(home.to_owned()),
        clock: Clock::system(),
        entropy: Entropy::seeded(1861),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

/// Accepts the engine's one connection and plays the browser on it.
async fn browser_end(listener: TcpListener, heard: Heard, plant: Option<Plant>) {
    let (stream, _) = listener
        .accept()
        .await
        .expect("invariant: the engine connects");
    let socket = tokio_tungstenite::accept_async(stream)
        .await
        .expect("invariant: the engine speaks WebSocket");
    browser(Wire::Socket(Box::new(socket)), heard, plant).await;
}

/// Plays each browser the pool launches, all at once, until the launcher is
/// gone.
async fn launched_ends(
    mut launched: goodfetch_page::ScriptedBrowsers,
    heard: Heard,
    plant: Option<Plant>,
) {
    let mut browsers = FuturesUnordered::new();
    loop {
        tokio::select! {
            pipe = launched.next() => match pipe {
                Some(pipe) => browsers.push(browser(Wire::pipe(pipe), heard.clone(), plant.clone())),
                None => break,
            },
            Some(()) = browsers.next(), if !browsers.is_empty() => {}
        }
    }
    while browsers.next().await.is_some() {}
}
