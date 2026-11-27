//! A storage page's commit against a browser played in process: the order
//! in which the browser's pipe delivers the main frame's commit and
//! `Page.navigate`'s answer is replayed to the connection's driver.

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_engine::CallError;
use goodfetch_test_support::runtime;
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{Origins, commit};
use crate::cdp::driver::page_reader::PageReader;
use crate::cdp::driver::replay::{self, FRAME, SESSION};
use crate::cdp::driver::{Browser, Connect, SessionDriver, SessionEgress};

/// The origin whose storage the page carries.
const ORIGIN: &str = "https://site.test";

/// How many times the browser's order is replayed, so that the wait takes
/// the commit first in some and the answer first in others.
const REPLAYS: usize = 64;

/// The main frame's commit of the origin's empty document.
fn committed_message() -> Value {
    replay::event(
        "Page.frameNavigated",
        json!({
            "frame": {
                "id": FRAME,
                "loaderId": "loader-o",
                "url": format!("{ORIGIN}/"),
                "domainAndRegistry": "site.test",
                "securityOrigin": ORIGIN,
                "mimeType": "text/html",
                "secureContextType": "Secure",
                "crossOriginIsolatedContextType": "NotIsolated",
                "gatedAPIFeatures": [],
            },
            "type": "Navigation",
        }),
    )
}

/// The played browser a Session's driver connects to.
struct Played(Browser);

impl Connect for Played {
    fn connect(&self) -> impl Future<Output = Result<Browser, CallError>> + Send {
        std::future::ready(Ok(self.0.clone()))
    }
}

/// How many turns of the runtime a watcher's task lags: far more than a
/// capture's commands take to answer in process.
const LAG: usize = 10_000;

/// An origin the browser reported before it answered the capture's first
/// command is captured, however far behind its watcher's task runs: the
/// capture waits for the watcher, so it goes on to open a page of the origin
/// to read its storage.
#[test]
fn a_capture_waits_for_a_watcher_behind_the_browsers_answer() {
    runtime().block_on(async {
        let ((connection, driver), mut played) = replay::connected();
        let browser = Browser::played(connection);
        let session = TargetSession(SESSION.to_owned());
        let origins = Origins::default();
        let watcher =
            PageReader::new(&browser.connection, &session).reading(Some(origins.clone().watcher()));
        let session_driver = SessionDriver::new(Played(browser.clone()), SessionEgress::default());
        session_driver
            .context
            .set("context".to_owned())
            .expect("invariant: the driver was just made");
        let captured = Notify::new();
        let (_, (), (), asked) = tokio::join!(
            driver.run(),
            async {
                for _ in 0..LAG {
                    tokio::task::yield_now().await;
                }
                watcher.await;
            },
            async {
                // Refused below, the page of the origin ends the capture.
                let _ = session_driver.capture(&origins).await;
                captured.notify_one();
                drop((session_driver, browser));
            },
            async {
                let command = played.command().await;
                let answer = replay::answer(&command, json!({ "cookies": [] }));
                assert!(
                    played.send(&[committed_message(), answer]).await,
                    "the client left"
                );
                let asked = tokio::select! {
                    command = played.command() => Some(command),
                    () = captured.notified() => None,
                };
                if let Some(command) = &asked {
                    let refused = json!({
                        "id": command["id"],
                        "error": { "code": -32000, "message": "refused" },
                    });
                    assert!(played.send(&[refused]).await, "the client left");
                    captured.notified().await;
                }
                drop(played);
                asked.map(|command| command["method"].clone())
            },
        );

        assert_eq!(asked, Some(json!("Target.createTarget")));
    });
}

/// One replay of a storage page's commit whose `Page.navigate` the browser
/// answers with `navigated`, in one write after the events `before`:
/// whether the page committed the origin's document.
async fn committed_once(before: &[Value], navigated: &Value) -> bool {
    let ((connection, driver), mut played) = replay::connected();
    let browser = Browser::played(connection);
    let session = TargetSession(SESSION.to_owned());
    let mut events = browser.connection.subscribe(Some(&session));
    let call = browser.opening();
    let (_, committed, ()) = tokio::join!(
        driver.run(),
        async {
            let committed = commit(&browser, &session, &mut events, ORIGIN, &call)
                .await
                .expect("invariant: the played browser answers");
            drop((events, browser));
            committed
        },
        async {
            let command = played.command().await;
            let mut messages = before.to_vec();
            messages.push(replay::answer(&command, navigated.clone()));
            assert!(played.send(&messages).await, "the client left");
            played.hold().await;
        }
    );
    committed
}

/// A storage page's commit ends as the browser's answer and the main
/// frame's commit say, whichever of the two the wait reads first (#1770):
/// it waits for both, and a refused navigation ends it whatever came
/// before, so the order the wait polls them in cannot change its end.
#[rstest::rstest]
#[case::committed_then_answered(json!({ "frameId": FRAME, "loaderId": "loader-o" }), true)]
#[case::committed_then_refused(
    json!({ "frameId": FRAME, "loaderId": "loader-o", "errorText": "net::ERR_ABORTED" }),
    false
)]
fn a_storage_pages_commit_ends_alike_whichever_it_reads_first(
    #[case] navigated: Value,
    #[case] committed: bool,
) {
    runtime().block_on(async {
        let mut ends = Vec::new();
        for _ in 0..REPLAYS {
            ends.push(committed_once(&[committed_message()], &navigated).await);
        }

        assert_eq!(ends, [committed; REPLAYS]);
    });
}

/// Carried cookies and `localStorage` round-trip through JSON bytes, report
/// emptiness accurately, and seed an `Origins` set with web origins alone.
#[test]
fn carried_state_round_trips_and_seeds_web_origins() {
    use super::Carried;
    use goodfetch_cdp::generated::network;

    assert!(Carried::default().is_empty());

    let carried = Carried {
        cookies: vec![network::CookieParam {
            name: "mark".to_owned(),
            value: "token-1".to_owned(),
            url: None,
            domain: Some("scope.test".to_owned()),
            path: Some("/".to_owned()),
            secure: Some(false),
            http_only: Some(false),
            same_site: Some(network::CookieSameSite::Lax),
            expires: None,
            priority: None,
            source_scheme: None,
            source_port: None,
            partition_key: None,
        }],
        storage: vec![(
            ORIGIN.to_owned(),
            vec![("mark".to_owned(), "token-1".to_owned())],
        )],
    };
    assert!(!carried.is_empty());

    let bytes = carried.to_bytes().expect("carried state encodes");
    let decoded = Carried::from_bytes(&bytes).expect("carried state decodes");
    assert_eq!(decoded, carried);
    assert_eq!(
        decoded.storage_map().get(ORIGIN),
        Some(&vec![("mark".to_owned(), "token-1".to_owned())])
    );

    let origins = Origins::default();
    origins.seed(
        decoded
            .origins()
            .chain(std::iter::once("about:blank".to_owned())),
    );
    assert_eq!(origins.list(), vec![ORIGIN.to_owned()]);
}
