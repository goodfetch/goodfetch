//! Partial results (ADR 0079) through the client crate: the corpus's script
//! of the `partial-results` client conformance case, which the TypeScript
//! and Python SDKs play too, with the Scripted engine as the Daemon. A
//! value of a declared type is complete: a navigation into a redirect loop
//! fails with `too_many_redirects`, whose typed details hold what the
//! navigation gathered before it failed, and a read of several fields into
//! the program's type answers each field's own outcome, a value or `None`
//! with why, never one error for the call. The Scripted engine fails on a
//! message it does not expect and on a step left unused, so its clean exit
//! says the crate sent exactly what the script holds. This is Rust's
//! `daemon` mode play.

use std::collections::BTreeMap;

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode, FieldValues};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::{DaemonEndpoint, HelloParams};
use goodfetch_protocol::page::read::{
    FieldLocator, FieldTransform, PageField, UnreadField, UnreadReason, ValueField,
};
use goodfetch_protocol::page::{RedirectLimit, TooManyRedirectsDetails};
use goodfetch_test_support::TestEnv;
use serde::Deserialize;
use serde_json::Number;

use crate::scripted_daemon::{daemon_hello, scripted_daemon_hearing};
use crate::start::{code_group_retryable, failure};

/// The kettle's fields as the program's type reads them: each one the
/// page may not give is an `Option`.
#[derive(Debug, Deserialize, PartialEq)]
struct Kettle {
    title: Option<String>,
    price: Option<Number>,
    rating: Option<Number>,
}

/// What the script's calls answered.
struct Played {
    /// The navigation into the redirect loop's failure.
    looping: Error,
    /// The read of the kettle's three fields.
    read: FieldValues<Kettle>,
}

/// The field `selector` finds, read by `transform`.
fn field(selector: &str, transform: FieldTransform) -> PageField {
    PageField::Value(ValueField {
        locator: FieldLocator::Css {
            selector: selector.to_owned(),
        },
        fallbacks: Vec::new(),
        attribute: None,
        matched: None,
        transform,
    })
}

/// The three fields the script's read declares.
fn kettle_fields() -> BTreeMap<String, PageField> {
    let number = || FieldTransform::Number {
        locale: "en".to_owned(),
    };
    BTreeMap::from([
        ("title".to_owned(), field("h1", FieldTransform::Text)),
        ("price".to_owned(), field(".price", number())),
        ("rating".to_owned(), field(".rating", number())),
    ])
}

/// Plays the script's calls on a Connection to the Daemon at `endpoint`.
async fn played(endpoint: DaemonEndpoint, hello: HelloParams) -> Played {
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        hello,
    )
    .await
    .unwrap();
    let run = connection.start().await.unwrap();
    let page = run.new_page().await.unwrap();
    let looping = failure(page.goto("https://shop.test/loop").await);
    let read = page.read(kettle_fields()).typed::<Kettle>().await.unwrap();
    run.finish().await.unwrap();
    connection.close().await.unwrap();
    Played { looping, read }
}

/// The `partial-results` client conformance case, played by the Scripted
/// engine's Daemon: a failed navigation's details hold the hops it
/// followed, and a read answers each field's own outcome.
///
/// Covers: story:11-35
#[test]
fn a_failure_carries_what_it_gathered_and_a_read_answers_each_field() {
    let env = TestEnv::new();
    let runtime = goodfetch_test_support::real_time_runtime();
    let hello = daemon_hello();
    let (process, endpoint) =
        scripted_daemon_hearing(&env, "sdk-rust/run/partial-results.json", &hello);
    let played = runtime.block_on(played(endpoint, hello));
    let status = process.child().wait().unwrap();

    let looping = &played.looping;
    assert_eq!(
        code_group_retryable(looping),
        (ErrorCode::TOO_MANY_REDIRECTS, ErrorGroup::Page, false)
    );
    assert_eq!(looping.operation(), Some("page.goto"));
    assert_eq!(
        looping.details::<TooManyRedirectsDetails>(),
        Some(TooManyRedirectsDetails {
            url: "https://shop.test/loop".to_owned(),
            hops: (1..20)
                .map(|hop| format!("https://shop.test/loop/{hop}"))
                .collect(),
            limit: RedirectLimit::Browser { count: 20 },
        }),
        "the error's details hold the hops the navigation followed before it failed"
    );
    assert_eq!(
        played.read.values,
        Kettle {
            title: Some("Blue kettle".to_owned()),
            price: None,
            rating: None,
        },
        "each field answers its own outcome"
    );
    assert_eq!(
        played.read.unread,
        [
            UnreadField {
                field: "/price".to_owned(),
                reason: UnreadReason::NoElement,
                text: None,
            },
            UnreadField {
                field: "/rating".to_owned(),
                reason: UnreadReason::NotParsed,
                text: Some("five stars".to_owned()),
            },
        ]
    );
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
