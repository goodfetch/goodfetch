//! A locator action's wait for the navigation of the page it starts (ADR
//! 0085), as Playwright's actions wait: the main frame's events are read
//! from just before the action's input, and a navigation of the main frame
//! the input requested in the page's own tab is waited for until its
//! document committed and the browser finished the navigation, or until it
//! ended without one. The wait holds neither the page's turn nor its
//! pointer, so that the engine's own clicks and a dialog the navigation
//! opens are handled meanwhile.
//!
//! The pinned Chromium's signals: the input's acknowledgement can come
//! before the main frame's `Page.frameRequestedNavigation`, which a round
//! trip into the renderer after the input orders before its answer, a
//! form's submission's included (Blink sends its request as it queues the
//! submission's task, in the input's own task), so the watch reads every
//! event the connection holds before it takes that answer; and the
//! renderer's `Page.frameNavigated` can come before the browser made the
//! new document the page's active one, which a round trip into the renderer
//! after the commit waits for, since the browser holds the commands it
//! sends the renderer from a navigation's start until it finished it.

use goodfetch_cdp::Subscription;
use goodfetch_cdp::generated::runtime;
use goodfetch_protocol::connection::WaitedFor;

use super::PageDriver;
use super::navigation::Navigation;

/// The main frame's events from just before a gesture's input on.
pub(crate) struct GestureWatch {
    events: Subscription,
    navigation: Navigation,
    /// Whether the connection ended, which sends no more events.
    ended: bool,
}

impl GestureWatch {
    /// Runs `gesture`, the input and what follows it in the page, while
    /// reading the main frame's events, so that the browser's connection
    /// never waits on this watch's unread ones. Every event the browser
    /// sent before the answer that ends `gesture` is read before it
    /// returns: the connection hands an event to its subscribers before it
    /// hands over a later answer, so the events are polled first (#1766).
    pub(crate) async fn during<T>(&mut self, gesture: impl Future<Output = T>) -> T {
        let mut gesture = std::pin::pin!(gesture);
        loop {
            tokio::select! {
                biased;
                event = self.events.next(), if !self.ended => match event {
                    Some(event) => self.navigation.feed(&event),
                    None => self.ended = true,
                },
                done = &mut gesture => return done,
            }
        }
    }
}

impl PageDriver {
    /// Starts watching the main frame for a navigation the gesture about to
    /// be sent requests.
    pub(crate) fn watch_gesture(&self) -> GestureWatch {
        GestureWatch {
            events: self.browser.connection.subscribe(Some(&self.session)),
            navigation: Navigation::of(self.frame.clone()),
            ended: false,
        }
    }

    /// Waits until the navigation of the main frame that the gesture
    /// `watch` watched requested has committed its document and the
    /// browser finished it, or ended without a document, within the page's
    /// navigation deadline, its waits for
    /// the gate's permits left out; at once when the gesture requested
    /// none. How it ended raises nothing here: a refused navigation is
    /// reported as no call waits on it, and the next call meets the
    /// document the page holds.
    pub(crate) async fn settle_gesture(&self, mut watch: GestureWatch) {
        let Some(url) = watch.navigation.requested().map(str::to_owned) else {
            return;
        };
        let settled = async {
            while !watch.navigation.requested_ended() {
                match watch.events.next().await {
                    Some(event) => watch.navigation.feed(&event),
                    None => return,
                }
            }
            if watch.navigation.crashed() {
                return;
            }
            // The main frame's document commits in its renderer before the
            // browser has made it the page's active one: until then the
            // browser refuses its own history commands for the page ("Not
            // attached to an active page"), and DevTools holds back what
            // it sends the renderer from the navigation's start until the
            // browser finished it. One round trip into the renderer thus
            // answers once the browser finished the navigation.
            let finished = self
                .browser
                .try_send(Some(&self.session), &runtime::GetIsolateIdParams {});
            if let Err(failure) = watch.during(finished).await {
                tracing::debug!(%failure, "the page's renderer did not answer after a gesture's navigation");
            }
        };
        if self
            .within_navigation_deadline(WaitedFor::NAVIGATION, Box::pin(settled))
            .await
            .is_err()
        {
            tracing::debug!(
                url.origin = goodfetch_engine::log::origin(&url),
                "the navigation a gesture started neither committed nor ended within `timeout.navigation`"
            );
        }
    }
}

#[cfg(test)]
mod tests;
