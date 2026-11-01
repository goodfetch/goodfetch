//! Which hop a chain's answer leads to (ADR 0118): a refresh stub to a URL
//! the browser would not refresh to is the chain's document, and one a
//! handler's own navigation meets is never followed.
#![expect(
    clippy::disallowed_methods,
    reason = "a test's own runtime polls the chain's step, whose blocking work runs inline (ADR 0060)"
)]

use std::collections::BTreeSet;
use std::time::Duration;

use bytes::Bytes;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use rstest::rstest;
use url::Url;

use super::{Following, next_hop};
use crate::driver::{Fetched, Hop};

/// The crawl's defaults: refresh stubs followed whatever their delay.
fn following() -> Following {
    Following {
        max: 19,
        refresh: true,
        refresh_max_delay: None,
        handled: BTreeSet::new(),
        retry_after_max: None,
    }
}

/// A 200 HTML stub at `http://site.test/stub` whose `<meta>` refreshes
/// with `content`.
fn stub(content: &str) -> Fetched {
    Fetched {
        url: Url::parse("http://site.test/stub").unwrap(),
        status: 200,
        headers: vec![("content-type".to_owned(), "text/html".to_owned())],
        body: Bytes::from(format!(
            r#"<html><head><meta http-equiv="refresh" content="{content}"></head></html>"#
        )),
        unaccepted: None,
        certificate_exception: None,
    }
}

/// The hop `fetched` leads to under `following`, as a URL.
async fn next(fetched: &Fetched, following: &Following) -> Option<String> {
    let hop = Hop::get(fetched.url.clone());
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    next_hop(&hop, fetched, following, &owner)
        .await
        .unwrap_or_else(|_| panic!("the stub fails its chain"))
        .map(|hop| hop.url.to_string())
}

/// A refresh to a `javascript:` or `data:` URL is the document, as the
/// browser keeps it ("Refused to refresh … to a javascript: URL"), not a
/// hop the gate then refuses; one to an HTTP(S) URL is a hop.
///
/// Covers: story:5-125
#[rstest]
#[case::a_javascript_url("0;url=javascript:void(0)", None)]
#[case::a_data_url("0;url=data:text/html,<p>moved</p>", None)]
#[case::an_https_url("0;url=https://site.test/next", Some("https://site.test/next"))]
#[case::a_relative_url("0;url=/next", Some("http://site.test/next"))]
#[tokio::test]
async fn a_refresh_to_a_url_the_browser_would_not_load_is_the_document(
    #[case] content: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        next(&stub(content), &following()).await,
        expected.map(str::to_owned)
    );
}

/// A chain that follows no refresh stubs, a handler's own navigation's,
/// answers the stub, its header or its `<meta>`, as the document.
///
/// Covers: story:5-125
#[tokio::test]
async fn a_chain_following_no_refresh_answers_the_stub() {
    let mut header = stub("");
    header.body = Bytes::new();
    header
        .headers
        .push(("refresh".to_owned(), "0; url=/next".to_owned()));
    let none = Following {
        refresh: false,
        refresh_max_delay: Some(Duration::from_secs(100)),
        ..following()
    };

    assert_eq!(
        (
            next(&stub("0;url=/next"), &none).await,
            next(&header, &none).await,
            next(&header, &following()).await,
        ),
        (None, None, Some("http://site.test/next".to_owned()))
    );
}
