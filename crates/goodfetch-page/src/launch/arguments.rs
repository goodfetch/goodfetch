//! The launch-argument builder (spec #577, ticket #696): the command line
//! of every Chromium the engine launches, from what the launch decided.
//! The browser runs over its debugging pipe, never a port (ADR 0017), on a
//! Browser data folder of the Run's own, its background traffic off by the table
//! `pinned_download` owns (ADR 0043); headless only when `browser.headless`
//! says so, the site seeing one identity either way (the driver's user agent, ADR
//! 0092); every page keeps its timers behind another tab or window (ADR
//! 0085's question); the back/forward cache is off, so going back or
//! forward always loads its document again (ADR 0121, with no setting); a
//! page whose load failed stays on its error page, never loaded again by
//! the browser on its own, so no request the program did not make leaves
//! and its failure is not raised again by the page's next call;
//! `network.resolve` reaches Chromium as host resolver rules (ADR 0059),
//! and every `.invalid` name fails without a DNS query; Chromium's sandbox
//! stays on, `--no-sandbox` passed only where `security.browser_sandbox` is
//! off (ADRs 0017 and 0049); the Run's domain
//! policy, when it refuses any host, reaches it as a proxy auto-config
//! script, so a connection no interception sees, a WebSocket's or WebRTC's
//! over TCP, is held to it too, and WebRTC sends no UDP, which no proxy
//! carries (ADRs 0014 and 0044); and a small
//! `/dev/shm`, such as the 64 MB Kubernetes
//! and Docker give, moves Chromium's shared memory to `/tmp` with a warning
//! (ADR 0049). On Linux the browser starts with a UTF-8 character type
//! whatever the host's locale (ADR 0073).

use std::fmt;
use std::net::IpAddr;
use std::path::Path;

use data_encoding::BASE64;
use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::pinned_download::chromium_background_traffic_off;

/// The `/dev/shm` below which Chromium's shared memory moves to `/tmp`:
/// 512 MiB, half the 1 GiB the Helm chart mounts (ADR 0049) and eight
/// times the 64 MB Kubernetes and Docker give.
const SMALL_SHARED_MEMORY: u64 = 512 * 1024 * 1024;

/// The flags that keep a page behind another tab or window of its Session
/// unthrottled (ADR 0085's question; spec #577's "Background pages").
const BACKGROUND_PAGES: [&str; 3] = [
    "--disable-background-timer-throttling",
    "--disable-renderer-backgrounding",
    "--disable-backgrounding-occluded-windows",
];

/// The host resolver rule that keeps every `.invalid` name from resolving
/// without a DNS query (RFC 6761, section 6.4), after the `resolve`
/// setting's rules, which come first.
const INVALID_NAMES: &str = "MAP *.invalid ~NOTFOUND";

/// WebRTC's policy: no UDP that no proxy carries, so each of its
/// connections goes over TCP, where the browser's proxy resolution holds it
/// (ADR 0044).
const WEBRTC_POLICY: &str = "--webrtc-ip-handling-policy=disable_non_proxied_udp";

/// No reload of a page left on a network error page: Chromium loads one
/// again on its own, a second after its error page and with backoff after
/// that, unless it runs with `--enable-automation`, which the engine never
/// passes. Such a reload is a request no call made, and its failure, which
/// the egress relay refuses again, would be raised by the page's next call
/// as that call's own (fix-queue 83).
const NO_AUTO_RELOAD: &str = "--disable-auto-reload";

/// Chromium's sandbox switched off, which only a launch whose
/// `security.browser_sandbox` is off passes (ADRs 0017 and 0049): it also
/// removes the renderers' seccomp-bpf layer.
const NO_SANDBOX: &str = "--no-sandbox";

/// The variables the browser starts with beside the engine's own. On
/// Linux, a character type of UTF-8 whatever the host's locale: under one
/// whose character type is not, `C`, none, or a `LANG` naming a locale the
/// system lacks, Chromium cannot open the `LevelDB` stores of a Browser data
/// folder whose path is not ASCII ("/LOCK: Access denied") and takes some
/// 6.6 s more to exit once asked to close (fix-queue 127). `LC_ALL` changes
/// only that: the browser's language still follows `LANG`.
#[cfg(target_os = "linux")]
const ENVIRONMENT: &[(&str, &str)] = &[("LC_ALL", "C.UTF-8")];

/// The variables the browser starts with beside the engine's own: none
/// where paths are not read through the locale's character type.
#[cfg(not(target_os = "linux"))]
const ENVIRONMENT: &[(&str, &str)] = &[];

/// What a launch decided.
pub(crate) struct Launch<'a> {
    /// The Browser data folder, the Run's own.
    pub(crate) data_folder: &'a Path,
    /// `browser.headless`.
    pub(crate) headless: bool,
    /// `network.resolve`: host names, each with its addresses.
    pub(crate) resolve: &'a HostMapping,
    /// The size of `/dev/shm` in bytes, where one was measured.
    pub(crate) shared_memory: Option<u64>,
    /// The Run's domain policy as a proxy auto-config script, when it
    /// refuses any host.
    pub(crate) proxy_script: Option<&'a str>,
    /// `security.browser_sandbox`: whether Chromium's sandbox stays on.
    pub(crate) sandbox: bool,
    /// The SHA-256 of the egress relay's certificate authority's key, in
    /// base64, when the Run lists an invalid-certificate exception: the
    /// browser takes the leaves the relay presents for the listed hosts
    /// (ADR 0042).
    pub(crate) relay_authority: Option<&'a str>,
}

/// The command line, the variables the browser starts with beside the
/// engine's own, and the warnings it comes with.
pub(crate) struct Arguments {
    pub(crate) command_line: Vec<String>,
    pub(crate) environment: &'static [(&'static str, &'static str)],
    pub(crate) warnings: Vec<Warning>,
}

/// A degraded but handled launch.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Warning {
    /// `/dev/shm` is too small for Chromium, which writes its shared memory
    /// to `/tmp` instead.
    SmallSharedMemory { bytes: u64 },
}

impl fmt::Display for Warning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SmallSharedMemory { bytes } => write!(
                formatter,
                "/dev/shm holds {} MiB, less than the {} MiB Chromium needs, so the browser keeps its shared memory in /tmp, which is slower; mount a larger /dev/shm (1 GiB, as the Helm chart does)",
                bytes / (1024 * 1024),
                SMALL_SHARED_MEMORY / (1024 * 1024)
            ),
        }
    }
}

/// The command line of the browser `launch` describes.
pub(crate) fn arguments(launch: &Launch<'_>) -> Arguments {
    let mut arguments = vec![
        "--remote-debugging-pipe".to_owned(),
        format!("--user-data-dir={}", launch.data_folder.display()),
    ];
    if launch.headless {
        arguments.push("--headless".to_owned());
    }
    arguments.extend(
        [
            "--no-first-run",
            "--no-default-browser-check",
            "--password-store=basic",
            "--use-mock-keychain",
            "--disable-back-forward-cache",
            NO_AUTO_RELOAD,
        ]
        .into_iter()
        .chain(BACKGROUND_PAGES)
        .map(str::to_owned),
    );
    arguments.extend(chromium_background_traffic_off());
    arguments.push(WEBRTC_POLICY.to_owned());
    if !launch.sandbox {
        arguments.push(NO_SANDBOX.to_owned());
    }
    if let Some(spki) = launch.relay_authority {
        arguments.push(format!("--ignore-certificate-errors-spki-list={spki}"));
    }
    if let Some(script) = launch.proxy_script {
        arguments.push(format!(
            "--proxy-pac-url=data:application/x-ns-proxy-autoconfig;base64,{}",
            BASE64.encode(script.as_bytes())
        ));
    }
    let mut warnings = Vec::new();
    if let Some(bytes) = launch
        .shared_memory
        .filter(|bytes| *bytes < SMALL_SHARED_MEMORY)
    {
        arguments.push("--disable-dev-shm-usage".to_owned());
        warnings.push(Warning::SmallSharedMemory { bytes });
    }
    let mut rules: Vec<String> = launch
        .resolve
        .iter()
        .filter_map(|(host, addresses)| {
            Some(match addresses.first()? {
                IpAddr::V6(address) => format!("MAP {host} [{address}]"),
                IpAddr::V4(address) => format!("MAP {host} {address}"),
            })
        })
        .collect();
    rules.push(INVALID_NAMES.to_owned());
    arguments.push(format!("--host-resolver-rules={}", rules.join(", ")));
    arguments.push("about:blank".to_owned());
    Arguments {
        command_line: arguments,
        environment: ENVIRONMENT,
        warnings,
    }
}

/// The size of `/dev/shm` in bytes, read from its `size=` option in
/// `mounts`, a mount table as `/proc/self/mounts` lists it; `None` when it
/// is not mounted on its own or names no size.
#[cfg(target_os = "linux")]
pub(crate) fn shared_memory_of(mounts: &str) -> Option<u64> {
    let options = mounts.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let point = fields.nth(1)?;
        (point == "/dev/shm").then(|| fields.nth(1)).flatten()
    })?;
    let size = options
        .split(',')
        .find_map(|option| option.strip_prefix("size="))?;
    let (digits, unit) = size.split_at(
        size.find(|character: char| !character.is_ascii_digit())
            .unwrap_or(size.len()),
    );
    let multiplier = match unit {
        "" => 1,
        "k" | "K" => 1024,
        "m" | "M" => 1024 * 1024,
        "g" | "G" => 1024 * 1024 * 1024,
        _ => return None,
    };
    digits.parse::<u64>().ok()?.checked_mul(multiplier)
}

#[cfg(test)]
mod tests;
