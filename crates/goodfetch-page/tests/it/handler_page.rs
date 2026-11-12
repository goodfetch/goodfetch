//! An engine of the engine's own operations and the page crate's, in
//! process, every page of which carries a Recording gate (ADR 0125): its
//! navigation gate, lease check and response observer, which Foxhound
//! fills and no Collie page does. A test drives it as a program does,
//! through the protocol over the in-memory network, against its own
//! Proving ground, Goodfetch home and Browser cache (ADR 0059), and reads
//! what the Recording gate recorded.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use goodfetch_engine::browser_cache::{BuildPlatform, CHROME_FOR_TESTING, PinnedArtifact};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_page::Launcher;
use goodfetch_page::blocks::NoBlocks;
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HELLO, ProductVersion, Surface};
use goodfetch_protocol::home::{RUN_TRACE_CDP_COMMANDS, RUN_TRACE_REQUESTS};
use goodfetch_recording_gate::RecordingGate;
use goodfetch_test_client::Hello;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{CHROME_FOR_TESTING_BUILD, ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};

/// How long one call may take: the first page launches the browser.
const CALL_DEADLINE: Duration = Duration::from_secs(60);

/// How long a test waits for what the browser does after a call answered.
const SETTLE_DEADLINE: Duration = Duration::from_secs(20);

/// The Run every test starts.
pub(crate) const RUN: &str = "handler-pages";

/// The digests of the Chrome for Testing builds Collie's version pins,
/// which every launch checks its build's files against: Collie's own file,
/// since this crate sits below Collie's executable crate.
const PINS: &str = include_str!("../../../goodfetch-collie/src/pins.json");

/// The pinned Chrome for Testing build the Browser cache holds, for this
/// platform, never downloaded.
fn pins() -> Vec<PinnedArtifact> {
    let platform = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        BuildPlatform::MacArm64
    } else if cfg!(target_os = "macos") {
        BuildPlatform::MacX64
    } else if cfg!(target_os = "windows") {
        BuildPlatform::WindowsX64
    } else {
        BuildPlatform::LinuxX64
    };
    let name = platform
        .chrome_for_testing_name()
        .expect("invariant: Chrome for Testing builds for this platform");
    let pins: Value = serde_json::from_str(PINS).expect("invariant: Collie's pins are JSON");
    let chrome = &pins["chrome_for_testing"];
    assert_eq!(chrome["build"], json!(CHROME_FOR_TESTING_BUILD));
    vec![PinnedArtifact {
        id: ArtifactId {
            kind: ArtifactKind::Browser,
            name: CHROME_FOR_TESTING.to_owned(),
            build: CHROME_FOR_TESTING_BUILD.to_owned(),
        },
        platform: Some(platform),
        url: "https://storage.googleapis.com/chrome-for-testing-public/unused.zip".to_owned(),
        archive_digest: None,
        application_settings: std::collections::BTreeMap::new(),
        digest: chrome["platforms"][name]["digest"]
            .as_str()
            .expect("Collie pins this platform's build")
            .parse()
            .expect("invariant: Collie's digests read"),
    }]
}

/// A Connection to the engine, as a program holds it.
pub(crate) struct Program {
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
    next_id: i64,
    /// The events read so far: `page.*` notifications of the event stream.
    events: Vec<Value>,
    /// Answers read while another was awaited, by ID.
    answers: Map<String, Value>,
    pub(crate) proving_ground: ProvingGround,
    /// The page the test drives.
    pub(crate) page: String,
    /// The test's Goodfetch home.
    home: PathBuf,
}

impl Program {
    async fn write(&mut self, message: &Value) {
        let mut line = message.to_string().into_bytes();
        line.push(b'\n');
        self.writer
            .write_all(&line)
            .await
            .expect("invariant: the engine reads its Connection");
    }

    /// Sends `method` with `params`, answering its ID.
    pub(crate) async fn request(&mut self, method: &str, params: &Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        id
    }

    /// Sends the notification `method`.
    pub(crate) async fn notify(&mut self, method: &str, params: &Value) {
        self.write(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await;
    }

    /// Reads one line, keeping an event or an answer.
    async fn read(&mut self, until: Instant) -> Option<()> {
        let line = tokio::time::timeout_at(until.into(), self.lines.next_line())
            .await
            .ok()?
            .expect("invariant: the engine's lines read")?;
        let message: Value = serde_json::from_str(&line).expect("invariant: a JSON line");
        match message.get("id") {
            Some(id) if message.get("method").is_none() => {
                self.answers.insert(id.to_string(), message);
            }
            _ => self.events.push(message),
        }
        Some(())
    }

    /// The answer to the request `id`, the whole message.
    #[expect(
        clippy::disallowed_methods,
        reason = "a real browser answers in real time, so the wait's deadline is real time (ADR 0060)"
    )]
    pub(crate) async fn answer(&mut self, id: i64) -> Value {
        let until = Instant::now() + CALL_DEADLINE;
        loop {
            if let Some(answer) = self.answers.remove(&id.to_string()) {
                return answer;
            }
            let read = self.read(until).await;
            assert!(
                read.is_some(),
                "no answer to request {id} within {CALL_DEADLINE:?}"
            );
        }
    }

    /// Calls `method` with `params` and answers the whole message.
    pub(crate) async fn call(&mut self, method: &str, params: &Value) -> Value {
        let id = self.request(method, params).await;
        self.answer(id).await
    }

    /// Calls `method` on the test's page with `params` added.
    pub(crate) async fn on_page(&mut self, method: &str, params: Value) -> Value {
        let mut params = params;
        params["page_id"] = json!(self.page);
        self.call(method, &params).await
    }

    /// Navigates the test's page to `path` on `host` and answers the call's
    /// result, which it requires.
    pub(crate) async fn goto(&mut self, host: &str, path: &str) -> Value {
        let url = self.proving_ground.url(host, path);
        let went = self.on_page("page.goto", json!({ "url": url })).await;
        went.get("result")
            .cloned()
            .unwrap_or_else(|| panic!("`page.goto` to {url} failed: {went}"))
    }

    /// Listens to `event` of the test's page, granting the listening call
    /// credits for every event it sends.
    pub(crate) async fn listen(&mut self, event: &str) {
        let page = self.page.clone();
        let id = self
            .request("page.on", &json!({ "page_id": page, "event": event }))
            .await;
        self.notify("$/credit", &json!({ "id": id, "credits": 10_000 }))
            .await;
    }

    /// The data of each event `name` read so far.
    pub(crate) fn events(&self, name: &str) -> Vec<Value> {
        // The event stream sends its events in batches.
        self.events
            .iter()
            .flat_map(|message| message["params"]["events"].as_array().into_iter().flatten())
            .filter(|event| event["event"] == json!(name))
            .map(|event| event["data"].clone())
            .collect()
    }

    /// Reads until the engine sent a request `method`, such as
    /// `route.handle`, and answers the first one: the whole message.
    #[expect(
        clippy::disallowed_methods,
        reason = "a real browser answers in real time, so the wait's deadline is real time (ADR 0060)"
    )]
    pub(crate) async fn until_request(&mut self, method: &str) -> Value {
        let until = Instant::now() + SETTLE_DEADLINE;
        loop {
            let asked = self
                .events
                .iter()
                .find(|message| message["method"] == json!(method) && message.get("id").is_some());
            if let Some(asked) = asked {
                return asked.clone();
            }
            let read = self.read(until).await;
            assert!(
                read.is_some(),
                "no `{method}` request within {SETTLE_DEADLINE:?}"
            );
        }
    }

    /// Replies `result` to the engine's request `request`, as a handler
    /// that ended.
    pub(crate) async fn reply(&mut self, request: &Value, result: Value) {
        self.write(&json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }))
            .await;
    }

    /// Reads until an event `name` whose data `holds` arrived.
    #[expect(
        clippy::disallowed_methods,
        reason = "a real browser answers in real time, so the wait's deadline is real time (ADR 0060)"
    )]
    pub(crate) async fn until_event(&mut self, name: &str, holds: impl Fn(&Value) -> bool) {
        let until = Instant::now() + SETTLE_DEADLINE;
        while !self.events(name).iter().any(&holds) {
            let read = self.read(until).await;
            assert!(
                read.is_some(),
                "no `{name}` event as awaited within {SETTLE_DEADLINE:?}: {:?}",
                self.events
            );
        }
    }
}

impl Program {
    /// Whether the Proving ground received a request for `path` on `host`.
    pub(crate) fn requested(&self, host: &str, path: &str) -> bool {
        self.proving_ground
            .received_requests()
            .expect("the Proving ground answers")
            .of_the_case()
            .any(|request| request.host == host && request.target == path)
    }

    /// What the scripts of the page at `page` (its path and query) last
    /// reported to the Proving ground for `check`.
    pub(crate) fn reported(&self, page: &str, check: &str) -> Option<String> {
        self.proving_ground
            .observations()
            .expect("the Proving ground answers")["reports"]
            .as_array()
            .into_iter()
            .flatten()
            .rfind(|report| report["page"] == json!(page) && report["check"] == json!(check))
            .and_then(|report| report["observed"].as_str().map(str::to_owned))
    }

    /// The method of each CDP command the Run trace's command log holds so
    /// far, in order. The log is there from the Run's browser on, so a log
    /// that is not fails the read: an absence a test counts would otherwise
    /// hold whatever the engine did.
    #[expect(
        clippy::disallowed_methods,
        reason = "a small Run trace file read between calls, which only the engine's own actor appends to"
    )]
    pub(crate) fn cdp_commands(&self) -> Vec<String> {
        let file = HomeLayout::of(&self.home).run_trace(RUN, RUN_TRACE_CDP_COMMANDS);
        std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("the command log {} is read: {error}", file.display()))
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter_map(|line| line["method"].as_str().map(str::to_owned))
            .collect()
    }

    /// The lines of the Run trace's request log so far.
    #[expect(
        clippy::disallowed_methods,
        reason = "a small Run trace file read between calls, which only the engine's own actor appends to"
    )]
    pub(crate) fn request_log(&self) -> Vec<Value> {
        let file = HomeLayout::of(&self.home).run_trace(RUN, RUN_TRACE_REQUESTS);
        std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

/// Waits until `holds` is true, as the browser acts after a call answered.
pub(crate) async fn settled(holds: impl FnMut() -> bool) {
    settled_or(holds, String::new).await;
}

/// Waits until `holds` is true, failing with what `seen` says it saw.
#[expect(
    clippy::disallowed_methods,
    reason = "a real browser acts in real time, so the wait's deadline is real time (ADR 0060)"
)]
pub(crate) async fn settled_or(mut holds: impl FnMut() -> bool, seen: impl Fn() -> String) {
    let until = Instant::now() + SETTLE_DEADLINE;
    while !holds() {
        assert!(
            Instant::now() < until,
            "not settled within {SETTLE_DEADLINE:?}: {}",
            seen()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Runs `test` on a Connection to an in-process engine whose pages carry
/// `gate`, once its Run started with `settings` beside `network.resolve`,
/// which maps the Proving ground's hosts to it, and its first page opened;
/// the engine stops its Run, closing its browser, once `test` ends.
pub(crate) fn with_handler_page(
    gate: &RecordingGate,
    settings: Map<String, Value>,
    test: impl AsyncFnOnce(&mut Program),
) {
    with_handler_page_of(gate, |_| settings, test);
}

/// As [`with_handler_page`], with the settings `settings_of` the test's
/// Proving ground, such as one naming its proxies' addresses.
#[expect(
    clippy::disallowed_methods,
    reason = "the tests build their own runtime with real time, since a real browser answers in real time (ADR 0060)"
)]
pub(crate) fn with_handler_page_of(
    gate: &RecordingGate,
    settings_of: impl FnOnce(&ProvingGround) -> Map<String, Value>,
    test: impl AsyncFnOnce(&mut Program),
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, 720).expect("the Proving ground starts");
    let settings = settings_of(&proving_ground);
    let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_page::contribution_with(
            (pins(), Launcher::Executable),
            gate.clone(),
            Arc::new(NoBlocks),
        ));
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").expect("invariant: a literal version"),
        channel: Channel::CratesIo,
    };
    let engine = Engine::new(identity, &composition).expect("invariant: one handler each");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("invariant: the runtime starts");
    runtime.block_on(async {
        // The dispatch records go into the test's exercise record (ADR 0072).
        let clock = Clock::system();
        let _subscriber =
            tracing::subscriber::set_default(goodfetch_test_support::subscriber(move || {
                clock.wall()
            }));
        let (network, dialer) = transport::in_memory();
        let dependencies = dependencies(env.home(), &cache);
        let program = async {
            let (reader, writer) = dialer
                .connect()
                .await
                .expect("invariant: the engine listens")
                .split();
            let mut program = Program {
                lines: BufReader::new(reader).lines(),
                writer,
                next_id: 0,
                events: Vec::new(),
                answers: Map::new(),
                proving_ground,
                page: String::new(),
                home: env.home().to_owned(),
            };
            program
                .call(
                    HELLO,
                    &Hello::needing(Product::Collie)
                        .surface(Surface::Rust)
                        .params(),
                )
                .await;
            let resolve: Map<String, Value> = program
                .proving_ground
                .description()
                .resolve
                .iter()
                .map(|(host, address)| (host.clone(), json!([address])))
                .collect();
            let mut settings = settings;
            // The egress relay judges a mapping's answers as any other, so
            // the Run lists the mapped hosts in its private ranges.
            settings.insert(
                "egress".to_owned(),
                json!({ "private_ranges": resolve.keys().collect::<Vec<_>>() }),
            );
            // The test's own `network` settings keep theirs beside it.
            let network = settings
                .entry("network".to_owned())
                .or_insert_with(|| json!({}));
            if let Some(network) = network.as_object_mut() {
                network.insert("resolve".to_owned(), json!(resolve));
            }
            let started = program
                .call("run.start", &json!({ "run_id": RUN, "settings": settings }))
                .await;
            assert!(started.get("result").is_some(), "{started}");
            let opened = program
                .call("run.new_page", &json!({ "run_id": RUN }))
                .await;
            program.page = opened["result"]["page_id"]
                .as_str()
                .unwrap_or_else(|| panic!("no page opened: {opened}"))
                .to_owned();
            // A failed test still closes its Connection, so the engine
            // stops its Run and its browser before the panic goes on.
            let tested = Unwinding(Box::pin(test(&mut program))).await;
            program
                .writer
                .shutdown()
                .await
                .expect("invariant: in memory");
            drop(program);
            tested
        };
        let (served, tested) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            program
        );
        served.expect("invariant: the engine got its Connection");
        if let Err(panic) = tested {
            std::panic::resume_unwind(panic);
        }
    });
}

/// A future that answers its panic rather than unwinding through the
/// runtime.
struct Unwinding<F>(Pin<Box<F>>);

impl<F: Future> Future for Unwinding<F> {
    type Output = std::thread::Result<F::Output>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let future = self.0.as_mut();
        match std::panic::catch_unwind(AssertUnwindSafe(|| future.poll(context))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Err(panic) => Poll::Ready(Err(panic)),
        }
    }
}

/// What the engine runs on: the test's home, its Browser cache, the OS's
/// clock and threads.
#[expect(
    clippy::disallowed_methods,
    reason = "the engine's entropy is seeded from the test's own seed (ADR 0059)"
)]
fn dependencies(home: &Path, cache: &Path) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(home.to_owned()).with_browsers_path(Some(cache.to_owned())),
        clock: Clock::system(),
        entropy: Entropy::seeded(720),
        blocking: BlockingPolicy::Threads,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}
