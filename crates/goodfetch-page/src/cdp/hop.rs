//! The primitives of the page a Session's request is sent from under the
//! browser-fetch tier (ADRs 0020 and 0118): the page made to send that
//! request alone, its document load started, the events it follows until
//! the load ends, and the body of a paused response read from the
//! browser's stream of it, chunk by chunk, which the interception's Route
//! fetches read too.

use data_encoding::DecodeError;
use goodfetch_cdp::Subscription;
use goodfetch_cdp::generated::{emulation, fetch, io, network, page};
use goodfetch_engine::CallError;
use goodfetch_engine::net_error::NetError;

use super::CdpPage;
use super::driver::cdp_response;
use super::paused::{AuthChallenge, LoadingFailed, NotTaken, PausedRequest};
use super::requests::PausedId;

/// What the browser-fetch tier follows of its page's events.
#[derive(Debug)]
pub(crate) enum HopEvent {
    /// A request, or its response, paused.
    Paused(PausedRequest),
    /// A request was challenged.
    AuthRequired(AuthChallenge),
    /// The response of the hop `network_id` arrived, from the network or,
    /// unless `from_the_network`, from the cache or a service worker.
    ResponseReceived {
        network_id: String,
        from_the_network: bool,
    },
    /// The hop `network_id`'s body ended whole, after `encoded` bytes on
    /// the wire as the browser counts them.
    LoadingFinished { network_id: String, encoded: f64 },
    /// A request failed.
    LoadingFailed(LoadingFailed),
}

/// The events of one page the browser-fetch tier follows, each read from
/// the page's subscription once.
pub(crate) struct HopEvents {
    events: Subscription,
}

impl HopEvents {
    /// The next event the browser-fetch tier follows; `None` once the
    /// page's target went.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub(crate) async fn next(&mut self) -> Option<HopEvent> {
        while let Some(event) = self.events.next().await {
            if let Some(paused) = event.read::<fetch::RequestPausedEvent>() {
                return Some(HopEvent::Paused(PausedRequest::of(paused)));
            }
            if let Some(required) = event.read::<fetch::AuthRequiredEvent>() {
                return Some(HopEvent::AuthRequired(AuthChallenge::of(required)));
            }
            if let Some(received) = event.read::<network::ResponseReceivedEvent>() {
                return Some(HopEvent::ResponseReceived {
                    from_the_network: cdp_response::from_the_network(&received.response),
                    network_id: received.request_id,
                });
            }
            if let Some(finished) = event.read::<network::LoadingFinishedEvent>() {
                return Some(HopEvent::LoadingFinished {
                    network_id: finished.request_id,
                    encoded: finished.encoded_data_length,
                });
            }
            if let Some(failed) = event.read::<network::LoadingFailedEvent>() {
                return Some(HopEvent::LoadingFailed(LoadingFailed(failed)));
            }
        }
        None
    }
}

/// The browser's stream of a paused response's body, until it is closed.
#[derive(Debug)]
pub(crate) struct BodyStream(String);

/// One chunk read from a [`BodyStream`].
#[derive(Debug)]
pub(crate) struct BodyChunk {
    /// Its bytes, after any content coding; an error when the browser gave
    /// them in Base64 that does not decode.
    pub(crate) bytes: Result<Vec<u8>, DecodeError>,
    /// Whether the stream ended with it.
    pub(crate) eof: bool,
}

impl CdpPage {
    /// The page's events the browser-fetch tier follows, from now.
    pub(crate) fn hop_events(&self) -> HopEvents {
        HopEvents {
            events: self.page.events(),
        }
    }

    /// Makes the page send one request alone, as a step of opening it: its
    /// scripts never run, no service worker answers its requests, and each
    /// of them pauses before it leaves, the challenges the page answers
    /// too.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn isolate_opening(&self) -> Result<(), CallError> {
        self.page
            .opening_command(&emulation::SetScriptExecutionDisabledParams { value: true })
            .await?;
        self.page
            .opening_command(&network::SetBypassServiceWorkerParams { bypass: true })
            .await?;
        self.page
            .opening_command(&fetch::EnableParams {
                patterns: Some(vec![fetch::RequestPattern {
                    url_pattern: Some("*".to_owned()),
                    resource_type: None,
                    request_stage: Some(fetch::RequestStage::Request),
                }]),
                handle_auth_requests: self.page.answers_authentication().then_some(true),
            })
            .await
            .map(drop)
    }

    /// Toggles the page's HTTP cache (`Network.setCacheDisabled`) so a tier
    /// race's navigation bypasses Chrome's HTTP cache lock behind its
    /// in-flight browser-fetch for the same URL (ADR 0040; issue 1630).
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn set_cache_disabled(&self, cache_disabled: bool) -> Result<(), CallError> {
        self.page
            .opening_command(&network::SetCacheDisabledParams { cache_disabled })
            .await
            .map(drop)
    }

    /// Starts a load of `url` as the main frame's document: answers the
    /// browser's network error when it refused the load before it reached
    /// the network, as it does a URL no load may go to.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the page went first.
    pub(crate) async fn load_document(&self, url: &str) -> Result<Option<NetError>, NotTaken> {
        let navigated = self
            .page
            .command(&page::NavigateParams {
                url: url.to_owned(),
                referrer: None,
                transition_type: None,
                frame_id: None,
                referrer_policy: None,
            })
            .await
            .map_err(NotTaken)?;
        Ok(navigated.error_text.as_deref().map(NetError::of))
    }

    /// Takes the body of the response paused at `id` as a stream the
    /// browser reads it from, after any content coding. The paused
    /// response cannot be sent on as it came after.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the browser gives no stream of it, or the
    /// response or the page went first.
    pub(crate) async fn take_body_stream(&self, id: &PausedId) -> Result<BodyStream, NotTaken> {
        let taken = self
            .page
            .command(&fetch::TakeResponseBodyAsStreamParams {
                request_id: id.as_str().to_owned(),
            })
            .await
            .map_err(NotTaken)?;
        Ok(BodyStream(taken.stream))
    }

    /// Reads at most `size` bytes more of `stream`.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the stream broke off, or the page went first.
    pub(crate) async fn read_body_stream(
        &self,
        stream: &BodyStream,
        size: u32,
    ) -> Result<BodyChunk, NotTaken> {
        let chunk = self
            .page
            .command(&io::ReadParams {
                handle: stream.0.clone(),
                offset: None,
                size: Some(i64::from(size)),
            })
            .await
            .map_err(NotTaken)?;
        Ok(BodyChunk {
            bytes: cdp_response::bytes_of(chunk.data, chunk.base64_encoded == Some(true)),
            eof: chunk.eof,
        })
    }

    /// Closes `stream`, so the browser holds none of the body more.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when it was closed already, or the page went first.
    pub(crate) async fn close_body_stream(&self, stream: BodyStream) -> Result<(), NotTaken> {
        self.taken(&io::CloseParams { handle: stream.0 }).await
    }
}
