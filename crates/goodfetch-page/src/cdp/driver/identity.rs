//! The identity a headless Chromium's pages and service workers show (spec
//! #577, ticket #696; ADRs 0021 and 0092): the one the same build shows
//! with a window, so a site sees one real browser whether a person watched
//! or not. A headless
//! build differs from its window only in its user agent's product token,
//! `HeadlessChrome/` for `Chrome/`; its client hints are the window's. Each
//! page then gets that user agent with the client hints read from the
//! browser itself, since a user agent set without them clears the
//! high-entropy hints, which no real browser does.
//!
//! The hints are read once per browser, on a page of the browser's own
//! default context, which no Session and no site shares: the build's
//! `chrome://version`, a secure context where `navigator.userAgentData`
//! answers. `Runtime.evaluate` runs there without enabling `Runtime`
//! (ADR 0019).

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_cdp::generated::{emulation, network, page, runtime, target};
use goodfetch_engine::CallError;
use goodfetch_protocol::page::{BROWSER_LAUNCH_FAILED, BrowserLaunchFailedDetails};

use super::{Browser, PageDriver};
use crate::cdp::ChildKind;

/// The product token of a headless build's user agent.
const HEADLESS_PRODUCT: &str = "HeadlessChrome/";

/// The product token the same build shows with a window.
const WINDOW_PRODUCT: &str = "Chrome/";

/// A page of the build's own, in a secure context.
const SECURE_PAGE: &str = "chrome://version/";

/// What the page reads: every client hint `navigator.userAgentData` gives,
/// in the shape of CDP's `UserAgentMetadata`.
const READ_HINTS: &str = r#"(async () => {
  const data = navigator.userAgentData;
  const high = await data.getHighEntropyValues([
    "architecture", "bitness", "formFactors", "fullVersionList", "model",
    "platformVersion", "wow64",
  ]);
  return {
    brands: data.brands,
    fullVersionList: high.fullVersionList,
    platform: data.platform,
    platformVersion: high.platformVersion,
    architecture: high.architecture,
    model: high.model,
    mobile: data.mobile,
    bitness: high.bitness,
    wow64: high.wow64,
    formFactors: high.formFactors,
  };
})()"#;

/// The identity a headless browser's pages show: its window's.
#[derive(Clone, Debug)]
pub(super) struct Identity {
    user_agent: String,
    hints: emulation::UserAgentMetadata,
}

impl Identity {
    /// The user agent this identity shows.
    pub(super) fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// The client hints this identity shows.
    pub(super) fn hints(&self) -> &emulation::UserAgentMetadata {
        &self.hints
    }

    /// The command that gives a page this identity.
    pub(super) fn command(&self) -> emulation::SetUserAgentOverrideParams {
        emulation::SetUserAgentOverrideParams {
            user_agent: self.user_agent.clone(),
            accept_language: None,
            platform: None,
            user_agent_metadata: Some(self.hints.clone()),
        }
    }

    /// The command that gives a worker this identity, through its `Network`
    /// override: it sets what the worker's requests send, its
    /// `navigator.userAgentData`, and a dedicated worker's
    /// `navigator.userAgent`; a service or shared worker's
    /// `navigator.userAgent` stays the browser's own.
    pub(super) fn worker_command(&self) -> network::SetUserAgentOverrideParams {
        network::SetUserAgentOverrideParams {
            user_agent: self.user_agent.clone(),
            accept_language: None,
            platform: None,
            user_agent_metadata: Some(self.hints.clone()),
        }
    }
}

impl PageDriver {
    /// Has the page's out-of-process frame or dedicated worker this driver
    /// drives show its headless browser's identity, as the page does,
    /// within a deadline of opening a page: a frame of another site's
    /// process takes none of its page's overrides. Nothing with a window.
    ///
    /// # Errors
    ///
    /// The code of opening a page when the browser refuses.
    pub(crate) async fn show_identity_opening(&self, kind: ChildKind) -> Result<(), CallError> {
        let Some(identity) = &self.browser.identity else {
            return Ok(());
        };
        match kind {
            ChildKind::Frame => self.opening_command(&identity.command()).await.map(drop),
            ChildKind::Worker => self
                .opening_command(&identity.worker_command())
                .await
                .map(drop),
        }
    }
}

/// The user agent `headless`, a headless build's, shows with a window.
fn window_user_agent(headless: &str) -> String {
    headless.replace(HEADLESS_PRODUCT, WINDOW_PRODUCT)
}

/// Reads the identity of `browser`, a headless build whose own user agent
/// is `user_agent`.
///
/// # Errors
///
/// `browser_launch_failed` when the browser does not open, load or read its
/// secure page; a lost browser raises what [`Browser::send`] raises.
pub(super) async fn read(browser: &Browser, user_agent: &str) -> Result<Identity, CallError> {
    let opening = browser.launching();
    let created = browser
        .send(
            None,
            &target::CreateTargetParams {
                url: "about:blank".to_owned(),
                browser_context_id: None,
                left: None,
                top: None,
                width: None,
                height: None,
                window_state: None,
                enable_begin_frame_control: None,
                new_window: None,
                background: Some(true),
                for_tab: None,
                hidden: None,
                focus: None,
            },
            &opening,
        )
        .await?;
    let session = browser.attach_own(&created.target_id, &opening).await?;
    let hints = hints_of(browser, &session).await;
    // The page is the engine's own; a failure to close it leaves a blank
    // tab no Session sees, so the identity stands.
    let _ = browser
        .send(
            None,
            &target::CloseTargetParams {
                target_id: created.target_id,
            },
            &opening,
        )
        .await;
    Ok(Identity {
        user_agent: window_user_agent(user_agent),
        hints: hints?,
    })
}

/// Loads the secure page on `session` and reads its client hints.
async fn hints_of(
    browser: &Browser,
    session: &TargetSession,
) -> Result<emulation::UserAgentMetadata, CallError> {
    let opening = browser.launching();
    let mut events = browser.connection.subscribe(Some(session));
    browser
        .send(Some(session), &page::EnableParams::default(), &opening)
        .await?;
    browser
        .send(
            Some(session),
            &page::SetLifecycleEventsEnabledParams { enabled: true },
            &opening,
        )
        .await?;
    let navigated = browser
        .send(
            Some(session),
            &page::NavigateParams {
                url: SECURE_PAGE.to_owned(),
                referrer: None,
                transition_type: None,
                frame_id: None,
                referrer_policy: None,
            },
            &opening,
        )
        .await?;
    if let Some(error) = navigated.error_text {
        return Err(unread(
            browser,
            &format!("{SECURE_PAGE} did not load: {error}"),
        ));
    }
    if let Some(loader) = navigated.loader_id {
        // The secure page loads within the launch's deadline.
        let loaded = tokio::time::timeout(browser.deadlines.of_call(&opening).duration(), async {
            while let Some(event) = events.next().await {
                if let Some(lifecycle) = event.read::<page::LifecycleEventEvent>()
                    && lifecycle.loader_id == loader
                    && lifecycle.name == "load"
                {
                    return true;
                }
            }
            false
        })
        .await;
        if loaded != Ok(true) {
            return Err(unread(browser, &format!("{SECURE_PAGE} did not load")));
        }
    }
    // No later command waits on a subscription nobody reads.
    drop(events);
    let evaluated = browser
        .send(
            Some(session),
            &runtime::EvaluateParams {
                expression: READ_HINTS.to_owned(),
                object_group: None,
                include_command_line_api: None,
                silent: Some(true),
                context_id: None,
                return_by_value: Some(true),
                generate_preview: None,
                user_gesture: None,
                await_promise: Some(true),
                throw_on_side_effect: None,
                timeout: None,
                disable_breaks: None,
                repl_mode: None,
                allow_unsafe_eval_blocked_by_csp: None,
                unique_context_id: None,
                serialization_options: None,
            },
            &opening,
        )
        .await?;
    if let Some(exception) = evaluated.exception_details {
        return Err(unread(browser, &exception.text));
    }
    serde_json::from_value(evaluated.result.value.unwrap_or_default())
        .map_err(|error| unread(browser, &error.to_string()))
}

/// `browser_launch_failed` for client hints the browser did not give.
fn unread(browser: &Browser, reason: &str) -> CallError {
    CallError::new(
        &BROWSER_LAUNCH_FAILED,
        format!("the headless browser did not give its client hints: {reason}"),
        &BrowserLaunchFailedDetails {
            executable: browser.executable.clone(),
            exit: None,
        },
    )
}
