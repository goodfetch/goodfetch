//! A storage page's commit against a browser played in process: the order
//! in which the browser's pipe delivers the main frame's commit and
//! `Page.navigate`'s answer is replayed to the connection's driver.

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_test_support::runtime;
use serde_json::{Value, json};

use super::commit;
use crate::driver::Browser;
use crate::driver::replay::{self, FRAME, SESSION};

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
