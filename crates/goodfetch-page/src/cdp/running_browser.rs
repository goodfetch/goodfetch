//! Consented remote debugging of the person's running Chrome, Edge, or
//! Chromium (`session.source = { kind: "running_browser" }`, ADR 0056;
//! SA-08; ticket #742).
//!
//! Probe findings (2026-11-28, ADR 0056 "to prove"):
//! - **Chrome (144+)** and **Edge (144+)** implement consented remote
//!   debugging (`chrome://inspect/#remote-debugging`,
//!   `edge://inspect/#remote-debugging`), writing `DevToolsActivePort`
//!   (line 1: TCP port, line 2: `/devtools/browser/<id>`) into their user
//!   data directory (including the OS default user data directory) and
//!   honouring `RemoteDebuggingAllowed` and `DeveloperToolsAvailability`.
//! - **Chromium (144+)** carries the same upstream `remote_debugging_server`
//!   implementation, so `running_browser` accepts `chrome`, `edge`, and
//!   `chromium`.
//! - **Brave**, **Firefox**, and **Safari** do not ship consented remote
//!   debugging and are refused with `running_browser_unsupported`.
//!
//! `DevToolsActivePort` is watched with the OS's file watcher (`inotify` on
//! Linux, `kqueue` on macOS), never polled.
#![cfg_attr(
    any(target_os = "linux", target_os = "macos"),
    expect(
        unsafe_code,
        reason = "watching DevToolsActivePort without polling requires the OS's file watcher syscalls: inotify on Linux, kqueue on macOS (ADR 0056)"
    )
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::egress::CdpUrlOrigin;
use goodfetch_engine::tasks::TaskStarter;
use goodfetch_protocol::connection::{CancelledBy, ClosedValue};
use goodfetch_protocol::page::cdp_url::{
    CDP_URL_UNREACHABLE, CdpUrlConnectFailure, CdpUrlUnreachableDetails,
};
use goodfetch_protocol::page::session::{
    REMOTE_DEBUGGING_DISABLED_BY_POLICY, RUNNING_BROWSER_UNSUPPORTED,
    RemoteDebuggingDisabledByPolicyDetails, RunningBrowserUnsupportedDetails,
};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_protocol::settings::session_source::BrowserName;
use tokio_util::sync::CancellationToken;

use crate::cdp::borrowed::CdpUrl;
use crate::errors::cancelled;
use crate::profile_directory;

/// The file Chromium writes inside its user data directory when consented
/// remote debugging is active.
const ACTIVE_PORT_FILE: &str = "DevToolsActivePort";

/// Parameters to discover and connect to the person's running browser.
#[derive(Clone, Debug)]
pub(crate) struct RunningBrowserRequest {
    pub(crate) browser: BrowserName,
    pub(crate) path: Option<PathBuf>,
    pub(crate) origin: CdpUrlOrigin,
}

/// The resolved consented debugging endpoint of a running browser.
#[derive(Clone, Debug)]
pub(crate) struct DiscoveredRunningBrowser {
    pub(crate) data_dir: PathBuf,
    pub(crate) cdp_url: CdpUrl,
}

/// Validates browser support and enterprise policy, then watches
/// `DevToolsActivePort` using the OS file watcher until a valid debugging
/// address appears or `deadline` / `stopping` fires (ADR 0056; ticket #742).
pub(crate) async fn discover(
    request: RunningBrowserRequest,
    deadline: Duration,
    stopping: CancellationToken,
    owner: &TaskStarter,
) -> Result<DiscoveredRunningBrowser, CallError> {
    check_browser_supported(request.browser)?;

    let browser = request.browser;
    let explicit_path = request.path.clone();
    let layer = request.origin.layer;
    let data_dir = owner
        .blocking(move || resolve_and_check_policy(browser, explicit_path, layer))
        .await?;

    let ws_url = watch_active_port(&data_dir, browser, layer, deadline, stopping).await?;

    Ok(DiscoveredRunningBrowser {
        data_dir,
        cdp_url: CdpUrl {
            url: ws_url,
            origin: request.origin,
        },
    })
}

/// Refuses browsers that do not implement consented remote debugging with
/// `running_browser_unsupported` (ADR 0056).
pub(crate) fn check_browser_supported(browser: BrowserName) -> Result<(), CallError> {
    match browser {
        BrowserName::Chrome | BrowserName::Edge | BrowserName::Chromium => Ok(()),
        BrowserName::Firefox => Err(unsupported(
            "firefox",
            "Firefox does not offer consented remote debugging on a running browser",
            "use `installed_browser` or `state_file` to import your Firefox login instead",
        )),
        BrowserName::Safari => Err(unsupported(
            "safari",
            "Safari does not offer consented remote debugging on a running browser",
            "use `installed_browser` or `state_file` to import your Safari login instead",
        )),
        BrowserName::Brave => Err(unsupported(
            "brave",
            "Brave does not implement Chrome's consented remote debugging mode",
            "use `installed_browser` or `profile_directory` to read Brave's login instead",
        )),
    }
}

fn unsupported(browser: &str, reason: &str, fix: &str) -> CallError {
    CallError::new(
        &RUNNING_BROWSER_UNSUPPORTED,
        format!("Cannot borrow running `{browser}`: {reason}; {fix}."),
        &RunningBrowserUnsupportedDetails {
            browser: browser.to_owned(),
            reason: reason.to_owned(),
            fix: fix.to_owned(),
        },
    )
}

fn browser_label(browser: BrowserName) -> &'static str {
    match browser {
        BrowserName::Chrome => "chrome",
        BrowserName::Edge => "edge",
        BrowserName::Brave => "brave",
        BrowserName::Chromium => "chromium",
        BrowserName::Firefox => "firefox",
        BrowserName::Safari => "safari",
    }
}

fn inspect_url(browser: BrowserName) -> &'static str {
    match browser {
        BrowserName::Edge => "edge://inspect/#remote-debugging",
        _ => "chrome://inspect/#remote-debugging",
    }
}

/// Resolves the user data directory and verifies that no administrator policy
/// disables remote debugging (runs on the blocking pool).
fn resolve_and_check_policy(
    browser: BrowserName,
    explicit_path: Option<PathBuf>,
    layer: SettingsLayer,
) -> Result<PathBuf, CallError> {
    let data_dir = match explicit_path {
        Some(path) => path,
        None => profile_directory::default_user_data_dir(browser).ok_or_else(|| {
            unreachable_active_port(
                &format!("running_browser:{}", browser_label(browser)),
                browser,
                layer,
                "could not resolve the browser's default user data directory",
            )
        })?,
    };

    check_remote_debugging_policy(browser, &data_dir)?;

    if !data_dir.is_dir() {
        return Err(unreachable_active_port(
            &data_dir.display().to_string(),
            browser,
            layer,
            "the browser's user data directory does not exist",
        ));
    }

    Ok(std::fs::canonicalize(&data_dir).unwrap_or(data_dir))
}

/// Checks OS managed policy directories and `<data_dir>/policies/managed` (plus
/// `Local State`) for administrator policies that disable remote debugging.
#[expect(
    clippy::disallowed_methods,
    reason = "called inside owner.blocking(...) to inspect managed policy files"
)]
pub(crate) fn check_remote_debugging_policy(
    browser: BrowserName,
    data_dir: &Path,
) -> Result<(), CallError> {
    let mut dirs = Vec::new();
    dirs.push(data_dir.join("policies").join("managed"));
    dirs.extend(os_managed_policy_dirs(browser));

    for dir in dirs {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = read_dir
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            })
            .collect();
        files.sort();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            if let Some(key) = policy_disables_remote_debugging(&json) {
                return Err(policy_disabled_error(
                    browser,
                    Some(file.display().to_string()),
                    key,
                ));
            }
        }
    }

    if let Ok(text) = std::fs::read_to_string(data_dir.join("Local State"))
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&text)
    {
        if json
            .pointer("/remote_debugging_allowed")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        {
            return Err(policy_disabled_error(
                browser,
                Some(data_dir.join("Local State").display().to_string()),
                "RemoteDebuggingAllowed",
            ));
        }
        if let Some(policy) = json.get("policy")
            && let Some(key) = policy_disables_remote_debugging(policy)
        {
            return Err(policy_disabled_error(
                browser,
                Some(data_dir.join("Local State").display().to_string()),
                key,
            ));
        }
    }

    Ok(())
}

fn policy_disables_remote_debugging(json: &serde_json::Value) -> Option<&'static str> {
    if json
        .get("RemoteDebuggingAllowed")
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        return Some("RemoteDebuggingAllowed");
    }
    if json
        .get("DeveloperToolsAvailability")
        .and_then(serde_json::Value::as_i64)
        == Some(2)
    {
        return Some("DeveloperToolsAvailability");
    }
    None
}

fn policy_disabled_error(
    browser: BrowserName,
    policy_path: Option<String>,
    policy_key: &str,
) -> CallError {
    let label = browser_label(browser);
    CallError::new(
        &REMOTE_DEBUGGING_DISABLED_BY_POLICY,
        format!(
            "Administrator policy `{policy_key}` disables consented remote debugging in `{label}` (ADR 0056)."
        ),
        &RemoteDebuggingDisabledByPolicyDetails {
            browser: label.to_owned(),
            policy_path,
            policy_key: policy_key.to_owned(),
            fix: "Ask your administrator to allow remote debugging, or import a login via `state_file`."
                .to_owned(),
        },
    )
}

fn os_managed_policy_dirs(browser: BrowserName) -> Vec<PathBuf> {
    if cfg!(target_os = "linux") {
        match browser {
            BrowserName::Chrome => vec![PathBuf::from("/etc/opt/chrome/policies/managed")],
            BrowserName::Edge => vec![PathBuf::from("/etc/opt/edge/policies/managed")],
            BrowserName::Chromium => vec![PathBuf::from("/etc/chromium/policies/managed")],
            _ => Vec::new(),
        }
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from("/Library/Managed Preferences")]
    } else {
        Vec::new()
    }
}

/// Parses `DevToolsActivePort`'s two-line `<port>\n<ws-path>` format into a
/// loopback WebSocket URL.
pub(crate) fn parse_active_port(contents: &str) -> Option<String> {
    let mut lines = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let port: u16 = lines.next()?.parse().ok()?;
    if port == 0 {
        return None;
    }
    let ws_path = lines.next()?;
    if !ws_path.starts_with('/') {
        return None;
    }
    Some(format!("ws://127.0.0.1:{port}{ws_path}"))
}

#[expect(
    clippy::disallowed_methods,
    reason = "reads the small DevToolsActivePort file triggered by an OS file watcher event"
)]
fn read_active_port(port_file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(port_file).ok()?;
    parse_active_port(&text)
}

fn unreachable_active_port(
    target: &str,
    browser: BrowserName,
    layer: SettingsLayer,
    why: &str,
) -> CallError {
    CallError::new(
        &CDP_URL_UNREACHABLE,
        format!(
            "No consented remote debugging endpoint found for `{}` at `{target}`: {why}.",
            browser_label(browser)
        ),
        &CdpUrlUnreachableDetails {
            url: target.to_owned(),
            layer,
            failure: CdpUrlConnectFailure::ConnectionRefused,
            fix: format!(
                "Open `{}` in `{}` and enable remote debugging, then approve the connection prompt.",
                inspect_url(browser),
                browser_label(browser),
            ),
        },
    )
}

/// Watches `data_dir` with the OS file watcher (never polling) until
/// `DevToolsActivePort` holds a valid endpoint, `deadline` expires, or
/// `stopping` is cancelled.
async fn watch_active_port(
    data_dir: &Path,
    browser: BrowserName,
    layer: SettingsLayer,
    deadline: Duration,
    stopping: CancellationToken,
) -> Result<String, CallError> {
    let port_file = data_dir.join(ACTIVE_PORT_FILE);

    let wait_result = tokio::select! {
        () = stopping.cancelled() => {
            return Err(cancelled(CancelledBy::Program, ClosedValue::Session));
        }
        result = tokio::time::timeout(deadline, wait_for_active_port_os(data_dir, &port_file)) => result,
    };

    match wait_result {
        Ok(Ok(url)) => Ok(url),
        Ok(Err(error)) => Err(unreachable_active_port(
            &port_file.display().to_string(),
            browser,
            layer,
            &format!("OS file watcher failed: {error}"),
        )),
        Err(_) => Err(unreachable_active_port(
            &port_file.display().to_string(),
            browser,
            layer,
            "timed out waiting for `DevToolsActivePort` to appear",
        )),
    }
}

/// Linux: watches `data_dir` using `inotify` registered with `tokio::io::unix::AsyncFd`.
#[cfg(target_os = "linux")]
async fn wait_for_active_port_os(data_dir: &Path, port_file: &Path) -> std::io::Result<String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::os::unix::ffi::OsStrExt as _;
    use tokio::io::Interest;
    use tokio::io::unix::AsyncFd;

    // SAFETY: `inotify_init1` takes flags only; its return value is checked below.
    let raw_fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if raw_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `raw_fd` was just returned by `inotify_init1` and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

    let c_dir = CString::new(data_dir.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let mask = libc::IN_CREATE | libc::IN_MOVED_TO | libc::IN_CLOSE_WRITE | libc::IN_MODIFY;
    // SAFETY: `fd` is a valid inotify descriptor and `c_dir` is a valid NUL-terminated path.
    let wd = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), c_dir.as_ptr(), mask) };
    if wd < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // Check immediately after arming the watch so a file written before or
    // during watch registration is never missed (no TOCTOU race).
    if let Some(url) = read_active_port(port_file) {
        return Ok(url);
    }

    // SAFETY: `fd` is an open `OwnedFd` whose descriptor stays valid for the
    // lifetime of the `AsyncFd` owning it.
    let async_fd = unsafe { AsyncFd::register_with_interest(fd, Interest::READABLE) }?;
    let mut buf = [0u8; 4096];

    loop {
        let mut guard = async_fd.readable().await?;
        let raw = async_fd.get_ref().as_raw_fd();
        // Drain all queued inotify events until EAGAIN/EWOULDBLOCK.
        loop {
            // SAFETY: `raw` is a valid non-blocking inotify fd and `buf` is a live writable buffer.
            let n = unsafe { libc::read(raw, buf.as_mut_ptr().cast(), buf.len()) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::WouldBlock {
                    break;
                }
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if n == 0 {
                break;
            }
        }
        guard.clear_ready();

        if let Some(url) = read_active_port(port_file) {
            return Ok(url);
        }
    }
}

/// macOS: watches `data_dir` using `kqueue` `EVFILT_VNODE` registered with `AsyncFd`.
#[cfg(target_os = "macos")]
async fn wait_for_active_port_os(data_dir: &Path, port_file: &Path) -> std::io::Result<String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::os::unix::ffi::OsStrExt as _;
    use tokio::io::Interest;
    use tokio::io::unix::AsyncFd;

    let c_dir = CString::new(data_dir.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    // SAFETY: `open` with `O_EVTONLY | O_CLOEXEC` opens the directory for vnode events only.
    let dir_raw = unsafe { libc::open(c_dir.as_ptr(), libc::O_EVTONLY | libc::O_CLOEXEC) };
    if dir_raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `dir_raw` was just returned by `open` and nothing else owns it.
    let _dir_fd = unsafe { OwnedFd::from_raw_fd(dir_raw) };

    // SAFETY: `kqueue` takes no arguments; return value checked below.
    let kq_raw = unsafe { libc::kqueue() };
    if kq_raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `kq_raw` was just returned by `kqueue` and nothing else owns it.
    let kq_fd = unsafe { OwnedFd::from_raw_fd(kq_raw) };

    let change = libc::kevent {
        ident: usize::try_from(_dir_fd.as_raw_fd()).unwrap_or(0),
        filter: libc::EVFILT_VNODE,
        flags: libc::EV_ADD | libc::EV_CLEAR,
        fflags: libc::NOTE_WRITE | libc::NOTE_EXTEND | libc::NOTE_ATTRIB | libc::NOTE_RENAME,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    // SAFETY: valid kqueue descriptor and stack-allocated kevent struct.
    let added = unsafe {
        libc::kevent(
            kq_fd.as_raw_fd(),
            &raw const change,
            1,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    };
    if added < 0 {
        return Err(std::io::Error::last_os_error());
    }

    if let Some(url) = read_active_port(port_file) {
        return Ok(url);
    }

    // SAFETY: `kq_fd` is an open `OwnedFd` whose descriptor stays valid for the
    // lifetime of the `AsyncFd` owning it.
    let async_fd = unsafe { AsyncFd::register_with_interest(kq_fd, Interest::READABLE) }?;
    let zero_timeout = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    loop {
        let mut guard = async_fd.readable().await?;
        let mut event = libc::kevent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        // SAFETY: drains one pending event without blocking.
        let _ = unsafe {
            libc::kevent(
                async_fd.get_ref().as_raw_fd(),
                std::ptr::null(),
                0,
                &raw mut event,
                1,
                &raw const zero_timeout,
            )
        };
        guard.clear_ready();

        if let Some(url) = read_active_port(port_file) {
            return Ok(url);
        }
    }
}

/// Other platforms: reads `DevToolsActivePort` immediately without polling.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
async fn wait_for_active_port_os(_data_dir: &Path, port_file: &Path) -> std::io::Result<String> {
    read_active_port(port_file).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "DevToolsActivePort not present",
        )
    })
}

#[cfg(test)]
mod tests;
