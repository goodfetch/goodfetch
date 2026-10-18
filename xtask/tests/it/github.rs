//! A local stand-in for GitHub's REST interface: it answers fixed JSON bodies
//! by request path, whatever the method, and 404 to anything else, and it
//! records every request so a test can assert what was written (TESTING.md,
//! true externals).
#![expect(
    clippy::disallowed_types,
    reason = "a test's stand-in server for a true external binds its own loopback socket (ADR 0060 governs product code)"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// One request the stand-in received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) body: String,
}

pub(crate) struct GitHub {
    address: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl GitHub {
    /// Serves each `(path, body)` pair; the path includes its query string.
    pub(crate) fn serving(routes: &[(&str, &str)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let routes: BTreeMap<String, String> = routes
            .iter()
            .map(|(path, body)| ((*path).to_owned(), (*body).to_owned()))
            .collect();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                answer(stream.unwrap(), &routes, &recorded);
            }
        });
        Self { address, requests }
    }

    pub(crate) fn url(&self) -> &str {
        &self.address
    }

    /// The requests answered so far whose method is not GET.
    pub(crate) fn writes(&self) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.method != "GET")
            .cloned()
            .collect()
    }
}

/// Records the request before answering it, so a client never holds an
/// answer to a request the test cannot see yet.
fn answer(
    mut stream: TcpStream,
    routes: &BTreeMap<String, String>,
    recorded: &Mutex<Vec<Request>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut length = 0;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header == "\r\n" || header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let (status, answer) = match routes.get(&path) {
        Some(body) => ("200 OK", body.as_str()),
        None => ("404 Not Found", r#"{"message":"Not Found"}"#),
    };
    recorded.lock().unwrap().push(Request {
        method,
        path,
        body: String::from_utf8(body).unwrap(),
    });
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    )
    .unwrap();
}
