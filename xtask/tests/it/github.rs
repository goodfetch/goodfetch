//! A local stand-in for GitHub's REST interface: it answers fixed JSON bodies
//! by request path and 404 to any other read, and records every write
//! (`POST`, `PATCH`, `PUT`, `DELETE`) with its JSON body (TESTING.md, true
//! externals).
#![expect(
    clippy::disallowed_types,
    reason = "a test's stand-in server for a true external binds its own loopback socket (ADR 0060 governs product code)"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::Value;

pub(crate) struct GitHub {
    address: String,
    writes: Arc<Mutex<Vec<Write>>>,
}

/// One write the stand-in received.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Write {
    pub(crate) method: String,
    /// The path, with its query string.
    pub(crate) path: String,
    pub(crate) body: Value,
}

impl GitHub {
    /// Serves each `(path, body)` pair to reads; the path includes its query
    /// string. A write is answered with the body routed as `<METHOD> <path>`,
    /// or `{}` when none is.
    pub(crate) fn serving(routes: &[(&str, &str)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let routes: BTreeMap<String, String> = routes
            .iter()
            .map(|(path, body)| ((*path).to_owned(), (*body).to_owned()))
            .collect();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&writes);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                answer(stream.unwrap(), &routes, &recorded);
            }
        });
        Self { address, writes }
    }

    pub(crate) fn url(&self) -> &str {
        &self.address
    }

    /// Every write received so far, in order.
    pub(crate) fn writes(&self) -> Vec<Write> {
        self.writes.lock().unwrap().clone()
    }
}

fn answer(mut stream: TcpStream, routes: &BTreeMap<String, String>, writes: &Mutex<Vec<Write>>) {
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
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let (status, body) = if method == "GET" {
        match routes.get(&path) {
            Some(body) => ("200 OK", body.clone()),
            None => ("404 Not Found", r#"{"message":"Not Found"}"#.to_owned()),
        }
    } else {
        let mut content = vec![0; length];
        reader.read_exact(&mut content).unwrap();
        let body = if content.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&content).unwrap()
        };
        let answer = routes
            .get(&format!("{method} {path}"))
            .cloned()
            .unwrap_or_else(|| "{}".to_owned());
        writes.lock().unwrap().push(Write { method, path, body });
        ("200 OK", answer)
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
}
