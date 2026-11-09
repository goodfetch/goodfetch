//! The replay proxy (ADR 0065: a page proxy, never a model recorder): a
//! browser that sends every request through the Proving ground gets each
//! archived real page at its original URL, over HTTP or through a `CONNECT`
//! to its host's port 443, and every other request is refused and recorded.
//! The archive's files are each at most 256 KiB with a `.source.toml`
//! naming how they were captured (ADR 0062).
#![expect(
    clippy::disallowed_types,
    reason = "the test opens its own sockets to the Proving ground, as a browser's proxy client"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

use rstest::rstest;
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};
use serde_json::{Value, json};

use crate::clients;
use crate::proving_ground::ProvingGround;

const SEED: u64 = 163;

const RUST_BLOG: &str = "http://blog.rust-lang.org/2025/12/19/what-do-people-love-about-rust";
const FDA: &str = "http://www.fda.gov/drugs/postmarket-drug-safety-information-patients-and-providers/fdas-concerns-unapproved-glp-1-drugs-used-weight-loss";

fn archive() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/archive")
}

/// The archived file of a page, as captured.
fn archived(file: &str) -> Vec<u8> {
    std::fs::read(archive().join(file)).expect("the archive holds the file")
}

/// A request in absolute form, as a browser sends it to its HTTP proxy.
fn through_the_proxy(url: &str) -> Vec<u8> {
    let rest = url.strip_prefix("http://").expect("an http URL");
    let host = rest.split('/').next().expect("a host");
    format!("GET {url} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").into_bytes()
}

fn refused(proving_ground: &ProvingGround) -> Value {
    let answer = proving_ground.control("/replay");
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()["refused"].clone()
}

/// Covers: story:2-40
#[rstest]
#[case::rust_blog(RUST_BLOG, "wcxb-0379.html")]
#[case::fda(FDA, "wcxb-0064.html")]
fn an_archived_page_is_served_at_its_original_url(#[case] url: &str, #[case] file: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let exchange = clients::raw(proving_ground.address(), &through_the_proxy(url));

    assert_eq!(exchange.status(), 200);
    assert_eq!(exchange.body(), archived(file));
}

/// Covers: story:2-40
#[rstest]
#[case::another_host("http://example.com/")]
#[case::another_page_of_an_archived_host("http://blog.rust-lang.org/inside-rust/")]
#[case::a_proving_ground_page("http://catalog.test/catalog/1")]
fn a_request_outside_the_archive_is_refused_and_recorded(#[case] url: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let exchange = clients::raw(proving_ground.address(), &through_the_proxy(url));

    assert_eq!(exchange.status(), 404);
    assert_eq!(
        refused(&proving_ground),
        json!([{ "method": "GET", "target": url }])
    );
}

/// Opens a tunnel to `authority` through the Proving ground, as a browser
/// does for HTTPS: the `CONNECT`'s status, and the stream if it is open.
fn connect(proving_ground: &ProvingGround, authority: &str) -> (u16, TcpStream) {
    let mut stream =
        TcpStream::connect(proving_ground.address()).expect("the Proving ground accepts");
    write!(
        stream,
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n"
    )
    .expect("the request is written");
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).expect("a whole head");
        head.push(byte[0]);
    }
    let status = String::from_utf8_lossy(&head)
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status line");
    (status, stream)
}

/// Covers: story:2-40
#[test]
fn a_connect_to_an_archived_host_serves_its_page_over_tls() {
    let proving_ground = ProvingGround::start(SEED);
    let config = clients::trusting(&proving_ground.certificate_authority(), &["http/1.1"]);
    let (status, stream) = connect(&proving_ground, "blog.rust-lang.org:443");
    assert_eq!(status, 200);
    let name = ServerName::try_from("blog.rust-lang.org").expect("a server name");
    let mut tls = StreamOwned::new(
        ClientConnection::new(config, name).expect("a TLS client"),
        stream,
    );

    tls.write_all(
        b"GET /2025/12/19/what-do-people-love-about-rust HTTP/1.1\r\nHost: blog.rust-lang.org\r\nConnection: close\r\n\r\n",
    )
    .expect("the request is written");
    let mut response = Vec::new();
    tls.read_to_end(&mut response)
        .expect("the response is read");

    let exchange = clients::Exchange {
        response,
        written: 0,
        read: 0,
        source: String::new(),
    };
    assert_eq!(exchange.status(), 200);
    assert_eq!(exchange.body(), archived("wcxb-0379.html"));
}

/// Covers: story:2-40
#[rstest]
#[case::another_host("example.com:443")]
#[case::another_port_of_an_archived_host("blog.rust-lang.org:8443")]
fn a_connect_elsewhere_is_refused_and_recorded(#[case] authority: &str) {
    let proving_ground = ProvingGround::start(SEED);

    let (status, _) = connect(&proving_ground, authority);

    assert_eq!(status, 403);
    assert_eq!(
        refused(&proving_ground),
        json!([{ "method": "CONNECT", "target": authority }])
    );
}

/// Covers: story:2-40
#[rstest]
#[case::rust_blog(
    "replay/1",
    RUST_BLOG,
    "What do people love about Rust?",
    "Niko Matsakis"
)]
#[case::fda(
    "replay/2",
    FDA,
    "FDA\u{2019}s Concerns with Unapproved GLP-1 Drugs Used for Weight Loss",
    "Center for Drug Evaluation and Research"
)]
fn the_headline_and_author_of_a_frozen_page_are_verified(
    #[case] task: &str,
    #[case] url: &str,
    #[case] title: &str,
    #[case] author: &str,
) {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&json!({
        "task": task,
        "status": "success",
        "records": [{ "id": url, "title": title, "author": author }],
    }));

    assert_eq!(answer["outcome"], "verified", "{answer}");
}

/// Covers: story:2-40
#[test]
fn a_frozen_pages_headline_with_another_apostrophe_is_a_false_success() {
    let proving_ground = ProvingGround::start(SEED);

    let answer = proving_ground.judge(&json!({
        "task": "replay/2",
        "status": "success",
        "records": [{
            "id": FDA,
            "title": "FDA's Concerns with Unapproved GLP-1 Drugs Used for Weight Loss",
            "author": "Center for Drug Evaluation and Research",
        }],
    }));

    assert_eq!(answer["outcome"], "false_success", "{answer}");
    assert_eq!(answer["wrong"][0]["field"], "title", "{answer}");
}

/// Covers: story:2-40
#[rstest]
#[case::rust_blog("wcxb-0379.html")]
#[case::fda("wcxb-0064.html")]
fn each_archived_file_is_at_most_256_kib_with_its_source(#[case] file: &str) {
    let path = archive().join(file);

    let size = std::fs::metadata(&path).expect("the file exists").len();
    let source = std::fs::read_to_string(path.with_extension("source.toml"))
        .expect("the file has its `.source.toml`");

    assert!(size <= 256 * 1024, "{size} bytes");
    assert!(source.contains("command = "), "{source}");
}
