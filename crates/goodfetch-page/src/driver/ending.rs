//! How a page's target ends without the engine closing its browser (ADR
//! 0113; spec #594's `TargetDestroyed`, `RendererCrash`, `BrowserQuit` and
//! `BrowserCrash`): its renderer's crash, its target destroyed, or its
//! browser's connection ending, told apart by the process's exit status.

use goodfetch_cdp::generated::target;
use goodfetch_protocol::page::BrowserExit;

use super::{EXIT_WAIT, PageDriver, navigation};

/// How a page's target ended when the engine did not close its browser
/// (ADR 0113; spec #594's `TargetDestroyed`, `RendererCrash`, `BrowserQuit`
/// and `BrowserCrash`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Lost {
    /// Its renderer crashed; the target stays.
    RendererCrashed,
    /// Its target was destroyed: closed in the browser, or by the engine's
    /// own close of its context.
    Destroyed,
    /// Its browser's connection ended and the process exited with status 0:
    /// the browser quit.
    BrowserQuit,
    /// Its browser's connection ended with no exit status 0: the process
    /// exited non-zero or by a signal, or none was seen in time.
    BrowserCrashed(Option<BrowserExit>),
    /// The engine closed its browser.
    EngineClosed,
}

/// The event of a target whose debugging session was detached, on the
/// target's own session.
const DETACHED: &str = "Inspector.detached";

impl PageDriver {
    /// Waits until the page's target ends other than by the engine closing
    /// its browser, and says how.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing: it only reads.
    pub(crate) async fn lost(&self) -> Lost {
        let connection = &self.browser.connection;
        let mut page = connection.subscribe(Some(&self.session)).await;
        let mut root = connection.subscribe(None).await;
        loop {
            tokio::select! {
                event = page.next() => match event {
                    Some(event) if event.method == navigation::CRASHED => return Lost::RendererCrashed,
                    Some(event) if event.method == DETACHED => break,
                    Some(_) => {}
                    None => break,
                },
                event = root.next() => match event {
                    Some(event) => {
                        if event
                            .read::<target::DetachedFromTargetEvent>()
                            .is_some_and(|detached| detached.session_id == self.session.0)
                        {
                            break;
                        }
                    }
                    None => break,
                },
            }
        }
        // A quitting browser closes its windows before it exits: a target
        // destroyed with the connection ending soon after is the quit's.
        let ending = async { while root.next().await.is_some() {} };
        if tokio::time::timeout(EXIT_WAIT, ending).await.is_err() {
            return Lost::Destroyed;
        }
        let mut exit = self.browser.exit.clone();
        let ended = tokio::time::timeout(EXIT_WAIT, exit.wait_for(Option::is_some))
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|ended| ended.clone());
        if self.browser.closing.is_cancelled() {
            return Lost::EngineClosed;
        }
        match ended {
            Some(BrowserExit {
                code: Some(0),
                signal: None,
            }) => Lost::BrowserQuit,
            other => Lost::BrowserCrashed(other),
        }
    }
}
