//! What the Consent prompt runner follows of a page's targets, each event
//! read once from a target's subscription, and the primitives it drives
//! them with (ADR 0048): its bundle put in the guard's world of every new
//! document, the world made and bound, a frame's owner located, a snippet
//! run in a frame's main world, and the pointer moved and pressed.

use goodfetch_cdp::Subscription;
use goodfetch_cdp::generated::{dom, page, runtime, target};
use goodfetch_engine::CallError;
use serde_json::Value;

use super::followed::FrameNavigated;
use super::paused::NotTaken;
use super::{CdpPage, TargetSession};

/// What the Consent prompt runner follows of one of a page's targets.
pub(crate) enum ConsentEvent {
    /// The out-of-process frame on this session detached.
    Detached(TargetSession),
    /// A frame committed a navigation.
    Navigated(FrameNavigated),
    /// The URL of `frame` changed within its document.
    WithinDocument { frame: String },
    /// `frame` detached.
    FrameDetached { frame: String },
    /// A world's binding `name` was called with `payload` in the world
    /// whose context is `context`.
    BindingCalled {
        name: String,
        context: i64,
        payload: String,
    },
}

impl ConsentEvent {
    fn of(event: &goodfetch_cdp::RawEvent) -> Option<Self> {
        if let Some(detached) = event.read::<target::DetachedFromTargetEvent>() {
            Some(Self::Detached(TargetSession(detached.session_id)))
        } else if let Some(navigated) = event.read::<page::FrameNavigatedEvent>() {
            Some(Self::Navigated(FrameNavigated::of(navigated)))
        } else if let Some(moved) = event.read::<page::NavigatedWithinDocumentEvent>() {
            Some(Self::WithinDocument {
                frame: moved.frame_id,
            })
        } else if let Some(detached) = event.read::<page::FrameDetachedEvent>() {
            Some(Self::FrameDetached {
                frame: detached.frame_id,
            })
        } else {
            event
                .read::<runtime::BindingCalledEvent>()
                .map(|called| Self::BindingCalled {
                    name: called.name,
                    context: called.execution_context_id,
                    payload: called.payload,
                })
        }
    }
}

/// One target's events the Consent prompt runner follows.
pub(crate) struct ConsentEvents(Subscription);

impl ConsentEvents {
    /// The next event the runner follows; `None` once the target went.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub(crate) async fn next(&mut self) -> Option<ConsentEvent> {
        while let Some(event) = self.0.next().await {
            if let Some(followed) = ConsentEvent::of(&event) {
                return Some(followed);
            }
        }
        None
    }
}

impl CdpPage {
    /// The events the Consent prompt runner follows of `session`, the
    /// page's target or one of its out-of-process frames', from now.
    pub(crate) fn consent_events(&self, session: &TargetSession) -> ConsentEvents {
        ConsentEvents(self.page.events_on(session))
    }

    /// Adds `source` to the scripts run in the world `world` of every
    /// document the page's target loads from now on, and of those it holds
    /// now, within a deadline of opening a page.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn add_world_script_opening(
        &self,
        source: String,
        world: &str,
    ) -> Result<(), CallError> {
        self.page
            .opening_command(&page::AddScriptToEvaluateOnNewDocumentParams {
                source,
                world_name: Some(world.to_owned()),
                include_command_line_api: None,
                run_immediately: Some(true),
            })
            .await
            .map(drop)
    }

    /// Has the page's target report its frames' navigations and detaching,
    /// from now, within a deadline of opening a page.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn follow_frames_opening(&self) -> Result<(), CallError> {
        self.page
            .opening_command(&page::EnableParams::default())
            .await
            .map(drop)
    }

    /// Makes the world `world` in the current document of `frame` on
    /// `session`, or finds the one there: answers its context.
    ///
    /// # Errors
    ///
    /// When the frame or its document went.
    pub(crate) async fn isolated_world(
        &self,
        (session, frame): (&TargetSession, &str),
        world: &str,
    ) -> Result<i64, NotTaken> {
        let made = self
            .page
            .command_on(
                session,
                &page::CreateIsolatedWorldParams {
                    frame_id: frame.to_owned(),
                    world_name: Some(world.to_owned()),
                    grant_univeral_access: None,
                    content_security_policy: None,
                },
            )
            .await?;
        Ok(made.execution_context_id)
    }

    /// Binds `binding` in every world named `world` on `session`, whose
    /// calls [`ConsentEvent::BindingCalled`] reports.
    ///
    /// # Errors
    ///
    /// When the target went.
    pub(crate) async fn bind(
        &self,
        session: &TargetSession,
        binding: &str,
        world: &str,
    ) -> Result<(), NotTaken> {
        self.page
            .command_on(
                session,
                &runtime::AddBindingParams {
                    name: binding.to_owned(),
                    execution_context_id: None,
                    execution_context_name: Some(world.to_owned()),
                },
            )
            .await?;
        Ok(())
    }

    /// Where the content box of `frame`'s owner element, on `session`, its
    /// parent's target, starts in the parent's viewport, the owner scrolled
    /// into the parent's view first.
    ///
    /// # Errors
    ///
    /// When the frame or its owner went.
    pub(crate) async fn frame_owner_origin(
        &self,
        (session, frame): (&TargetSession, &str),
    ) -> Result<(f64, f64), NotTaken> {
        let owner = self
            .page
            .command_on(
                session,
                &dom::GetFrameOwnerParams {
                    frame_id: frame.to_owned(),
                },
            )
            .await?;
        let _ = self
            .page
            .command_on(
                session,
                &dom::ScrollIntoViewIfNeededParams {
                    node_id: None,
                    backend_node_id: Some(owner.backend_node_id),
                    object_id: None,
                    rect: None,
                },
            )
            .await;
        let model = self
            .page
            .command_on(
                session,
                &dom::GetBoxModelParams {
                    node_id: None,
                    backend_node_id: Some(owner.backend_node_id),
                    object_id: None,
                },
            )
            .await?
            .model;
        Ok((
            model.content.first().copied().unwrap_or_default(),
            model.content.get(1).copied().unwrap_or_default(),
        ))
    }

    /// Runs `code` in the main world of the document whose guard world is
    /// `context` on `session`, with `Runtime` off: the guard's world's
    /// `document` resolved, by its node, into the main world, where the
    /// code is called on it; whether it answered `true`.
    ///
    /// # Errors
    ///
    /// When the document went.
    pub(crate) async fn true_in_main_world(
        &self,
        (session, context): (&TargetSession, i64),
        code: &str,
    ) -> Result<bool, NotTaken> {
        let document = self
            .page
            .command_on(
                session,
                &runtime::EvaluateParams {
                    expression: "document".to_owned(),
                    object_group: None,
                    include_command_line_api: None,
                    silent: Some(true),
                    context_id: Some(context),
                    return_by_value: Some(false),
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
                },
            )
            .await?;
        let Some(isolated) = document.result.object_id else {
            return Ok(false);
        };
        let node = self
            .page
            .command_on(
                session,
                &dom::DescribeNodeParams {
                    node_id: None,
                    backend_node_id: None,
                    object_id: Some(isolated.clone()),
                    depth: None,
                    pierce: None,
                },
            )
            .await;
        self.release(session, isolated).await;
        let resolved = self
            .page
            .command_on(
                session,
                &dom::ResolveNodeParams {
                    node_id: None,
                    backend_node_id: Some(node?.node.backend_node_id),
                    object_group: None,
                    execution_context_id: None,
                },
            )
            .await?;
        let Some(main) = resolved.object.object_id else {
            return Ok(false);
        };
        let called = self
            .page
            .command_on(
                session,
                &runtime::CallFunctionOnParams {
                    function_declaration: format!("function () {{ return Boolean({code}); }}"),
                    object_id: Some(main.clone()),
                    arguments: None,
                    silent: Some(true),
                    return_by_value: Some(true),
                    generate_preview: None,
                    user_gesture: None,
                    await_promise: Some(true),
                    execution_context_id: None,
                    object_group: None,
                    throw_on_side_effect: None,
                    unique_context_id: None,
                    serialization_options: None,
                },
            )
            .await;
        self.release(session, main).await;
        let called = called?;
        Ok(called.exception_details.is_none() && called.result.value == Some(Value::Bool(true)))
    }

    /// Lets the browser forget `object` of `session`.
    async fn release(&self, session: &TargetSession, object: runtime::RemoteObjectId) {
        let _ = self
            .page
            .command_on(session, &runtime::ReleaseObjectParams { object_id: object })
            .await;
    }

    /// Clicks at (`x`, `y`) in the page's viewport with the left button, as
    /// trusted input, wherever in the page's frames that point lies, with
    /// no deadline: one burst, as [`Self::pointer_click`]'s.
    ///
    /// # Errors
    ///
    /// When the page went.
    ///
    /// # Cancel safety
    ///
    /// As [`Self::pointer_click`]'s.
    pub(crate) async fn pointer_press(&self, x: f64, y: f64) -> Result<(), NotTaken> {
        self.page.press(x, y).await.map_err(NotTaken)
    }

    /// Moves the pointer to (`x`, `y`) in the page's viewport, as trusted
    /// input, pressing nothing.
    ///
    /// # Errors
    ///
    /// When the page went.
    pub(crate) async fn pointer_move(&self, x: f64, y: f64) -> Result<(), NotTaken> {
        self.page.move_pointer(x, y).await.map_err(NotTaken)
    }
}
