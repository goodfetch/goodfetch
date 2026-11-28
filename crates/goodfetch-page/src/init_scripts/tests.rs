//! A page's init scripts end with the request they belong to (ADR 0146).

use goodfetch_protocol::page::PageId;
use goodfetch_test_support::{TestEnv, runtime};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::InitScripts;
use crate::cdp::driver::replay::Played;
use crate::cdp::{CdpPage, TargetSession};
use crate::live_run::{LiveRun, played_page};

/// The session of the page's out-of-process iframe.
const IFRAME: &str = "iframe";

/// The browser's answer to `command`, on the session it was sent on,
/// `result`.
fn answered(command: &Value, result: &Value) -> Value {
    json!({ "id": command["id"], "sessionId": command["sessionId"], "result": result })
}

/// The next command `played` reads, answered with `result`: its session,
/// its method and the script it names.
async fn taken(played: &mut Played, result: &Value) -> (Value, Value, Value) {
    let command = played.command().await;
    assert!(played.send(&[answered(&command, result)]).await);
    (
        command["sessionId"].clone(),
        command["method"].clone(),
        command["params"]["identifier"].clone(),
    )
}

/// A replacement of the page's request part removes the earlier
/// request's script from each frame that holds it, once each: the page's
/// own session and its out-of-process iframe's; its call, dropped after,
/// removes nothing more.
#[test]
fn a_replaced_request_removes_its_script_once_from_the_page_and_each_iframe() {
    let first = PageId("page-1".to_owned());

    let (removals, left_after_drop) = runtime().block_on(async {
        let env = TestEnv::new();
        let (live, mut engine) = LiveRun::start(&env).await;
        let (driver, mut played, mut connection) = played_page();
        let cell = watch::Sender::new(first.clone());
        let cdp = CdpPage::of(&driver);
        let scripts = InitScripts::new(cdp.clone(), live.run.identity.clone(), cell.subscribe());
        let replaced = async {
            // An out-of-process iframe attached before the script, which
            // takes it once the page did.
            let iframe = cdp.child(&TargetSession(IFRAME.to_owned()));
            scripts.attached(iframe, CancellationToken::new()).await;
            let registering = scripts.register("1;".to_owned(), scripts.begin(), &first);
            let added = async {
                let on_page = taken(&mut played, &json!({ "identifier": "on-page" })).await;
                let on_iframe = taken(&mut played, &json!({ "identifier": "on-iframe" })).await;
                (on_page, on_iframe)
            };
            let (registered, (on_page, on_iframe)) = tokio::join!(registering, added);
            let registered = registered.expect("the page takes its init script");
            let add = json!("Page.addScriptToEvaluateOnNewDocument");
            assert_eq!((&on_page.0, &on_page.1), (&json!("page"), &add));
            assert_eq!((&on_iframe.0, &on_iframe.1), (&json!(IFRAME), &add));

            // The request part's one cell names the next request, as the
            // page's events write it, before the replacement.
            cell.send_replace(PageId("page-2".to_owned()));
            scripts.replace_request();
            let removals = [
                taken(&mut played, &json!({})).await,
                taken(&mut played, &json!({})).await,
            ];
            if let Some(settling) = scripts.settling() {
                settling.await;
            }
            drop(registered);
            (removals, scripts.settling().is_some())
        };
        let ran = tokio::select! {
            biased;
            ran = replaced => ran,
            () = &mut engine => panic!("the engine ended before the test"),
            () = &mut connection => panic!("the page's connection ended before the test"),
        };
        drop((scripts, live));
        // The Run ends once the page's reads did, at the connection's end.
        drop((driver, played));
        tokio::join!(engine, connection);
        ran
    });

    let remove = json!("Page.removeScriptToEvaluateOnNewDocument");
    assert_eq!(
        removals,
        [
            (json!("page"), remove.clone(), json!("on-page")),
            (json!(IFRAME), remove, json!("on-iframe")),
        ]
    );
    assert!(
        !left_after_drop,
        "the dropped call removed its script a second time"
    );
}
