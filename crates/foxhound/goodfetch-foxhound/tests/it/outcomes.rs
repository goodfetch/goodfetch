//! A reader of a crawl's outcomes that stalls never slows the crawl (ADRs
//! 0102 and 0103), on Foxhound's Daemon (spec #213's Seam 1): one
//! Connection drives a crawl of the Proving ground's `scale.test` items,
//! its handler fetching each page, while another Connection asks for the
//! crawl's outcomes again and again and never reads an answer. The crawl
//! still fetches and commits every page, and the reader, once it reads,
//! finds every answer it asked for, each a prefix of the commit order.
//! No test asserts elapsed time: the throughput shown is that every page
//! went through while the reader held its answers unread.
#![expect(
    clippy::disallowed_types,
    reason = "a test's own Clients reach the Daemon's socket and the Proving ground from outside the engine (ADR 0060)"
)]

use std::io::{BufRead as _, BufReader, Read, Write};
use std::path::Path;

use goodfetch_test_support::{DAEMON_DEADLINE, Daemon, ProvingGround, TestEnv};
use serde_json::{Value, json};

const FOXHOUND: &str = env!("CARGO_BIN_EXE_goodfetch-foxhound");

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The Proving ground's seed.
const SEED: u64 = 1545;

/// How many `scale.test` items the crawl fetches: more than the handler
/// concurrency, so the crawl leases its work items again and again while
/// the reader stalls.
const PAGES: u64 = 200;

/// How many reads the stalled reader sends: their answers, each up to 200
/// outcomes, fill its socket many times over.
const STALLED_READS: usize = 64;

/// One Client's Connection to the Daemon.
pub(crate) struct Connection {
    reader: BufReader<Box<dyn Read>>,
    writer: Box<dyn Write>,
}

impl Connection {
    pub(crate) fn open(daemon: &Daemon) -> Self {
        let description = daemon.description();
        let (reader, writer): (Box<dyn Read>, Box<dyn Write>) =
            if cfg!(windows) || !Path::new(&description.socket).is_absolute() {
                let stream = std::net::TcpStream::connect(&description.socket).unwrap();
                (Box::new(stream.try_clone().unwrap()), Box::new(stream))
            } else {
                unix_socket(&description.socket)
            };
        let mut connection = Self {
            reader: BufReader::new(reader),
            writer,
        };
        connection.send(&json!({
            "jsonrpc": "2.0", "id": "hello", "method": "hello",
            "params": {
                "protocol_version": "0.1", "surface": "sdk_python", "token": description.token,
                "needs": "foxhound", "product_version": VERSION,
            },
        }));
        let hello = connection.next();
        assert_eq!(hello["result"]["product"], "foxhound", "{hello}");
        connection
    }

    pub(crate) fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        self.writer.write_all(&line).unwrap();
        self.writer.flush().unwrap();
    }

    pub(crate) fn next(&mut self) -> Value {
        let mut line = String::new();
        assert_ne!(
            self.reader.read_line(&mut line).unwrap(),
            0,
            "the Daemon closed"
        );
        serde_json::from_str(&line).unwrap()
    }

    /// Sends the call `id` and answers each handler request with `handler`
    /// until its reply comes.
    pub(crate) fn call(
        &mut self,
        id: &str,
        method: &str,
        params: &Value,
        handler: &mut impl FnMut(&Value) -> Value,
    ) -> Value {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.next();
            if message["method"] == "crawl_request.handle" {
                let mut reply = handler(&message["params"]);
                reply["jsonrpc"] = json!("2.0");
                reply["id"] = message["id"].clone();
                self.send(&reply);
            } else if message["id"] == id {
                return message;
            }
        }
    }
}

#[cfg(unix)]
fn unix_socket(path: &str) -> (Box<dyn Read>, Box<dyn Write>) {
    let stream = std::os::unix::net::UnixStream::connect(path).unwrap();
    (Box::new(stream.try_clone().unwrap()), Box::new(stream))
}

#[cfg(not(unix))]
fn unix_socket(_: &str) -> (Box<dyn Read>, Box<dyn Write>) {
    unreachable!("a Daemon listens on loopback TCP here")
}

/// Fetches `url`, a page of the Proving ground, as the crawl's handler
/// does, and answers its status line.
fn fetch(proving_ground: &ProvingGround, url: &str) -> String {
    let authority_and_path = url.strip_prefix("http://").unwrap();
    let (authority, path) = authority_and_path.split_at(authority_and_path.find('/').unwrap());
    let mut stream = std::net::TcpStream::connect(proving_ground.address()).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response.lines().next().unwrap_or_default().to_owned()
}

/// Covers: story:3-149, operation:run.outcomes
#[test]
fn a_reader_that_stalls_does_not_hold_back_a_crawl_of_the_proving_ground() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let daemon = Daemon::start_program(&env, Path::new(FOXHOUND), DAEMON_DEADLINE).unwrap();
    let mut crawler = Connection::open(&daemon);
    let mut stalled = Connection::open(&daemon);
    let seeds: Vec<Value> = (1..=PAGES)
        .map(|item| {
            json!({ "url": proving_ground.url("scale.test", &format!("/items/{item}")), "label": "item" })
        })
        .collect();
    let mut handler = |params: &Value| {
        let url = params["crawl_request"]["url"].as_str().unwrap();
        let status = fetch(&proving_ground, url);
        json!({ "result": { "records": [{ "dataset": "items", "data": { "url": url, "status": status } }] } })
    };
    crawler.call(
        "start",
        "crawl.start",
        &json!({ "run_id": "scale", "seeds": seeds, "datasets": [{ "name": "items" }] }),
        &mut handler,
    );
    for read in 0..STALLED_READS {
        stalled.send(&json!({
            "jsonrpc": "2.0", "id": read, "method": "run.outcomes", "params": { "run_id": "scale" },
        }));
    }

    let report = crawler.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "scale" }),
        &mut handler,
    );
    let fetched = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request["host"] == "scale.test")
        .count();
    let mut prefixes = 0;
    for _ in 0..STALLED_READS {
        let answer = stalled.next();
        let positions: Vec<u64> = answer["result"]["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|outcome| outcome["position"].as_u64().unwrap())
            .collect();
        let count = u64::try_from(positions.len()).unwrap();
        if count > 0 && positions == (1..=count).collect::<Vec<_>>() {
            prefixes += 1;
        }
    }
    let read = stalled.call(
        "all",
        "run.outcomes",
        &json!({ "run_id": "scale", "kinds": ["succeeded"] }),
        &mut |_| json!({ "result": {} }),
    );
    drop((crawler, stalled));
    daemon.stop(&env).unwrap();

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            fetched,
            prefixes,
            read["result"]["outcomes"].as_array().unwrap().len(),
            &read["result"]["ended"],
        ),
        (
            &json!("finished"),
            usize::try_from(PAGES).unwrap(),
            STALLED_READS,
            usize::try_from(PAGES).unwrap(),
            &json!(true),
        )
    );
}
