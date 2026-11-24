//! Model recordings replayed at an adapter's base URL (ADR 0065): every
//! exchange of a folder's recording files is one mock of a local `httpmock`
//! server, matched on method, path with its query and the canonical JSON
//! body (RFC 8785), and served once, in file order. A last mock takes each
//! request no exchange matched, answers it 404 and keeps it; finishing, or
//! dropping, fails on such a request and on an exchange left unused.
//! `httpmock`'s own playback is not used: it matches a body on its bytes,
//! so a request whose JSON keys come in another order would find nothing.
//!
//! A stream is served whole. Re-cut, a relay on loopback in front of the
//! server writes each response in pieces whose sizes it draws from the
//! test's seed, a short pause apart, so an adapter's stream parser meets
//! its lines and events cut anywhere, the response's head among them.
#![expect(
    clippy::disallowed_types,
    reason = "the re-cut relay's sockets and the replay's records live on plain threads of the test's own, never an async worker (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "recording files are read in the test's setup, and the re-cut relay's threads and pauses run outside any async worker"
)]

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chacha20::ChaCha20Rng;
use chacha20::rand_core::Rng as _;
use httpmock::{HttpMockRequest, Mock, MockServer};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::seeded;

/// The status of an answer to a request no exchange matched.
const UNMATCHED_STATUS: u16 = 404;

/// The largest piece the re-cut relay writes, in bytes.
const PIECE_MAX: u32 = 64;

/// The pause between two pieces, long enough for a client on loopback to
/// read each apart.
const PIECE_PAUSE: Duration = Duration::from_millis(1);

/// A recording file: the exchanges one adapter test replays.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelRecording {
    /// Served in this order, each once.
    pub exchanges: Vec<RecordedExchange>,
}

/// One request to a Model provider and the response it got.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordedExchange {
    pub request: RecordedRequest,
    pub response: RecordedResponse,
    /// A fault written by hand rather than recorded: an explicit stub, which
    /// [`ServedExchanges`] reports apart.
    #[serde(default)]
    pub synthetic: bool,
}

/// What a request must be to get its exchange's response.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordedRequest {
    /// `POST`, `GET`, …, in any case.
    pub method: String,
    /// The path with its query, which matches exactly: `/v1/chat/completions`.
    pub path: String,
    /// The JSON body, which matches in canonical form; absent, the request
    /// has no body.
    #[serde(default)]
    pub body: Option<Value>,
}

/// The response an exchange serves.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(try_from = "ResponseFields")]
#[schemars(with = "ResponseFields")]
pub struct RecordedResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub payload: Payload,
}

/// A response's body, whole or as the chunks of a stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    /// A JSON body, served as its JSON text; a string is served as it is.
    Body(Value),
    /// A stream's chunks, as the provider sent them: served whole, or re-cut
    /// at seeded boundaries.
    Chunks(Vec<String>),
}

impl Payload {
    fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Body(Value::String(text)) => text.clone().into_bytes(),
            Self::Body(value) => value.to_string().into_bytes(),
            Self::Chunks(chunks) => chunks.concat().into_bytes(),
        }
    }
}

/// A response as its file writes it: a status, headers, and either `body`
/// or `chunks`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("oneOf" = [{ "required": ["body"] }, { "required": ["chunks"] }]))]
struct ResponseFields {
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    #[schemars(extend("not" = { "type": "null" }))]
    body: Option<Value>,
    #[serde(default)]
    #[schemars(with = "Vec<String>")]
    chunks: Option<Vec<String>>,
}

impl TryFrom<ResponseFields> for RecordedResponse {
    type Error = &'static str;

    fn try_from(fields: ResponseFields) -> Result<Self, Self::Error> {
        let payload = match (fields.body, fields.chunks) {
            (Some(body), None) => Payload::Body(body),
            (None, Some(chunks)) => Payload::Chunks(chunks),
            _ => return Err("a response has either `body` or `chunks`"),
        };
        Ok(Self {
            status: fields.status,
            headers: fields.headers,
            payload,
        })
    }
}

/// An exchange by its file's name and its place in the file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExchangeName {
    pub file: String,
    pub index: usize,
}

impl fmt::Display for ExchangeName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}#{}", self.file, self.index)
    }
}

/// A request no exchange matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmatchedRequest {
    pub method: String,
    /// The path with its query.
    pub path: String,
    /// The body as text, invalid UTF-8 replaced.
    pub body: String,
}

impl UnmatchedRequest {
    fn of(request: &HttpMockRequest) -> Self {
        Self {
            method: request.method_str().to_owned(),
            path: path_and_query(request),
            body: String::from_utf8_lossy(request.body_ref()).into_owned(),
        }
    }
}

/// What fails a replay: exchanges no request used, and requests no exchange
/// matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unfinished {
    pub unused: Vec<ExchangeName>,
    pub unmatched: Vec<UnmatchedRequest>,
}

impl fmt::Display for Unfinished {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Model recordings left unfinished (ADR 0065):")?;
        for name in &self.unused {
            write!(formatter, "\n  unused: {name}")?;
        }
        for request in &self.unmatched {
            write!(
                formatter,
                "\n  unmatched: {} {} {}",
                request.method, request.path, request.body
            )?;
        }
        Ok(())
    }
}

/// The exchanges a finished replay served, recorded ones and synthetic ones
/// apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedExchanges {
    pub recorded: Vec<ExchangeName>,
    pub synthetic: Vec<ExchangeName>,
}

/// Why a folder's recordings cannot be served.
#[derive(Debug, thiserror::Error)]
pub enum RecordingsError {
    #[error("cannot read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} is no Model recording")]
    Format {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{folder} holds no exchange")]
    Empty { folder: PathBuf },
}

/// An exchange's mock on the server.
struct Registered {
    name: ExchangeName,
    mock: usize,
    synthetic: bool,
}

/// The Model recordings of one folder, served until the value drops.
pub struct Recordings {
    server: MockServer,
    exchanges: Vec<Registered>,
    unmatched: Arc<Mutex<Vec<UnmatchedRequest>>>,
    relay: Option<Relay>,
    finished: bool,
}

impl fmt::Debug for Recordings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Recordings")
            .field("base_url", &self.base_url())
            .field(
                "exchanges",
                &self
                    .exchanges
                    .iter()
                    .map(|exchange| exchange.name.to_string())
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl Recordings {
    /// Serves every exchange of the `*.json` files in `folder`, the files in
    /// name order: each once, a stream whole.
    ///
    /// # Errors
    ///
    /// When the folder or a file cannot be read, a file is no
    /// [`ModelRecording`], or the files hold no exchange.
    pub fn serve(folder: &Path) -> Result<Self, RecordingsError> {
        let files = read_folder(folder)?;
        if files
            .iter()
            .all(|(_, recording)| recording.exchanges.is_empty())
        {
            return Err(RecordingsError::Empty {
                folder: folder.to_owned(),
            });
        }
        let server = MockServer::start();
        let exchanges = files
            .iter()
            .flat_map(|(file, recording)| {
                recording
                    .exchanges
                    .iter()
                    .enumerate()
                    .map(move |(index, exchange)| (file, index, exchange))
            })
            .map(|(file, index, exchange)| Registered {
                name: ExchangeName {
                    file: file.clone(),
                    index,
                },
                mock: register(&server, exchange),
                synthetic: exchange.synthetic,
            })
            .collect();
        // Registered last, it is asked only once every exchange refused the
        // request: `httpmock` serves the first mock that matches, in the
        // order they were made.
        let unmatched = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&unmatched);
        server.mock(move |when, then| {
            when.is_true(move |request| {
                kept.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(UnmatchedRequest::of(request));
                true
            });
            then.status(UNMATCHED_STATUS)
                .header("content-type", "text/plain")
                .body("no Model recording matches this request");
        });
        Ok(Self {
            server,
            exchanges,
            unmatched,
            relay: None,
            finished: false,
        })
    }

    /// Serves `folder` as [`serve`](Self::serve) does, behind a relay that
    /// writes each response in pieces of 1 to 64 bytes drawn from `seed`.
    ///
    /// # Errors
    ///
    /// As [`serve`](Self::serve).
    ///
    /// # Panics
    ///
    /// When the relay cannot listen on loopback.
    pub fn serve_recut(folder: &Path, seed: u64) -> Result<Self, RecordingsError> {
        let mut recordings = Self::serve(folder)?;
        recordings.relay = Some(Relay::start(*recordings.server.address(), seed));
        Ok(recordings)
    }

    /// The URL to give the adapter as its provider's, with no path and no
    /// trailing slash: `http://127.0.0.1:<port>`.
    pub fn base_url(&self) -> String {
        match &self.relay {
            Some(relay) => format!("http://{}", relay.address),
            None => self.server.base_url(),
        }
    }

    /// The sizes of the pieces each response was written in, in the order
    /// the responses went; none when the stream is served whole.
    pub fn pieces(&self) -> Vec<Vec<usize>> {
        self.relay.as_ref().map_or_else(Vec::new, |relay| {
            relay
                .cuts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pieces
                .clone()
        })
    }

    /// Ends the replay: the exchanges it served, or what fails it.
    ///
    /// # Errors
    ///
    /// When an exchange was left unused or a request matched none.
    pub fn finish(mut self) -> Result<ServedExchanges, Unfinished> {
        self.finished = true;
        self.outcome()
    }

    fn outcome(&self) -> Result<ServedExchanges, Unfinished> {
        let mut served = ServedExchanges {
            recorded: Vec::new(),
            synthetic: Vec::new(),
        };
        let mut unused = Vec::new();
        for exchange in &self.exchanges {
            let name = exchange.name.clone();
            if Mock::new(exchange.mock, &self.server).calls() == 0 {
                unused.push(name);
            } else if exchange.synthetic {
                tracing::info!(exchange = %name, "served a synthetic Model recording");
                served.synthetic.push(name);
            } else {
                served.recorded.push(name);
            }
        }
        let unmatched = self
            .unmatched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if unused.is_empty() && unmatched.is_empty() {
            Ok(served)
        } else {
            Err(Unfinished { unused, unmatched })
        }
    }
}

impl Drop for Recordings {
    fn drop(&mut self) {
        if self.finished || std::thread::panicking() {
            return;
        }
        if let Err(unfinished) = self.outcome() {
            panic!("{unfinished}");
        }
    }
}

/// The recordings of `folder`'s `*.json` files, by file name, in name order.
fn read_folder(folder: &Path) -> Result<Vec<(String, ModelRecording)>, RecordingsError> {
    let read_error = |source| RecordingsError::Read {
        path: folder.to_owned(),
        source,
    };
    let mut paths = std::fs::read_dir(folder)
        .map_err(read_error)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(read_error)?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).map_err(|source| RecordingsError::Read {
                path: path.clone(),
                source,
            })?;
            let recording =
                serde_json::from_str(&text).map_err(|source| RecordingsError::Format {
                    path: path.clone(),
                    source,
                })?;
            let file = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok((file, recording))
        })
        .collect()
}

/// Makes `exchange`'s mock, served once, and returns its ID.
fn register(server: &MockServer, exchange: &RecordedExchange) -> usize {
    let method = exchange.request.method.clone();
    let path = exchange.request.path.clone();
    let body = exchange.request.body.as_ref().map(canonical);
    let response = &exchange.response;
    server
        .mock(|when, then| {
            when.is_true(move |request| {
                request.method_str().eq_ignore_ascii_case(&method)
                    && path_and_query(request) == path
                    && body_matches(request.body_ref(), body.as_deref())
            });
            let then = response
                .headers
                .iter()
                .fold(then.status(response.status), |then, (name, value)| {
                    then.header(name, value)
                });
            then.body(response.payload.bytes()).up_to_n_times(1);
        })
        .id()
}

fn path_and_query(request: &HttpMockRequest) -> String {
    request
        .uri()
        .path_and_query()
        .map_or_else(String::new, |path| path.as_str().to_owned())
}

/// Whether a request's body is the recorded one: none when the recording
/// has none, else the same JSON in canonical form.
fn body_matches(body: &[u8], recorded: Option<&str>) -> bool {
    match recorded {
        None => body.is_empty(),
        Some(recorded) => {
            serde_json::from_slice::<Value>(body).is_ok_and(|value| canonical(&value) == recorded)
        }
    }
}

/// The canonical JSON (RFC 8785) of a value.
fn canonical(value: &Value) -> String {
    serde_json_canonicalizer::to_string(value)
        .expect("invariant: a JSON value read from text is canonicalized")
}

/// The relay on loopback that re-cuts responses.
struct Relay {
    address: SocketAddr,
    cuts: Arc<Mutex<Cuts>>,
}

/// The seeded draws of piece sizes, shared by the relay's connections in
/// the order their responses go, and the pieces drawn so far.
struct Cuts {
    draws: ChaCha20Rng,
    pieces: Vec<Vec<usize>>,
}

impl Cuts {
    /// The sizes of the pieces of a response of `length` bytes.
    fn cut(&mut self, length: usize) -> Vec<usize> {
        let mut sizes = Vec::new();
        let mut left = length;
        while left > 0 {
            let size = usize::try_from(self.draws.next_u32() % PIECE_MAX + 1)
                .expect("invariant: a piece of at most 64 bytes has a usize size")
                .min(left);
            sizes.push(size);
            left -= size;
        }
        self.pieces.push(sizes.clone());
        sizes
    }
}

impl Relay {
    /// Listens on loopback, and relays each connection to `upstream` on
    /// threads of its own, for as long as the test's process runs.
    fn start(upstream: SocketAddr, seed: u64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a test can listen on loopback");
        let address = listener
            .local_addr()
            .expect("a listening socket has an address");
        let cuts = Arc::new(Mutex::new(Cuts {
            draws: seeded::stream(seed, b"model-recordings"),
            pieces: Vec::new(),
        }));
        let shared = Arc::clone(&cuts);
        std::thread::spawn(move || {
            for client in listener.incoming().map_while(Result::ok) {
                let cuts = Arc::clone(&shared);
                std::thread::spawn(move || {
                    let _ = relay(client, upstream, &cuts);
                });
            }
        });
        Self { address, cuts }
    }
}

/// Copies `client`'s requests to `upstream` as they come, and writes each
/// response back cut into seeded pieces.
fn relay(mut client: TcpStream, upstream: SocketAddr, cuts: &Mutex<Cuts>) -> io::Result<()> {
    let mut server = TcpStream::connect(upstream)?;
    client.set_nodelay(true)?;
    let mut client_side = client.try_clone()?;
    let mut server_side = server.try_clone()?;
    std::thread::spawn(move || {
        let _ = io::copy(&mut client_side, &mut server_side);
        let _ = server_side.shutdown(Shutdown::Write);
    });
    let mut buffer = Vec::new();
    while let Some(response) = next_response(&mut server, &mut buffer)? {
        let sizes = cuts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cut(response.len());
        let mut start = 0;
        for (index, size) in sizes.into_iter().enumerate() {
            if index > 0 {
                std::thread::sleep(PIECE_PAUSE);
            }
            client.write_all(&response[start..start + size])?;
            start += size;
        }
    }
    client.shutdown(Shutdown::Write)
}

/// The next whole response `server` sends, its head and the body its
/// `content-length` gives (every `httpmock` response has one), or what came
/// before the server closed; `None` once it closed with nothing.
fn next_response(server: &mut TcpStream, buffer: &mut Vec<u8>) -> io::Result<Option<Vec<u8>>> {
    loop {
        if let Some(length) = response_length(buffer)
            && buffer.len() >= length
        {
            return Ok(Some(buffer.drain(..length).collect()));
        }
        let mut chunk = [0; 8192];
        let read = server.read(&mut chunk)?;
        if read == 0 {
            return Ok((!buffer.is_empty()).then(|| std::mem::take(buffer)));
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

/// The length of the response at the start of `buffer`, once its head is
/// whole and names its body's length.
fn response_length(buffer: &[u8]) -> Option<usize> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut response = httparse::Response::new(&mut headers);
    let httparse::Status::Complete(head) = response.parse(buffer).ok()? else {
        return None;
    };
    let body = response
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("content-length"))
        .and_then(|header| {
            std::str::from_utf8(header.value)
                .ok()?
                .trim()
                .parse::<usize>()
                .ok()
        })?;
    Some(head + body)
}
