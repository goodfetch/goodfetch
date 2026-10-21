//! The launch-argument builder (spec #577, ticket #696): the command line
//! of every Chromium the engine launches, from what the launch decided.
//! The browser runs over its debugging pipe, never a port (ADR 0017), on a
//! profile of the Run's own; headless only when `browser.headless` says so,
//! the site seeing one identity either way (the driver's user agent, ADR
//! 0092); every page keeps its timers behind another tab or window (ADR
//! 0085's question); the back/forward cache is off, so going back or
//! forward always loads its document again (ADR 0121, with no setting);
//! `network.resolve` reaches Chromium as host resolver rules (ADR 0059),
//! and every `.invalid` name fails without a DNS query; and a small
//! `/dev/shm`, such as the 64 MB Kubernetes
//! and Docker give, moves Chromium's shared memory to `/tmp` with a warning
//! (ADR 0049).

use std::fmt;
use std::net::IpAddr;
use std::path::Path;

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

/// What a launch decided.
pub(crate) struct Launch<'a> {
    /// The profile folder, the Run's own.
    pub(crate) profile: &'a Path,
    /// `browser.headless`.
    pub(crate) headless: bool,
    /// `network.resolve`: host names, each with its addresses.
    pub(crate) resolve: &'a [(String, Vec<IpAddr>)],
    /// The size of `/dev/shm` in bytes, where one was measured.
    pub(crate) shared_memory: Option<u64>,
}

/// The command line, and the warnings it comes with.
pub(crate) struct Arguments {
    pub(crate) arguments: Vec<String>,
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
        format!("--user-data-dir={}", launch.profile.display()),
    ];
    if launch.headless {
        arguments.push("--headless".to_owned());
    }
    arguments.extend(
        [
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            "--password-store=basic",
            "--use-mock-keychain",
            "--disable-back-forward-cache",
        ]
        .into_iter()
        .chain(BACKGROUND_PAGES)
        .map(str::to_owned),
    );
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
        arguments,
        warnings,
    }
}

/// The size of `/dev/shm` in bytes, read from its `size=` option in
/// `mounts`, a mount table as `/proc/self/mounts` lists it; `None` when it
/// is not mounted on its own or names no size.
#[cfg_attr(
    all(not(target_os = "linux"), not(test)),
    expect(dead_code, reason = "only Linux mounts a `/dev/shm` to measure")
)]
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
