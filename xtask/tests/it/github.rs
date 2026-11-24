//! A local stand-in for GitHub's REST interface: it answers fixed JSON bodies
//! by request path, whatever the method, a redirect to the paths it is given
//! one for, and 404 to anything else, and it
//! records every request so a test can assert what was written (TESTING.md,
//! true externals).
#![expect(
    clippy::disallowed_types,
    reason = "a test's stand-in for a true external binds its own loopback socket and shares its request log through a std Mutex; ADR 0060's sockets and its loom-replaced sync module govern product code"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, ErrorKind, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// One request the stand-in received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) body: String,
    /// The `Authorization` header, when the request carried one.
    pub(crate) authorization: Option<String>,
    /// The `Accept` header, when the request carried one.
    pub(crate) accept: Option<String>,
}

pub(crate) struct GitHub {
    address: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl GitHub {
    /// Serves each `(path, body)` pair; the path includes its query string,
    /// and a route written `<METHOD> <path>` answers that method only, before
    /// a route of the bare path. A route written `<METHOD> <path> <text>`
    /// answers, before both, a request of that method and path whose body
    /// holds `<text>`.
    pub(crate) fn serving(routes: &[(&str, &str)]) -> Self {
        Self::serving_with_redirects(routes, &[])
    }

    /// As [`GitHub::serving`], and answers each `(path, target)` of
    /// `redirects` with a `302 Found` to `target` on the stand-in, as
    /// GitHub's API sends a release asset's download to its storage.
    pub(crate) fn serving_with_redirects(
        routes: &[(&str, &str)],
        redirects: &[(&str, &str)],
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let routes: BTreeMap<String, String> = routes
            .iter()
            .map(|(path, body)| ((*path).to_owned(), (*body).to_owned()))
            .collect();
        let redirects: BTreeMap<String, String> = redirects
            .iter()
            .map(|(path, target)| ((*path).to_owned(), format!("{address}{target}")))
            .collect();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        // The stand-in answers on its own thread for the test's life, and
        // nextest ends the test's process with it, so nothing joins it.
        #[expect(
            clippy::disallowed_methods,
            reason = "a test's stand-in serves on a plain thread; the TaskTracker rule governs product code (STYLE.md)"
        )]
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                answer(stream.unwrap(), &routes, &redirects, &recorded);
            }
        });
        Self { address, requests }
    }

    pub(crate) fn url(&self) -> &str {
        &self.address
    }

    /// Every request answered so far.
    pub(crate) fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
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
    redirects: &BTreeMap<String, String>,
    recorded: &Mutex<Vec<Request>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut length = 0;
    let mut authorization = None;
    let mut accept = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header == "\r\n" || header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-length") {
                length = value.parse().unwrap();
            } else if name.eq_ignore_ascii_case("authorization") {
                authorization = Some(value.to_owned());
            } else if name.eq_ignore_ascii_case("accept") {
                accept = Some(value.to_owned());
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let body = String::from_utf8(body).unwrap();
    let route = format!("{method} {path}");
    let by_body = routes.iter().find_map(|(key, answer)| {
        key.strip_prefix(&route)
            .and_then(|rest| rest.strip_prefix(' '))
            .filter(|text| body.contains(text))
            .map(|_| answer)
    });
    let matching = by_body
        .or_else(|| routes.get(&route))
        .or_else(|| routes.get(&path));
    let (status, location, answer) = match (redirects.get(&path), matching) {
        (Some(target), _) => ("302 Found", format!("location: {target}\r\n"), ""),
        (None, Some(body)) => ("200 OK", String::new(), body.as_str()),
        (None, None) => ("404 Not Found", String::new(), r#"{"message":"Not Found"}"#),
    };
    recorded.lock().unwrap().push(Request {
        method,
        path,
        body,
        authorization,
        accept,
    });
    let written = write!(
        stream,
        "HTTP/1.1 {status}\r\n{location}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
        answer.len()
    );
    // A client may close before the whole answer is written, as ureq does
    // once a redirect's `location` line arrives: that ends this exchange,
    // never the stand-in.
    if let Err(error) = written {
        assert!(
            matches!(
                error.kind(),
                ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
            ),
            "writing an answer: {error}"
        );
    }
}
