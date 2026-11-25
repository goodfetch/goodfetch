//! The program's test, on Collie's testing kit: a test engine whose Runs
//! reach only the hosts `network.resolve` maps to loopback, here the page
//! this test serves itself, so it needs no network.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use goodfetch_collie::settings::{Settings, network};
use goodfetch_collie::testing::{TestEngine, TestEngineOptions};

use super::read_title;

const PAGE: &str = "<!doctype html><title>Hello from the starter</title>";

/// Serves `PAGE` on a loopback port, whatever the request, and answers the
/// port.
fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{PAGE}",
                PAGE.len()
            );
            let _ = reader.get_mut().write_all(response.as_bytes());
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_the_pages_title() {
    let port = serve();
    let resolve = BTreeMap::from([("starter.test".to_owned(), vec!["127.0.0.1".to_owned()])]);
    let options = TestEngineOptions::new().settings(Settings::new().set(network::RESOLVE, resolve));
    let engine = TestEngine::start(options).await.unwrap();

    let title = read_title(engine.connection(), &format!("http://starter.test:{port}/")).await;
    engine.close().await.unwrap();

    assert_eq!(title.unwrap(), "Hello from the starter");
}
