//! Browser host conformance and lifecycle tests (ADRs 0022, 0095, 0096, 0129
//! and 0132; spec #577, issue 753).
//!
//! Proves:
//! - Page API conformance cases run through `browser-host` on loopback by
//!   configuring `browser.host` alone (AC1, AC3).
//! - The same program runs locally and through `browser-host` with only
//!   configuration changed; `--listen 127.0.0.1:0 --token-file <path>`
//!   reports its bound port, serves one Session, wipes its ephemeral profile
//!   after it ends, and exits 0; non-loopback without TLS is refused for both
//!   `--listen` and `--connect` (AC3).
//! - `hello` from either Product (`collie` or `foxhound`) is accepted as
//!   Collie, and another protocol major is refused in the reply to `hello`
//!   before any other message (AC4).
//! - Both executables (`goodfetch-collie` and `goodfetch-foxhound`) carry
//!   `browser-host` (AC5).
//! - A Browser host Session refuses `egress.local_addresses` with
//!   `local_address_unavailable` (issue 638).
#![expect(
    clippy::disallowed_types,
    reason = "these tests spawn Browser host processes and loopback TCP forwarders outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use goodfetch_conformance::{
    EngineMode, EngineTarget, ProtocolSuite, ProvingGround, run_protocol_suite,
};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::cli::BROWSER_HOST;
use goodfetch_protocol::page::browser_host::BrowserHostReady;
use goodfetch_protocol::result_document::{CapabilityResult, CaseOutcome, Outcome};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, ProvingGround as LocalCopy, TestEnv, Tool, browser_cache, tools,
};
use serde_json::{Value, json};

/// Visibly fake non-secret test token written to `--token-file` in tests.
const TEST_TOKEN: &str = "browser-host-test-token-not-a-secret";

/// Spawns a real `<executable> browser-host --listen 127.0.0.1:0 --token-file <path>`
/// process and reads its [`BrowserHostReady`] JSON line from stdout.
struct SpawnedBrowserHost {
    child: Child,
    ready: BrowserHostReady,
    home: PathBuf,
}

impl SpawnedBrowserHost {
    fn start(executable: &Path, env: &TestEnv, browsers: &Path, token_path: &Path) -> Self {
        let mut child = Command::new(executable)
            .env_clear()
            .envs(env.env())
            .env(BROWSERS_PATH_VARIABLE, browsers)
            .args([BROWSER_HOST, "--listen", "127.0.0.1:0", "--token-file"])
            .arg(token_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let read = reader.read_line(&mut line).unwrap();
        assert!(
            read > 0,
            "browser-host exited before printing BrowserHostReady"
        );
        let ready: BrowserHostReady = serde_json::from_str(line.trim()).unwrap_or_else(|error| {
            panic!("invalid BrowserHostReady `{line}`: {error}");
        });
        // Keep draining stdout in background so the child never blocks on write.
        thread::spawn(move || {
            let mut sink = String::new();
            let _ = reader.read_to_string(&mut sink);
        });
        Self {
            child,
            ready,
            home: env.home().to_path_buf(),
        }
    }

    fn wait_exit(mut self) -> std::process::ExitStatus {
        self.child.wait().unwrap()
    }
}

impl Drop for SpawnedBrowserHost {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Loopback TCP front-end that spawns a fresh `goodfetch-collie browser-host --listen 127.0.0.1:0`
/// for each Session connection from the engine, mirroring how an orchestrator
/// hands each Session an ephemeral Browser host pod without adding any
/// test-only mode to `browser-host`.
struct PerSessionBrowserHost {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl PerSessionBrowserHost {
    fn start(
        executable: PathBuf,
        host_env: std::collections::BTreeMap<String, String>,
        browsers: PathBuf,
        token_path: PathBuf,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut workers = Vec::new();
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((client, _)) => {
                        let exe = executable.clone();
                        let env = host_env.clone();
                        let browsers = browsers.clone();
                        let token = token_path.clone();
                        workers.push(thread::spawn(move || {
                            proxy_one_session(client, &exe, &env, &browsers, &token);
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
            for worker in workers {
                let _ = worker.join();
            }
        });
        Self {
            address,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for PerSessionBrowserHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn proxy_one_session(
    mut client: TcpStream,
    executable: &Path,
    host_env: &std::collections::BTreeMap<String, String>,
    browsers: &Path,
    token_path: &Path,
) {
    let Ok(mut child) = Command::new(executable)
        .env_clear()
        .envs(host_env)
        .env(BROWSERS_PATH_VARIABLE, browsers)
        .args([BROWSER_HOST, "--listen", "127.0.0.1:0", "--token-file"])
        .arg(token_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    let Ok(ready) = serde_json::from_str::<BrowserHostReady>(line.trim()) else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    let Ok(mut backend) = TcpStream::connect(&ready.address) else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    let _ = client.set_nonblocking(false);
    let mut client_r = client.try_clone().unwrap();
    let mut backend_w = backend.try_clone().unwrap();
    let c2b = thread::spawn(move || {
        let _ = std::io::copy(&mut client_r, &mut backend_w);
        let _ = backend_w.shutdown(Shutdown::Write);
    });
    let _ = std::io::copy(&mut backend, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = c2b.join();
    let _ = child.wait();
}

/// Drives a short JSON-RPC session against an Attached engine executable and
/// returns the title/text evaluated on the Proving ground's home page.
fn run_same_program(env: &TestEnv, browsers: &Path, page_url: &str) -> Value {
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_path_buf();
    let mut child = Command::new(&collie)
        .env_clear()
        .envs(env.env())
        .env(BROWSERS_PATH_VARIABLE, browsers)
        .arg(goodfetch_protocol::cli::ATTACHED_ENGINE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let mut rpc = |msg: Value| -> Value {
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            let read = stdout.read_line(&mut line).unwrap();
            assert!(read > 0, "engine closed stdout before replying to {msg}");
            let parsed: Value = serde_json::from_str(line.trim()).unwrap();
            if parsed.get("id").is_some() {
                return parsed;
            }
        }
    };

    let hello = rpc(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "hello",
        "params": {
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "surface": "rust",
            "needs": "collie",
            "product_version": "0.0.0",
        }
    }));
    assert!(hello.get("result").is_some(), "{hello}");

    let started = rpc(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "run.start",
        "params": { "run_id": "same-program" }
    }));
    assert!(started.get("result").is_some(), "{started}");

    let session = rpc(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "run.new_session",
        "params": { "run_id": "same-program" }
    }));
    let session_id = session["result"]["session_id"]
        .as_str()
        .unwrap_or_else(|| panic!("expected session_id: {session}"))
        .to_owned();

    let page = rpc(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "session.new_page",
        "params": { "session_id": session_id }
    }));
    let page_id = page["result"]["page_id"]
        .as_str()
        .unwrap_or_else(|| panic!("expected page_id: {page}"))
        .to_owned();

    let navigated = rpc(json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "page.goto",
        "params": { "page_id": page_id, "url": page_url }
    }));
    assert!(navigated.get("result").is_some(), "{navigated}");

    let evaluated = rpc(json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "page.evaluate",
        "params": {
            "page_id": page_id,
            "function": "() => ({ title: document.title, href: location.href })"
        }
    }));
    assert!(evaluated.get("result").is_some(), "{evaluated}");

    let closed = rpc(json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "session.close",
        "params": { "session_id": session_id }
    }));
    assert!(closed.get("result").is_some(), "{closed}");

    let finished = rpc(json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "run.finish",
        "params": { "run_id": "same-program" }
    }));
    assert!(finished.get("result").is_some(), "{finished}");

    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "engine exited with {status}");
    evaluated["result"]["value"].clone()
}

/// Sends one JSON-RPC line over a TCP stream and reads back the matching
/// JSON-RPC reply line (skipping any intervening server notifications such as
/// `$/run_trace`).
fn tcp_exchange(stream: &mut TcpStream, message: &Value) -> Option<Value> {
    writeln!(stream, "{message}").unwrap();
    stream.flush().unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).unwrap();
        if read == 0 {
            return None;
        }
        let parsed: Value = serde_json::from_str(line.trim()).unwrap();
        if parsed.get("id").is_some() {
            return Some(parsed);
        }
    }
}

/// AC1 + AC3: Page API conformance cases run through `browser-host` on loopback
/// when only `browser.host` is configured in user configuration.
///
/// Covers: story:5-108, setting:browser.host, command:browser-host
#[test]
fn a_session_driven_through_browser_host_passes_page_api_conformance_cases() {
    let env = TestEnv::new();
    let host_env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let browsers = browser_cache(&env).unwrap();
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_path_buf();

    let token_path = env.folder().join("browser-host.token");
    std::fs::write(&token_path, TEST_TOKEN).unwrap();
    let forwarder = PerSessionBrowserHost::start(
        collie.clone(),
        host_env.env().clone(),
        browsers.clone(),
        token_path.clone(),
    );

    env.user_configuration(&format!(
        "browser.host = {{ address = \"{}\", token_file = \"{}\" }}\n",
        forwarder.address,
        token_path.display()
    ));

    let mut environment = env.env().clone();
    environment.insert(
        BROWSERS_PATH_VARIABLE.to_owned(),
        browsers.display().to_string(),
    );
    let target = EngineTarget {
        executable: collie,
        mode: EngineMode::Attached,
        environment,
        folder: env.folder().to_path_buf(),
    };

    // Representative Page API capabilities spanning default/named Sessions,
    // locators & guard, routes (reverse `route.handle`), dialogs (reverse
    // `dialog.handle`), popups (`session.on` streaming), page functions, read,
    // waits, formats, and events.
    let capabilities = [
        "sessions.default",
        "formats",
        "locators.guard",
        "routes",
        "dialogs",
        "popups",
        "read",
        "waits",
    ];
    let suite = ProtocolSuite::page()
        .with_capabilities(&capabilities)
        .unwrap();

    let document = run_protocol_suite(&suite, &target, &proving_ground).unwrap();
    crate::keep(&document, "page-collie-browser-host");

    let cases: Vec<(&str, CaseOutcome)> = document
        .capabilities()
        .iter()
        .flat_map(CapabilityResult::cases)
        .map(|case| (case.facts().id.as_str(), case.outcome()))
        .collect();
    assert_eq!(
        document
            .capabilities()
            .iter()
            .map(|capability| (capability.name(), capability.outcome()))
            .collect::<Vec<_>>(),
        capabilities
            .iter()
            .map(|capability| (*capability, Outcome::Proved))
            .collect::<Vec<_>>(),
        "{cases:?} {:?}",
        document
            .capabilities()
            .iter()
            .flat_map(CapabilityResult::cases)
            .flat_map(|case| case.facts().entries.iter().flat_map(|entry| entry
                .checks
                .iter()
                .filter(|check| !check.passed)
                .map(|check| check.name.clone())))
            .collect::<Vec<_>>()
    );
}

/// AC3: The same program runs locally and through a single `--listen 127.0.0.1:0`
/// Browser host with only configuration changed; the Browser host reports its
/// bound port, serves one Session, wipes its ephemeral profile folder after it
/// ends, and exits 0.
///
/// Covers: story:5-108, setting:browser.host, command:browser-host
#[test]
fn the_same_program_runs_locally_and_through_browser_host_and_wipes_its_profile() {
    let local_env = TestEnv::new();
    let remote_env = TestEnv::new();
    let host_env = TestEnv::new();
    let copy = LocalCopy::start(&local_env, local_env.seed()).unwrap();
    let url = format!("http://{}/", copy.address());
    let browsers = browser_cache(&local_env).unwrap();
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_path_buf();

    local_env.user_configuration("egress.private_ranges = [\"127.0.0.0/8\"]\n");
    let local_result = run_same_program(&local_env, &browsers, &url);

    let token_path = host_env.folder().join("host.token");
    std::fs::write(&token_path, TEST_TOKEN).unwrap();
    let host = SpawnedBrowserHost::start(&collie, &host_env, &browsers, &token_path);
    let bound: SocketAddr = host.ready.address.parse().unwrap();
    assert!(
        bound.port() > 0,
        "expected ephemeral bound port: {:?}",
        host.ready
    );
    assert_eq!(host.ready.product, Product::Collie);

    remote_env.user_configuration(&format!(
        "egress.private_ranges = [\"127.0.0.0/8\"]\nbrowser.host = {{ address = \"{}\", token_file = \"{}\" }}\n",
        host.ready.address,
        token_path.display()
    ));

    let remote_result = run_same_program(&remote_env, &browsers, &url);
    assert_eq!(remote_result, local_result);

    let host_home = host.home.clone();
    let exit = host.wait_exit();
    assert!(exit.success(), "browser-host exited with {exit}");

    // Ephemeral profile folder wiped; no Run index entries written in host_home.
    let leftovers: Vec<_> = std::fs::read_dir(&host_home)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("browser-host-"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "ephemeral profile folders were not wiped: {leftovers:?}"
    );
}

/// AC3: A listening Browser host serves at most one Session and refuses a
/// second `run.new_session` call.
///
/// Covers: story:5-108, command:browser-host
#[test]
fn a_listening_browser_host_serves_one_session_and_refuses_a_second() {
    let host_env = TestEnv::new();
    let browsers = browser_cache(&host_env).unwrap();
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_path_buf();
    let token_path = host_env.folder().join("single.token");
    std::fs::write(&token_path, TEST_TOKEN).unwrap();

    let host = SpawnedBrowserHost::start(&collie, &host_env, &browsers, &token_path);
    let mut stream = TcpStream::connect(&host.ready.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();

    let hello = tcp_exchange(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "hello",
            "params": {
                "protocol_version": PROTOCOL_VERSION.to_string(),
                "surface": "rust",
                "needs": "collie",
                "product_version": "0.0.0",
                "token": TEST_TOKEN,
            }
        }),
    )
    .unwrap();
    assert!(hello.get("result").is_some(), "{hello}");

    let started = tcp_exchange(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "run.start",
            "params": { "run_id": "one-session" }
        }),
    )
    .unwrap();
    assert!(started.get("result").is_some(), "{started}");

    let first = tcp_exchange(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "run.new_session",
            "params": { "run_id": "one-session" }
        }),
    )
    .unwrap();
    assert!(first.get("result").is_some(), "{first}");

    let second = tcp_exchange(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "run.new_session",
            "params": { "run_id": "one-session" }
        }),
    )
    .unwrap();
    assert_eq!(second["error"]["data"]["code"], json!("closed"), "{second}");

    drop(stream);
    assert!(host.wait_exit().success());
}

/// AC3: A non-loopback address without TLS is refused for both `--listen` and
/// `--connect`.
///
/// Covers: story:5-108, command:browser-host
#[test]
fn browser_host_refuses_non_loopback_address_without_tls() {
    let env = TestEnv::new();
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_path_buf();
    let token_path = env.folder().join("refused.token");
    std::fs::write(&token_path, TEST_TOKEN).unwrap();

    for args in [
        vec![BROWSER_HOST, "--listen", "192.0.2.1:9444", "--token-file"],
        vec![
            BROWSER_HOST,
            "--connect",
            "ws://192.0.2.1:9444",
            "--token-file",
        ],
    ] {
        let output = Command::new(&collie)
            .env_clear()
            .envs(env.env())
            .args(&args)
            .arg(&token_path)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "expected refusal for {args:?}, got success"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("non-loopback") || stderr.contains("TLS"),
            "expected TLS/non-loopback message for {args:?}, got: {stderr}"
        );
    }
}

/// AC4: A `hello` from an engine of either Product (`collie` or `foxhound`) is
/// accepted as Collie; another protocol major is refused in the reply to
/// `hello` before any other message.
///
/// Covers: story:5-108, command:browser-host
#[test]
fn browser_host_accepts_hello_from_either_product_as_collie_and_refuses_another_major() {
    let env = TestEnv::new();
    let browsers = browser_cache(&env).unwrap();
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_path_buf();
    let token_path = env.folder().join("hello.token");
    std::fs::write(&token_path, TEST_TOKEN).unwrap();

    for needed_product in ["collie", "foxhound"] {
        let host = SpawnedBrowserHost::start(&collie, &env, &browsers, &token_path);
        let mut stream = TcpStream::connect(&host.ready.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let reply = tcp_exchange(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "hello",
                "params": {
                    "protocol_version": PROTOCOL_VERSION.to_string(),
                    "surface": "rust",
                    "needs": needed_product,
                    "product_version": "0.0.0",
                    "token": TEST_TOKEN,
                }
            }),
        )
        .unwrap();
        assert_eq!(
            reply["result"]["product"],
            json!("collie"),
            "expected Collie reply for needs={needed_product}: {reply}"
        );
        drop(stream);
        assert!(host.wait_exit().success());
    }

    // Another protocol major is refused in the reply to `hello` before any other message.
    let host = SpawnedBrowserHost::start(&collie, &env, &browsers, &token_path);
    let mut stream = TcpStream::connect(&host.ready.address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let wrong_major = format!("{}.0", PROTOCOL_VERSION.major + 1);
    let reply = tcp_exchange(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "hello",
            "params": {
                "protocol_version": wrong_major,
                "surface": "rust",
                "needs": "collie",
                "product_version": "0.0.0",
                "token": TEST_TOKEN,
            }
        }),
    )
    .unwrap();
    assert_eq!(
        reply["error"]["data"]["code"],
        json!("protocol_version_unsupported"),
        "{reply}"
    );
    // Connection closes after refusing `hello`.
    let mut trailing = String::new();
    let mut reader = BufReader::new(stream);
    let after = reader.read_line(&mut trailing).unwrap_or(0);
    assert_eq!(
        after, 0,
        "expected connection close after major refusal, got `{trailing}`"
    );
    drop(host);
}

/// AC5: Both executables (`goodfetch-collie` and `goodfetch-foxhound`) carry
/// the `browser-host` command.
///
/// Covers: story:5-108, command:browser-host
#[test]
fn both_executables_carry_browser_host_command() {
    let env = TestEnv::new();
    for tool in [Tool::Collie, Tool::Foxhound] {
        let executable = tools().unwrap().path(tool).unwrap().to_path_buf();
        let output = Command::new(&executable)
            .env_clear()
            .envs(env.env())
            .args([BROWSER_HOST, "--help"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{tool:?} browser-host --help failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        for flag in ["--listen", "--connect", "--token-file"] {
            assert!(
                stdout.contains(flag),
                "{tool:?} browser-host --help missing `{flag}`: {stdout}"
            );
        }
    }
}

/// Issue 638 soft dependency: A Session with `egress.local_addresses` on a
/// Browser host is refused with `local_address_unavailable`.
///
/// Covers: story:5-108, setting:browser.host, setting:egress.local_addresses, error:local_address_unavailable
#[test]
fn browser_host_session_refuses_egress_local_addresses() {
    let env = TestEnv::new();
    let browsers = browser_cache(&env).unwrap();
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_path_buf();
    let token_path = env.folder().join("local-addr.token");
    std::fs::write(&token_path, TEST_TOKEN).unwrap();

    env.user_configuration(&format!(
        "browser.host = {{ address = \"127.0.0.1:9444\", token_file = \"{}\" }}\n",
        token_path.display()
    ));

    let mut child = Command::new(&collie)
        .env_clear()
        .envs(env.env())
        .env(BROWSERS_PATH_VARIABLE, &browsers)
        .arg(goodfetch_protocol::cli::ATTACHED_ENGINE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let mut rpc = |msg: Value| -> Value {
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            let read = stdout.read_line(&mut line).unwrap();
            assert!(read > 0, "engine closed stdout before replying to {msg}");
            let parsed: Value = serde_json::from_str(line.trim()).unwrap();
            if parsed.get("id").is_some() {
                return parsed;
            }
        }
    };

    let hello = rpc(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "hello",
        "params": {
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "surface": "rust",
            "needs": "collie",
            "product_version": "0.0.0",
        }
    }));
    assert!(hello.get("result").is_some(), "{hello}");

    let started = rpc(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "run.start",
        "params": {
            "run_id": "local-addr-refused",
            "settings": {
                "egress": {
                    "local_addresses": ["127.0.0.1"]
                }
            }
        }
    }));
    assert!(started.get("result").is_some(), "{started}");

    let opened = rpc(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "run.new_session",
        "params": { "run_id": "local-addr-refused" }
    }));
    assert_eq!(
        opened["error"]["data"]["code"],
        json!("local_address_unavailable"),
        "{opened}"
    );

    drop(stdin);
    let _ = child.wait();
}
