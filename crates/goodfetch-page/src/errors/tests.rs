//! The table of fabricated CDP failures (ticket #695): each is what an
//! in-process browser end answers over a real CDP connection, then the
//! Error code the call that met it raises.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_cdp::{Connection, Failure, Framer};
use goodfetch_engine::deadline::Deadline;
use goodfetch_protocol::page::BrowserExit;
use goodfetch_protocol::settings::TIMEOUT_NAVIGATION;
use goodfetch_test_support::runtime;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream};

use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};

use super::{Call, call_error};

/// What the fabricated browser does with the command it reads.
#[derive(Clone, Copy, Debug)]
enum Browser {
    /// Answers with this error message.
    Refuses(&'static str),
    /// Never answers.
    Silent,
    /// Closes its end of the pipe.
    Exits,
}

/// Reads one command, then does what `browser` says.
async fn fabricate(mut stream: DuplexStream, browser: Browser) {
    let mut framer = Framer::default();
    let command: Value = loop {
        if let Some(bytes) = framer.next_frame().unwrap() {
            break serde_json::from_slice(&bytes).unwrap();
        }
        let mut buffer = vec![0; 4096];
        let count = stream.read(&mut buffer).await.unwrap();
        framer.push(&buffer[..count]);
    };
    match browser {
        Browser::Refuses(message) => {
            let mut bytes = serde_json::to_vec(
                &json!({ "id": command["id"], "error": { "code": -32000, "message": message } }),
            )
            .unwrap();
            bytes.push(0);
            stream.write_all(&bytes).await.unwrap();
            // The client ends the connection once it has its answer.
            let _ = stream.read(&mut [0; 1]).await;
        }
        Browser::Silent => {
            let _ = stream.read(&mut [0; 1]).await;
        }
        Browser::Exits => drop(stream),
    }
}

/// The failure a command meets when the browser does what `browser` says.
fn failure(browser: Browser) -> Failure {
    runtime().block_on(async {
        let (client, end) = tokio::io::duplex(1 << 16);
        let (reader, writer) = tokio::io::split(client);
        let (connection, driver) = Connection::new(reader, writer);
        let (_, failure, ()) = tokio::join!(
            driver.run(),
            async {
                let answer = connection
                    .send_raw(
                        "Page.navigate",
                        None,
                        json!({ "url": "http://catalog.test/" }),
                        Duration::from_secs(30),
                    )
                    .await;
                drop(connection);
                answer.unwrap_err()
            },
            fabricate(end, browser)
        );
        failure
    })
}

fn navigation() -> Call {
    Call::Navigation {
        url: "http://catalog.test/".to_owned(),
    }
}

fn document() -> Call {
    Call::Document
}

fn open_page() -> Call {
    Call::OpenPage {
        executable: "chrome".to_owned(),
    }
}

fn action() -> Call {
    Call::Action {
        operation: "locator.click",
    }
}

/// Covers: error:navigation_failed, error:page_crashed
#[rstest]
#[case::unknown_refusal_of_a_navigation(
    navigation(),
    Browser::Refuses("Something new"),
    "navigation_failed"
)]
#[case::invalid_url(
    navigation(),
    Browser::Refuses("Cannot navigate to invalid URL"),
    "navigation_failed"
)]
/// Covers: error:deadline_exceeded
#[case::navigation_timeout(navigation(), Browser::Silent, "deadline_exceeded")]
#[case::target_closed_under_a_navigation(
    navigation(),
    Browser::Refuses("Target closed"),
    "page_crashed"
)]
#[case::browser_lost_under_a_navigation(navigation(), Browser::Exits, "page_crashed")]
/// Covers: error:browser_launch_failed
#[case::unknown_refusal_of_a_new_page(
    open_page(),
    Browser::Refuses("Something new"),
    "browser_launch_failed"
)]
/// Covers: error:deadline_exceeded
#[case::new_page_timeout(open_page(), Browser::Silent, "deadline_exceeded")]
#[case::session_gone_under_a_new_page(
    open_page(),
    Browser::Refuses("Session with given id not found"),
    "page_crashed"
)]
#[case::browser_lost_under_a_new_page(open_page(), Browser::Exits, "page_crashed")]
#[case::unknown_refusal_of_a_document_read(
    document(),
    Browser::Refuses("Could not find node with given id"),
    "page_crashed"
)]
/// Covers: error:deadline_exceeded
#[case::document_read_timeout(document(), Browser::Silent, "deadline_exceeded")]
#[case::browser_lost_under_a_document_read(document(), Browser::Exits, "page_crashed")]
/// Covers: error:deadline_exceeded
#[case::action_timeout(action(), Browser::Silent, "deadline_exceeded")]
#[case::browser_lost_under_an_action(action(), Browser::Exits, "page_crashed")]
fn each_cdp_failure_raises_the_code_of_the_call_that_met_it(
    #[case] call: Call,
    #[case] browser: Browser,
    #[case] code: &str,
) {
    let deadline = Deadline::declared(&TIMEOUT_NAVIGATION);

    assert_eq!(
        call_error(&call, &failure(browser), None, &deadline).code(),
        code
    );
}

/// A browser lost before it came up, such as one that exits at start for a
/// missing library, raises `browser_launch_failed`, not retryable, naming
/// its executable and how it ended; a refusal while it comes up does too.
/// Covers: error:browser_launch_failed
#[rstest]
#[case::lost_at_start(Browser::Exits)]
#[case::refusing_at_start(Browser::Refuses("Target closed"))]
fn a_browser_lost_while_it_comes_up_raises_browser_launch_failed_naming_how_it_ended(
    #[case] browser: Browser,
) {
    let launch = Call::Launch {
        executable: "chrome".to_owned(),
    };
    let ended = BrowserExit {
        code: Some(127),
        signal: None,
    };

    let error = call_error(
        &launch,
        &failure(browser),
        Some(ended),
        &Deadline::declared(&TIMEOUT_NAVIGATION),
    );

    assert_eq!(
        (
            error.code(),
            error.retryable(),
            error.details().cloned().map(Value::Object)
        ),
        (
            "browser_launch_failed",
            false,
            Some(json!({ "executable": "chrome", "exit": { "code": 127, "signal": null } }))
        )
    );
}

/// A navigation through the page's history whose read failed names no URL,
/// since only the history names it, rather than an empty one.
/// Covers: error:navigation_failed
#[test]
fn a_failed_history_read_names_no_url() {
    let refused = call_error(
        &Call::History,
        &failure(Browser::Refuses("Something new")),
        None,
        &Deadline::declared(&TIMEOUT_NAVIGATION),
    );

    assert_eq!(
        (
            refused.code(),
            refused.details().and_then(|details| details.get("url"))
        ),
        ("navigation_failed", None)
    );
}

/// Chromium's own refusal of a certificate it verified itself, on a host
/// the relay tunnels, becomes `certificate_invalid` with the wire's name of
/// the failure (ADR 0127's translation point): one case per known failure
/// Chromium names apart, and an unknown one as its name in text.
/// Covers: error:certificate_invalid
#[rstest]
#[case::name_mismatch(
    "net::ERR_CERT_COMMON_NAME_INVALID",
    Some(known(KnownCertificateFailure::NameMismatch))
)]
#[case::unknown_issuer(
    "net::ERR_CERT_AUTHORITY_INVALID",
    Some(known(KnownCertificateFailure::UnknownIssuer))
)]
#[case::revoked("net::ERR_CERT_REVOKED", Some(known(KnownCertificateFailure::Revoked)))]
#[case::expired_or_not_yet_valid("net::ERR_CERT_DATE_INVALID", Some(text("date_invalid")))]
#[case::unknown("net::ERR_CERT_WEAK_KEY", Some(text("weak_key")))]
#[case::transparency(
    "net::ERR_CERTIFICATE_TRANSPARENCY_REQUIRED",
    Some(text("certificate_transparency_required"))
)]
#[case::no_certificate_failure("net::ERR_CONNECTION_REFUSED", None)]
fn a_certificate_chromium_refused_is_named_as_the_wire_names_it(
    #[case] error: &str,
    #[case] expected: Option<CertificateFailure>,
) {
    assert_eq!(super::certificate_failure(error), expected);
}

fn known(failure: KnownCertificateFailure) -> CertificateFailure {
    CertificateFailure::Known(failure)
}

fn text(name: &str) -> CertificateFailure {
    CertificateFailure::Unrecognized(name.to_owned())
}
