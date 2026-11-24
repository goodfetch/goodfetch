//! How a page's target ends without the engine closing its browser (ADR
//! 0113; spec #594's `TargetDestroyed`, `RendererCrash`, `BrowserQuit` and
//! `BrowserCrash`): its renderer's crash, its target destroyed, or its
//! browser's connection ending, told apart by the process's exit status for
//! a browser the engine launched and read as a quit for a Borrowed one
//! (decision 11). A browser's own connection is followed the same way, so
//! its pool learns of its loss whether or not it holds a page.

use std::time::Duration;

use goodfetch_protocol::page::BrowserExit;

use super::{Browser, PageDriver, navigation};

/// How soon after a target's end its browser's connection ends when the
/// browser quit: a quitting browser closes its windows, then exits at
/// once. It tells a quit from a tab closed in the browser, so it is the
/// browser's own pace, never a wait on a call's behalf, and no setting
/// moves it (ADR 0059's note of 2026-10-24).
const QUIT_WINDOW: Duration = Duration::from_secs(2);

/// The longest a page's end takes to be known once its target or its
/// browser's connection went: a destroyed target waits [`QUIT_WINDOW`] for
/// its browser's quit, and a lost launched browser [`QUIT_WINDOW`] more for
/// its exit status.
pub(crate) const ENDING_KNOWN_WITHIN: Duration = Duration::from_secs(2 * QUIT_WINDOW.as_secs());

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
        // Its page's reader ends it when the browser detached the page's
        // target, which the browser tells on its own session.
        let mut page = self.events();
        while let Some(event) = page.next().await {
            if event.method == navigation::CRASHED {
                return Lost::RendererCrashed;
            }
            if event.method == DETACHED {
                break;
            }
        }
        // A quitting browser closes its windows before it exits: a target
        // destroyed with the connection ending soon after is the quit's.
        if tokio::time::timeout(QUIT_WINDOW, self.browser.connection.closed())
            .await
            .is_err()
        {
            return Lost::Destroyed;
        }
        self.browser.loss().await
    }
}

impl Browser {
    /// Waits until the browser's connection ends, and says how the browser
    /// ended: the browser's own follower, which a browser with no page has
    /// too.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses nothing: it only reads.
    pub(crate) async fn ended(&self) -> Lost {
        self.connection.closed().await;
        self.loss().await
    }

    /// How the browser ended once its connection did: closed by the engine;
    /// a Borrowed browser quit, since no exit of its separates a quit from a
    /// crash (decision 11); a launched one quit when its process exited
    /// with status 0, and crashed otherwise or with no exit seen in time.
    async fn loss(&self) -> Lost {
        if !self.launched {
            if self.closing.is_cancelled() {
                return Lost::EngineClosed;
            }
            return Lost::BrowserQuit;
        }
        let mut exit = self.exit.clone();
        let ended = tokio::time::timeout(QUIT_WINDOW, exit.wait_for(Option::is_some))
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|ended| ended.clone());
        if self.closing.is_cancelled() {
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
