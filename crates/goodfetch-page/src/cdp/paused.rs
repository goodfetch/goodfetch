//! A page's paused requests as the interception reads and answers them:
//! the events it follows, each read once from the page's subscription, and
//! the primitives that pause, send on, answer, fail and authenticate a
//! request.

use std::collections::BTreeMap;
use std::fmt;

use data_encoding::BASE64;
use goodfetch_cdp::generated::{fetch, network, page};
use goodfetch_cdp::{Failure, Subscription};
use goodfetch_clean_up_rules::matching::ResourceKind;
use goodfetch_engine::CallError;
use goodfetch_engine::mask::{Masks, Scope};
use goodfetch_engine::net_error::NetError;
use goodfetch_protocol::page::route::AbortReason;

use super::CdpPage;
use super::driver::cdp_response;
use super::requests::{ContinueRequest, Fulfill, Header, PausedId, RequestHeaders, kind_of};

/// The browser's ID of a request's hop, as the page's events and its
/// responses name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NetworkId(String);

impl NetworkId {
    /// The ID as the browser names it.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// A request, or its response, the browser paused.
#[derive(Debug)]
pub(crate) struct PausedRequest {
    id: PausedId,
    network_id: Option<NetworkId>,
    event: fetch::RequestPausedEvent,
}

impl PausedRequest {
    /// The request `event` reports paused, its IDs taken out of it.
    pub(crate) fn of(mut event: fetch::RequestPausedEvent) -> Self {
        Self {
            id: PausedId::new(std::mem::take(&mut event.request_id)),
            network_id: event.network_id.take().map(NetworkId),
            event,
        }
    }

    /// The ID every answer to it names.
    pub(crate) fn id(&self) -> &PausedId {
        &self.id
    }

    /// The ID of its hop, when the browser names one.
    pub(crate) fn network_id(&self) -> Option<&NetworkId> {
        self.network_id.as_ref()
    }

    /// The frame that sent it.
    pub(crate) fn frame(&self) -> &str {
        &self.event.frame_id
    }

    /// Its URL, as sent.
    pub(crate) fn url(&self) -> &str {
        &self.event.request.url
    }

    /// Its method, as sent.
    pub(crate) fn method(&self) -> &str {
        &self.event.request.method
    }

    /// The headers it carries.
    pub(crate) fn headers(&self) -> RequestHeaders<'_> {
        RequestHeaders::of(&self.event.request.headers)
    }

    /// Whether it sends a body.
    pub(crate) fn has_body(&self) -> bool {
        self.event.request.has_post_data == Some(true)
    }

    /// Its body, masked with `masks` as [`post_data`] masks it.
    pub(crate) fn post_data(&self, masks: &Masks) -> Option<String> {
        post_data(&self.event.request, masks)
    }

    /// Its body as it is sent, unmasked; none when it sends a body the
    /// browser does not show, or none at all.
    pub(crate) fn body(&self) -> Option<Vec<u8>> {
        body(&self.event.request)
    }

    /// What it loads, as Playwright's `resourceType()` names it.
    pub(crate) fn resource_type(&self) -> String {
        resource_type(self.event.resource_type)
    }

    /// Whether it loads a frame's document, the main frame's or an
    /// iframe's.
    pub(crate) fn loads_document(&self) -> bool {
        self.event.resource_type == network::ResourceType::Document
    }

    /// What it loads, as the Filter lists match it; a document is a frame's
    /// unless `main_frame`.
    pub(crate) fn kind(&self, main_frame: bool) -> ResourceKind {
        kind_of(self.event.resource_type, main_frame)
    }

    /// Whether a redirect sent it.
    pub(crate) fn redirected(&self) -> bool {
        self.event.redirected_request_id.is_some()
    }

    /// Whether it paused at its response, one that came or one that failed.
    pub(crate) fn at_response(&self) -> bool {
        self.event.response_status_code.is_some() || self.event.response_error_reason.is_some()
    }

    /// Whether it paused at a response that came.
    pub(crate) fn responded(&self) -> bool {
        self.event.response_status_code.is_some()
    }

    /// Its response's status; 0 when it paused before a response arrived.
    pub(crate) fn status(&self) -> u16 {
        self.event
            .response_status_code
            .map_or(0, cdp_response::status)
    }

    /// Its response's status text, empty when it has none.
    pub(crate) fn status_text(&self) -> &str {
        self.event
            .response_status_text
            .as_deref()
            .unwrap_or_default()
    }

    /// The headers of its response, as the browser gives them.
    fn response_entries(&self) -> &[fetch::HeaderEntry] {
        self.event.response_headers.as_deref().unwrap_or_default()
    }

    /// The value of its response's header `name`, whatever its case: the
    /// first given.
    pub(crate) fn response_header(&self, name: &str) -> Option<&str> {
        self.response_entries()
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    /// Its response's headers by lower-cased name, a repeated one's values
    /// on lines of their own.
    pub(crate) fn response_headers(&self) -> BTreeMap<String, String> {
        cdp_response::paused_headers(self.response_entries())
    }

    /// Its response's headers in their order, each with its lower-cased
    /// name, a repeated one's entries kept apart.
    pub(crate) fn response_header_list(&self) -> Vec<(String, String)> {
        cdp_response::paused_header_list(self.response_entries())
    }

    /// Its response's headers as given, but those named in `left_out`,
    /// whatever their case.
    pub(crate) fn response_headers_without(&self, left_out: &[&str]) -> Vec<Header> {
        self.response_entries()
            .iter()
            .filter(|header| {
                !left_out
                    .iter()
                    .any(|name| header.name.eq_ignore_ascii_case(name))
            })
            .map(|header| Header {
                name: header.name.clone(),
                value: header.value.clone(),
            })
            .collect()
    }
}

impl RequestHeaders<'_> {
    /// The headers by lower-cased name, each value a string.
    pub(crate) fn by_name(self) -> BTreeMap<String, String> {
        cdp_response::headers(self.map())
    }
}

/// A 407 or 401 the browser reports for a paused request.
#[derive(Debug)]
pub(crate) struct AuthChallenge {
    id: PausedId,
    event: fetch::AuthRequiredEvent,
}

impl AuthChallenge {
    /// The challenge `event` reports, its ID taken out of it.
    pub(crate) fn of(mut event: fetch::AuthRequiredEvent) -> Self {
        Self {
            id: PausedId::new(std::mem::take(&mut event.request_id)),
            event,
        }
    }

    /// The ID its answer names.
    pub(crate) fn id(&self) -> &PausedId {
        &self.id
    }

    /// The URL of the request challenged.
    pub(crate) fn url(&self) -> &str {
        &self.event.request.url
    }

    /// Whether a proxy, not the site, challenged it.
    pub(crate) fn by_proxy(&self) -> bool {
        self.event.auth_challenge.source == Some(fetch::AuthChallengeSource::Proxy)
    }

    /// Whether the request challenged loads a frame's document.
    pub(crate) fn loads_document(&self) -> bool {
        self.event.resource_type == network::ResourceType::Document
    }

    /// The frame that sent the request challenged.
    pub(crate) fn frame(&self) -> &str {
        &self.event.frame_id
    }
}

/// How a challenge is answered, as `WebDriver BiDi`'s `continueWithAuth`
/// names its actions.
#[derive(Debug)]
pub(crate) enum AuthAnswer {
    /// As the browser would answer it with no credentials.
    Default,
    /// With these credentials.
    ProvideCredentials { username: String, password: String },
    /// Cancelled: the request gets the challenge as its response.
    Cancel,
}

impl AuthAnswer {
    /// The answer as the browser takes it.
    fn into_response(self) -> fetch::AuthChallengeResponse {
        let (response, username, password) = match self {
            Self::Default => (fetch::AuthChallengeResponseResponse::Default, None, None),
            Self::ProvideCredentials { username, password } => (
                fetch::AuthChallengeResponseResponse::ProvideCredentials,
                Some(username),
                Some(password),
            ),
            Self::Cancel => (fetch::AuthChallengeResponseResponse::CancelAuth, None, None),
        };
        fetch::AuthChallengeResponse {
            response,
            username,
            password,
        }
    }
}

/// A request of the page's that failed.
#[derive(Debug)]
pub(crate) struct LoadingFailed(pub(super) network::LoadingFailedEvent);

impl LoadingFailed {
    /// The ID of its hop.
    pub(crate) fn network_id(&self) -> &str {
        &self.0.request_id
    }

    /// Whether it loaded a frame's document.
    pub(crate) fn loads_document(&self) -> bool {
        self.0.r#type == network::ResourceType::Document
    }

    /// The browser's network error.
    pub(crate) fn error(&self) -> NetError {
        NetError::of(&self.0.error_text)
    }
}

/// What the interception follows of a page's events.
#[derive(Debug)]
pub(crate) enum InterceptionEvent {
    /// The page's `load`.
    Loaded,
    /// A frame started a navigation to `url`.
    NavigationStarted {
        frame: String,
        url: String,
        same_document: bool,
    },
    /// A frame scheduled, or asked for, a navigation to `url`.
    NavigationScheduled { frame: String, url: String },
    /// A request failed.
    LoadingFailed(LoadingFailed),
    /// A response carried `Set-Cookie` headers (which Chromium omits from
    /// `Fetch.requestPaused.responseHeaders` and delivers only on
    /// `Network.responseReceivedExtraInfo`).
    ResponseSetCookie {
        network_id: String,
        values: Vec<String>,
    },
    /// The page opened a WebSocket to `url`.
    WebSocketCreated { url: String },
    /// A request was challenged.
    AuthRequired(AuthChallenge),
    /// A request, or its response, paused.
    Paused(PausedRequest),
}

/// The events of one page the interception follows, each read from the
/// page's subscription once.
pub(crate) struct InterceptionEvents {
    events: Subscription,
    /// Whether the page's `load` is read: a page's is, a frame's or a
    /// worker's is not.
    loads: bool,
}

impl InterceptionEvents {
    /// The next event the interception follows; `None` once the page's
    /// target went.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses no event.
    pub(crate) async fn next(&mut self) -> Option<InterceptionEvent> {
        while let Some(event) = self.events.next().await {
            if self.loads && event.read::<page::LoadEventFiredEvent>().is_some() {
                return Some(InterceptionEvent::Loaded);
            }
            if let Some(started) = event.read::<page::FrameStartedNavigatingEvent>() {
                return Some(InterceptionEvent::NavigationStarted {
                    same_document: started.navigation_type
                        == page::FrameStartedNavigatingNavigationType::SameDocument,
                    frame: started.frame_id,
                    url: started.url,
                });
            }
            let scheduled = event
                .read::<page::FrameScheduledNavigationEvent>()
                .map(|scheduled| (scheduled.frame_id, scheduled.url))
                .or_else(|| {
                    event
                        .read::<page::FrameRequestedNavigationEvent>()
                        .map(|requested| (requested.frame_id, requested.url))
                });
            if let Some((frame, url)) = scheduled {
                return Some(InterceptionEvent::NavigationScheduled { frame, url });
            }
            if let Some(failed) = event.read::<network::LoadingFailedEvent>() {
                return Some(InterceptionEvent::LoadingFailed(LoadingFailed(failed)));
            }
            if let Some(extra) = event.read::<network::ResponseReceivedExtraInfoEvent>() {
                let values: Vec<String> = extra
                    .headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
                    .filter_map(|(_, value)| value.as_str())
                    .flat_map(|value| value.lines())
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned)
                    .collect();
                if !values.is_empty() {
                    return Some(InterceptionEvent::ResponseSetCookie {
                        network_id: extra.request_id,
                        values,
                    });
                }
            }
            if let Some(created) = event.read::<network::WebSocketCreatedEvent>() {
                return Some(InterceptionEvent::WebSocketCreated { url: created.url });
            }
            if let Some(required) = event.read::<fetch::AuthRequiredEvent>() {
                return Some(InterceptionEvent::AuthRequired(AuthChallenge::of(required)));
            }
            if let Some(paused) = event.read::<fetch::RequestPausedEvent>() {
                return Some(InterceptionEvent::Paused(PausedRequest::of(paused)));
            }
        }
        None
    }
}

/// A command the browser did not take: the request it names went, or the
/// page did.
pub(crate) struct NotTaken(pub(super) Failure);

impl NotTaken {
    /// A command never sent, as what sends the page's commands went.
    pub(crate) fn gone() -> Self {
        Self(Failure::Disconnected)
    }
}

impl From<Failure> for NotTaken {
    fn from(failure: Failure) -> Self {
        Self(failure)
    }
}

impl fmt::Display for NotTaken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

impl fmt::Debug for NotTaken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl CdpPage {
    /// The page's events the interception follows, from now; its `load`
    /// among them when `loads`.
    pub(crate) fn paused_events(&self, loads: bool) -> InterceptionEvents {
        InterceptionEvents {
            events: self.page.events(),
            loads,
        }
    }

    /// Pauses the page's requests at [`patterns`], as a step of opening it:
    /// when the page answers challenges, an HTTP or HTTPS proxy's 407
    /// reaches the engine too, which answers it with the Session's
    /// credentials (SA-22).
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn pause_requests_opening(&self) -> Result<(), CallError> {
        self.page.opening_command(&self.pausing()).await.map(drop)
    }

    /// Pauses the page's requests as [`Self::pause_requests_opening`] does,
    /// on a page already open.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the page went first.
    pub(crate) async fn pause_requests(&self) -> Result<(), NotTaken> {
        self.taken(&self.pausing()).await
    }

    /// The command that pauses the page's requests.
    fn pausing(&self) -> fetch::EnableParams {
        fetch::EnableParams {
            patterns: Some(patterns()),
            handle_auth_requests: self.page.answers_authentication().then_some(true),
        }
    }

    /// Reports each `WebSocket` the page opens, which no pause sees.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the page went first.
    pub(crate) async fn follow_web_sockets(&self) -> Result<(), NotTaken> {
        self.taken(&network::EnableParams::default()).await
    }

    /// Sends a paused request on as `request` says.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the request or the page went first.
    pub(crate) async fn continue_request(&self, request: ContinueRequest) -> Result<(), NotTaken> {
        self.taken(&request.into_params()).await
    }

    /// Answers a paused request as `response` says.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the request or the page went first.
    pub(crate) async fn fulfill(&self, response: Fulfill) -> Result<(), NotTaken> {
        self.taken(&response.into_params()).await
    }

    /// Fails the paused request `id` with the network error `reason`.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the request or the page went first.
    pub(crate) async fn fail(&self, id: &PausedId, reason: AbortReason) -> Result<(), NotTaken> {
        self.taken(&fetch::FailRequestParams {
            request_id: id.as_str().to_owned(),
            error_reason: error_reason(reason),
        })
        .await
    }

    /// Answers the challenge of the paused request `id` with `answer`.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the request or the page went first.
    pub(crate) async fn continue_with_auth(
        &self,
        id: &PausedId,
        answer: AuthAnswer,
    ) -> Result<(), NotTaken> {
        self.taken(&fetch::ContinueWithAuthParams {
            request_id: id.as_str().to_owned(),
            auth_challenge_response: answer.into_response(),
        })
        .await
    }

    /// Lets the response paused at `id` go on as it came.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the response or the page went first.
    pub(crate) async fn continue_response(&self, id: &PausedId) -> Result<(), NotTaken> {
        self.taken(&fetch::ContinueResponseParams {
            request_id: id.as_str().to_owned(),
            response_code: None,
            response_phrase: None,
            response_headers: None,
            binary_response_headers: None,
        })
        .await
    }

    /// Lets the response `paused` go on with its status and `headers` in
    /// place of its own.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the response or the page went first.
    pub(crate) async fn continue_response_with(
        &self,
        paused: &PausedRequest,
        headers: Vec<Header>,
    ) -> Result<(), NotTaken> {
        self.taken(&fetch::ContinueResponseParams {
            request_id: paused.id.as_str().to_owned(),
            response_code: paused.event.response_status_code,
            response_phrase: None,
            response_headers: Some(headers.into_iter().map(fetch::HeaderEntry::from).collect()),
            binary_response_headers: None,
        })
        .await
    }

    /// The names of the cookies the browser would send with a request to
    /// `url`.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the page went first.
    pub(crate) async fn cookie_names(&self, url: &str) -> Result<Vec<String>, NotTaken> {
        let found = self
            .page
            .command(&network::GetCookiesParams {
                urls: Some(vec![url.to_owned()]),
            })
            .await
            .map_err(NotTaken)?;
        Ok(found
            .cookies
            .into_iter()
            .map(|cookie| cookie.name)
            .collect())
    }

    /// The names of the cookies of the page's browser context the browser
    /// would send with a request to `url`, matched by [`sent_to`]: how a
    /// shared or service worker's are asked, as its target answers no
    /// `Network.getCookies`. Neither `SameSite` nor a cookie's partition
    /// is judged, so every cookie the request could carry is named.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the browser went first.
    pub(crate) async fn context_cookie_names(
        &self,
        url: &url::Url,
    ) -> Result<Vec<String>, NotTaken> {
        let cookies = self.page.context_cookies().await.map_err(NotTaken)?;
        Ok(cookies
            .into_iter()
            .filter(|cookie| sent_to(cookie, url))
            .map(|cookie| cookie.name)
            .collect())
    }

    /// The `Cookie` header the browser would send with a request to `url`,
    /// which a paused request does not show: each cookie's `name=value`,
    /// joined by `; `; empty for none.
    ///
    /// # Errors
    ///
    /// [`NotTaken`] when the page went first.
    pub(crate) async fn cookie_header(&self, url: &str) -> Result<String, NotTaken> {
        let found = self
            .page
            .command(&network::GetCookiesParams {
                urls: Some(vec![url.to_owned()]),
            })
            .await
            .map_err(NotTaken)?;
        Ok(found
            .cookies
            .iter()
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; "))
    }

    /// Sends `command`, whose answer says nothing more than that the
    /// browser took it.
    pub(super) async fn taken<C: goodfetch_cdp::Command>(
        &self,
        command: &C,
    ) -> Result<(), NotTaken> {
        self.page.command(command).await.map(drop).map_err(NotTaken)
    }
}

/// Whether a request to `url` carries `cookie` by its domain, path and
/// `Secure` flag (RFC 6265, section 5.4): a cookie of `.site.test` goes to
/// that host and its subdomains, any other to its one host, and its path
/// is the request's or a folder of it.
fn sent_to(cookie: &network::Cookie, url: &url::Url) -> bool {
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let domain = cookie.domain.to_ascii_lowercase();
    let by_domain = match domain.strip_prefix('.') {
        Some(parent) => {
            host == parent
                || host
                    .strip_suffix(parent)
                    .is_some_and(|sub| sub.ends_with('.'))
        }
        None => host == domain,
    };
    let by_path = url
        .path()
        .strip_prefix(cookie.path.as_str())
        .is_some_and(|rest| rest.is_empty() || cookie.path.ends_with('/') || rest.starts_with('/'));
    let by_scheme = !cookie.secure || matches!(url.scheme(), "https" | "wss");
    by_domain && by_path && by_scheme
}

/// What a page's requests pause at: every request before it leaves, and
/// a document's response, whose redirect the egress policy judges (a
/// `Location` to a scheme no request may use sends no request of its own)
/// and a navigation gate judges before the page renders it.
fn patterns() -> Vec<fetch::RequestPattern> {
    let stage = |resource_type, request_stage| fetch::RequestPattern {
        url_pattern: Some("*".to_owned()),
        resource_type,
        request_stage: Some(request_stage),
    };
    vec![
        stage(None, fetch::RequestStage::Request),
        stage(
            Some(network::ResourceType::Document),
            fetch::RequestStage::Response,
        ),
    ]
}

/// The browser's network error for `reason`.
fn error_reason(reason: AbortReason) -> network::ErrorReason {
    match reason {
        AbortReason::Aborted => network::ErrorReason::Aborted,
        AbortReason::AccessDenied => network::ErrorReason::AccessDenied,
        AbortReason::AddressUnreachable => network::ErrorReason::AddressUnreachable,
        AbortReason::BlockedByClient => network::ErrorReason::BlockedByClient,
        AbortReason::BlockedByResponse => network::ErrorReason::BlockedByResponse,
        AbortReason::ConnectionAborted => network::ErrorReason::ConnectionAborted,
        AbortReason::ConnectionClosed => network::ErrorReason::ConnectionClosed,
        AbortReason::ConnectionFailed => network::ErrorReason::ConnectionFailed,
        AbortReason::ConnectionRefused => network::ErrorReason::ConnectionRefused,
        AbortReason::ConnectionReset => network::ErrorReason::ConnectionReset,
        AbortReason::InternetDisconnected => network::ErrorReason::InternetDisconnected,
        AbortReason::NameNotResolved => network::ErrorReason::NameNotResolved,
        AbortReason::TimedOut => network::ErrorReason::TimedOut,
        AbortReason::Failed => network::ErrorReason::Failed,
    }
}

/// A request's body in Base64, from its entries' bytes or else its text,
/// with every value `masks` masks in what a call returns replaced by its
/// `%name%` before it is encoded: a Secret a form sent is masked in the
/// body itself, however the form encoded it (ADR 0092).
pub(crate) fn post_data(request: &network::Request, masks: &Masks) -> Option<String> {
    let bytes = body(request)?;
    Some(BASE64.encode(&masks.apply_bytes(&bytes, Scope::Returned)))
}

/// A request's body as it is sent, from its entries' bytes or else its
/// text; none when it has none the browser shows.
fn body(request: &network::Request) -> Option<Vec<u8>> {
    if let Some(entries) = &request.post_data_entries {
        let mut bytes = Vec::new();
        for entry in entries {
            bytes.extend(BASE64.decode(entry.bytes.as_deref()?.as_bytes()).ok()?);
        }
        Some(bytes)
    } else {
        Some(request.post_data.as_ref()?.as_bytes().to_vec())
    }
}

/// What a request loads, as Playwright's `resourceType()` names it: the
/// browser's name in lowercase, such as `document` or `xhr`.
pub(crate) fn resource_type(resource_type: network::ResourceType) -> String {
    serde_json::to_value(resource_type)
        .ok()
        .and_then(|name| name.as_str().map(str::to_ascii_lowercase))
        .unwrap_or_else(|| "other".to_owned())
}

/// Whether a request of `resource_type` loads a frame's document, the main
/// frame's or an iframe's, as Playwright's `isNavigationRequest()` says.
pub(crate) fn is_navigation(resource_type: Option<network::ResourceType>) -> bool {
    resource_type == Some(network::ResourceType::Document)
}

#[cfg(test)]
mod tests;
