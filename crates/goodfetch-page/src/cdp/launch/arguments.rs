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

/// One Chromium switch reconciled against Playwright 1.64.1's
/// `chromiumSwitches.ts` (`disabledFeatures` and `chromiumSwitches`) plus
/// `--enable-automation` (issue 2330).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SwitchKind {
    /// A command-line flag (`--name` or `--name=value`).
    Flag(&'static str),
    /// A feature inside the launch's single `--disable-features=...` list.
    DisabledFeature(&'static str),
    /// A feature inside `--enable-features=...`.
    EnabledFeature(&'static str),
}

/// Whether Goodfetch passes a switch from Playwright's list on Chromium
/// launches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Disposition {
    /// Passed on every Chromium launch.
    Taken,
    /// Passed only when a runtime condition holds (for example `/dev/shm`
    /// below [`SMALL_SHARED_MEMORY`], ADR 0049).
    Conditional,
    /// Never passed on stock launches; [`PlaywrightSwitch::reason`] explains
    /// why.
    Refused,
}

/// One row of [`PLAYWRIGHT_SWITCHES`]: the switch, whether Goodfetch takes it,
/// and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlaywrightSwitch {
    pub(crate) switch: SwitchKind,
    pub(crate) disposition: Disposition,
    pub(crate) reason: &'static str,
}

/// Switch-by-switch reconciliation of Collie's Chromium launch arguments with
/// Playwright 1.64.1's `chromiumSwitches.ts` (and `--enable-automation`, which
/// Playwright adds outside that file). Both [`arguments`] and its unit tests
/// read [`Disposition::Taken`] entries from this single table (issue 2330).
pub(crate) const PLAYWRIGHT_SWITCHES: &[PlaywrightSwitch] = &[
    // --- `disabledFeatures` (Playwright 1.64.1 `chromiumSwitches.ts`) ---
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("AvoidUnnecessaryBeforeUnloadCheckSync"),
        disposition: Disposition::Taken,
        reason: "Avoids synchronous beforeunload check hangs during programmatic navigation (Playwright #14047); invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("DestroyProfileOnBrowserClose"),
        disposition: Disposition::Taken,
        reason: "Destroys ephemeral run profiles cleanly on browser exit; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("DialMediaRouteProvider"),
        disposition: Disposition::Taken,
        reason: "Disables DIAL local-network media discovery traffic (Playwright #13854); invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("GlobalMediaControls"),
        disposition: Disposition::Taken,
        reason: "Disables browser toolbar media controls UI; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("HttpsUpgrades"),
        disposition: Disposition::Taken,
        reason: "Prevents silent HTTP-to-HTTPS upgrades so explicit http:// navigations hit the requested URL (Playwright #27605).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("LensOverlay"),
        disposition: Disposition::Taken,
        reason: "Disables the Lens address-bar UI not used in unofficial builds; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("MediaRouter"),
        disposition: Disposition::Taken,
        reason: "Disables Cast/MediaRouter local-network discovery (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("PaintHolding"),
        disposition: Disposition::Taken,
        reason: "Stops Chromium holding the previous page painted across navigations (Playwright #28023); invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("ThirdPartyStoragePartitioning"),
        disposition: Disposition::Refused,
        reason: "Stock Chrome partitions third-party storage by default; disabling it weakens cross-site isolation and diverges from an ordinary browser (ADR 0021).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("BlockOriginHeaderModificationOnRedirect"),
        disposition: Disposition::Taken,
        reason: "Prevents Chromium 149+ from rejecting redirected Fetch-intercepted requests carrying Origin with net::ERR_INVALID_ARGUMENT (Playwright #41690).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("AvoidCorsURLLoaderRestartOnRedirect"),
        disposition: Disposition::Taken,
        reason: "Prevents Chromium 155+ from aborting preflighted cross-origin redirects with net::ERR_ABORTED under Fetch interception (Playwright #43242).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("Translate"),
        disposition: Disposition::Taken,
        reason: "Disables translation prompts and background fetches (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("AutoDeElevate"),
        disposition: Disposition::Taken,
        reason: "Prevents Chromium on Windows from relaunching de-elevated and dropping its debugging pipe (Chromium 435410220).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("OptimizationHints"),
        disposition: Disposition::Taken,
        reason: "Disables startup optimization-hints fetches (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("NetworkTimeServiceQuerying"),
        disposition: Disposition::Taken,
        reason: "Disables startup network-time queries (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("AimEnabled"),
        disposition: Disposition::Taken,
        reason: "Disables AI Mode eligibility fetches (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("msForceBrowserSignIn"),
        disposition: Disposition::Refused,
        reason: "Microsoft Edge-only feature; Goodfetch launches Chrome for Testing / Goodfetch Chromium.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::DisabledFeature("msEdgeUpdateLaunchServicesPreferredVersion"),
        disposition: Disposition::Refused,
        reason: "Microsoft Edge macOS-only feature; Goodfetch does not launch Edge.",
    },
    // --- `chromiumSwitches` (Playwright 1.64.1 `chromiumSwitches.ts`) ---
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-field-trial-config"),
        disposition: Disposition::Taken,
        reason: "Disables field-trial variation fetches (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-background-networking"),
        disposition: Disposition::Taken,
        reason: "Disables periodic background networking (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-background-timer-throttling"),
        disposition: Disposition::Taken,
        reason: "Keeps timers unthrottled in background tabs (ADR 0085).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-backgrounding-occluded-windows"),
        disposition: Disposition::Taken,
        reason: "Keeps occluded browser windows unthrottled (ADR 0085).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-back-forward-cache"),
        disposition: Disposition::Taken,
        reason: "Ensures go_back and go_forward reload documents cleanly through interception (ADR 0121).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-breakpad"),
        disposition: Disposition::Taken,
        reason: "Disables crash report uploads (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-client-side-phishing-detection"),
        disposition: Disposition::Taken,
        reason: "Disables Safe Browsing phishing model downloads (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-component-extensions-with-background-pages"),
        disposition: Disposition::Taken,
        reason: "Stops built-in component extensions from spawning background pages or fetches; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-component-update"),
        disposition: Disposition::Taken,
        reason: "Disables component updater background traffic (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--no-default-browser-check"),
        disposition: Disposition::Taken,
        reason: "Suppresses the default-browser prompt on fresh run profiles; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-default-apps"),
        disposition: Disposition::Taken,
        reason: "Skips default web-app installation into ephemeral run profiles; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-dev-shm-usage"),
        disposition: Disposition::Conditional,
        reason: "Passed only when /dev/shm is below 512 MiB with Warning::SmallSharedMemory (ADR 0049), keeping faster /dev/shm otherwise.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-edgeupdater"),
        disposition: Disposition::Refused,
        reason: "Microsoft Edge macOS-only updater switch; Goodfetch does not launch Edge.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-extensions"),
        disposition: Disposition::Taken,
        reason: "Keeps extensions off in ephemeral run profiles (ADR 0047 rejects extensions); invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::EnabledFeature("CDPScreenshotNewSurface"),
        disposition: Disposition::Refused,
        reason: "Not required by Goodfetch's screenshot path on pinned Chrome 156; keeps --enable-features reserved for Session-scoped features such as GlobalPrivacyControlTest.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--allow-pre-commit-input"),
        disposition: Disposition::Taken,
        reason: "Accepts programmatic input arriving before first paint commit instead of dropping it; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-hang-monitor"),
        disposition: Disposition::Taken,
        reason: "Prevents the native Page Unresponsive modal from interrupting slow or paused pages; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-ipc-flooding-protection"),
        disposition: Disposition::Taken,
        reason: "Prevents Chromium from throttling rapid programmatic history/IPC updates (issue 2330).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-popup-blocking"),
        disposition: Disposition::Taken,
        reason: "Allows script-opened popups without a user activation gesture so page.on(\"popup\") receives them (issue 2330).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-prompt-on-repost"),
        disposition: Disposition::Taken,
        reason: "Suppresses the native Confirm Form Resubmission prompt when reloading a POST response (issue 2330).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-renderer-backgrounding"),
        disposition: Disposition::Taken,
        reason: "Keeps background renderers unthrottled (ADR 0085).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-updater-scheduler"),
        disposition: Disposition::Refused,
        reason: "Microsoft Edge macOS-only updater switch; Goodfetch does not launch Edge.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--force-color-profile=srgb"),
        disposition: Disposition::Refused,
        reason: "Forces sRGB gamut and alters color-gamut media queries and canvas color output away from the host display, an observable fingerprint divergence (ADR 0021).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--metrics-recording-only"),
        disposition: Disposition::Taken,
        reason: "Prevents metrics upload (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--no-first-run"),
        disposition: Disposition::Taken,
        reason: "Suppresses first-run UI on fresh run profiles; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--password-store=basic"),
        disposition: Disposition::Taken,
        reason: "Keeps ephemeral run profiles off the host OS keychain (ADR 0056).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--use-mock-keychain"),
        disposition: Disposition::Taken,
        reason: "Keeps ephemeral run profiles off the macOS keychain prompt (ADR 0056).",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--no-service-autorun"),
        disposition: Disposition::Taken,
        reason: "Prevents registering OS background service autorun on launch; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--export-tagged-pdf"),
        disposition: Disposition::Taken,
        reason: "Emits accessible tagged structure in PDF exports; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-search-engine-choice-screen"),
        disposition: Disposition::Taken,
        reason: "Suppresses the first-run search-engine choice modal in fresh run profiles; invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--unsafely-disable-devtools-self-xss-warnings"),
        disposition: Disposition::Refused,
        reason: "Only affects interactive DevTools Console paste warnings; Goodfetch drives CDP directly and keeps security warnings intact.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--edge-skip-compat-layer-relaunch"),
        disposition: Disposition::Refused,
        reason: "Microsoft Edge Windows-only compatibility-layer switch; Goodfetch does not launch Edge.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-infobars"),
        disposition: Disposition::Taken,
        reason: "Suppresses the Chrome for Testing infobar in headful windows; ignored elsewhere and invisible to sites.",
    },
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--disable-sync"),
        disposition: Disposition::Taken,
        reason: "Disables browser sync background traffic (ADR 0043, CHROMIUM_BACKGROUND_TRAFFIC_OFF).",
    },
    // --- Added by Playwright outside `chromiumSwitches.ts` unless ignored ---
    PlaywrightSwitch {
        switch: SwitchKind::Flag("--enable-automation"),
        disposition: Disposition::Refused,
        reason: "Exposes navigator.webdriver = true, an automation leak forbidden by ADR 0021; replaced where needed by explicit switches such as --disable-auto-reload.",
    },
];

/// The host resolver rule that keeps every `.invalid` name from resolving
/// without a DNS query (RFC 6761, section 6.4), after the `resolve`
/// setting's rules, which come first.
const INVALID_NAMES: &str = "MAP *.invalid ~NOTFOUND";

/// WebRTC's policy: no UDP that no proxy carries, so each of its
/// connections goes over TCP, where the browser's proxy resolution holds it
/// (ADR 0044).
const WEBRTC_POLICY: &str = "--webrtc-ip-handling-policy=disable_non_proxied_udp";

/// [`WEBRTC_POLICY`] for the headless shell, which reads the content
/// layer's switch and not the full build's (#3224).
const SHELL_WEBRTC_POLICY: &str = "--force-webrtc-ip-handling-policy=disable_non_proxied_udp";

/// No reload of a page left on a network error page: Chromium loads one
/// again on its own, a second after its error page and with backoff after
/// that, unless it runs with `--enable-automation`, which the engine never
/// passes. Such a reload is a request no call made, and its failure, which
/// the egress relay refuses again, would be raised by the page's next call
/// as that call's own (fix-queue 83).
const NO_AUTO_RELOAD: &str = "--disable-auto-reload";

/// Chromium features switched off beside the background-traffic table's
/// `--disable-features` list so Speculation Rules prerendering and its
/// fallback prefetch never activate or pre-fetch a document outside
/// `waitForURL`, the Page check and the navigation gate (ADR 0121, #730).
const PRERENDER_OFF: [&str; 2] = ["Prerender2", "Prerender2FallbackPrefetchSpecRules"];

/// Chromium's sandbox switched off, which only a launch whose
/// `security.browser_sandbox` is off passes (ADRs 0017 and 0049): it also
/// removes the renderers' seccomp-bpf layer.
const NO_SANDBOX: &str = "--no-sandbox";

/// Chromium's Global Privacy Control base feature, passed ONLY for a
/// Session with its own browser whose GPC plan enables it (ADR 0048,
/// issue 758): by itself the feature exposes `navigator.globalPrivacyControl`,
/// which is absent from stock Chrome.
const GPC_FEATURE: &str = "--enable-features=GlobalPrivacyControlTest";

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

/// Whether this dedicated Chromium launch enables `GlobalPrivacyControlTest`
/// for `Browser.setGlobalPrivacyControl` (ADR 0048, issue 758).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GlobalPrivacyControl {
    Off,
    On,
}

/// Whether Chromium launches on an engine-owned fresh data folder or on a
/// user's Profile directory in place (ADR 0056; ticket #737).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProfileMode<'a> {
    /// Engine-owned fresh data folder.
    Fresh,
    /// User's Profile directory used in place: omits `--use-mock-keychain` and
    /// `--password-store=basic`, and optionally selects `--profile-directory`.
    InPlace { chromium_profile: Option<&'a str> },
}

/// What a launch decided.
pub(crate) struct Launch<'a> {
    /// The Browser data folder, the Run's own.
    pub(crate) data_folder: &'a Path,
    /// `browser.headless`.
    pub(crate) headless: bool,
    /// Whether the browser is the headless shell (`browser.headless_shell`).
    pub(crate) headless_shell: bool,
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
    /// Whether this dedicated browser enables Chromium's
    /// `GlobalPrivacyControlTest` feature for `Browser.setGlobalPrivacyControl`
    /// (ADR 0048, issue 758).
    pub(crate) gpc: GlobalPrivacyControl,
    /// Fresh engine folder or in-place user Profile directory.
    pub(crate) profile_mode: ProfileMode<'a>,
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

/// Appends `flag` to `arguments` only if it is not already present.
fn push_unique(arguments: &mut Vec<String>, flag: &str) {
    if !arguments.iter().any(|existing| existing == flag) {
        arguments.push(flag.to_owned());
    }
}

/// The command line of the browser `launch` describes.
pub(crate) fn arguments(launch: &Launch<'_>) -> Arguments {
    let mut arguments = vec![
        "--remote-debugging-pipe".to_owned(),
        format!("--user-data-dir={}", launch.data_folder.display()),
    ];
    let in_place_profile = matches!(launch.profile_mode, ProfileMode::InPlace { .. });
    if let ProfileMode::InPlace {
        chromium_profile: Some(profile),
    } = launch.profile_mode
    {
        arguments.push(format!("--profile-directory={profile}"));
    }
    if launch.headless {
        arguments.push("--headless".to_owned());
    }
    let mut disabled_features: Vec<String> = Vec::new();
    for argument in chromium_background_traffic_off() {
        if let Some(features) = argument.strip_prefix("--disable-features=") {
            for feature in features.split(',').filter(|feature| !feature.is_empty()) {
                push_unique(&mut disabled_features, feature);
            }
        } else {
            push_unique(&mut arguments, &argument);
        }
    }
    for entry in PLAYWRIGHT_SWITCHES
        .iter()
        .filter(|entry| entry.disposition == Disposition::Taken)
    {
        match entry.switch {
            SwitchKind::Flag(flag) => {
                // An in-place Profile directory keeps its real OS keychain and
                // password store (ADR 0056; ticket #737).
                if in_place_profile
                    && matches!(flag, "--use-mock-keychain" | "--password-store=basic")
                {
                    continue;
                }
                push_unique(&mut arguments, flag);
            }
            SwitchKind::DisabledFeature(feature) => push_unique(&mut disabled_features, feature),
            SwitchKind::EnabledFeature(_) => {}
        }
    }
    push_unique(&mut arguments, NO_AUTO_RELOAD);
    for feature in PRERENDER_OFF {
        push_unique(&mut disabled_features, feature);
    }
    arguments.push(format!(
        "--disable-features={}",
        disabled_features.join(",")
    ));
    arguments.push(
        if launch.headless_shell {
            SHELL_WEBRTC_POLICY
        } else {
            WEBRTC_POLICY
        }
        .to_owned(),
    );
    if !launch.sandbox {
        arguments.push(NO_SANDBOX.to_owned());
    }
    if launch.gpc == GlobalPrivacyControl::On {
        arguments.push(GPC_FEATURE.to_owned());
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
