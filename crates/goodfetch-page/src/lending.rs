//! Scoped WebSocket CDP endpoint for a Lent page (`page.lend()`, ADRs 0017,
//! 0059, 0060, 0067, 0089 and 0140).
//!
//! Opens a loopback WebSocket listener (binding port 0 and reporting its
//! address, ADR 0059), validates `Host`, `Origin` and a per-attach header
//! token on upgrade, and relays CDP commands through `goodfetch-cdp`'s single
//! browser connection (`PageDriver::send_raw_cdp_on`), never a second socket
//! or pipe to the browser (ADR 0140).
//!
//! **Auto-attach mechanism (ADR 0089):** Chromium's browser-level
//! `Target.setAutoAttach` attaches to every target in every browser context,
//! and `Target.autoAttachRelated` only follows targets related to one target
//! (missing tabs the library creates via `Target.createTarget`) and does not
//! filter `Target.getTargets` or `Target.setDiscoverTargets`. The endpoint
//! therefore maintains the exact visible target tree (`lent_target_id`,
//! targets created by the library via `Target.createTarget`, and child
//! targets/popups opened from either) and performs endpoint-side filtering on
//! `Target.getTargets`, `Target.setDiscoverTargets`, and browser-level
//! `Target.setAutoAttach`.
//!
//! Size: one concern, the scoped WebSocket CDP lending endpoint, its command
//! classifier, and its target-tree filter.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_engine::CallError;
use goodfetch_engine::endpoint::{self, Authorities};
use goodfetch_protocol::page::PageId;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tokio_util::sync::CancellationToken;

use crate::cdp::driver::PageDriver;
use crate::errors::Call;
use crate::sessions::OpenPage;
use crate::trace;

/// The maximum WebSocket frame / message size for a lent CDP connection
/// (matching Chromium's large snapshots and screenshots).
const MAX_MESSAGE_BYTES: usize = 256 * 1024 * 1024;

/// Standard CDP JSON-RPC server error code for refused commands.
const CDP_REFUSED_CODE: i64 = -32000;

/// What the endpoint decides for one incoming CDP command (ADR 0089).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommandDecision {
    /// Forward directly to the browser on the validated session.
    Pass,
    /// Inject the lent page's `browserContextId` when omitted; refuse when
    /// another context is named.
    RescopeContext,
    /// `Browser.close`: answer `{}` and end the attach, never closing the
    /// browser.
    EndAttach,
    /// Download-behaviour commands: answer `{}` without forwarding so
    /// downloads stay the Run's.
    NoOp,
    /// Target discovery, creation, attachment and lifecycle commands handled
    /// against the endpoint's visible target tree.
    HandleTargets,
    /// Refuse as a CDP error naming `cdp_command_refused`.
    Refuse(&'static str),
}

/// Validates a WebSocket upgrade request's `Host`, `Origin`, and per-attach
/// `Authorization: Bearer <token>` header (ADRs 0017, 0067, 0089).
pub(crate) fn validate_handshake(
    host: Option<&str>,
    origin: Option<&str>,
    authorization: Option<&str>,
    authorities: &Authorities,
    expected_token: &str,
) -> Result<(), (StatusCode, &'static str)> {
    let Some(host) = host else {
        return Err((StatusCode::FORBIDDEN, "Host header is required"));
    };
    if !authorities.admits_host(host) {
        return Err((StatusCode::FORBIDDEN, "Host header is not allowed"));
    }
    if let Some(origin) = origin
        && !authorities.admits_origin(origin)
    {
        return Err((StatusCode::FORBIDDEN, "Origin header is not allowed"));
    }
    let Some(auth) = authorization else {
        return Err((StatusCode::UNAUTHORIZED, "Authorization token is required"));
    };
    let Some((scheme, token)) = auth.split_once(' ') else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Authorization header must use Bearer",
        ));
    };
    if !scheme.eq_ignore_ascii_case("bearer") || !endpoint::same_token(token.trim(), expected_token)
    {
        return Err((StatusCode::UNAUTHORIZED, "Authorization token is invalid"));
    }
    Ok(())
}

/// Classifies a CDP command sent by a lent library according to ADR 0089's
/// command table (fail closed at browser level).
#[expect(
    clippy::too_many_lines,
    reason = "single cohesive ADR 0089 command classification table"
)]
pub(crate) fn classify_command(
    method: &str,
    session_id: Option<&str>,
    params: &Value,
    browser_context_id: &str,
    shared_host: bool,
) -> CommandDecision {
    // 1. Ending the attach: never closes the real browser.
    if method == "Browser.close" {
        return CommandDecision::EndAttach;
    }

    // 2. Answered without effect: downloads stay the Run's.
    if matches!(
        method,
        "Browser.setDownloadBehavior" | "Page.setDownloadBehavior"
    ) {
        return CommandDecision::NoOp;
    }

    // 3. Always refused on both browser and target sessions.
    if method.starts_with("Extensions.") {
        return CommandDecision::Refuse("the Extensions domain is refused on a Lent page");
    }
    match method {
        "Target.createBrowserContext" | "Target.disposeBrowserContext" => {
            return CommandDecision::Refuse(
                "creating or disposing a browser context is refused on a Lent page",
            );
        }
        "Target.attachToBrowserTarget" => {
            return CommandDecision::Refuse(
                "attaching to the browser target is refused on a Lent page",
            );
        }
        "Target.exposeDevToolsProtocol" => {
            return CommandDecision::Refuse(
                "Target.exposeDevToolsProtocol is refused on a Lent page",
            );
        }
        "Browser.crash" | "Browser.crashGpuProcess" => {
            return CommandDecision::Refuse("crashing the browser is refused on a Lent page");
        }
        "Security.setIgnoreCertificateErrors" => {
            return CommandDecision::Refuse(
                "ignoring certificate errors is refused on a Lent page",
            );
        }
        _ => {}
    }

    // 4. Shared-host restrictions (#1377 / ADR 0089): refuse local file reads.
    if shared_host {
        if method == "DOM.setFileInputFiles" {
            return CommandDecision::Refuse(
                "uploading files by engine path is refused under shared_host",
            );
        }
        if method == "Page.navigate"
            && params
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|url| url.trim_start().to_ascii_lowercase().starts_with("file:"))
        {
            return CommandDecision::Refuse("file: navigation is refused under shared_host");
        }
    }

    // 5. Context-bearing commands: verify context if specified, rescope if omitted.
    if matches!(
        method,
        "Storage.getCookies"
            | "Storage.setCookies"
            | "Storage.clearCookies"
            | "Browser.grantPermissions"
            | "Browser.setPermission"
            | "Browser.resetPermissions"
    ) {
        if let Some(named) = params.get("browserContextId").and_then(Value::as_str)
            && named != browser_context_id
        {
            return CommandDecision::Refuse("command names another browser context");
        }
        return CommandDecision::RescopeContext;
    }

    // 6. Target management commands.
    if matches!(
        method,
        "Target.getTargets"
            | "Target.getBrowserContexts"
            | "Target.setDiscoverTargets"
            | "Target.setAutoAttach"
            | "Target.attachToTarget"
            | "Target.detachFromTarget"
            | "Target.createTarget"
            | "Target.closeTarget"
            | "Target.activateTarget"
            | "Target.getTargetInfo"
            | "Target.sendMessageToTarget"
    ) {
        if method == "Target.createTarget"
            && let Some(named) = params.get("browserContextId").and_then(Value::as_str)
            && named != browser_context_id
        {
            return CommandDecision::Refuse("Target.createTarget names another browser context");
        }
        return CommandDecision::HandleTargets;
    }

    // 7. Browser-level commands fail closed unless explicitly classed above or
    //    in the safe read-only / stream / window table below.
    if session_id.is_none() {
        return match method {
            "Browser.getVersion"
            | "Browser.getWindowForTarget"
            | "Browser.getWindowBounds"
            | "Browser.setWindowBounds"
            | "Browser.setDockTile"
            | "SystemInfo.getInfo"
            | "IO.read"
            | "IO.close" => CommandDecision::Pass,
            _ => CommandDecision::Refuse(
                "browser-level command is not allowed on a Lent page endpoint",
            ),
        };
    }

    // 8. Target-level commands on a verified visible session pass through.
    CommandDecision::Pass
}

/// An active lending WebSocket endpoint bound to one page.
pub(crate) struct LendEndpoint {
    /// The WebSocket URL (`ws://127.0.0.1:<port>/devtools/browser/<id>`).
    pub(crate) url: String,
    /// The headers carrying the per-attach token.
    pub(crate) headers: BTreeMap<String, String>,
    /// Cancelling this stops the listener and disconnects any active library.
    shutdown: CancellationToken,
}

impl LendEndpoint {
    /// Stops the lending listener and disconnects any active library.
    pub(crate) fn shutdown(&self) {
        self.shutdown.cancel();
    }
}

/// Parameters for starting a lending endpoint.
pub(crate) struct LendStart {
    pub(crate) endpoint_id: String,
    pub(crate) page_id: PageId,
    pub(crate) page: OpenPage,
    pub(crate) shared_host: bool,
    /// Callback invoked once when the attach ends from the library/listener side
    /// (with the reason string, e.g. `"browser_close"`, `"client_disconnected"`,
    /// `"attach_ended"`).
    pub(crate) on_end: Arc<dyn Fn(&'static str) + Send + Sync>,
}

/// Opens a loopback WebSocket lending listener bound to port 0 (ADRs 0059,
/// 0060, 0089).
#[expect(
    clippy::too_many_lines,
    reason = "binds listener, validates WebSocket upgrade handshake, and drives accept loop"
)]
pub(crate) async fn start(params: LendStart) -> Result<LendEndpoint, CallError> {
    #[expect(
        clippy::disallowed_types,
        reason = "ADR 0060: lending WebSocket listener binds loopback port 0 only in lending"
    )]
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| {
            CallError::new(
                &goodfetch_protocol::connection::INTERNAL,
                format!("could not bind lending WebSocket listener: {error}"),
                &json!({ "kind": "lending_bind_failed" }),
            )
        })?;
    let address = listener.local_addr().map_err(|error| {
        CallError::new(
            &goodfetch_protocol::connection::INTERNAL,
            format!("could not read lending listener address: {error}"),
            &json!({ "kind": "lending_local_addr_failed" }),
        )
    })?;
    let token = endpoint::new_token().map_err(|error| {
        CallError::new(
            &goodfetch_protocol::connection::INTERNAL,
            format!("could not generate lending attach token: {error}"),
            &json!({ "kind": "lending_token_failed" }),
        )
    })?;

    let authorities = Authorities::loopback(address);
    let url = format!("ws://{address}/devtools/browser/{}", params.endpoint_id);
    let mut headers = BTreeMap::new();
    headers.insert("Authorization".to_owned(), format!("Bearer {token}"));

    let shutdown = CancellationToken::new();
    let task_shutdown = shutdown.clone();
    let page = params.page.clone();
    let page_id = params.page_id.clone();
    let shared_host = params.shared_host;
    let on_end = Arc::clone(&params.on_end);

    let owner = page.owner.clone();
    owner.tasks().spawn(async move {
        let mut connected_once = false;
        loop {
            let accept = tokio::select! {
                biased;
                () = task_shutdown.cancelled() => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, _peer)) = accept else {
                break;
            };
            if task_shutdown.is_cancelled() {
                break;
            }
            let auth_check = authorities.clone();
            let expected = token.clone();
            #[expect(
                clippy::result_large_err,
                reason = "tokio-tungstenite accept_hdr callback requires ErrorResponse"
            )]
            let callback = move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
                let host = request
                    .headers()
                    .get("host")
                    .and_then(|v| v.to_str().ok());
                let origin = request
                    .headers()
                    .get("origin")
                    .and_then(|v| v.to_str().ok());
                let auth = request
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok());
                if let Err((status, message)) =
                    validate_handshake(host, origin, auth, &auth_check, &expected)
                {
                    let mut reply = ErrorResponse::new(Some(message.to_owned()));
                    *reply.status_mut() = status;
                    reply
                        .headers_mut()
                        .insert("content-type", HeaderValue::from_static("text/plain"));
                    return Err(reply);
                }
                Ok(response)
            };

            let mut config = WebSocketConfig::default();
            config.max_message_size = Some(MAX_MESSAGE_BYTES);
            config.max_frame_size = Some(MAX_MESSAGE_BYTES);

            let ws = match tokio_tungstenite::accept_hdr_async_with_config(
                stream,
                callback,
                Some(config),
            )
            .await
            {
                Ok(ws) => ws,
                Err(error) => {
                    tracing::debug!(%error, page_id = %page_id.0, "lending WebSocket handshake refused or failed");
                    continue;
                }
            };

            connected_once = true;
            let end_reason = serve_connection(
                ws,
                &page,
                &page_id,
                shared_host,
                task_shutdown.clone(),
            )
            .await;
            // Once the library's connection ends (Browser.close, disconnect,
            // lease expiry, or shutdown), revoke the token and end the attach.
            task_shutdown.cancel();
            (on_end)(end_reason);
            break;
        }
        if !connected_once && !task_shutdown.is_cancelled() {
            task_shutdown.cancel();
            (on_end)("listener_stopped");
        }
    });

    Ok(LendEndpoint {
        url,
        headers,
        shutdown,
    })
}

/// An event forwarded from an attached target session back to the WS loop.
struct SessionEventMsg {
    session_id: String,
    method: String,
    params: Value,
}

/// Serves one authenticated lending WebSocket connection until it closes,
/// sends `Browser.close`, its lease expires, or `shutdown` cancels.
#[expect(
    clippy::too_many_lines,
    reason = "single select loop multiplexing WebSocket commands, browser events, target events, and shutdown cleanup"
)]
async fn serve_connection<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    page: &OpenPage,
    page_id: &PageId,
    shared_host: bool,
    shutdown: CancellationToken,
) -> &'static str
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut ws_tx, mut ws_rx) = ws.split();
    let driver = &page.driver;
    let ctx_id = driver.browser_context_id().to_owned();
    let lent_target_id = driver.main_frame().clone();

    let mut visible_targets: BTreeSet<String> = BTreeSet::new();
    visible_targets.insert(lent_target_id.clone());
    let mut created_targets: BTreeSet<String> = BTreeSet::new();
    let mut client_sessions: BTreeMap<String, String> = BTreeMap::new();
    let mut runtime_enabled: BTreeSet<String> = BTreeSet::new();
    let mut discover_targets = false;
    let mut auto_attach_browser = false;

    let mut browser_events = driver.subscribe_raw_cdp(None);
    let (event_tx, mut event_rx) = mpsc::channel::<SessionEventMsg>(256);
    let connection_stop = shutdown.child_token();

    let end_reason = loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => {
                break "closed";
            }
            Some(msg) = event_rx.recv() => {
                if !client_sessions.contains_key(&msg.session_id) {
                    continue;
                }
                // Track child targets auto-attached on a visible target's session.
                if msg.method == "Target.attachedToTarget"
                    && let Some(child_sid) = msg.params.get("sessionId").and_then(Value::as_str)
                    && let Some(child_tid) = msg
                        .params
                        .get("targetInfo")
                        .and_then(|info| info.get("targetId"))
                        .and_then(Value::as_str)
                {
                    let parent_is_created = client_sessions
                        .get(&msg.session_id)
                        .is_some_and(|parent_tid| created_targets.contains(parent_tid));
                    visible_targets.insert(child_tid.to_owned());
                    if parent_is_created {
                        created_targets.insert(child_tid.to_owned());
                    }
                    if !client_sessions.contains_key(child_sid) {
                        client_sessions.insert(child_sid.to_owned(), child_tid.to_owned());
                        spawn_session_forwarder(
                            driver,
                            page,
                            child_sid.to_owned(),
                            event_tx.clone(),
                            connection_stop.clone(),
                        );
                    }
                } else if msg.method == "Target.detachedFromTarget"
                    && let Some(child_sid) = msg.params.get("sessionId").and_then(Value::as_str)
                {
                    client_sessions.remove(child_sid);
                    runtime_enabled.remove(child_sid);
                }

                let frame = json!({
                    "sessionId": msg.session_id,
                    "method": msg.method,
                    "params": msg.params,
                });
                if send_ws_json(&mut ws_tx, &frame).await.is_err() {
                    break "client_disconnected";
                }
            }
            next = browser_events.next() => {
                let Some(event) = next else {
                    break "browser_closed";
                };
                if handle_browser_event(
                    &event.method,
                    &event.raw_params(),
                    &ctx_id,
                    &mut visible_targets,
                    &mut created_targets,
                    &mut client_sessions,
                    discover_targets,
                    auto_attach_browser,
                    driver,
                    page,
                    &event_tx,
                    &connection_stop,
                    &mut ws_tx,
                )
                .await
                .is_err()
                {
                    break "client_disconnected";
                }
            }
            incoming = ws_rx.next() => {
                let Some(Ok(message)) = incoming else {
                    break "client_disconnected";
                };
                match message {
                    Message::Close(_) => break "client_disconnected",
                    Message::Ping(payload) => {
                        if ws_tx.send(Message::Pong(payload)).await.is_err() {
                            break "client_disconnected";
                        }
                    }
                    Message::Text(text) => {
                        let Ok(req) = serde_json::from_str::<Value>(text.as_str()) else {
                            continue;
                        };
                        let id = req.get("id").cloned().unwrap_or(Value::Null);
                        let Some(method) = req.get("method").and_then(Value::as_str) else {
                            continue;
                        };
                        let session_id = req.get("sessionId").and_then(Value::as_str);
                        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

                        // Check handler lease before every command (ADR 0003).
                        if let Some(lease) = page.lease()
                            && lease.check().is_err()
                        {
                            let _ = send_ws_error(
                                &mut ws_tx,
                                &id,
                                session_id,
                                "attach_ended: work item lease expired",
                            )
                            .await;
                            break "attach_ended";
                        }

                        // Verify session_id belongs to this client's visible targets.
                        if let Some(sid) = session_id
                            && !client_sessions.contains_key(sid)
                        {
                            record_refused(page, page_id, method, "unknown or out-of-scope sessionId").await;
                            if send_ws_error(
                                &mut ws_tx,
                                &id,
                                session_id,
                                "cdp_command_refused: unknown or out-of-scope sessionId",
                            )
                            .await
                            .is_err()
                            {
                                break "client_disconnected";
                            }
                            continue;
                        }

                        let decision = classify_command(method, session_id, &params, &ctx_id, shared_host);
                        match decision {
                            CommandDecision::EndAttach => {
                                let _ = send_ws_result(&mut ws_tx, &id, session_id, json!({})).await;
                                break "browser_close";
                            }
                            CommandDecision::NoOp => {
                                if send_ws_result(&mut ws_tx, &id, session_id, json!({})).await.is_err() {
                                    break "client_disconnected";
                                }
                            }
                            CommandDecision::Refuse(reason) => {
                                record_refused(page, page_id, method, reason).await;
                                if send_ws_error(
                                    &mut ws_tx,
                                    &id,
                                    session_id,
                                    &format!("cdp_command_refused: {reason}"),
                                )
                                .await
                                .is_err()
                                {
                                    break "client_disconnected";
                                }
                            }
                            CommandDecision::RescopeContext => {
                                let mut rescoped = params;
                                if !ctx_id.is_empty()
                                    && let Some(obj) = rescoped.as_object_mut()
                                    && !obj.contains_key("browserContextId")
                                {
                                    obj.insert("browserContextId".to_owned(), Value::String(ctx_id.clone()));
                                }
                                if forward_raw(driver, &mut ws_tx, &id, session_id, method, rescoped)
                                    .await
                                    .is_err()
                                {
                                    break "client_disconnected";
                                }
                            }
                            CommandDecision::HandleTargets => {
                                let mut state = TargetHandlerState {
                                    ctx_id: &ctx_id,
                                    visible_targets: &mut visible_targets,
                                    created_targets: &mut created_targets,
                                    client_sessions: &mut client_sessions,
                                    runtime_enabled: &mut runtime_enabled,
                                    discover_targets: &mut discover_targets,
                                    auto_attach_browser: &mut auto_attach_browser,
                                    event_tx: &event_tx,
                                    connection_stop: &connection_stop,
                                };
                                if handle_target_command(
                                    driver,
                                    page,
                                    page_id,
                                    &mut ws_tx,
                                    &id,
                                    session_id,
                                    method,
                                    params,
                                    &mut state,
                                )
                                .await
                                .is_err()
                                {
                                    break "client_disconnected";
                                }
                            }
                            CommandDecision::Pass => {
                                if let Some(sid) = session_id {
                                    if method == "Runtime.enable" {
                                        runtime_enabled.insert(sid.to_owned());
                                    } else if method == "Runtime.disable" {
                                        runtime_enabled.remove(sid);
                                    }
                                }
                                if forward_raw(driver, &mut ws_tx, &id, session_id, method, params)
                                    .await
                                    .is_err()
                                {
                                    break "client_disconnected";
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    };

    // Stop all per-session forwarders.
    connection_stop.cancel();

    // Cleanup on attach end (ADR 0089 & #913):
    // 1. Disable Runtime on any session where the library enabled it.
    let cleanup_call = Call::Action {
        operation: "cdp_endpoint.close",
    };
    for sid in &runtime_enabled {
        let _ = driver
            .send_raw_cdp_on(
                Some(sid),
                "Runtime.disable".to_owned(),
                json!({}),
                &cleanup_call,
            )
            .await;
    }
    // 2. Detach all client CDP sessions.
    for sid in client_sessions.keys() {
        let _ = driver
            .send_raw_cdp_on(
                None,
                "Target.detachFromTarget".to_owned(),
                json!({ "sessionId": sid }),
                &cleanup_call,
            )
            .await;
    }
    // 3. Close targets created by the library (and popups opened from them);
    //    never close the lent page itself or popups opened from it.
    for tid in &created_targets {
        if tid != &lent_target_id {
            let _ = driver
                .send_raw_cdp_on(
                    None,
                    "Target.closeTarget".to_owned(),
                    json!({ "targetId": tid }),
                    &cleanup_call,
                )
                .await;
        }
    }

    end_reason
}

struct TargetHandlerState<'a> {
    ctx_id: &'a str,
    visible_targets: &'a mut BTreeSet<String>,
    created_targets: &'a mut BTreeSet<String>,
    client_sessions: &'a mut BTreeMap<String, String>,
    runtime_enabled: &'a mut BTreeSet<String>,
    discover_targets: &'a mut bool,
    auto_attach_browser: &'a mut bool,
    event_tx: &'a mpsc::Sender<SessionEventMsg>,
    connection_stop: &'a CancellationToken,
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "handles scoped Target.* commands with shared target state"
)]
async fn handle_target_command<S>(
    driver: &PageDriver,
    page: &OpenPage,
    page_id: &PageId,
    ws_tx: &mut S,
    id: &Value,
    session_id: Option<&str>,
    method: &str,
    params: Value,
    state: &mut TargetHandlerState<'_>,
) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let call = Call::Action {
        operation: "page.lend",
    };
    match method {
        "Target.getBrowserContexts" => {
            let contexts = if state.ctx_id.is_empty() {
                Vec::<Value>::new()
            } else {
                vec![Value::String(state.ctx_id.to_owned())]
            };
            send_ws_result(
                ws_tx,
                id,
                session_id,
                json!({ "browserContextIds": contexts }),
            )
            .await
        }
        "Target.getTargets" => {
            match driver
                .send_raw_cdp_on(None, method.to_owned(), params, &call)
                .await
            {
                Ok(mut answer) => {
                    if let Some(infos) = answer.get_mut("targetInfos").and_then(Value::as_array_mut)
                    {
                        infos.retain(|info| {
                            info.get("targetId")
                                .and_then(Value::as_str)
                                .is_some_and(|tid| state.visible_targets.contains(tid))
                        });
                    }
                    send_ws_result(ws_tx, id, session_id, answer).await
                }
                Err(error) => send_ws_error(ws_tx, id, session_id, &error.to_string()).await,
            }
        }
        "Target.getTargetInfo" => {
            let requested = params.get("targetId").and_then(Value::as_str);
            if let Some(tid) = requested
                && !state.visible_targets.contains(tid)
            {
                record_refused(
                    page,
                    page_id,
                    method,
                    "target is outside the Lent page's scope",
                )
                .await;
                return send_ws_error(
                    ws_tx,
                    id,
                    session_id,
                    "cdp_command_refused: target is outside the Lent page's scope",
                )
                .await;
            }
            forward_raw(driver, ws_tx, id, session_id, method, params).await
        }
        "Target.createTarget" => {
            let mut create_params = params;
            if !state.ctx_id.is_empty()
                && let Some(obj) = create_params.as_object_mut()
                && !obj.contains_key("browserContextId")
            {
                obj.insert(
                    "browserContextId".to_owned(),
                    Value::String(state.ctx_id.to_owned()),
                );
            }
            match driver
                .send_raw_cdp_on(None, method.to_owned(), create_params, &call)
                .await
            {
                Ok(answer) => {
                    if let Some(tid) = answer.get("targetId").and_then(Value::as_str) {
                        state.visible_targets.insert(tid.to_owned());
                        state.created_targets.insert(tid.to_owned());
                        if *state.auto_attach_browser {
                            let _ = attach_visible_target(
                                driver,
                                page,
                                tid,
                                state.client_sessions,
                                state.event_tx,
                                state.connection_stop,
                                ws_tx,
                            )
                            .await;
                        }
                    }
                    send_ws_result(ws_tx, id, session_id, answer).await
                }
                Err(error) => send_ws_error(ws_tx, id, session_id, &error.to_string()).await,
            }
        }
        "Target.attachToTarget" => {
            let Some(tid) = params.get("targetId").and_then(Value::as_str) else {
                return send_ws_error(
                    ws_tx,
                    id,
                    session_id,
                    "cdp_command_refused: targetId required",
                )
                .await;
            };
            if !state.visible_targets.contains(tid) {
                record_refused(
                    page,
                    page_id,
                    method,
                    "cannot attach to target outside Lent page",
                )
                .await;
                return send_ws_error(
                    ws_tx,
                    id,
                    session_id,
                    "cdp_command_refused: cannot attach to target outside Lent page",
                )
                .await;
            }
            match driver
                .send_raw_cdp_on(None, method.to_owned(), params.clone(), &call)
                .await
            {
                Ok(answer) => {
                    if let Some(sid) = answer.get("sessionId").and_then(Value::as_str) {
                        state.client_sessions.insert(sid.to_owned(), tid.to_owned());
                        spawn_session_forwarder(
                            driver,
                            page,
                            sid.to_owned(),
                            state.event_tx.clone(),
                            state.connection_stop.clone(),
                        );
                    }
                    send_ws_result(ws_tx, id, session_id, answer).await
                }
                Err(error) => send_ws_error(ws_tx, id, session_id, &error.to_string()).await,
            }
        }
        "Target.detachFromTarget" => {
            if let Some(sid) = params.get("sessionId").and_then(Value::as_str) {
                if !state.client_sessions.contains_key(sid) {
                    record_refused(
                        page,
                        page_id,
                        method,
                        "cannot detach session outside Lent page",
                    )
                    .await;
                    return send_ws_error(
                        ws_tx,
                        id,
                        session_id,
                        "cdp_command_refused: cannot detach session outside Lent page",
                    )
                    .await;
                }
                state.client_sessions.remove(sid);
                state.runtime_enabled.remove(sid);
            }
            forward_raw(driver, ws_tx, id, session_id, method, params).await
        }
        "Target.closeTarget" | "Target.activateTarget" => {
            let Some(tid) = params.get("targetId").and_then(Value::as_str) else {
                return send_ws_error(
                    ws_tx,
                    id,
                    session_id,
                    "cdp_command_refused: targetId required",
                )
                .await;
            };
            if !state.visible_targets.contains(tid) {
                record_refused(
                    page,
                    page_id,
                    method,
                    "target is outside the Lent page's scope",
                )
                .await;
                return send_ws_error(
                    ws_tx,
                    id,
                    session_id,
                    "cdp_command_refused: target is outside the Lent page's scope",
                )
                .await;
            }
            if method == "Target.closeTarget" {
                state.visible_targets.remove(tid);
                state.created_targets.remove(tid);
            }
            forward_raw(driver, ws_tx, id, session_id, method, params).await
        }
        "Target.setDiscoverTargets" => {
            let discover = params
                .get("discover")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            *state.discover_targets = discover;
            if discover
                && let Ok(answer) = driver
                    .send_raw_cdp_on(None, "Target.getTargets".to_owned(), json!({}), &call)
                    .await
                && let Some(infos) = answer.get("targetInfos").and_then(Value::as_array)
            {
                for info in infos {
                    if info
                        .get("targetId")
                        .and_then(Value::as_str)
                        .is_some_and(|tid| state.visible_targets.contains(tid))
                    {
                        let evt = json!({
                            "method": "Target.targetCreated",
                            "params": { "targetInfo": info },
                        });
                        send_ws_json(ws_tx, &evt).await?;
                    }
                }
            }
            send_ws_result(ws_tx, id, session_id, json!({})).await
        }
        "Target.setAutoAttach" => {
            if session_id.is_some() {
                // On an already-verified visible target's session, forward so
                // its OOPIFs and dedicated workers auto-attach.
                return forward_raw(driver, ws_tx, id, session_id, method, params).await;
            }
            let enable = params
                .get("autoAttach")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            *state.auto_attach_browser = enable;
            if enable {
                let already_attached: BTreeSet<String> =
                    state.client_sessions.values().cloned().collect();
                let to_attach: Vec<String> = state
                    .visible_targets
                    .iter()
                    .filter(|tid| !already_attached.contains(*tid))
                    .cloned()
                    .collect();
                for tid in to_attach {
                    attach_visible_target(
                        driver,
                        page,
                        &tid,
                        state.client_sessions,
                        state.event_tx,
                        state.connection_stop,
                        ws_tx,
                    )
                    .await?;
                }
            }
            send_ws_result(ws_tx, id, session_id, json!({})).await
        }
        "Target.sendMessageToTarget" => {
            if let Some(sid) = params.get("sessionId").and_then(Value::as_str)
                && !state.client_sessions.contains_key(sid)
            {
                record_refused(
                    page,
                    page_id,
                    method,
                    "sessionId is outside the Lent page's scope",
                )
                .await;
                return send_ws_error(
                    ws_tx,
                    id,
                    session_id,
                    "cdp_command_refused: sessionId is outside the Lent page's scope",
                )
                .await;
            }
            forward_raw(driver, ws_tx, id, session_id, method, params).await
        }
        _ => {
            record_refused(page, page_id, method, "target command refused").await;
            send_ws_error(
                ws_tx,
                id,
                session_id,
                "cdp_command_refused: target command refused",
            )
            .await
        }
    }
}

/// Attaches the library to a visible target and emits `Target.attachedToTarget`.
async fn attach_visible_target<S>(
    driver: &PageDriver,
    page: &OpenPage,
    target_id: &str,
    client_sessions: &mut BTreeMap<String, String>,
    event_tx: &mpsc::Sender<SessionEventMsg>,
    connection_stop: &CancellationToken,
    ws_tx: &mut S,
) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let call = Call::Action {
        operation: "page.lend",
    };
    let Ok(attached) = driver
        .send_raw_cdp_on(
            None,
            "Target.attachToTarget".to_owned(),
            json!({ "targetId": target_id, "flatten": true }),
            &call,
        )
        .await
    else {
        return Ok(());
    };
    let Some(sid) = attached.get("sessionId").and_then(Value::as_str) else {
        return Ok(());
    };
    client_sessions.insert(sid.to_owned(), target_id.to_owned());
    spawn_session_forwarder(
        driver,
        page,
        sid.to_owned(),
        event_tx.clone(),
        connection_stop.clone(),
    );
    let target_info = driver
        .send_raw_cdp_on(
            None,
            "Target.getTargetInfo".to_owned(),
            json!({ "targetId": target_id }),
            &call,
        )
        .await
        .ok()
        .and_then(|v| v.get("targetInfo").cloned())
        .unwrap_or_else(|| json!({ "targetId": target_id, "type": "page", "attached": true }));

    let evt = json!({
        "method": "Target.attachedToTarget",
        "params": {
            "sessionId": sid,
            "targetInfo": target_info,
            "waitingForDebugger": false,
        },
    });
    send_ws_json(ws_tx, &evt).await
}

/// Filters browser-level `Target.*` events to the endpoint's visible target tree.
#[expect(
    clippy::too_many_arguments,
    reason = "routes browser-level Target events with target visibility state"
)]
async fn handle_browser_event<S>(
    method: &str,
    params: &Value,
    ctx_id: &str,
    visible_targets: &mut BTreeSet<String>,
    created_targets: &mut BTreeSet<String>,
    client_sessions: &mut BTreeMap<String, String>,
    discover_targets: bool,
    auto_attach_browser: bool,
    driver: &PageDriver,
    page: &OpenPage,
    event_tx: &mpsc::Sender<SessionEventMsg>,
    connection_stop: &CancellationToken,
    ws_tx: &mut S,
) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    match method {
        "Target.targetCreated" => {
            let Some(info) = params.get("targetInfo") else {
                return Ok(());
            };
            let Some(tid) = info.get("targetId").and_then(Value::as_str) else {
                return Ok(());
            };
            let same_context = ctx_id.is_empty()
                || info
                    .get("browserContextId")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c == ctx_id);
            let opened_from_visible = info
                .get("openerId")
                .and_then(Value::as_str)
                .is_some_and(|opener| visible_targets.contains(opener));
            let opened_from_created = info
                .get("openerId")
                .and_then(Value::as_str)
                .is_some_and(|opener| created_targets.contains(opener));

            if same_context && opened_from_visible && !visible_targets.contains(tid) {
                visible_targets.insert(tid.to_owned());
                if opened_from_created {
                    created_targets.insert(tid.to_owned());
                }
                if discover_targets {
                    send_ws_json(ws_tx, &json!({ "method": method, "params": params })).await?;
                }
                if auto_attach_browser {
                    attach_visible_target(
                        driver,
                        page,
                        tid,
                        client_sessions,
                        event_tx,
                        connection_stop,
                        ws_tx,
                    )
                    .await?;
                }
            } else if visible_targets.contains(tid) && discover_targets {
                send_ws_json(ws_tx, &json!({ "method": method, "params": params })).await?;
            }
        }
        "Target.targetInfoChanged" => {
            let visible = params
                .get("targetInfo")
                .and_then(|info| info.get("targetId"))
                .and_then(Value::as_str)
                .is_some_and(|tid| visible_targets.contains(tid));
            if visible && discover_targets {
                send_ws_json(ws_tx, &json!({ "method": method, "params": params })).await?;
            }
        }
        "Target.targetDestroyed" => {
            let Some(tid) = params.get("targetId").and_then(Value::as_str) else {
                return Ok(());
            };
            if visible_targets.remove(tid) {
                created_targets.remove(tid);
                if discover_targets {
                    send_ws_json(ws_tx, &json!({ "method": method, "params": params })).await?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Spawns a task under the page's `TaskOwner` that forwards raw CDP events from
/// `session_id` to `event_tx` until `stop` cancels or the session ends.
fn spawn_session_forwarder(
    driver: &PageDriver,
    page: &OpenPage,
    session_id: String,
    event_tx: mpsc::Sender<SessionEventMsg>,
    stop: CancellationToken,
) {
    let mut subscription = driver.subscribe_raw_cdp(Some(&session_id));
    let owner = page.owner.clone();
    owner.tasks().spawn(async move {
        loop {
            let event = tokio::select! {
                biased;
                () = stop.cancelled() => break,
                next = subscription.next() => {
                    let Some(event) = next else {
                        break;
                    };
                    event
                }
            };
            if event_tx
                .send(SessionEventMsg {
                    session_id: session_id.clone(),
                    method: event.method.clone(),
                    params: event.raw_params(),
                })
                .await
                .is_err()
            {
                break;
            }
        }
    });
}

async fn forward_raw<S>(
    driver: &PageDriver,
    ws_tx: &mut S,
    id: &Value,
    session_id: Option<&str>,
    method: &str,
    params: Value,
) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let call = Call::Action {
        operation: "page.lend",
    };
    match driver
        .send_raw_cdp_on(session_id, method.to_owned(), params, &call)
        .await
    {
        Ok(result) => send_ws_result(ws_tx, id, session_id, result).await,
        Err(error) => send_ws_error(ws_tx, id, session_id, &error.to_string()).await,
    }
}

async fn send_ws_result<S>(
    ws_tx: &mut S,
    id: &Value,
    session_id: Option<&str>,
    result: Value,
) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let mut obj = Map::new();
    obj.insert("id".to_owned(), id.clone());
    if let Some(sid) = session_id {
        obj.insert("sessionId".to_owned(), Value::String(sid.to_owned()));
    }
    obj.insert("result".to_owned(), result);
    send_ws_json(ws_tx, &Value::Object(obj)).await
}

async fn send_ws_error<S>(
    ws_tx: &mut S,
    id: &Value,
    session_id: Option<&str>,
    message: &str,
) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let mut obj = Map::new();
    obj.insert("id".to_owned(), id.clone());
    if let Some(sid) = session_id {
        obj.insert("sessionId".to_owned(), Value::String(sid.to_owned()));
    }
    obj.insert(
        "error".to_owned(),
        json!({ "code": CDP_REFUSED_CODE, "message": message }),
    );
    send_ws_json(ws_tx, &Value::Object(obj)).await
}

async fn send_ws_json<S>(ws_tx: &mut S, value: &Value) -> Result<(), ()>
where
    S: futures_util::Sink<Message> + Unpin,
{
    let text = serde_json::to_string(value).map_err(|_| ())?;
    ws_tx
        .send(Message::Text(Utf8Bytes::from(text)))
        .await
        .map_err(|_| ())
}

async fn record_refused(page: &OpenPage, page_id: &PageId, method: &str, reason: &str) {
    trace::provenance(
        &page.run.trace,
        json!({
            "action": "lent_command_refused",
            "page_id": page_id,
            "session_id": page.session,
            "method": method,
            "reason": reason,
        }),
    )
    .await;
}

#[cfg(test)]
mod tests;
