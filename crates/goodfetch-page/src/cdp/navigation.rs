//! The primitives a Page's navigation sends and follows (ADR 0146): the
//! history entry it goes to, the command that starts it and the page's
//! events it follows until it ends or reaches the state it waits for, a
//! renderer that crashed under it told apart, and the encoding its
//! document was decoded with, read in a world, never through
//! `Runtime.enable` (ADRs 0019 and 0119).

use goodfetch_cdp::generated::{page, runtime};
use goodfetch_cdp::{Failure, Subscription};
use goodfetch_engine::CallError;
use goodfetch_protocol::page::WaitUntil;

use super::CdpPage;
use super::driver::detached;
use super::driver::navigation::{
    Arrived, Ended, Expected, Followed, Navigation, abandoned, navigated,
};
use super::paused::NotTaken;

use crate::errors::Call;

/// The expression that reads the encoding a document was decoded with.
const DOCUMENT_ENCODING: &str = "document.characterSet";

/// Where a navigation goes: its URL, and the command that goes there.
pub(crate) struct Going {
    url: String,
    step: Step,
}

/// The command that starts a navigation.
enum Step {
    /// `Page.navigate`.
    Navigate,
    /// `Page.reload`.
    Reload,
    /// `Page.navigateToHistoryEntry` to this entry.
    Entry(i64),
}

impl Going {
    /// A navigation to `url`.
    pub(crate) fn to(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            step: Step::Navigate,
        }
    }

    /// The URL it goes to.
    pub(crate) fn url(&self) -> &str {
        &self.url
    }
}

/// The page's events a navigation follows, read from before it starts.
pub(crate) struct NavigationEvents(Subscription);

impl CdpPage {
    /// Where a navigation `offset` entries away in the page's history goes:
    /// the current entry again for 0, by a reload; none for an entry the
    /// page does not have.
    ///
    /// # Errors
    ///
    /// The browser's failure as reading the history's code.
    pub(crate) async fn history_entry(&self, offset: i64) -> Result<Option<Going>, CallError> {
        let history = self
            .page
            .command_for(&page::GetNavigationHistoryParams {}, &Call::History)
            .await?;
        let Some(entry) = usize::try_from(history.current_index + offset)
            .ok()
            .and_then(|index| history.entries.get(index))
        else {
            return Ok(None);
        };
        let step = if offset == 0 {
            Step::Reload
        } else {
            Step::Entry(entry.id)
        };
        Ok(Some(Going {
            url: entry.url.clone(),
            step,
        }))
    }

    /// The page's events from now, which a navigation opens before it
    /// starts, so it misses none of its own.
    pub(crate) fn navigation_events(&self) -> NavigationEvents {
        NavigationEvents(self.page.events())
    }

    /// Starts the navigation `going` names, as the call `call`, and follows
    /// it through `events`, read while its command runs too, until it ends
    /// or reaches the state `wait_until` names. `superseded` says the
    /// engine started another navigation of the page after it.
    ///
    /// # Errors
    ///
    /// The code of `call` when the browser refused its command or its
    /// connection ended.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the navigation to go on in the browser.
    pub(crate) async fn navigate(
        &self,
        going: &Going,
        events: &mut NavigationEvents,
        (call, wait_until): (&Call, WaitUntil),
        superseded: &(impl Fn() -> bool + Sync),
    ) -> Result<Result<Arrived, Ended>, CallError> {
        let command = async {
            Ok(match going.step {
                Step::Navigate => {
                    let answer = self
                        .page
                        .command_for(
                            &page::NavigateParams {
                                url: going.url.clone(),
                                referrer: None,
                                transition_type: None,
                                frame_id: None,
                                referrer_policy: None,
                            },
                            call,
                        )
                        .await?;
                    navigated(&going.url, answer, superseded())
                }
                Step::Reload => {
                    self.page
                        .command_for(&page::ReloadParams::default(), call)
                        .await?;
                    Ok(Some(Expected::FirstStarted))
                }
                Step::Entry(entry_id) => {
                    self.page
                        .command_for(&page::NavigateToHistoryEntryParams { entry_id }, call)
                        .await?;
                    Ok(Some(Expected::FirstStarted))
                }
            })
        };
        self.follow(command, &mut events.0, call, wait_until, superseded)
            .await
    }

    /// The encoding the document of the world whose context is `context`
    /// was decoded with; none when it names none.
    ///
    /// # Errors
    ///
    /// When the world's document went, or the page's target.
    pub(crate) async fn document_encoding(&self, context: i64) -> Result<Option<String>, NotTaken> {
        let evaluated = self.page.command(&encoding_read(context)).await?;
        Ok(evaluated
            .result
            .value
            .and_then(|value| value.as_str().map(str::to_owned)))
    }

    /// Follows the navigation `command` starts through the page's `events`,
    /// read while the command runs too, until it ends or reaches the state
    /// `wait_until` names.
    async fn follow(
        &self,
        command: impl Future<Output = Result<Result<Option<Expected>, Ended>, CallError>>,
        events: &mut Subscription,
        call: &Call,
        wait_until: WaitUntil,
        superseded: impl Fn() -> bool,
    ) -> Result<Result<Arrived, Ended>, CallError> {
        let mut navigation = Navigation::of(self.page.main_frame().clone());
        let command = self.page.after_events(command);
        let (outcome, expected) = match navigation
            .follow(command, events, wait_until, &superseded)
            .await?
        {
            Followed::Ended { outcome, expected } => (outcome, expected),
            Followed::Disconnected => {
                return Err(self.page.raised(call, &Failure::Disconnected).await);
            }
        };
        match outcome {
            Err(Ended::Failed(error)) if abandoned(&error) => {
                if self.renderer_crashed(events, &mut navigation, call).await? {
                    return Ok(Err(Ended::Crashed));
                }
                // Every event the browser sent before the renderer answered
                // is read now: a navigation the page started that cancelled
                // this one has started by then.
                Ok(Err(expected
                    .and_then(|expected| navigation.outcome(&expected, wait_until, superseded()))
                    .and_then(Result::err)
                    .unwrap_or(Ended::Failed(error))))
            }
            outcome => Ok(outcome),
        }
    }

    /// Whether the renderer of a navigation the browser abandoned crashed;
    /// else it is the browser's own failure of it, such as for an answer
    /// with no document, or a navigation the page started superseded it.
    /// The pinned Chromium abandons the navigation of a renderer that
    /// crashes before it reports the crash, once the crash's dump is
    /// written (~200 ms idle, longer under load), so the page's renderer is
    /// asked for its frame tree, the main frame's events read meanwhile: a
    /// live one answers, and a crashed one never does before its crash is
    /// reported. A target the browser detached meanwhile, as when the
    /// engine closes its browser, did not crash: the frame tree's answer,
    /// failed with it, ends the check, so the navigation raises its own
    /// failure, such as a Limit's that stopped the Run.
    async fn renderer_crashed(
        &self,
        events: &mut Subscription,
        navigation: &mut Navigation,
        call: &Call,
    ) -> Result<bool, CallError> {
        let alive = self.page.command(&page::GetFrameTreeParams {});
        let mut alive = std::pin::pin!(alive);
        let mut reading = true;
        loop {
            if navigation.crashed() {
                return Ok(true);
            }
            tokio::select! {
                biased;
                event = events.next(), if reading => match event {
                    Some(event) => navigation.feed(&event),
                    None if detached(events) => reading = false,
                    None => return Err(self.page.raised(call, &Failure::Disconnected).await),
                },
                answered = &mut alive => return match answered {
                    Err(Failure::Disconnected) => {
                        Err(self.page.raised(call, &Failure::Disconnected).await)
                    }
                    Ok(_) | Err(_) => Ok(false),
                },
            }
        }
    }
}

/// The command that reads the document's encoding in the world whose
/// context is `context`, never through `Runtime.enable` (ADR 0019).
fn encoding_read(context: runtime::ExecutionContextId) -> runtime::EvaluateParams {
    runtime::EvaluateParams {
        expression: DOCUMENT_ENCODING.to_owned(),
        object_group: None,
        include_command_line_api: None,
        silent: Some(true),
        context_id: Some(context),
        return_by_value: Some(true),
        generate_preview: None,
        user_gesture: None,
        await_promise: None,
        throw_on_side_effect: None,
        timeout: None,
        disable_breaks: None,
        repl_mode: None,
        allow_unsafe_eval_blocked_by_csp: None,
        unique_context_id: None,
        serialization_options: None,
    }
}
