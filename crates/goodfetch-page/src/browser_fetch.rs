//! The browser-fetch tier (ADRs 0020, 0118, 0119 and 0121): one request sent
//! from inside a Session's browser, so its identity, its TLS and HTTP/2
//! fingerprint and its cookies among it, is the Session's own. It is sent
//! as the document load of a page the engine opened for it in the
//! Session's browser context, whose scripts never run, which no service
//! worker controls (`Network.setBypassServiceWorker`), and whose every other
//! request is failed, so no page script reads the request or its response
//! and no service worker answers it. The page is hidden, shown nowhere in
//! the browser, so Chromium never asks the site for the `/favicon.ico` of the
//! text document it loads, a request its own that would leave once the
//! page's interception stopped (ticket #1608). The load passes steps 3 to 8 of the
//! page's interception at each hop (its [`EngineRequests`]), the caller's method,
//! headers and body set over the browser's own on its first.
//!
//! Its response is paused once it arrives: a 3xx is counted against the
//! request's count of redirects; any other is read whole, its raw bytes
//! after any content coding, and then sent on to the page as plain text, so
//! nothing renders it or downloads it, while the browser still reports how
//! its body ended: whole, cut (a length mismatch, a chunked stream or a
//! connection ending early), or a content coding that failed to decode
//! (ADR 0118's note on the pinned Chromium's signals), and a body with no
//! content coding shorter than its `Content-Length` is cut even when the
//! browser lost that failure and finished the load with no bytes counted
//! (ADR 0118's note on fix-queue NEW-2). The Set-Cookie
//! headers it carries are the browser's to keep, in the Session's context.
//!
//! The body is judged by the Run's limits on what the engine reads (SC-05):
//! one whose `Content-Length` is past `network.max_body_size`, with no
//! content coding, is refused before it is read; any other once it is,
//! by its decoded bytes against the size, and against the bytes it took on
//! the wire under `network.max_decompression_ratio`. Each refusal is
//! `response_limit_exceeded`. A body read whole is one message of the
//! browser's: one past what the connection reads in a message is refused
//! as past the size, and the connection goes on. A caller that needs no
//! word of how the body ended, the robots.txt read, reads it from the
//! browser's stream instead ([`BodyRead::Streamed`]), judged as it arrives
//! and stopped at the first limit, so neither the browser nor the engine
//! holds more: the pinned Chromium reports no network or decoding failure
//! of a body whose stream was taken (ADR 0067's #644 note).

use std::collections::BTreeMap;

use data_encoding::BASE64;
use goodfetch_cdp::generated::{emulation, fetch, network, page};
use goodfetch_cdp::{Failure, NetError, Subscription};
use goodfetch_engine::CallError;
use goodfetch_engine::response_limits::Limits;
use goodfetch_protocol::page::{
    PageResponse, RedirectLimit, TOO_MANY_REDIRECTS, TooManyRedirectsDetails,
};

use crate::driver::PageDriver;
use crate::errors::{network_error, site_preferences_refused};
use crate::interception::{EngineRequests, Overrides};
use crate::page_value::{Navigation, NavigationKind, Permit};
use crate::paused_body::{coded, declared_within_limits, streamed};

/// The headers the page's copy of the response loses, so that nothing
/// renders it or turns it into a download.
const SHOWN_AS_TEXT: [&str; 2] = ["content-type", "content-disposition"];

/// The `Content-Length` `response` declared.
fn declared_length(response: &PageResponse) -> Option<u64> {
    response
        .headers
        .get("content-length")
        .and_then(|length| length.trim().parse().ok())
}

/// The browser's count of a body's encoded bytes, when it counted any.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "CDP counts bytes in a double; a count is whole and far below 2^53"
)]
fn encoded_bytes(encoded: f64) -> Option<u64> {
    (encoded.is_finite() && encoded >= 1.0).then_some(encoded as u64)
}

/// Whether a load the browser reported finished, of `response` to a
/// request of `method`, holding `bytes` decoded after `encoded` bytes the
/// browser counted, broke off all the same: one with no content coding,
/// whose body is shorter than its `Content-Length`, which the browser
/// finished with no bytes counted.
///
/// The pinned Chromium finishes so a load whose body broke off when the
/// network's failure reaches the paused response only after the response
/// went on: its interception then ends the load itself, counting no bytes,
/// and the failure is lost (fix-queue NEW-2). A load the network finished
/// counts its headers at least, and one the cache answered is whole. A
/// chunked or coded body cut so cannot be told from a whole one.
fn cut_though_finished(
    method: Option<&str>,
    response: &PageResponse,
    bytes: usize,
    encoded: f64,
) -> bool {
    encoded_bytes(encoded).is_none() && short_of_its_length(method, response, bytes)
}

/// Whether `bytes`, the body of `response` to a request of `method`, are
/// fewer than the `Content-Length` it declared with no content coding: the
/// one cut the bytes alone show.
fn short_of_its_length(method: Option<&str>, response: &PageResponse, bytes: usize) -> bool {
    let bodiless = method.is_some_and(|method| method.eq_ignore_ascii_case("HEAD"))
        || matches!(response.status, 100..=199 | 204 | 304);
    !coded(&response.headers)
        && !bodiless
        && declared_length(response)
            .is_some_and(|declared| u64::try_from(bytes).unwrap_or(u64::MAX) < declared)
}

/// `response`, whose body broke off after `bytes`, decoded.
fn cut(response: PageResponse, bytes: &[u8]) -> Fetched {
    let expected = declared_length(&response);
    Fetched {
        response,
        body: Body::Cut {
            received: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            expected,
        },
    }
}

/// One request, as its caller gives it.
pub(crate) struct Request<'a> {
    pub(crate) url: &'a str,
    /// Its method; the browser's `GET` when none.
    pub(crate) method: Option<&'a str>,
    /// Headers over the browser's own, by name without regard to case.
    pub(crate) headers: Option<&'a BTreeMap<String, String>>,
    /// Its body, in Base64.
    pub(crate) post_data: Option<&'a str>,
    /// The most redirects it follows: 0 answers a 3xx itself.
    pub(crate) max_redirects: u32,
    /// The option that set that count, in the caller's spelling, which
    /// `too_many_redirects` names.
    pub(crate) option: &'a str,
    /// The Run's limits on the body.
    pub(crate) limits: Limits,
    /// How its body is read.
    pub(crate) read: BodyRead,
}

/// How a request's body is read, once its response arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BodyRead {
    /// Whole, in one message, then sent on to the page, so the browser
    /// still reports how the body ended: whole, cut, or a content coding
    /// that failed to decode (ADR 0118). The browser holds the whole body
    /// meanwhile, and one past what a message carries (about 192 MiB
    /// decoded) is refused as past `network.max_body_size`.
    Whole,
    /// In chunks of the browser's stream, judged by the limits as they
    /// arrive and stopped at the first passed, so neither the browser nor
    /// the engine holds more. The browser then reports nothing of how the
    /// body ended: only a body shorter than its `Content-Length`, with no
    /// content coding, is known cut.
    Streamed,
}

/// The response a request got, and its body.
pub(crate) struct Fetched {
    pub(crate) response: PageResponse,
    pub(crate) body: Body,
}

/// A response's body, as it arrived.
pub(crate) enum Body {
    /// Whole: its bytes after any content coding.
    Whole(Vec<u8>),
    /// Broken off after `received` bytes of the `expected` its
    /// `Content-Length` declared.
    Cut {
        received: u64,
        expected: Option<u64>,
    },
}

/// Sends `request` as the document load of `driver`'s page, a page of the
/// Session the engine opened for it, through `sender`, within the page's
/// `timeout.navigation`.
///
/// # Errors
///
/// `egress_refused` or `navigation_failed` for a hop the interception
/// stops; `too_many_redirects` past the request's count; the Session's
/// proxy's codes; `navigation_failed` with the browser's network error for
/// a request that got no response, or whose content coding failed to
/// decode; `deadline_exceeded` naming `timeout.navigation` for one with no
/// response by then; and the page's codes when the browser fails a command.
///
/// # Cancel safety
///
/// Dropping the future leaves the page's request, or its response, paused:
/// its caller closes the page, as the Session's engine page does when
/// dropped.
pub(crate) async fn send(
    driver: &PageDriver,
    sender: &EngineRequests,
    request: &Request<'_>,
) -> Result<Fetched, CallError> {
    let events = driver.events();
    driver
        .opening_command(&emulation::SetScriptExecutionDisabledParams { value: true })
        .await?;
    driver
        .opening_command(&network::SetBypassServiceWorkerParams { bypass: true })
        .await?;
    driver
        .opening_command(&fetch::EnableParams {
            patterns: Some(vec![fetch::RequestPattern {
                url_pattern: Some("*".to_owned()),
                resource_type: None,
                request_stage: Some(fetch::RequestStage::Request),
            }]),
            handle_auth_requests: driver.answers_authentication().then_some(true),
        })
        .await?;
    // A wait for the navigation gate's permits counts against no timeout.
    driver
        .within_navigation_deadline(
            "response",
            Load::new(driver, sender, request).follow(events),
        )
        .await?
}

/// The document load that carries one request, as it goes.
struct Load<'a> {
    driver: &'a PageDriver,
    sender: &'a EngineRequests,
    request: &'a Request<'a>,
    /// The browser's ID of the load, once its first hop paused.
    network_id: Option<String>,
    /// The URL of each hop, the first included.
    chain: Vec<String>,
    /// Whether the Session's proxy was given its credentials already.
    authenticated: bool,
    /// Where the Session's egress relay's refusals stood as the load
    /// began, if it has a relay.
    relay_mark: Option<u64>,
    /// The response, once it arrived, and its bytes, once read.
    answered: Option<(PageResponse, Vec<u8>)>,
    /// The permit the page's navigation gate gave the hop under way, held
    /// until its response arrives.
    permit: Option<Permit>,
}

impl<'a> Load<'a> {
    fn new(driver: &'a PageDriver, sender: &'a EngineRequests, request: &'a Request<'a>) -> Self {
        Self {
            driver,
            sender,
            request,
            network_id: None,
            chain: Vec::new(),
            authenticated: false,
            relay_mark: driver.relay().map(crate::egress_relay::EgressRelay::mark),
            answered: None,
            permit: None,
        }
    }

    /// Starts the load and follows it through `events` until its body
    /// ended.
    async fn follow(mut self, mut events: Subscription) -> Result<Fetched, CallError> {
        let driver = self.driver;
        let navigation = page::NavigateParams {
            url: self.request.url.to_owned(),
            referrer: None,
            transition_type: None,
            frame_id: None,
            referrer_policy: None,
        };
        let navigate = driver.command(&navigation);
        let mut navigate = std::pin::pin!(navigate);
        let mut navigating = true;
        // A page whose target went, as one its Session's close closed,
        // sends no event more: the load ends with it.
        let lost = driver.lost();
        let mut lost = std::pin::pin!(lost);
        loop {
            tokio::select! {
                _ = &mut lost => {
                    return Err(network_error(self.request.url, "the browser closed the request's page"));
                }
                navigated = &mut navigate, if navigating => {
                    navigating = false;
                    match navigated {
                        // A load that never reached the network, such as
                        // one to a URL the browser refuses, pauses nothing.
                        Ok(page::NavigateReturns { error_text: Some(error), .. })
                            if self.network_id.is_none() =>
                        {
                            return Err(network_error(self.request.url, NetError::of(&error).name()));
                        }
                        Ok(_) => {}
                        Err(failure) => return Err(network_error(self.request.url, &failure.to_string())),
                    }
                }
                event = events.next() => {
                    let Some(event) = event else {
                        return Err(network_error(self.request.url, "the browser closed the request's page"));
                    };
                    if let Some(paused) = event.read::<fetch::RequestPausedEvent>() {
                        if let Some(ended) = self.paused(paused).await {
                            return ended;
                        }
                    } else if let Some(required) = event.read::<fetch::AuthRequiredEvent>() {
                        self.authenticate(required).await;
                    } else if let Some(finished) = event.read::<network::LoadingFinishedEvent>() {
                        if self.ours(&finished.request_id) && let Some((response, bytes)) = self.answered.take() {
                            if cut_though_finished(self.request.method, &response, bytes.len(), finished.encoded_data_length) {
                                return Ok(cut(response, &bytes));
                            }
                            return self.within_limits(&response, &bytes, finished.encoded_data_length)
                                .map(|()| Fetched { response, body: Body::Whole(bytes) });
                        }
                    } else if let Some(failed) = event.read::<network::LoadingFailedEvent>()
                        && self.ours(&failed.request_id)
                    {
                        return self.failed(&NetError::of(&failed.error_text));
                    }
                }
            }
        }
    }

    /// Whether `network_id` names the load.
    fn ours(&self, network_id: &str) -> bool {
        self.network_id.as_deref() == Some(network_id)
    }

    /// Acts on a paused request or response; answers the outcome once the
    /// load ends there.
    async fn paused(
        &mut self,
        paused: fetch::RequestPausedEvent,
    ) -> Option<Result<Fetched, CallError>> {
        let load = paused.resource_type == network::ResourceType::Document
            && &paused.frame_id == self.driver.main_frame()
            && (self.network_id.is_none() || paused.network_id == self.network_id);
        if !load {
            // No other request of the page goes out, its icon's included.
            self.fail(&paused.request_id, network::ErrorReason::BlockedByClient)
                .await;
            return None;
        }
        let at_response =
            paused.response_status_code.is_some() || paused.response_error_reason.is_some();
        if at_response {
            // The hop's response arrived: its permit is released.
            self.permit = None;
            return self.responded(paused).await;
        }
        let first = self.network_id.is_none();
        let overrides = if first {
            self.network_id.clone_from(&paused.network_id);
            self.overrides(&paused)
        } else {
            Overrides::default()
        };
        let url = overrides
            .url
            .clone()
            .unwrap_or_else(|| paused.request.url.clone());
        self.chain.push(url.clone());
        match self.sender.send_on(&paused, &overrides) {
            Ok(command) => {
                // Step 6: the page's navigation gate, for each hop.
                let navigation = Navigation {
                    method: command
                        .method
                        .clone()
                        .unwrap_or_else(|| paused.request.method.clone()),
                    url,
                    kind: if first {
                        NavigationKind::Request
                    } else {
                        NavigationKind::Redirect
                    },
                };
                match self.driver.admit(&navigation).await {
                    Ok(permit) => self.permit = permit,
                    Err(refusal) => {
                        self.fail(&paused.request_id, network::ErrorReason::Aborted)
                            .await;
                        return Some(Err(site_preferences_refused(&navigation.url, &refusal)));
                    }
                }
                self.send(&command).await;
                // The gate that admitted it hears it leave (ADR 0086).
                self.driver.sent(&navigation.url).await;
                None
            }
            Err(stopped) => {
                self.fail(&paused.request_id, network::ErrorReason::BlockedByClient)
                    .await;
                Some(Err(stopped))
            }
        }
    }

    /// The caller's method, headers and body over what the browser sends
    /// with `paused`, the load's first hop.
    fn overrides(&self, paused: &fetch::RequestPausedEvent) -> Overrides {
        let headers = self.request.headers.map(|given| {
            let mut headers: BTreeMap<String, String> = paused
                .request
                .headers
                .iter()
                .filter(|(name, _)| !given.keys().any(|set| set.eq_ignore_ascii_case(name)))
                .map(|(name, value)| {
                    let value = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned);
                    (name.clone(), value)
                })
                .collect();
            headers.extend(
                given
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
            headers
        });
        Overrides {
            url: None,
            method: self.request.method.map(str::to_owned),
            headers,
            post_data: self.request.post_data.map(str::to_owned),
        }
    }

    /// Acts on the load's response at `paused`: a redirect is followed or
    /// counted, a failure left to the browser to report, and any other read
    /// whole and sent on as text.
    async fn responded(
        &mut self,
        paused: fetch::RequestPausedEvent,
    ) -> Option<Result<Fetched, CallError>> {
        let Some(status) = paused.response_status_code else {
            // The browser reports how it failed as the load fails.
            self.send(&fetch::ContinueRequestParams {
                request_id: paused.request_id.clone(),
                url: None,
                method: None,
                post_data: None,
                headers: None,
                intercept_response: None,
            })
            .await;
            return None;
        };
        let response = self.response_of(&paused, status);
        let redirect = (300..400).contains(&status) && response.headers.contains_key("location");
        if redirect {
            let followed = u32::try_from(self.chain.len().saturating_sub(1)).unwrap_or(u32::MAX);
            if self.request.max_redirects == 0 {
                self.fail(&paused.request_id, network::ErrorReason::Aborted)
                    .await;
                return Some(Ok(Fetched {
                    response,
                    body: Body::Whole(Vec::new()),
                }));
            }
            if followed >= self.request.max_redirects {
                self.fail(&paused.request_id, network::ErrorReason::Aborted)
                    .await;
                return Some(Err(self.too_many_redirects()));
            }
            self.send(&fetch::ContinueRequestParams {
                request_id: paused.request_id.clone(),
                url: None,
                method: None,
                post_data: None,
                headers: None,
                intercept_response: None,
            })
            .await;
            return None;
        }
        if let Err(exceeded) =
            declared_within_limits(&response.url, &response.headers, &self.request.limits)
        {
            self.fail(&paused.request_id, network::ErrorReason::Aborted)
                .await;
            return Some(Err(exceeded));
        }
        if self.request.read == BodyRead::Streamed {
            let read = streamed(
                self.driver,
                &paused.request_id,
                &response.url,
                declared_length(&response),
                &self.request.limits,
            )
            .await;
            // Its body taken, the request cannot go on to the page: the
            // load ends here.
            self.fail(&paused.request_id, network::ErrorReason::Aborted)
                .await;
            return Some(read.map(|bytes| self.streamed_fetched(response, bytes)));
        }
        let bytes = match self.whole(&paused.request_id, &response.url).await {
            Ok(bytes) => bytes,
            Err(refused) => return Some(Err(refused)),
        };
        let headers = paused
            .response_headers
            .iter()
            .flatten()
            .filter(|header| {
                !SHOWN_AS_TEXT
                    .iter()
                    .any(|name| header.name.eq_ignore_ascii_case(name))
            })
            .cloned()
            .chain([fetch::HeaderEntry {
                name: "Content-Type".to_owned(),
                value: "text/plain".to_owned(),
            }])
            .collect();
        self.answered = Some((response, bytes));
        let continued = self
            .driver
            .command(&fetch::ContinueResponseParams {
                request_id: paused.request_id,
                response_code: Some(status),
                response_phrase: None,
                response_headers: Some(headers),
                binary_response_headers: None,
            })
            .await;
        if let Err(failure) = continued {
            tracing::debug!(%failure, "the response went before it was sent on");
        }
        None
    }

    /// The response `paused` holds, of `status`, as a page's response is
    /// answered: its headers' names lower-cased, a repeated one's values on
    /// lines of their own, as the browser gives them.
    fn response_of(&self, paused: &fetch::RequestPausedEvent, status: i64) -> PageResponse {
        let mut headers: BTreeMap<String, String> = BTreeMap::new();
        for header in paused.response_headers.iter().flatten() {
            headers
                .entry(header.name.to_ascii_lowercase())
                .and_modify(|value| {
                    value.push('\n');
                    value.push_str(&header.value);
                })
                .or_insert_with(|| header.value.clone());
        }
        PageResponse {
            url: self
                .chain
                .last()
                .cloned()
                .unwrap_or_else(|| paused.request.url.clone()),
            status: u16::try_from(status).unwrap_or_default(),
            status_text: paused.response_status_text.clone().unwrap_or_default(),
            headers,
            encoding: None,
            request_id: self.network_id.clone().unwrap_or_default(),
            from_service_worker: false,
        }
    }

    /// The outcome of a load the browser failed with `error`: a body that
    /// broke off after its response arrived is cut, one whose content coding
    /// failed to decode, or a request with no response, fails.
    fn failed(&mut self, error: &NetError) -> Result<Fetched, CallError> {
        match self.answered.take() {
            Some((response, bytes)) if !error.fails_to_decode() => Ok(cut(response, &bytes)),
            _ => {
                let url = self.chain.last().map_or(self.request.url, String::as_str);
                let name = self
                    .driver
                    .relay()
                    .and_then(|relay| relay.net_error(url, error.name(), self.relay_mark))
                    .unwrap_or(error.name());
                Err(self
                    .driver
                    .relay()
                    .and_then(|relay| {
                        Some(relay.take_refusal(url, Some(self.relay_mark?))?.error())
                    })
                    .or_else(|| {
                        self.driver
                            .proxy()
                            .and_then(|proxy| proxy.failure(url, name))
                    })
                    .unwrap_or_else(|| network_error(url, name)))
            }
        }
    }

    /// The body of `url`'s response paused at `id`, read whole in one
    /// message: empty, and logged why, when the browser gives none.
    ///
    /// # Errors
    ///
    /// `response_limit_exceeded` naming `network.max_body_size` when the
    /// browser's answer was past what one message carries: the body
    /// decoded far past what any limit reads whole, and the request fails.
    async fn whole(&self, id: &fetch::RequestId, url: &str) -> Result<Vec<u8>, CallError> {
        match self
            .driver
            .command(&fetch::GetResponseBodyParams {
                request_id: id.clone(),
            })
            .await
        {
            Ok(read) if read.base64_encoded => {
                Ok(BASE64.decode(read.body.as_bytes()).unwrap_or_default())
            }
            Ok(read) => Ok(read.body.into_bytes()),
            Err(Failure::TooLarge { .. }) => {
                self.fail(id, network::ErrorReason::Aborted).await;
                Err(self.request.limits.past_the_size(url).error())
            }
            Err(failure) => {
                tracing::debug!(%failure, "the response's body did not read");
                Ok(Vec::new())
            }
        }
    }

    /// `response` with `bytes`, its body read from the browser's stream,
    /// which the limits judged already: cut when shorter than its
    /// `Content-Length` with no content coding, else whole.
    fn streamed_fetched(&self, response: PageResponse, bytes: Vec<u8>) -> Fetched {
        if short_of_its_length(self.request.method, &response, bytes.len()) {
            return cut(response, &bytes);
        }
        Fetched {
            response,
            body: Body::Whole(bytes),
        }
    }

    /// Judges `bytes`, the body of `response` the browser decoded, by the
    /// Run's limits: its size, and its ratio to what it took on the wire,
    /// its `Content-Length` when it declared one, else the browser's count
    /// of encoded bytes, `encoded` (which a body read at the response stage
    /// may leave at 0: no ratio is judged then).
    fn within_limits(
        &self,
        response: &PageResponse,
        bytes: &[u8],
        encoded: f64,
    ) -> Result<(), CallError> {
        let decoded = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let counted = encoded_bytes(encoded);
        let received = declared_length(response).or(counted);
        self.request
            .limits
            .judge_decoded(&response.url, decoded, received)
            .map_err(|exceeded| exceeded.error())
    }

    /// `too_many_redirects` for the load's chain, past its count.
    fn too_many_redirects(&self) -> CallError {
        let first = self
            .chain
            .first()
            .cloned()
            .unwrap_or_else(|| self.request.url.to_owned());
        let hops = self.chain.iter().skip(1).cloned().collect();
        let count = self.request.max_redirects;
        CallError::new(
            &TOO_MANY_REDIRECTS,
            format!(
                "the request to {first} was redirected again after {count} redirects, past its `{}`",
                self.request.option
            ),
            &TooManyRedirectsDetails {
                url: first.clone(),
                hops,
                limit: RedirectLimit::Option {
                    name: self.request.option.to_owned(),
                    count,
                },
            },
        )
    }

    /// Answers a 407 or 401: the Session's proxy's with its credentials,
    /// once; a site's as the browser would with none.
    async fn authenticate(&mut self, required: fetch::AuthRequiredEvent) {
        let proxied = required.auth_challenge.source == Some(fetch::AuthChallengeSource::Proxy)
            && (self.driver.proxy().is_some() || self.driver.relay().is_some());
        let credentials = self.driver.proxy_credentials().filter(|_| proxied);
        let mut credentials_given = false;
        let response = match credentials {
            Some((username, password)) if !self.authenticated => {
                self.authenticated = true;
                credentials_given = true;
                fetch::AuthChallengeResponse {
                    response: fetch::AuthChallengeResponseResponse::ProvideCredentials,
                    username: Some(username.to_owned()),
                    password: Some(password.to_owned()),
                }
            }
            _ if proxied => fetch::AuthChallengeResponse {
                response: fetch::AuthChallengeResponseResponse::CancelAuth,
                username: None,
                password: None,
            },
            _ => fetch::AuthChallengeResponse {
                response: fetch::AuthChallengeResponseResponse::Default,
                username: None,
                password: None,
            },
        };
        self.send(&fetch::ContinueWithAuthParams {
            request_id: required.request_id,
            auth_challenge_response: response,
        })
        .await;
        // The load's hop leaves only now, with the credentials: its gate
        // hears that send too (ADR 0086).
        if credentials_given {
            self.driver.sent(&required.request.url).await;
        }
    }

    /// Sends `command` for a paused request, which may be gone.
    async fn send<C: goodfetch_cdp::Command>(&self, command: &C) {
        if let Err(failure) = self.driver.command(command).await {
            tracing::debug!(%failure, "a paused request went before the command reached it");
        }
    }

    /// Fails the paused request `id` with `reason`.
    async fn fail(&self, id: &str, reason: network::ErrorReason) {
        self.send(&fetch::FailRequestParams {
            request_id: id.to_owned(),
            error_reason: reason,
        })
        .await;
    }
}

#[cfg(test)]
mod tests;
