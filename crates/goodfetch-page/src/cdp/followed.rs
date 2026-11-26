//! What a page's events follow of the browser's: its requests, their
//! responses and bodies, its sockets, its main frame's document, and its
//! console calls and uncaught exceptions, each read once from the page's
//! subscription, or from one of its out-of-process frames'; and the
//! primitives that turn them on and read a body.

use std::collections::BTreeMap;

use goodfetch_cdp::generated::{network, page, runtime};
use goodfetch_cdp::{RawEvent, Subscription, SubscriptionEnd};
use goodfetch_engine::CallError;
use goodfetch_engine::mask::Masks;
use goodfetch_protocol::page::events::{ConsoleMessage, WebError};
use goodfetch_protocol::page::{PageId, PageResponse};

use super::paused::{LoadingFailed, NotTaken, is_navigation, post_data, resource_type};
use super::runtime_values::{console_message, web_error};
use super::{CdpPage, TargetSession};
use crate::driver::cdp_response::{bytes_of, headers, response_of};

/// What a page's events follow of the page's own events, or of one of its
/// out-of-process frames'.
#[expect(
    clippy::large_enum_variant,
    reason = "each event lives on the stack for one take-in, and boxing a sent request would allocate once per request"
)]
pub(crate) enum FollowedEvent {
    /// A request was sent, or its next hop after a redirect.
    RequestSent(RequestSent),
    /// A request's response arrived.
    ResponseReceived(ResponseReceived),
    /// `length` decoded bytes of the body of the request's hop
    /// `network_id` arrived.
    DataReceived { network_id: String, length: u64 },
    /// The request's hop `network_id` finished.
    LoadingFinished { network_id: String },
    /// A request failed.
    LoadingFailed(LoadingFailed),
    /// The page opened the WebSocket `id` to `url`.
    SocketCreated { id: String, url: String },
    /// The page sent a frame on the WebSocket `id`.
    SocketFrameSent { id: String, frame: SocketFrame },
    /// A frame arrived on the WebSocket `id`.
    SocketFrameReceived { id: String, frame: SocketFrame },
    /// The WebSocket `id` closed.
    SocketClosed { id: String },
    /// The page's document fired `DOMContentLoaded`.
    DomContentLoaded,
    /// The page's document fired `load`.
    Loaded,
    /// A console call.
    ConsoleCalled(ConsoleCalled),
    /// An uncaught exception.
    ExceptionThrown(ExceptionThrown),
    /// A frame committed a navigation.
    FrameNavigated(FrameNavigated),
    /// A frame's URL changed within its document, to `url`.
    NavigatedWithinDocument { frame: String, url: String },
    /// A frame's document, loaded by `loader`, reached the lifecycle event
    /// `name`.
    Lifecycle {
        frame: String,
        loader: String,
        name: String,
    },
}

impl FollowedEvent {
    /// The event `event` is, when the page's events follow it.
    fn of(event: &RawEvent) -> Option<Self> {
        if let Some(failed) = event.read::<network::LoadingFailedEvent>() {
            Some(Self::LoadingFailed(LoadingFailed(failed)))
        } else if let Some(sent) = event.read::<network::RequestWillBeSentEvent>() {
            Some(Self::RequestSent(RequestSent(sent)))
        } else if let Some(received) = event.read::<network::ResponseReceivedEvent>() {
            Some(Self::ResponseReceived(ResponseReceived(received)))
        } else if let Some(data) = event.read::<network::DataReceivedEvent>() {
            Some(Self::DataReceived {
                network_id: data.request_id,
                length: u64::try_from(data.data_length).unwrap_or_default(),
            })
        } else if let Some(finished) = event.read::<network::LoadingFinishedEvent>() {
            Some(Self::LoadingFinished {
                network_id: finished.request_id,
            })
        } else {
            Self::of_document(event).or_else(|| Self::of_other(event))
        }
    }

    /// The event of the main frame's document `event` is, if any.
    fn of_document(event: &RawEvent) -> Option<Self> {
        if let Some(navigated) = event.read::<page::FrameNavigatedEvent>() {
            Some(Self::FrameNavigated(FrameNavigated::of(navigated)))
        } else if let Some(within) = event.read::<page::NavigatedWithinDocumentEvent>() {
            Some(Self::NavigatedWithinDocument {
                frame: within.frame_id,
                url: within.url,
            })
        } else {
            event
                .read::<page::LifecycleEventEvent>()
                .map(|lifecycle| Self::Lifecycle {
                    frame: lifecycle.frame_id,
                    loader: lifecycle.loader_id,
                    name: lifecycle.name,
                })
        }
    }

    /// The event of a socket, of the page's document or of its runtime
    /// `event` is, if any.
    fn of_other(event: &RawEvent) -> Option<Self> {
        if let Some(created) = event.read::<network::WebSocketCreatedEvent>() {
            Some(Self::SocketCreated {
                id: created.request_id,
                url: created.url,
            })
        } else if let Some(sent) = event.read::<network::WebSocketFrameSentEvent>() {
            Some(Self::SocketFrameSent {
                id: sent.request_id,
                frame: SocketFrame::of(&sent.response),
            })
        } else if let Some(received) = event.read::<network::WebSocketFrameReceivedEvent>() {
            Some(Self::SocketFrameReceived {
                id: received.request_id,
                frame: SocketFrame::of(&received.response),
            })
        } else if let Some(closed) = event.read::<network::WebSocketClosedEvent>() {
            Some(Self::SocketClosed {
                id: closed.request_id,
            })
        } else if event.read::<page::DomContentEventFiredEvent>().is_some() {
            Some(Self::DomContentLoaded)
        } else if event.read::<page::LoadEventFiredEvent>().is_some() {
            Some(Self::Loaded)
        } else if let Some(called) = event.read::<runtime::ConsoleAPICalledEvent>() {
            Some(Self::ConsoleCalled(ConsoleCalled(called)))
        } else {
            event
                .read::<runtime::ExceptionThrownEvent>()
                .map(|thrown| Self::ExceptionThrown(ExceptionThrown(thrown)))
        }
    }
}

/// A request sent, or its next hop after a redirect.
pub(crate) struct RequestSent(network::RequestWillBeSentEvent);

impl RequestSent {
    /// The browser's ID of the request, which each of its hops keeps.
    pub(crate) fn network_id(&self) -> &str {
        &self.0.request_id
    }

    /// The browser's ID of the request, taken out of it.
    pub(crate) fn into_network_id(self) -> String {
        self.0.request_id
    }

    /// The ID of the load its document request belongs to.
    pub(crate) fn loader(&self) -> &str {
        &self.0.loader_id
    }

    /// Its URL, as sent.
    pub(crate) fn url(&self) -> &str {
        &self.0.request.url
    }

    /// Its method, as sent.
    pub(crate) fn method(&self) -> &str {
        &self.0.request.method
    }

    /// The headers it carries, by name.
    pub(crate) fn headers(&self) -> BTreeMap<String, String> {
        headers(&self.0.request.headers)
    }

    /// Its body, with the Run's secrets masked, when it has one.
    pub(crate) fn post_data(&self, masks: &Masks) -> Option<String> {
        post_data(&self.0.request, masks)
    }

    /// Its resource type, in Playwright's spelling.
    pub(crate) fn resource_type(&self) -> String {
        resource_type(self.0.r#type.unwrap_or(network::ResourceType::Other))
    }

    /// Whether it navigates a frame.
    pub(crate) fn is_navigation(&self) -> bool {
        is_navigation(self.0.r#type)
    }

    /// Whether it is a stream that stays open, a WebSocket's or a
    /// server-sent-event stream's connection.
    pub(crate) fn streams(&self) -> bool {
        matches!(
            self.0.r#type,
            Some(network::ResourceType::EventSource | network::ResourceType::WebSocket)
        )
    }

    /// The response that redirected the hop before it, when one did.
    pub(crate) fn redirect(&self) -> Option<BrowserResponse<'_>> {
        self.0.redirect_response.as_ref().map(BrowserResponse)
    }
}

/// A response as the browser reported it.
pub(crate) struct BrowserResponse<'a>(&'a network::Response);

impl BrowserResponse<'_> {
    /// The response to the request's hop the engine names `request_id`.
    pub(crate) fn of(&self, request_id: &str) -> PageResponse {
        response_of(request_id, self.0)
    }
}

/// A request's response, arrived.
pub(crate) struct ResponseReceived(network::ResponseReceivedEvent);

impl ResponseReceived {
    /// The browser's ID of the request's hop it answers.
    pub(crate) fn network_id(&self) -> &str {
        &self.0.request_id
    }

    /// The response itself.
    pub(crate) fn response(&self) -> BrowserResponse<'_> {
        BrowserResponse(&self.0.response)
    }
}

/// One frame of a WebSocket: text, or bytes in Base64, which the browser
/// already gives.
pub(crate) struct SocketFrame {
    pub(crate) payload: String,
    pub(crate) binary: bool,
}

impl SocketFrame {
    /// The frame `frame` reports.
    fn of(frame: &network::WebSocketFrame) -> Self {
        Self {
            payload: frame.payload_data.clone(),
            #[expect(
                clippy::float_cmp,
                reason = "the browser gives an opcode, a small integer, as a JSON number"
            )]
            binary: frame.opcode == 2.0,
        }
    }
}

/// A console call.
pub(crate) struct ConsoleCalled(runtime::ConsoleAPICalledEvent);

impl ConsoleCalled {
    /// When it was made, in milliseconds since the epoch.
    pub(crate) fn timestamp(&self) -> f64 {
        self.0.timestamp
    }

    /// The call as Playwright's `ConsoleMessage` of the page `page_id`.
    pub(crate) fn message(&self, page_id: &PageId) -> ConsoleMessage {
        console_message(page_id, &self.0)
    }
}

/// An uncaught exception.
pub(crate) struct ExceptionThrown(runtime::ExceptionThrownEvent);

impl ExceptionThrown {
    /// When it was thrown, in milliseconds since the epoch.
    pub(crate) fn timestamp(&self) -> f64 {
        self.0.timestamp
    }

    /// The exception as Playwright's `WebError` of the page `page_id`.
    pub(crate) fn error(&self, page_id: &PageId) -> WebError {
        web_error(page_id, &self.0.exception_details)
    }
}

/// A navigation a frame committed.
pub(crate) struct FrameNavigated {
    /// The frame.
    pub(crate) frame: String,
    /// Whether the frame is a page's main frame: it has no parent.
    pub(crate) main: bool,
    /// The frame's URL now, its fragment included.
    pub(crate) url: String,
    /// The ID of the load its document came by.
    pub(crate) loader: String,
}

impl FrameNavigated {
    /// The navigation `navigated` tells.
    pub(super) fn of(navigated: page::FrameNavigatedEvent) -> Self {
        let frame = navigated.frame;
        Self {
            url: format!(
                "{}{}",
                frame.url,
                frame.url_fragment.as_deref().unwrap_or_default()
            ),
            main: frame.parent_id.is_none(),
            frame: frame.id,
            loader: frame.loader_id,
        }
    }
}

/// The events a page's events follow, each read from its subscription
/// once.
pub(crate) struct FollowedEvents {
    events: Subscription,
    /// Whether they are an out-of-process frame's, whose network, console
    /// and uncaught exceptions alone are followed.
    frame: bool,
}

/// The prefix of the `Network` domain's events.
const NETWORK: &str = "Network.";

/// The events of the `Runtime` domain a frame's own console and uncaught
/// exceptions come as.
const FRAME_RUNTIME: [&str; 2] = ["Runtime.consoleAPICalled", "Runtime.exceptionThrown"];

impl FollowedEvents {
    /// The next event followed; `None` once the page's target, or the
    /// frame's, went, or its subscription fell behind.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub(crate) async fn next(&mut self) -> Option<FollowedEvent> {
        while let Some(event) = self.events.next().await {
            if self.frame
                && !event.method.starts_with(NETWORK)
                && !FRAME_RUNTIME.contains(&event.method.as_str())
            {
                continue;
            }
            if let Some(followed) = FollowedEvent::of(&event) {
                return Some(followed);
            }
        }
        None
    }

    /// Whether they ended because their subscription fell too far behind,
    /// and events were lost.
    pub(crate) fn fell_behind(&mut self) -> bool {
        self.events.ended() == Some(SubscriptionEnd::FellBehind)
    }
}

impl CdpPage {
    /// The page's events its events follow, from now.
    pub(crate) fn followed_events(&self) -> FollowedEvents {
        FollowedEvents {
            events: self.page.events(),
            frame: false,
        }
    }

    /// The events its events follow of the page's out-of-process frame on
    /// `session`, from now: its network, console and uncaught exceptions.
    pub(crate) fn frame_events(&self, session: &TargetSession) -> FollowedEvents {
        FollowedEvents {
            events: self.page.events_on(session),
            frame: true,
        }
    }

    /// Has the page report its console calls and uncaught exceptions, from
    /// now, within a deadline of opening a page.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn follow_console_opening(&self) -> Result<(), CallError> {
        self.page
            .opening_command(&runtime::EnableParams {})
            .await
            .map(drop)
    }

    /// Has the page's out-of-process frame on `session` report its console
    /// calls and uncaught exceptions, as
    /// [`Self::follow_console_opening`] does.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn follow_frame_console_opening(
        &self,
        session: &TargetSession,
    ) -> Result<(), CallError> {
        self.page
            .opening_command_on(session, &runtime::EnableParams {})
            .await
            .map(drop)
    }

    /// Has the page's out-of-process frame on `session` report its console
    /// calls and uncaught exceptions, with no deadline of its own.
    ///
    /// # Errors
    ///
    /// When the frame went meanwhile.
    pub(crate) async fn follow_frame_console(
        &self,
        session: &TargetSession,
    ) -> Result<(), NotTaken> {
        self.page
            .command_on(session, &runtime::EnableParams {})
            .await
            .map(drop)
            .map_err(NotTaken)
    }

    /// Has the page's out-of-process frame on `session` report its
    /// network, from now.
    ///
    /// # Errors
    ///
    /// When the frame went meanwhile.
    pub(crate) async fn follow_frame_network(
        &self,
        session: &TargetSession,
    ) -> Result<(), NotTaken> {
        self.page
            .command_on(session, &network::EnableParams::default())
            .await
            .map(drop)
            .map_err(NotTaken)
    }

    /// The body of the finished request's hop `network_id`, after any
    /// content coding, as the browser holds it, read on the out-of-process
    /// frame's `session` its events came on, else the page's; none when the
    /// browser no longer holds it.
    pub(crate) async fn response_body(
        &self,
        network_id: &str,
        session: Option<&TargetSession>,
    ) -> Option<Vec<u8>> {
        let params = network::GetResponseBodyParams {
            request_id: network_id.to_owned(),
        };
        let read = match session {
            Some(session) => self.page.command_on(session, &params).await,
            None => self.page.command(&params).await,
        }
        .ok()?;
        bytes_of(read.body, read.base64_encoded).ok()
    }
}
