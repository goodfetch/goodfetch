//! Discovery clicks on a browser page (issue 1087): finds links that exist
//! only in scripts (`onclick`, popups, `history.pushState`) by clicking each
//! matched element in document order through the guard (`check.js` by handle)
//! while the interception pipeline records and aborts navigations, popups and
//! unsafe requests (`method` not `GET`/`HEAD`/`OPTIONS`) unsent.

use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::{CallContext, CallError};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::PageId;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cdp::CdpPage;
use crate::guard::{self, Action, Request, SentMark, Window};
use crate::interception::DiscoveryCapture;
use crate::page_value::DiscoveredByClicking;
use crate::sessions::{OpenPage, PageEvent, Sessions};

/// Operation name passed to guard checks during discovery clicks.
const OPERATION: &str = "crawl_request.enqueue_links_by_clicking_elements";

/// Maximum wait after a click that produced no synchronous `pushState` for an
/// asynchronous navigation, popup attach or unsafe fetch to reach interception.
const QUIET_SETTLE: Duration = Duration::from_millis(200);

/// Maximum wait for an attached popup's first intercepted request to arrive.
const POPUP_SETTLE: Duration = Duration::from_secs(2);

/// Queries every element matching `selector` in document order (including open
/// shadow roots, matching `guard/check.js`), registers each in the guard
/// world's target map under a fresh handle, and returns those handles with the
/// document's current and base URLs.
const COLLECT_HANDLES: &str = r#"function({ selector }) {
  const world = globalThis;
  const KEY = Symbol.for("goodfetch.guard.targets");
  const NEXT = Symbol.for("goodfetch.guard.discovery_next_handle");
  const targets = (world[KEY] ??= new Map());
  let next = typeof world[NEXT] === "number" ? world[NEXT] : 1000000;
  const found = [];
  const visit = (root) => {
    found.push(...root.querySelectorAll(selector));
    for (const element of root.querySelectorAll("*"))
      if (element.shadowRoot) visit(element.shadowRoot);
  };
  try {
    visit(document);
  } catch (error) {
    return { state: "invalid", message: String(error?.message ?? error) };
  }
  const handles = [];
  for (const element of found) {
    const handle = next++;
    targets.set(handle, element);
    handles.push(handle);
  }
  world[NEXT] = next;
  return {
    state: "ok",
    handles,
    href: location.href,
    base_url: document.baseURI || location.href,
  };
}"#;

/// Reads `location.href` after a click; if History API (`pushState` or
/// `replaceState`) moved it away from `original`, restores `original` and
/// returns the discovered URL, otherwise `null`.
const READ_AND_RESTORE_HREF: &str = r#"function({ original }) {
  const current = location.href;
  if (current !== original) {
    try { history.replaceState(history.state, "", original); } catch {}
    return current;
  }
  return null;
}"#;

/// Removes temporary discovery handles from the guard world's target map.
const CLEAR_HANDLES: &str = r#"function({ handles }) {
  const targets = globalThis[Symbol.for("goodfetch.guard.targets")];
  if (targets) {
    for (const handle of handles) targets.delete(handle);
  }
  return null;
}"#;

#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Collected {
    Ok {
        handles: Vec<u64>,
        href: String,
        base_url: String,
    },
    Invalid {
        message: String,
    },
}

/// Runs discovery clicks for `selector` on the open browser page `page_id`.
pub(super) async fn run(
    sessions: &Sessions,
    page: &OpenPage,
    page_id: &PageId,
    selector: &str,
    context: &CallContext,
) -> Result<DiscoveredByClicking, CallError> {
    let collected = collect(page, selector).await?;
    let (handles, original_href, base_url) = match collected {
        Collected::Ok {
            handles,
            href,
            base_url,
        } => (handles, href, base_url),
        Collected::Invalid { message } => {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!("`{selector}` is no CSS selector the page parses: {message}"),
            ));
        }
    };

    let mut urls = Vec::new();
    let mut aborted = Vec::new();
    let mut refused = Vec::new();

    for (index, &handle) in handles.iter().enumerate() {
        let outcome = click_one(
            sessions,
            page,
            page_id,
            selector,
            handle,
            &original_href,
            context,
        )
        .await;
        match outcome {
            ClickOutcome::Clicked {
                urls: found,
                aborted: unsafe_reqs,
            } => {
                urls.extend(found);
                aborted.extend(unsafe_reqs);
            }
            ClickOutcome::Refused(error) => {
                refused.push((
                    u64::try_from(index).unwrap_or(u64::MAX),
                    error.code().to_owned(),
                    error.message().to_owned(),
                ));
            }
        }
    }

    clear_handles(page, &handles).await;
    Ok(DiscoveredByClicking {
        base_url,
        urls,
        aborted,
        refused,
    })
}

enum ClickOutcome {
    Clicked {
        urls: Vec<String>,
        aborted: Vec<(String, String)>,
    },
    Refused(CallError),
}

async fn click_one(
    sessions: &Sessions,
    page: &OpenPage,
    page_id: &PageId,
    selector: &str,
    handle: u64,
    original_href: &str,
    context: &CallContext,
) -> ClickOutcome {
    let capture = DiscoveryCapture::new();
    let armed = page
        .owner
        .interception()
        .arm_discovery(Arc::clone(&capture));
    let (limit, duration) = page.action_timeout.clone();
    let request = Request {
        operation: OPERATION,
        selector,
        handle: Some(handle),
        action: Action::Click,
        window: Window {
            limit,
            duration,
            spent: Duration::ZERO,
        },
        form: context.form(),
        hiding: page
            .owner
            .hiding()
            .hidden(&page.run.trace, (page_id, &page.session)),
        owner: &page.owner,
        sent: SentMark {
            window: &page.run.send_window,
            lease: page.lease(),
        },
        human_input: None,
    };
    let acted = page
        .life
        .call(
            PageEvent::Gesture,
            guard::act(&page.driver, request, &page.clock),
        )
        .await;
    if let Err(error) = acted {
        drop(armed);
        close_popups(sessions, &capture).await;
        return ClickOutcome::Refused(error);
    }

    let mut found_urls = Vec::new();
    let history_url = read_and_restore_href(page, original_href).await;
    let saw_history = if let Some(url) = history_url {
        found_urls.push(url);
        true
    } else {
        false
    };

    settle_capture(&capture, saw_history, &page.clock).await;
    drop(armed);

    let (captured_urls, captured_aborted, popups) = capture.take_all();
    found_urls.extend(captured_urls);
    for popup in popups {
        let _ = sessions.close_page(&popup, false).await;
    }
    ClickOutcome::Clicked {
        urls: found_urls,
        aborted: captured_aborted,
    }
}

async fn close_popups(sessions: &Sessions, capture: &DiscoveryCapture) {
    let (_, _, popups) = capture.take_all();
    for popup in popups {
        let _ = sessions.close_page(&popup, false).await;
    }
}

async fn settle_capture(capture: &DiscoveryCapture, saw_history: bool, clock: &Clock) {
    let mut changes = capture.subscribe();
    if !saw_history && !capture.has_activity() && capture.pending_popups() == 0 {
        tokio::select! {
            biased;
            _ = changes.changed() => {}
            () = clock.sleep(QUIET_SETTLE) => {}
        }
    }
    if capture.pending_popups() > 0 {
        let deadline = clock.now() + POPUP_SETTLE;
        while capture.pending_popups() > 0 {
            let remaining = deadline.saturating_duration_since(clock.now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                biased;
                res = changes.changed() => {
                    if res.is_err() {
                        break;
                    }
                }
                () = clock.sleep(remaining) => break,
            }
        }
    }
}

async fn collect(page: &OpenPage, selector: &str) -> Result<Collected, CallError> {
    let value = page
        .life
        .call(PageEvent::Read, async {
            page.owner
                .reads(CdpPage::of(&page.driver))
                .await?
                .in_page(COLLECT_HANDLES, json!({ "selector": selector }), OPERATION)
                .await
        })
        .await?
        .unwrap_or(Value::Null);
    Collected::deserialize(&value).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("unreadable discovery-click handle collection result: {error}"),
        )
    })
}

async fn read_and_restore_href(page: &OpenPage, original: &str) -> Option<String> {
    let value = page
        .life
        .call(PageEvent::Read, async {
            page.owner
                .reads(CdpPage::of(&page.driver))
                .await?
                .in_page(
                    READ_AND_RESTORE_HREF,
                    json!({ "original": original }),
                    OPERATION,
                )
                .await
        })
        .await
        .ok()??;
    value.as_str().map(str::to_owned)
}

async fn clear_handles(page: &OpenPage, handles: &[u64]) {
    if handles.is_empty() {
        return;
    }
    let _ = page
        .life
        .call(PageEvent::Read, async {
            page.owner
                .reads(CdpPage::of(&page.driver))
                .await?
                .in_page(CLEAR_HANDLES, json!({ "handles": handles }), OPERATION)
                .await
        })
        .await;
}
