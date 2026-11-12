//! The Stub gateway as a Client reaches it (ADR 0096): no file served, a
//! token presented in the Client's `hello`, `run.list` from its own index, each
//! run on an Attached engine of the executable its Product needs, one per
//! Run ID however many starts name it at once, and every other call
//! forwarded to the engine that owns the run, which takes the gateway's
//! `hello` as Goodfetch Cloud's.
#![expect(
    clippy::disallowed_types,
    reason = "a test opens its own WebSocket to the gateway, outside the engine (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "a synchronous test writes the gateway's token file and polls its ready file"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::TcpStream;
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_test_support::{Process, TestEnv, tools};
use serde_json::{Value, json};
use tungstenite::client::IntoClientRequest as _;
use tungstenite::{Message, WebSocket};

const GATEWAY: &str = env!("CARGO_BIN_EXE_goodfetch-stub-gateway");

/// The sign-in token the gateway is configured with.
pub(crate) const TOKEN: &str = "stub-gateway-sign-in";

/// This commit's Product version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long an interrupted gateway has to stop its engines and exit.
const STOP_DEADLINE: Duration = Duration::from_secs(10);

/// A started gateway and the address its ready file reports.
pub(crate) struct Gateway {
    process: Process,
    pub(crate) address: String,
}

impl Gateway {
    /// Starts the gateway on port 0 with this commit's executables, once
    /// it reports its address.
    pub(crate) fn start(env: &TestEnv) -> Self {
        Self::launch(env, true)
    }

    /// The same, given no Foxhound executable, so it runs Collie's engine
    /// alone.
    pub(crate) fn start_without_foxhound(env: &TestEnv) -> Self {
        Self::launch(env, false)
    }

    fn launch(env: &TestEnv, foxhound: bool) -> Self {
        let tools = tools().unwrap();
        let token_file = env.folder().join("token");
        std::fs::write(&token_file, format!("{TOKEN}\n")).unwrap();
        let ready = env.folder().join("ready.json");
        let mut command = env.process_command(GATEWAY);
        command
            .arg("--collie")
            .arg(tools.collie.unwrap())
            .arg("--token-file")
            .arg(&token_file)
            .arg("--state")
            .arg(env.folder().join("gateway"))
            .arg("--ready")
            .arg(&ready);
        if foxhound {
            command.arg("--foxhound").arg(tools.foxhound.unwrap());
        }
        let process = env.spawn(&mut command).unwrap();
        let reported = wait_for(&ready);
        Self {
            process,
            address: reported["address"].as_str().unwrap().to_owned(),
        }
    }

    /// A Client's WebSocket at `/protocol`.
    pub(crate) fn socket(&self) -> WebSocket<TcpStream> {
        let address = self.address.trim_start_matches("http://");
        let request = format!("ws://{address}/protocol")
            .into_client_request()
            .unwrap();
        let stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        tungstenite::client(request, stream).unwrap().0
    }

    /// The Connection of a program that needs `needs`, signed in, after
    /// its `hello`.
    fn connection(&self, needs: &str) -> Connection {
        let mut connection = Connection {
            socket: self.socket(),
            next: 0,
        };
        let hello = connection.call(
            "hello",
            &json!({
                "protocol_version": PROTOCOL_VERSION.to_string(),
                "surface": "rust",
                "token": TOKEN,
                "needs": needs,
                "product_version": VERSION,
            }),
        );
        assert_eq!(hello["result"]["product"], json!(needs), "{hello}");
        connection
    }
}

impl Drop for Gateway {
    /// Interrupts the gateway, which kills and reaps its engines before it
    /// exits, and reaps it: an engine shares the test's stderr, which
    /// nextest counts as leaked until every holder has closed it.
    fn drop(&mut self) {
        let _ = self.process.interrupt_and_wait(STOP_DEADLINE);
    }
}

/// A program's Connection to the gateway.
struct Connection {
    socket: WebSocket<TcpStream>,
    next: u64,
}

impl Connection {
    /// The reply to a request of `method` with `params`, skipping
    /// notifications.
    fn call(&mut self, method: &str, params: &Value) -> Value {
        let id = self.send(method, params);
        self.reply(id)
    }

    /// Sends a request of `method` with `params`, answering its ID.
    fn send(&mut self, method: &str, params: &Value) -> u64 {
        self.next += 1;
        let id = self.next;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.socket
            .send(Message::text(request.to_string()))
            .unwrap();
        id
    }

    /// The reply to the request `id`, skipping notifications.
    fn reply(&mut self, id: u64) -> Value {
        let [reply] = self.replies([id]);
        reply
    }

    /// The replies to the requests `ids`, in that order, in whichever
    /// order they arrive, skipping notifications.
    fn replies<const N: usize>(&mut self, ids: [u64; N]) -> [Value; N] {
        let mut replies = [const { None }; N];
        while replies.iter().any(Option::is_none) {
            let Message::Text(text) = self.socket.read().unwrap() else {
                continue;
            };
            let reply: Value = serde_json::from_str(&text).unwrap();
            if let Some(slot) = ids.iter().position(|id| reply["id"] == json!(id)) {
                replies[slot] = Some(reply);
            }
        }
        replies.map(|reply| reply.expect("every reply was read"))
    }

    /// The Run ID of a run the program starts with `method` and `params`.
    fn start(&mut self, method: &str, params: &Value) -> String {
        let started = self.call(method, params);
        started["result"]["run_id"]
            .as_str()
            .unwrap_or_else(|| panic!("{method} failed: {started}"))
            .to_owned()
    }
}

/// A crawl's start whose one seed the Frontier does not add, so the crawl
/// fetches nothing and runs no handler.
fn crawl_of_nothing() -> Value {
    json!({"seeds": [{"url": "ftp://example.invalid/", "label": "start"}]})
}

/// The JSON the gateway writes at `ready` once it serves.
fn wait_for(ready: &Path) -> Value {
    for _ in 0..600 {
        if let Ok(text) = std::fs::read_to_string(ready)
            && let Ok(value) = serde_json::from_str(&text)
        {
            return value;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("the gateway wrote no ready file at {}", ready.display());
}

/// Covers: story:12-232
#[test]
fn serves_no_file() {
    let env = TestEnv::new();
    let gateway = Gateway::start(&env);

    let statuses = ["/", "/index.html", "/runs", "/sign-in", "/session"].map(|path| {
        ureq::get(format!("{}{path}", gateway.address))
            .config()
            .http_status_as_error(false)
            .build()
            .call()
            .unwrap()
            .status()
            .as_u16()
    });

    assert_eq!(statuses, [404; 5]);
}

/// Covers: story:12-232, error:not_in_product
#[test]
fn a_collie_programs_run_starts_on_collies_engine() {
    let env = TestEnv::new();
    let gateway = Gateway::start(&env);
    let mut program = gateway.connection("collie");
    let run_id = program.start("run.start", &json!({}));

    let refused = program.call("run.seeds", &json!({"run_id": run_id}));

    assert_eq!(
        (
            &refused["error"]["data"]["code"],
            &refused["error"]["data"]["product"],
            &refused["error"]["data"]["needs"],
        ),
        (
            &json!("not_in_product"),
            &json!("collie"),
            &json!("foxhound")
        ),
        "{refused}"
    );
}

/// Foxhound's engine refuses a crawl key on a Run that is not a crawl with
/// `setting_not_applicable`, where Collie's would answer `not_in_product`.
///
/// Covers: story:12-232, error:setting_not_applicable
#[test]
fn a_foxhound_programs_run_setting_a_crawl_key_starts_on_foxhounds_engine() {
    let env = TestEnv::new();
    let gateway = Gateway::start(&env);
    let mut program = gateway.connection("foxhound");

    let refused = program.call(
        "run.start",
        &json!({"settings": {"crawl": {"max_requests": 10}}}),
    );

    assert_eq!(
        refused["error"]["data"]["code"],
        json!("setting_not_applicable"),
        "{refused}"
    );
}

/// Covers: story:12-232
#[test]
fn a_crawl_a_program_starts_runs_on_foxhounds_engine_which_answers_its_calls() {
    let env = TestEnv::new();
    let gateway = Gateway::start(&env);
    let mut program = gateway.connection("foxhound");
    let run_id = program.start("crawl.start", &crawl_of_nothing());

    let answered = program.call("run.seeds", &json!({"run_id": run_id}));

    assert!(answered.get("result").is_some(), "{answered}");
}

/// Covers: story:12-232
#[test]
fn run_list_answers_every_run_its_index_holds_on_either_engine() {
    let env = TestEnv::new();
    let gateway = Gateway::start(&env);
    let on_collie = gateway.connection("collie").start("run.start", &json!({}));
    let mut program = gateway.connection("foxhound");
    let on_foxhound = program.start("crawl.start", &crawl_of_nothing());

    let listed = program.call("run.list", &json!({}));

    let mut run_ids: Vec<&str> = listed["result"]["runs"]
        .as_array()
        .unwrap_or_else(|| panic!("run.list failed: {listed}"))
        .iter()
        .map(|row| row["run_id"].as_str().unwrap())
        .collect();
    run_ids.sort_unstable();
    let mut expected = vec![on_collie.as_str(), on_foxhound.as_str()];
    expected.sort_unstable();
    assert_eq!(run_ids, expected);
}

/// A program's `hello`, presenting `token`.
fn programs_hello(token: &str) -> Message {
    Message::text(
        json!({"jsonrpc": "2.0", "id": 1, "method": "hello", "params": {
            "protocol_version": PROTOCOL_VERSION.to_string(),
            "surface": "rust",
            "token": token,
            "needs": "collie",
            "product_version": VERSION,
        }})
        .to_string(),
    )
}

/// A program signs in with the token its `hello` presents (spec #586's
/// duty 8): one that presents another is closed unanswered, as an engine
/// closes it.
#[rstest::rstest]
#[case::the_configured_token(TOKEN, true)]
#[case::another_token("another-token", false)]
fn a_program_signs_in_with_the_token_its_hello_presents(
    #[case] token: &str,
    #[case] answered: bool,
) {
    let env = TestEnv::new();
    let gateway = Gateway::start(&env);
    let mut socket = gateway.socket();
    socket.send(programs_hello(token)).unwrap();

    let reply = match socket.read() {
        Ok(Message::Text(text)) => Some(serde_json::from_str::<Value>(&text).unwrap()),
        _ => None,
    };

    assert_eq!(
        reply.is_some_and(|reply| reply["result"]["product"] == json!("collie")),
        answered
    );
}

/// `run.start` is an idempotent write: two starts of one new Run ID sent at
/// once reach one engine, which starts the run once and answers the other
/// with the run it already has.
///
/// Covers: story:12-232
#[test]
fn two_starts_of_one_new_run_id_at_once_start_one_run() {
    let env = TestEnv::new();
    let gateway = Gateway::start(&env);
    let mut program = gateway.connection("collie");
    let params = json!({"run_id": "a-run-started-twice"});
    let (first, second) = (
        program.send("run.start", &params),
        program.send("run.start", &params),
    );

    let mut started_as = program.replies([first, second]).map(|reply| {
        reply["result"]["started_as"]
            .as_str()
            .unwrap_or_else(|| panic!("the start failed: {reply}"))
            .to_owned()
    });

    started_as.sort_unstable();
    assert_eq!(started_as, ["existing", "new"]);
}

/// Interrupted, the gateway kills each engine it started and exits only
/// once it has reaped them, so no engine outlives it holding the test's
/// stderr.
#[cfg(target_os = "linux")]
#[test]
fn an_interrupted_gateway_exits_once_it_has_reaped_its_engines() {
    let env = TestEnv::new();
    let gateway = Gateway::start(&env);
    let mut on_collie = gateway.connection("collie");
    on_collie.start("run.start", &json!({}));
    let mut on_foxhound = gateway.connection("foxhound");
    on_foxhound.start("crawl.start", &crawl_of_nothing());
    let engines = children_of(gateway.process.child().id());

    let status = gateway.process.interrupt_and_wait(STOP_DEADLINE).unwrap();

    let left: Vec<u32> = engines
        .iter()
        .copied()
        .filter(|engine| Path::new(&format!("/proc/{engine}")).exists())
        .collect();
    assert_eq!(
        (engines.len(), status.success(), left),
        (2, true, Vec::new())
    );
}

/// The processes whose parent is `pid`, from each process's `stat`.
#[cfg(target_os = "linux")]
fn children_of(pid: u32) -> Vec<u32> {
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|entry| {
            let child: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
            let stat = std::fs::read_to_string(format!("/proc/{child}/stat")).ok()?;
            // The parent's ID is the second field after the command's
            // closing parenthesis.
            let parent: u32 = stat
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .nth(1)?
                .parse()
                .ok()?;
            (parent == pid).then_some(child)
        })
        .collect()
}
