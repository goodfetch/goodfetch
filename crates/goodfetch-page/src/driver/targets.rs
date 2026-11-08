//! What the Consent prompt runner drives besides a locator action (ADR
//! 0048): a page's frames' own targets, attached under the page, by their
//! sessions; calls into a world on any of them; the page's pointer, which
//! a locator action and the runner share for each gesture, a locator
//! action waiting for its target letting go of it between checks; and a trusted
//! click at a point of the page's viewport, wherever in its frames it lies.

use goodfetch_cdp::generated::{input, runtime};
use goodfetch_cdp::{Command, Failure, Refusal, SessionId as TargetSession, Subscription};
use goodfetch_engine::CallError;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use tokio::sync::MutexGuard;

use super::{PageDriver, Unanswered, world};

impl PageDriver {
    /// The page's own session.
    pub(crate) fn session(&self) -> &TargetSession {
        &self.session
    }

    /// The events of `session`, the page's or one of its frames' targets,
    /// from when this answers.
    pub(crate) fn events_on(&self, session: &TargetSession) -> Subscription {
        self.browser.connection.subscribe(Some(session))
    }

    /// Sends `params` on `session`, the page's or one of its frames'
    /// targets, recorded in the Run trace, answering the failure itself.
    pub(crate) async fn command_on<C: Command>(
        &self,
        session: &TargetSession,
        params: &C,
    ) -> Result<C::Returns, Failure> {
        self.browser.try_send(Some(session), params).await
    }

    /// Sends `params` on `session`, within the limit of a command of the
    /// page's opening, raising what such a command raises.
    pub(crate) async fn opening_command_on<C: Command>(
        &self,
        session: &TargetSession,
        params: &C,
    ) -> Result<C::Returns, CallError> {
        self.browser
            .send(Some(session), params, &self.browser.opening())
            .await
    }

    /// Calls `function`, a JavaScript function's source, with `argument` in
    /// the world whose context is `context` on `session`, and answers its
    /// value, never through `Runtime.enable`.
    pub(crate) async fn call_in(
        &self,
        session: &TargetSession,
        context: runtime::ExecutionContextId,
        function: &str,
        argument: serde_json::Value,
    ) -> Result<serde_json::Value, Unanswered> {
        match self
            .browser
            .try_send(Some(session), &world::call(function, argument, context))
            .await
        {
            Ok(called) => match called.exception_details {
                Some(thrown) => Err(Unanswered::Failed(CallError::protocol(
                    ProtocolFailure::InternalError,
                    format!("an engine script threw in the page: {}", thrown.text),
                ))),
                None => Ok(called.result.value.unwrap_or(serde_json::Value::Null)),
            },
            Err(Failure::Refused {
                refusal: Refusal::ContextGone,
                ..
            }) => Err(Unanswered::WorldGone),
            Err(failure) => Err(Unanswered::Failed(CallError::protocol(
                ProtocolFailure::InternalError,
                format!("an engine script's call failed: {failure}"),
            ))),
        }
    }

    /// The page's pointer, held for one gesture.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future gives up the place in the order.
    pub(crate) async fn gesture(&self) -> MutexGuard<'_, ()> {
        self.gesture.lock().await
    }

    /// Clicks at (`x`, `y`) in the page's viewport with the left button, as
    /// trusted input, wherever in the page's frames that point lies.
    pub(crate) async fn press(&self, x: f64, y: f64) -> Result<(), Failure> {
        for params in pointer_click(x, y) {
            self.browser.try_send(Some(&self.session), &params).await?;
        }
        Ok(())
    }

    /// Moves the pointer to (`x`, `y`) in the page's viewport, as trusted
    /// input, pressing nothing.
    pub(crate) async fn move_pointer(&self, x: f64, y: f64) -> Result<(), Failure> {
        let [moved, ..] = pointer_click(x, y);
        self.browser
            .try_send(Some(&self.session), &moved)
            .await
            .map(drop)
    }
}

/// The mouse events of a left click at (`x`, `y`) in the page's viewport:
/// the pointer moves there, presses and releases.
pub(super) fn pointer_click(x: f64, y: f64) -> [input::DispatchMouseEventParams; 3] {
    [
        (input::DispatchMouseEventType::MouseMoved, None, None),
        (
            input::DispatchMouseEventType::MousePressed,
            Some(input::MouseButton::Left),
            Some(1),
        ),
        (
            input::DispatchMouseEventType::MouseReleased,
            Some(input::MouseButton::Left),
            Some(0),
        ),
    ]
    .map(|(kind, button, buttons)| input::DispatchMouseEventParams {
        r#type: kind,
        x,
        y,
        modifiers: None,
        timestamp: None,
        button,
        buttons,
        click_count: button.map(|_| 1),
        force: None,
        tangential_pressure: None,
        tilt_x: None,
        tilt_y: None,
        twist: None,
        delta_x: None,
        delta_y: None,
        pointer_type: None,
    })
}
