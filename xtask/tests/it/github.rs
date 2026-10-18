//! A local stand-in for GitHub's REST interface: it answers fixed JSON bodies
//! by request path and 404 to anything else (TESTING.md, true externals).
#![expect(
    clippy::disallowed_types,
    reason = "a test's stand-in server for a true external binds its own loopback socket (ADR 0060 governs product code)"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};

pub(crate) struct GitHub {
    address: String,
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
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                answer(stream.unwrap(), &routes);
            }
        });
        Self { address }
    }

    pub(crate) fn url(&self) -> &str {
        &self.address
    }
}

fn answer(mut stream: TcpStream, routes: &BTreeMap<String, String>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header == "\r\n" || header.is_empty() {
            break;
        }
    }
    let path = request_line.split(' ').nth(1).unwrap_or_default();
    let (status, body) = match routes.get(path) {
        Some(body) => ("200 OK", body.as_str()),
        None => ("404 Not Found", r#"{"message":"Not Found"}"#),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
}
