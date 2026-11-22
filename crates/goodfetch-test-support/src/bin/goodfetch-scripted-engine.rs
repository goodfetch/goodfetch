//! `goodfetch-scripted-engine` (ADR 0069): the Scripted engine as a
//! process, for the faults neither Product's executable can produce on
//! demand (a refused `hello`, a crash mid-call, garbage on stdout, a dead
//! PID in the runtime file, a dropped connection). It plays one script of
//! `test-support/fixtures/engine-scripts/` to one Client, records every
//! message it saw, and exits non-zero, with the failure's code and reason
//! on stderr, on a message no step expects or a step left unused.
//!
//! `--script <file> --mode <mode>` names the script, and the mode how the
//! Client reaches it:
//! - `stdio`: over its own stdin and stdout, as an Attached engine;
//! - `socket`: as a Daemon, on a Unix socket on macOS and Linux and on
//!   loopback TCP on Windows (ADR 0024), found through the Daemon's runtime
//!   file it writes into the per-major folder of `GOODFETCH_HOME`, holding
//!   the endpoint, `--token`, `--product-version` and its PID, and holding
//!   the Daemon's serving lock beside it while it plays, as a live Daemon
//!   does, so a Client takes it for running until it exits. It prints
//!   `{"address": …, "runtime_file": …}` once a Client can find it, and
//!   removes the file and the socket when the play ends, unless the script
//!   ends with `exit`, which leaves them as a crashed Daemon does;
//! - `tcp`: on loopback TCP on port 0, printing `{"address": …}`;
//! - `websocket`: as an Exposed Daemon's WebSocket (ADR 0049), on loopback
//!   TCP on port 0, printing `{"address": …}`. The handshake must carry the
//!   address's own `Origin` over `http`, as a Client outside a browser sends
//!   it (ADR 0067), or it is answered `403` and never played. As the Daemon
//!   does, `--subprotocol` is answered when the handshake offers it and
//!   none is answered otherwise, as by a Daemon of another protocol major.
//!   Each WebSocket message carries one JSON-RPC message each way, and the
//!   WebSocket closes when the play ends. A `drop` step drops the Client's
//!   connection with no close, as a lost network link does, and plays the
//!   steps after it to the next Client that connects, the one mode that
//!   takes another;
//! - `http`: as a Daemon's HTTP listener (ADRs 0027 and 0070), on loopback
//!   TCP on port 0, printing `{"address": …, "token": …}`. Each request
//!   posts one JSON-RPC message, played only behind `--token` as its bearer
//!   token: any other request is answered `401` and never played. A request
//!   is answered once the play reads its next message or ends, with every
//!   line the play wrote since it, or `202` when it wrote none for a
//!   notification; a request the play wrote nothing for when it ends gets no
//!   answer, as from a Daemon that dropped it, and one it reads on past is
//!   held until the Client gives up. The Client keeps one connection for the
//!   play, and closing it is the Client closing its end.
//!
//! The `hello` token check is the script's own `expect` of a `hello`
//! carrying the token, never a built-in.
//!
//! An SDK reaches it as an Attached engine instead: its Product's
//! engine-path variable (`GOODFETCH_COLLIE_ENGINE_PATH` or
//! `GOODFETCH_FOXHOUND_ENGINE_PATH`, ADR 0090) names this executable, which
//! the SDK starts as `attached-engine` with its environment map; the script
//! is then `scripted-engine/script.json` under the map's `GOODFETCH_HOME`,
//! and the record `scripted-engine/record.jsonl` beside it. A Client that
//! starts a Daemon's process as a Product's `daemon start` does, such as the
//! conformance harness, starts `daemon launched` instead: it plays the same
//! script in `socket` mode and, as a launched Daemon does, prints the
//! runtime file it wrote once a Client can find it.
//!
//! A test-only executable the Simulation never runs, it opens its own
//! stdio and sockets and writes its own stdout, the one writer to stdout
//! beyond `STYLE.md`'s three (ADRs 0060 and 0069).
#![expect(
    clippy::disallowed_types,
    reason = "the test-only Scripted engine opens its own sockets, outside `transport` (ADRs 0060 and 0069)"
)]

use std::cell::{OnceCell, RefCell};
use std::fs::File;
use std::io::{self, BufRead as _, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use goodfetch_test_support::{
    ATTACHED_RECORD, ATTACHED_SCRIPT, DaemonEndpoint, Ending, RuntimeFile, SCRIPTED_DAEMON_TOKEN,
    Script, play, play_relinking,
};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, handshake, http};

#[derive(Parser)]
#[command(
    name = "goodfetch-scripted-engine",
    about = "Plays one script of protocol messages to a Client, as a stand-in engine in tests",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Arguments {
    /// The script to play.
    #[arg(long, required = true)]
    script: Option<PathBuf>,
    /// How the Client reaches the engine.
    #[arg(long, value_enum, required = true)]
    mode: Option<Mode>,
    /// Where to write the record of every message, one JSON line each.
    #[arg(long)]
    record: Option<PathBuf>,
    /// The per-launch token the runtime file holds, in `--mode socket`, and
    /// the bearer token a request takes, in `--mode http`.
    #[arg(long, default_value = SCRIPTED_DAEMON_TOKEN)]
    token: String,
    /// The Product version the runtime file holds, whose major names its
    /// per-major folder, in `--mode socket`.
    #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
    product_version: String,
    /// The WebSocket subprotocol answered to a handshake that offers it, in
    /// `--mode websocket`, such as `goodfetch.v0`.
    #[arg(long)]
    subprotocol: Option<String>,
    #[command(subcommand)]
    form: Option<Form>,
}

/// How the Client reaches the engine.
#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    /// Over the engine's own stdin and stdout, as an Attached engine.
    Stdio,
    /// As a Daemon, found through the runtime file in `GOODFETCH_HOME`.
    Socket,
    /// On loopback TCP, at the address it prints.
    Tcp,
    /// As an Exposed Daemon's WebSocket, at the address it prints.
    Websocket,
    /// As a Daemon's HTTP listener, at the address and behind the token it
    /// prints.
    Http,
}

#[derive(Subcommand)]
enum Form {
    /// Runs as an SDK starts its Attached engine: plays
    /// `scripted-engine/script.json` under `GOODFETCH_HOME` over stdio.
    AttachedEngine,
    /// Runs as a Product's Daemon is started.
    #[command(subcommand)]
    Daemon(DaemonForm),
}

#[derive(Subcommand)]
enum DaemonForm {
    /// Runs as a Daemon's launched process: plays
    /// `scripted-engine/script.json` under `GOODFETCH_HOME` in `socket` mode
    /// and prints the runtime file once ready.
    Launched,
}

/// The line printed once a Client can find the Daemon.
#[derive(Clone, Copy)]
enum Ready {
    /// `{"address": …, "runtime_file": …}`, for a test that started it.
    Address,
    /// The runtime file's contents, as a launched Daemon prints them.
    RuntimeFile,
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match run(&arguments) {
        Ok(Ending::Finished | Ending::Closed) => ExitCode::SUCCESS,
        Ok(Ending::Exit(exit)) => {
            if let Some(lines) = &exit.stderr {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "the test-only Scripted engine writes a script's crash output on its own stderr (ADRs 0060 and 0069)"
                )]
                let mut stderr = io::stderr().lock();
                // A crash's output: the process ends next, whatever came of it.
                let _ = stderr.write_all(lines.text().as_bytes());
            }
            ExitCode::from(exit.code)
        }
        Err(failure) => {
            #[expect(
                clippy::print_stderr,
                reason = "the Scripted engine reports a failed play on its own stderr (ADRs 0060 and 0069)"
            )]
            {
                eprintln!("goodfetch-scripted-engine: {failure}");
            }
            ExitCode::FAILURE
        }
    }
}

type Failure = Box<dyn std::error::Error>;

fn run(arguments: &Arguments) -> Result<Ending, Failure> {
    let mut ready = Ready::Address;
    let (script, record, mode) = match (&arguments.form, &arguments.script, arguments.mode) {
        (Some(Form::AttachedEngine), _, _) => {
            let home = home()?;
            (
                home.join(ATTACHED_SCRIPT),
                Some(home.join(ATTACHED_RECORD)),
                Mode::Stdio,
            )
        }
        (Some(Form::Daemon(DaemonForm::Launched)), _, _) => {
            let home = home()?;
            ready = Ready::RuntimeFile;
            (
                home.join(ATTACHED_SCRIPT),
                Some(home.join(ATTACHED_RECORD)),
                Mode::Socket,
            )
        }
        (None, Some(script), Some(mode)) => (script.clone(), arguments.record.clone(), mode),
        (None, _, _) => return Err("invalid_arguments: --script and --mode name no play".into()),
    };
    let script = Script::read(&script).map_err(|error| format!("invalid_script: {error}"))?;
    let record: Box<dyn Write> = match record {
        Some(path) => Box::new(create(&path)?),
        None => Box::new(io::sink()),
    };
    match mode {
        Mode::Stdio => {
            #[expect(
                clippy::disallowed_methods,
                reason = "the test-only Scripted engine plays its script over its own stdio, outside `transport` (ADRs 0060 and 0069)"
            )]
            let (stdin, stdout) = (io::stdin().lock(), io::stdout().lock());
            Ok(play(&script, stdin, stdout, record)?)
        }
        Mode::Tcp => {
            let listener = Listener::Tcp(
                TcpListener::bind("127.0.0.1:0")
                    .map_err(|error| format!("listen_failed: {error}"))?,
            );
            announce(&json!({ "address": listener.endpoint()?.to_string() }))?;
            listener.play(&script, record)
        }
        Mode::Websocket => {
            let Some(subprotocol) = &arguments.subprotocol else {
                return Err(
                    "invalid_arguments: --mode websocket takes the --subprotocol it requires"
                        .into(),
                );
            };
            let listener = TcpListener::bind("127.0.0.1:0")
                .map_err(|error| format!("listen_failed: {error}"))?;
            announce(&json!({ "address": listener.local_addr()?.to_string() }))?;
            WebSocketListener::new(&listener, subprotocol).play(&script, record)
        }
        Mode::Http => {
            let listener = TcpListener::bind("127.0.0.1:0")
                .map_err(|error| format!("listen_failed: {error}"))?;
            announce(&json!({
                "address": listener.local_addr()?.to_string(),
                "token": arguments.token,
            }))?;
            HttpListener::new(&listener, &arguments.token).play(&script, record)
        }
        Mode::Socket => {
            let daemon = Daemon::publish(&home()?, arguments, ready)
                .map_err(|error| format!("publish_failed: {error}"))?;
            let ending = daemon.listener.play(&script, record);
            if let Ok(Ending::Exit(_)) = ending {
                daemon.abandon();
                return ending;
            }
            // The play's own failure comes first: a test matches on its code.
            let withdrawn = daemon.withdraw();
            ending.and_then(|ending| withdrawn.map(|()| ending))
        }
    }
}

/// The Scripted engine published as a Daemon: listening, and named in its
/// runtime file.
struct Daemon {
    listener: Listener,
    endpoint: DaemonEndpoint,
    runtime_file: PathBuf,
    /// The socket's own folder, made outside the home when the socket's
    /// path in the home is too long for the OS.
    socket_folder: Option<tempfile::TempDir>,
    /// The serving lock, held until the process exits.
    _serving: std::fs::File,
}

impl Daemon {
    /// Listens, writes the runtime file and prints the line saying a Client
    /// can find it.
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
    )]
    fn publish(home: &Path, arguments: &Arguments, ready: Ready) -> Result<Self, Failure> {
        let runtime_file = RuntimeFile::path(home, &arguments.product_version);
        let folder = runtime_file
            .parent()
            .expect("invariant: a runtime file sits in a per-major folder");
        create_folder(folder)?;
        let serving = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(folder.join(goodfetch_test_support::DAEMON_SERVING_LOCK))?;
        serving.lock()?;
        let (listener, socket_folder) = Listener::daemon(folder)?;
        let endpoint = listener.endpoint()?;
        let contents = RuntimeFile {
            endpoint: endpoint.clone(),
            token: arguments.token.clone(),
            version: arguments.product_version.clone(),
            pid: std::process::id(),
        };
        let daemon = Self {
            listener,
            endpoint: endpoint.clone(),
            runtime_file,
            socket_folder,
            _serving: serving,
        };
        contents.write(&daemon.runtime_file)?;
        match ready {
            Ready::Address => announce(&json!({
                "address": endpoint.to_string(),
                "runtime_file": daemon.runtime_file,
            }))?,
            Ready::RuntimeFile => announce(&serde_json::to_value(&contents)?)?,
        }
        Ok(daemon)
    }

    /// Removes the runtime file and the socket, as a Daemon that stops.
    fn withdraw(self) -> Result<(), Failure> {
        remove(&self.runtime_file)?;
        if let DaemonEndpoint::Unix(path) = &self.endpoint {
            remove(path)?;
        }
        Ok(())
    }

    /// Leaves the runtime file and a socket in the home, as a crashed
    /// Daemon does; only a socket folder outside the home goes. A killed
    /// engine runs no destructor, so its `TestEnv` removes that folder at
    /// teardown.
    fn abandon(self) {
        drop(self.socket_folder);
    }
}

/// Where the Scripted engine waits for its one Client.
enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

impl Listener {
    /// A Daemon's listener: a Unix socket in `folder`, readable by its
    /// owner only, or in a folder of its own under `/tmp` when that path is
    /// longer than the OS takes (104 bytes on macOS), named with a space and
    /// a non-ASCII letter like a test's home (ADR 0073).
    #[cfg(unix)]
    fn daemon(folder: &Path) -> Result<(Self, Option<tempfile::TempDir>), Failure> {
        use std::os::unix::fs::PermissionsExt as _;
        let socket = folder.join(goodfetch_test_support::DAEMON_SOCKET);
        // The caller holds the serving lock, so a socket left here is a dead
        // Daemon's: a starting Daemon replaces its files (ADR 0024).
        remove(&socket)?;
        let (listener, path, own_folder) = match UnixListener::bind(&socket) {
            Ok(listener) => (listener, socket, None),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                let own = tempfile::Builder::new()
                    .prefix(goodfetch_test_support::SOCKET_FOLDER_PREFIX)
                    .permissions(std::fs::Permissions::from_mode(0o700))
                    .tempdir_in("/tmp")?;
                let path = own.path().join(goodfetch_test_support::DAEMON_SOCKET);
                (UnixListener::bind(&path)?, path, Some(own))
            }
            Err(error) => return Err(error.into()),
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok((Self::Unix(listener), own_folder))
    }

    /// A Daemon's listener: loopback TCP, as on Windows (ADR 0024).
    #[cfg(not(unix))]
    fn daemon(_folder: &Path) -> Result<(Self, Option<tempfile::TempDir>), Failure> {
        Ok((Self::Tcp(TcpListener::bind("127.0.0.1:0")?), None))
    }

    fn endpoint(&self) -> io::Result<DaemonEndpoint> {
        match self {
            Self::Tcp(listener) => listener.local_addr().map(DaemonEndpoint::Tcp),
            #[cfg(unix)]
            Self::Unix(listener) => {
                let address = listener.local_addr()?;
                let path = address.as_pathname().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "the socket has no path")
                })?;
                Ok(DaemonEndpoint::Unix(path.to_owned()))
            }
        }
    }

    /// Plays `script` to the first Client to connect, accepting it only
    /// when a step first reads or writes, so a script that exits at once
    /// never waits for one.
    fn play(&self, script: &Script, record: impl Write) -> Result<Ending, Failure> {
        let connection = Connection {
            listener: self,
            stream: OnceCell::new(),
        };
        let ending = play(script, BufReader::new(&connection), &connection, record);
        connection.close();
        Ok(ending?)
    }
}

/// The one Client's connection, accepted on first use.
struct Connection<'a> {
    listener: &'a Listener,
    stream: OnceCell<Stream>,
}

impl Connection<'_> {
    fn stream(&self) -> io::Result<&Stream> {
        if let Some(stream) = self.stream.get() {
            return Ok(stream);
        }
        let stream = match &self.listener {
            Listener::Tcp(listener) => Stream::Tcp(listener.accept()?.0),
            #[cfg(unix)]
            Listener::Unix(listener) => Stream::Unix(listener.accept()?.0),
        };
        Ok(self.stream.get_or_init(|| stream))
    }

    /// Closes the connection both ways, so the Client reads its end at
    /// once.
    fn close(&self) {
        // A Client that already went leaves nothing to shut down.
        let _ = match self.stream.get() {
            Some(Stream::Tcp(stream)) => stream.shutdown(Shutdown::Both),
            #[cfg(unix)]
            Some(Stream::Unix(stream)) => stream.shutdown(Shutdown::Both),
            None => Ok(()),
        };
    }
}

enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Read for &Connection<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self.stream()? {
            Stream::Tcp(stream) => (&*stream).read(buffer),
            #[cfg(unix)]
            Stream::Unix(stream) => (&*stream).read(buffer),
        }
    }
}

impl Write for &Connection<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self.stream()? {
            Stream::Tcp(stream) => (&*stream).write(buffer),
            #[cfg(unix)]
            Stream::Unix(stream) => (&*stream).write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.stream()? {
            Stream::Tcp(stream) => (&*stream).flush(),
            #[cfg(unix)]
            Stream::Unix(stream) => (&*stream).flush(),
        }
    }
}

/// The Scripted engine as a Daemon's HTTP listener: one JSON-RPC message
/// per request, behind a bearer token (ADRs 0027 and 0070).
struct HttpListener<'a> {
    listener: &'a TcpListener,
    token: &'a str,
    exchange: RefCell<Exchange>,
}

impl<'a> HttpListener<'a> {
    fn new(listener: &'a TcpListener, token: &'a str) -> Self {
        Self {
            listener,
            token,
            exchange: RefCell::new(Exchange::default()),
        }
    }

    /// Plays `script` to the first Client to connect, accepting it only
    /// when the play first reads, so a script that exits at once never
    /// waits for one.
    fn play(&self, script: &Script, record: impl Write) -> Result<Ending, Failure> {
        let ending = play(script, BufReader::new(self), self, record);
        let ended = self.exchange.borrow_mut().end();
        // The play's own failure comes first: a test matches on its code.
        let ending = ending?;
        ended.map_err(|error| format!("client_gone: {error}"))?;
        Ok(ending)
    }
}

/// The one Client's connection, and the request waiting on the play.
#[derive(Default)]
struct Exchange {
    connection: Option<BufReader<TcpStream>>,
    /// The last message posted, as one line, until the play reads it all.
    posted: io::Cursor<Vec<u8>>,
    waiting: Option<Waiting>,
    /// Every line the play wrote since the request waiting.
    answer: Vec<u8>,
}

/// What a request waits on.
#[derive(Clone, Copy)]
enum Waiting {
    /// A JSON-RPC request, which carries an ID: its answer.
    Answer,
    /// A notification, or what is no JSON-RPC message: its acceptance.
    Acceptance,
}

/// A request's head and body, as the listener takes them.
struct Request {
    bearer: Option<String>,
    body: Vec<u8>,
}

impl Exchange {
    /// Answers the request waiting with every line the play wrote since it.
    /// Returns `false` when it waits on an answer the play did not write:
    /// that request is held until the Client gives up and closes the
    /// connection.
    fn answered(&mut self) -> io::Result<bool> {
        match self.waiting.take() {
            None => Ok(true),
            Some(_) if !self.answer.is_empty() => {
                let answer = std::mem::take(&mut self.answer);
                self.respond("200 OK", &[], &answer).map(|()| true)
            }
            Some(Waiting::Acceptance) => self.respond("202 Accepted", &[], &[]).map(|()| true),
            Some(Waiting::Answer) => {
                io::copy(self.connection()?, &mut io::sink())?;
                Ok(false)
            }
        }
    }

    /// The next message the Client posts behind `token`, as one line, or
    /// `None` once the Client closed its connection. A request without
    /// the token is answered `401` and never played.
    fn next_message(&mut self, listener: &TcpListener, token: &str) -> io::Result<Option<Vec<u8>>> {
        if self.connection.is_none() {
            self.connection = Some(BufReader::new(listener.accept()?.0));
        }
        loop {
            let Some(request) = self.request()? else {
                return Ok(None);
            };
            if request.bearer.as_deref() != Some(token) {
                self.respond("401 Unauthorized", &[("WWW-Authenticate", "Bearer")], &[])?;
                continue;
            }
            let carries_id = serde_json::from_slice::<Value>(&request.body)
                .is_ok_and(|message| message.get("id").is_some());
            self.waiting = Some(if carries_id {
                Waiting::Answer
            } else {
                Waiting::Acceptance
            });
            // JSON allows no line break inside a string, so the message
            // stays one message on one line.
            let mut line: Vec<u8> = request
                .body
                .into_iter()
                .map(|byte| {
                    if byte == b'\n' || byte == b'\r' {
                        b' '
                    } else {
                        byte
                    }
                })
                .collect();
            line.push(b'\n');
            return Ok(Some(line));
        }
    }

    /// The connection's next request, or `None` once the Client closed it.
    fn request(&mut self) -> io::Result<Option<Request>> {
        let connection = self.connection()?;
        let mut head = Vec::new();
        loop {
            let start = head.len();
            if connection.read_until(b'\n', &mut head)? == 0 {
                return if head.is_empty() {
                    Ok(None)
                } else {
                    Err(io::ErrorKind::UnexpectedEof.into())
                };
            }
            if head[start..]
                .iter()
                .all(|byte| matches!(byte, b'\r' | b'\n'))
            {
                // An empty line before the request line is skipped (RFC 9112).
                if start == 0 {
                    head.clear();
                    continue;
                }
                break;
            }
        }
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut parsed = httparse::Request::new(&mut headers);
        match parsed.parse(&head) {
            Ok(httparse::Status::Complete(_)) => {}
            Ok(httparse::Status::Partial) => return Err(invalid("a request head cut short")),
            Err(error) => return Err(invalid(&format!("no HTTP request head: {error}"))),
        }
        let header = |name: &str| {
            parsed
                .headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case(name))
                .map(|header| String::from_utf8_lossy(header.value).into_owned())
        };
        if header("transfer-encoding").is_some() {
            return Err(invalid("a body sent without its length"));
        }
        let length = match header("content-length") {
            None => 0,
            Some(value) => value
                .trim()
                .parse()
                .map_err(|_| invalid("a Content-Length that is no length"))?,
        };
        let bearer = header("authorization").and_then(|value| {
            let (scheme, token) = value.split_once(' ')?;
            scheme
                .eq_ignore_ascii_case("bearer")
                .then(|| token.trim().to_owned())
        });
        let mut body = vec![0; length];
        connection.read_exact(&mut body)?;
        Ok(Some(Request { bearer, body }))
    }

    /// Answers the request waiting with `status`, `headers` and `body`.
    fn respond(&mut self, status: &str, headers: &[(&str, &str)], body: &[u8]) -> io::Result<()> {
        use std::fmt::Write as _;
        let mut head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n",
            body.len()
        );
        if !body.is_empty() {
            head.push_str("Content-Type: application/json\r\n");
        }
        for (name, value) in headers {
            write!(head, "{name}: {value}\r\n").expect("invariant: a String takes every write");
        }
        head.push_str("\r\n");
        let stream = self.connection()?.get_mut();
        stream.write_all(head.as_bytes())?;
        stream.write_all(body)?;
        stream.flush()
    }

    /// Answers the request still waiting with every line the play wrote
    /// since it, then closes the connection both ways; a request the play
    /// wrote nothing for gets no answer, as from a Daemon that dropped it.
    fn end(&mut self) -> io::Result<()> {
        let answered = match self.waiting.take() {
            Some(_) if !self.answer.is_empty() => {
                let answer = std::mem::take(&mut self.answer);
                self.respond("200 OK", &[], &answer)
            }
            _ => Ok(()),
        };
        if let Some(connection) = self.connection.take() {
            // A Client that already went leaves nothing to shut down.
            let _ = connection.get_ref().shutdown(Shutdown::Both);
        }
        answered
    }

    fn connection(&mut self) -> io::Result<&mut BufReader<TcpStream>> {
        self.connection
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no Client connected"))
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

impl Read for &HttpListener<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut exchange = self.exchange.borrow_mut();
        loop {
            let read = exchange.posted.read(buffer)?;
            if read > 0 || buffer.is_empty() {
                return Ok(read);
            }
            // The play reads its next message: the last one is answered.
            if !exchange.answered()? {
                return Ok(0);
            }
            let Some(line) = exchange.next_message(self.listener, self.token)? else {
                return Ok(0);
            };
            exchange.posted = io::Cursor::new(line);
        }
    }
}

impl Write for &HttpListener<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.exchange.borrow_mut().answer.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The Scripted engine as an Exposed Daemon's WebSocket (ADR 0049): one
/// JSON-RPC message per WebSocket message each way, under the subprotocol
/// it answers when the Client offers it.
struct WebSocketListener<'a> {
    listener: &'a TcpListener,
    subprotocol: &'a str,
    /// The Client's WebSocket, none until a step first reads or writes or
    /// once a `drop` step dropped it.
    socket: RefCell<Option<WebSocket<TcpStream>>>,
    /// The last message received, as one line, until the play reads it all.
    received: RefCell<io::Cursor<Vec<u8>>>,
    /// What the play wrote past its last full line.
    written: RefCell<Vec<u8>>,
}

impl<'a> WebSocketListener<'a> {
    fn new(listener: &'a TcpListener, subprotocol: &'a str) -> Self {
        Self {
            listener,
            subprotocol,
            socket: RefCell::new(None),
            received: RefCell::new(io::Cursor::new(Vec::new())),
            written: RefCell::new(Vec::new()),
        }
    }

    /// Plays `script` to the first Client to connect, accepting it only
    /// when a step first reads or writes, so a script that exits at once
    /// never waits for one; a `drop` step drops it, and the steps after
    /// play to the next Client. The WebSocket is closed when the play ends.
    fn play(&self, script: &Script, record: impl Write) -> Result<Ending, Failure> {
        let ending = play_relinking(script, BufReader::new(self), self, record, &mut || {
            self.drop_client();
            Ok(())
        });
        if let Some(socket) = self.socket.borrow_mut().as_mut() {
            // A Client that already went leaves nothing to close.
            let _ = socket.close(None).and_then(|()| socket.flush());
            let _ = socket.get_ref().shutdown(Shutdown::Both);
        }
        Ok(ending?)
    }

    /// Drops the Client's connection with no WebSocket close, as a lost
    /// network link does, and forgets what it sent and was sent, so the
    /// next read or write accepts the next Client. Each message is one
    /// line, so the play's reader holds nothing of it past the last line.
    fn drop_client(&self) {
        if let Some(socket) = self.socket.borrow_mut().take() {
            // A Client that already went leaves nothing to shut down.
            let _ = socket.get_ref().shutdown(Shutdown::Both);
        }
        *self.received.borrow_mut() = io::Cursor::new(Vec::new());
        self.written.borrow_mut().clear();
    }

    /// Runs `act` on the Client's WebSocket, accepting the Client first when
    /// none is connected.
    fn with_socket<T>(&self, act: impl FnOnce(&mut WebSocket<TcpStream>) -> T) -> io::Result<T> {
        let mut socket = self.socket.borrow_mut();
        if socket.is_none() {
            *socket = Some(self.accept()?);
        }
        let socket = socket
            .as_mut()
            .expect("invariant: the Client's WebSocket was just accepted");
        Ok(act(socket))
    }

    /// The WebSocket of the next Client, whose handshake must carry the
    /// listener's own `Origin`, by its loopback address or `localhost`, as a
    /// Client outside a browser sends it (ADR 0067), or it is answered `403`
    /// and the play reads its end. As the Daemon does (ADR 0049), the
    /// subprotocol is answered when the handshake offers it and none is
    /// answered otherwise.
    fn accept(&self) -> io::Result<WebSocket<TcpStream>> {
        let port = self.listener.local_addr()?.port();
        let origins = [
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
        ];
        let (stream, _) = self.listener.accept()?;
        #[expect(
            clippy::result_large_err,
            reason = "tungstenite's handshake callback answers its own `ErrorResponse`"
        )]
        let check =
            |request: &handshake::server::Request,
             mut response: handshake::server::Response|
             -> Result<handshake::server::Response, handshake::server::ErrorResponse> {
                let header = |name| {
                    request
                        .headers()
                        .get(name)
                        .and_then(|value| value.to_str().ok())
                };
                let own_origin = header(http::header::ORIGIN)
                    .is_some_and(|origin| origins.iter().any(|own| own == origin));
                if !own_origin {
                    let mut refused = handshake::server::ErrorResponse::new(None);
                    *refused.status_mut() = http::StatusCode::FORBIDDEN;
                    return Err(refused);
                }
                let offered = header(http::header::SEC_WEBSOCKET_PROTOCOL).is_some_and(|offered| {
                    offered
                        .split(',')
                        .any(|offered| offered.trim() == self.subprotocol)
                });
                if offered {
                    let subprotocol = http::HeaderValue::from_str(self.subprotocol)
                        .map_err(|_| handshake::server::ErrorResponse::new(None))?;
                    response
                        .headers_mut()
                        .insert(http::header::SEC_WEBSOCKET_PROTOCOL, subprotocol);
                }
                Ok(response)
            };
        tungstenite::accept_hdr(stream, check)
            .map_err(|error| io::Error::new(io::ErrorKind::ConnectionRefused, error.to_string()))
    }
}

impl Read for &WebSocketListener<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut received = self.received.borrow_mut();
        loop {
            let read = received.read(buffer)?;
            if read > 0 || buffer.is_empty() {
                return Ok(read);
            }
            let message = match self.with_socket(WebSocket::read)? {
                Ok(message) => message,
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    return Ok(0);
                }
                Err(tungstenite::Error::Io(error)) => return Err(error),
                Err(error) => return Err(io::Error::other(error.to_string())),
            };
            let mut line = match message {
                Message::Text(text) => text.as_bytes().to_vec(),
                Message::Binary(bytes) => bytes.to_vec(),
                Message::Close(_) => return Ok(0),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            };
            line.push(b'\n');
            *received = io::Cursor::new(line);
        }
    }
}

impl Write for &WebSocketListener<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut written = self.written.borrow_mut();
        written.extend_from_slice(buffer);
        while let Some(end) = written.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = written.drain(..=end).take(end).collect();
            let text = String::from_utf8(line).map_err(|_| invalid("a line that is no UTF-8"))?;
            self.with_socket(|socket| socket.send(Message::text(text)))?
                .map_err(|error| io::Error::other(error.to_string()))?;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.socket.borrow_mut().as_mut() {
            Some(socket) => socket
                .flush()
                .map_err(|error| io::Error::other(error.to_string())),
            None => Ok(()),
        }
    }
}

/// Prints the one line saying where a Client reaches the engine.
fn announce(line: &Value) -> Result<(), Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine prints the address a test reads, its own stdout (ADRs 0060 and 0069)"
    )]
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{line}")?;
    stdout.flush()?;
    Ok(())
}

/// The Goodfetch home the Client's environment map names.
fn home() -> Result<PathBuf, Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine reads the Goodfetch home its Client's environment map gives it, as an engine's `main` does (ADR 0069)"
    )]
    let home = std::env::var_os("GOODFETCH_HOME");
    home.map(PathBuf::from)
        .ok_or_else(|| "no_home: GOODFETCH_HOME names no Goodfetch home".into())
}

fn create(path: &Path) -> Result<File, Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine is synchronous and starts no async worker to block (ADR 0069)"
    )]
    let file = File::create(path);
    file.map_err(|error| {
        format!(
            "record_failed: {} cannot be written: {error}",
            path.display()
        )
        .into()
    })
}

fn create_folder(folder: &Path) -> Result<(), Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine is synchronous and starts no async worker to block (ADR 0069)"
    )]
    let made = std::fs::create_dir_all(folder);
    Ok(made?)
}

fn remove(path: &Path) -> Result<(), Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the Scripted engine is synchronous and starts no async worker to block (ADR 0069)"
    )]
    let removed = std::fs::remove_file(path);
    match removed {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}
