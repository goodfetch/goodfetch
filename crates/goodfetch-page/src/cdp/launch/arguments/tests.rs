#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;

use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::pinned_download::{CHROMIUM_BACKGROUND_TRAFFIC_OFF, ChromiumSwitch};

#[cfg(target_os = "linux")]
use super::shared_memory_of;
use super::{
    Arguments, Disposition, GlobalPrivacyControl, Launch, PLAYWRIGHT_SWITCHES, SwitchKind, Warning,
    arguments,
};

const MEBIBYTE: u64 = 1024 * 1024;

/// What Playwright, Puppeteer and ADR 0085's question name for a page that
/// must keep its timers behind another tab or window.
const BACKGROUND_FLAGS: [&str; 3] = [
    "--disable-background-timer-throttling",
    "--disable-renderer-backgrounding",
    "--disable-backgrounding-occluded-windows",
];

fn launch(headless: bool, shared_memory: Option<u64>) -> Launch<'static> {
    Launch {
        data_folder: Path::new("/runs/r/browser-data/1"),
        headless,
        headless_shell: false,
        resolve: Box::leak(Box::default()),
        shared_memory,
        proxy_script: None,
        sandbox: true,
        relay_authority: None,
        gpc: GlobalPrivacyControl::Off,
    }
}

fn has(built: &Arguments, flag: &str) -> bool {
    built.command_line.iter().any(|argument| argument == flag)
}

#[test]
fn every_launch_keeps_background_pages_unthrottled_headless_or_not() {
    let headless = arguments(&launch(true, None));
    let headful = arguments(&launch(false, None));

    assert_eq!(
        BACKGROUND_FLAGS.map(|flag| (has(&headless, flag), has(&headful, flag))),
        [(true, true); 3]
    );
}

/// Chromium's sandbox stays on unless the launch switched it off: only
/// then is `--no-sandbox` on the command line (ADR 0017, SC-07).
/// Covers: story:4-85, setting:security.browser_sandbox
#[test]
fn only_a_launch_with_the_sandbox_off_passes_no_sandbox() {
    let sandboxed = arguments(&launch(true, None));
    let unsandboxed = arguments(&Launch {
        sandbox: false,
        ..launch(true, None)
    });

    assert_eq!(
        (
            has(&sandboxed, "--no-sandbox"),
            has(&unsandboxed, "--no-sandbox")
        ),
        (false, true)
    );
}

/// Chromium's `GlobalPrivacyControlTest` feature is passed only for a
/// dedicated GPC browser launch, never for a stock browser (ADR 0048,
/// issue 758).
/// Covers: story:5-103, setting:consent.gpc
#[test]
fn only_a_gpc_launch_enables_global_privacy_control_test_feature() {
    let stock = arguments(&launch(true, None));
    let with_gpc = arguments(&Launch {
        gpc: GlobalPrivacyControl::On,
        ..launch(true, None)
    });

    assert_eq!(
        (
            has(&stock, "--enable-features=GlobalPrivacyControlTest"),
            has(&with_gpc, "--enable-features=GlobalPrivacyControlTest"),
        ),
        (false, true)
    );
}

#[test]
fn only_a_headless_launch_passes_headless() {
    assert_eq!(
        (
            has(&arguments(&launch(true, None)), "--headless"),
            has(&arguments(&launch(false, None)), "--headless"),
        ),
        (true, false)
    );
}

#[test]
fn the_browser_runs_over_its_pipe_on_the_runs_profile_and_opens_a_blank_page_last() {
    let built = arguments(&launch(true, None));

    assert_eq!(
        (
            has(&built, "--remote-debugging-pipe"),
            built
                .command_line
                .iter()
                .any(|argument| argument.starts_with("--remote-debugging-port")),
            has(&built, "--user-data-dir=/runs/r/browser-data/1"),
            built.command_line.last().map(String::as_str),
        ),
        (true, false, true, Some("about:blank"))
    );
}

#[test]
fn resolve_reaches_chromium_as_host_resolver_rules_each_host_on_its_first_address() {
    let resolve = HostMapping::new([
        (
            "catalog.test",
            vec!["127.0.0.1".parse().unwrap(), "127.0.0.2".parse().unwrap()],
        ),
        ("ipv6.test", vec!["::1".parse().unwrap()]),
        ("nowhere.test", vec![]),
    ]);
    let built = arguments(&Launch {
        resolve: &resolve,
        ..launch(true, None)
    });

    assert!(
        has(
            &built,
            "--host-resolver-rules=MAP catalog.test 127.0.0.1, MAP ipv6.test [::1], MAP *.invalid ~NOTFOUND"
        ),
        "{:?}",
        built.command_line
    );
}

#[test]
fn with_no_resolve_mapping_only_invalid_names_are_refused_without_a_query() {
    assert!(has(
        &arguments(&launch(true, None)),
        "--host-resolver-rules=MAP *.invalid ~NOTFOUND"
    ));
}

#[test]
fn every_launch_turns_the_back_forward_cache_off() {
    assert_eq!(
        (
            has(
                &arguments(&launch(true, None)),
                "--disable-back-forward-cache"
            ),
            has(
                &arguments(&launch(false, None)),
                "--disable-back-forward-cache"
            ),
        ),
        (true, true)
    );
}

/// A page left on a network error page stays there: Chromium loads one
/// again on its own a second later unless told not to, or unless it runs
/// with `--enable-automation`, which the engine never passes (fix-queue 83).
#[test]
fn every_launch_leaves_a_failed_page_on_its_error_page() {
    assert_eq!(
        (
            has(&arguments(&launch(true, None)), "--disable-auto-reload"),
            has(&arguments(&launch(false, None)), "--disable-auto-reload"),
        ),
        (true, true)
    );
}

#[test]
fn a_small_dev_shm_moves_chromiums_shared_memory_to_tmp_with_a_warning() {
    let built = arguments(&launch(true, Some(64 * MEBIBYTE)));

    assert_eq!(
        (has(&built, "--disable-dev-shm-usage"), built.warnings),
        (
            true,
            vec![Warning::SmallSharedMemory {
                bytes: 64 * MEBIBYTE
            }]
        )
    );
}

#[test]
fn a_large_or_unmeasured_dev_shm_keeps_chromiums_shared_memory_there() {
    let large = arguments(&launch(true, Some(1024 * MEBIBYTE)));
    let unmeasured = arguments(&launch(true, None));

    assert_eq!(
        (
            has(&large, "--disable-dev-shm-usage"),
            large.warnings.is_empty(),
            has(&unmeasured, "--disable-dev-shm-usage"),
            unmeasured.warnings.is_empty(),
        ),
        (false, true, false, true)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn the_dev_shm_size_is_read_from_its_mount_options() {
    let docker = "overlay / overlay rw,relatime 0 0\n\
        shm /dev/shm tmpfs rw,nosuid,nodev,noexec,relatime,size=65536k 0 0\n";
    let unsized_mount = "tmpfs /dev/shm tmpfs rw,nosuid,nodev 0 0\n";
    let none = "proc /proc proc rw 0 0\n";

    assert_eq!(
        (
            shared_memory_of(docker),
            shared_memory_of(unsized_mount),
            shared_memory_of(none),
        ),
        (Some(64 * MEBIBYTE), None, None)
    );
}

/// A launch on the Run's own profile never reaches the OS keychain, which
/// would ask the person at the machine for its password (ADR 0056 keeps
/// the keychain only for a Profile directory used in place).
#[test]
fn a_launch_on_the_runs_profile_keeps_away_from_the_os_keychain() {
    let built = arguments(&launch(false, None));

    assert_eq!(
        (
            has(&built, "--use-mock-keychain"),
            has(&built, "--password-store=basic"),
            has(&built, "--no-first-run"),
        ),
        (true, true, true)
    );
}

/// Every entry of `pinned_download`'s background-traffic table (ADR 0043)
/// is on the command line of a launch, headless or not: each flag as an
/// argument, each feature in the launch's one `--disable-features`.
/// Covers: story:4-83
#[test]
fn every_launch_carries_every_entry_of_the_background_traffic_table() {
    let omitted = |built: &Arguments| {
        let disabled_features: Vec<&str> = built
            .command_line
            .iter()
            .filter_map(|argument| argument.strip_prefix("--disable-features="))
            .collect();
        let missing: Vec<ChromiumSwitch> = CHROMIUM_BACKGROUND_TRAFFIC_OFF
            .into_iter()
            .filter(|switch| match switch {
                ChromiumSwitch::Flag(flag) => !has(built, flag),
                ChromiumSwitch::DisabledFeature(feature) => !disabled_features
                    .last()
                    .is_some_and(|features| features.split(',').any(|named| named == *feature)),
            })
            .collect();
        (missing, disabled_features.len())
    };

    assert_eq!(
        (
            omitted(&arguments(&launch(true, None))),
            omitted(&arguments(&launch(false, None))),
        ),
        ((Vec::new(), 1), (Vec::new(), 1))
    );
}

/// Every Chromium launch disables Speculation Rules prerendering and its
/// fallback prefetch inside its single `--disable-features` switch
/// (ADR 0121, #730).
/// Covers: story:5-25
#[test]
fn every_launch_disables_prerendering_in_its_one_disable_features_switch() {
    let has_prerender_off = |built: &Arguments| {
        let switches: Vec<&str> = built
            .command_line
            .iter()
            .filter_map(|argument| argument.strip_prefix("--disable-features="))
            .collect();
        switches.len() == 1
            && ["Prerender2", "Prerender2FallbackPrefetchSpecRules"]
                .into_iter()
                .all(|feature| switches[0].split(',').any(|named| named == feature))
    };

    assert_eq!(
        (
            has_prerender_off(&arguments(&launch(true, None))),
            has_prerender_off(&arguments(&launch(false, None))),
        ),
        (true, true)
    );
}

/// The Run's domain policy reaches the browser as the proxy auto-config
/// script it gives, whole, and only when it gives one; WebRTC sends no UDP
/// a proxy could not carry either way (ADRs 0014 and 0044).
#[test]
fn a_policy_script_reaches_the_browser_as_its_proxy_configuration() {
    let script = "function FindProxyForURL(url, host) { return \"DIRECT\"; }";
    let held = arguments(&Launch {
        proxy_script: Some(script),
        ..launch(true, None)
    });
    let open = arguments(&launch(true, None));
    let configured = |built: &Arguments| {
        built
            .command_line
            .iter()
            .filter_map(|argument| {
                argument
                    .strip_prefix("--proxy-pac-url=data:application/x-ns-proxy-autoconfig;base64,")
            })
            .map(|encoded| {
                String::from_utf8(data_encoding::BASE64.decode(encoded.as_bytes()).unwrap())
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let webrtc = "--webrtc-ip-handling-policy=disable_non_proxied_udp";

    assert_eq!(
        (
            configured(&held),
            configured(&open),
            has(&held, webrtc),
            has(&open, webrtc)
        ),
        (vec![script.to_owned()], Vec::new(), true, true)
    );
}

/// The headless shell reads WebRTC's policy from the content layer's
/// switch alone, and the full build from its own alone, so each launch
/// passes its build's (#3224): without it the shell sent UDP around the
/// egress relay (ADR 0044).
#[test]
fn the_headless_shell_holds_webrtc_to_proxied_udp_by_its_own_switch() {
    let full = arguments(&launch(true, None));
    let shell = arguments(&Launch {
        headless_shell: true,
        ..launch(true, None)
    });
    let policy = |built: &Arguments| {
        built
            .command_line
            .iter()
            .filter(|argument| argument.contains("webrtc"))
            .cloned()
            .collect::<Vec<_>>()
    };

    assert_eq!(
        (policy(&full), policy(&shell)),
        (
            vec!["--webrtc-ip-handling-policy=disable_non_proxied_udp".to_owned()],
            vec!["--force-webrtc-ip-handling-policy=disable_non_proxied_udp".to_owned()]
        )
    );
}

/// Story 4-40: a Run with a certificate exception launches its browser
/// trusting the egress relay's in-memory authority by its key's hash, and
/// one with none trusts nothing more.
#[test]
fn a_run_with_a_relay_authority_trusts_it_by_its_key_hash_alone() {
    let trusting = arguments(&Launch {
        relay_authority: Some("c2hhMjU2LW9mLXRoZS1rZXk="),
        ..launch(true, None)
    });
    let plain = arguments(&launch(true, None));
    let flags = |built: &Arguments| -> Vec<String> {
        built
            .command_line
            .iter()
            .filter(|argument| argument.starts_with("--ignore-certificate-errors"))
            .cloned()
            .collect()
    };

    assert_eq!(
        (flags(&trusting), flags(&plain)),
        (
            vec!["--ignore-certificate-errors-spki-list=c2hhMjU2LW9mLXRoZS1rZXk=".to_owned()],
            Vec::new()
        )
    );
}

/// On Linux the browser's character type is UTF-8 whatever the host's
/// locale, so a Browser data folder whose path is not ASCII opens and
/// closes in time under `LANG=C` or a locale the system lacks (fix-queue
/// 127).
#[cfg(target_os = "linux")]
#[test]
fn every_linux_launch_gives_the_browser_a_utf8_character_type_whatever_the_hosts_locale() {
    let built = arguments(&launch(true, None));

    assert_eq!(built.environment, [("LC_ALL", "C.UTF-8")]);
}

/// Every switch of Playwright 1.64.1's `chromiumSwitches.ts` (`disabledFeatures`
/// and `chromiumSwitches`), plus `--enable-automation` which Playwright adds
/// outside that file, pinned here so the reconciliation table must answer every
/// one with a disposition and a non-empty reason (issue 2330).
const PLAYWRIGHT_1_64_1_SWITCHES: [SwitchKind; 53] = [
    // `disabledFeatures` in Playwright 1.64.1 `chromiumSwitches.ts`
    SwitchKind::DisabledFeature("AvoidUnnecessaryBeforeUnloadCheckSync"),
    SwitchKind::DisabledFeature("DestroyProfileOnBrowserClose"),
    SwitchKind::DisabledFeature("DialMediaRouteProvider"),
    SwitchKind::DisabledFeature("GlobalMediaControls"),
    SwitchKind::DisabledFeature("HttpsUpgrades"),
    SwitchKind::DisabledFeature("LensOverlay"),
    SwitchKind::DisabledFeature("MediaRouter"),
    SwitchKind::DisabledFeature("PaintHolding"),
    SwitchKind::DisabledFeature("ThirdPartyStoragePartitioning"),
    SwitchKind::DisabledFeature("BlockOriginHeaderModificationOnRedirect"),
    SwitchKind::DisabledFeature("AvoidCorsURLLoaderRestartOnRedirect"),
    SwitchKind::DisabledFeature("Translate"),
    SwitchKind::DisabledFeature("AutoDeElevate"),
    SwitchKind::DisabledFeature("OptimizationHints"),
    SwitchKind::DisabledFeature("NetworkTimeServiceQuerying"),
    SwitchKind::DisabledFeature("AimEnabled"),
    SwitchKind::DisabledFeature("msForceBrowserSignIn"),
    SwitchKind::DisabledFeature("msEdgeUpdateLaunchServicesPreferredVersion"),
    // `chromiumSwitches` in Playwright 1.64.1 `chromiumSwitches.ts`
    SwitchKind::Flag("--disable-field-trial-config"),
    SwitchKind::Flag("--disable-background-networking"),
    SwitchKind::Flag("--disable-background-timer-throttling"),
    SwitchKind::Flag("--disable-backgrounding-occluded-windows"),
    SwitchKind::Flag("--disable-back-forward-cache"),
    SwitchKind::Flag("--disable-breakpad"),
    SwitchKind::Flag("--disable-client-side-phishing-detection"),
    SwitchKind::Flag("--disable-component-extensions-with-background-pages"),
    SwitchKind::Flag("--disable-component-update"),
    SwitchKind::Flag("--no-default-browser-check"),
    SwitchKind::Flag("--disable-default-apps"),
    SwitchKind::Flag("--disable-dev-shm-usage"),
    SwitchKind::Flag("--disable-edgeupdater"),
    SwitchKind::Flag("--disable-extensions"),
    SwitchKind::EnabledFeature("CDPScreenshotNewSurface"),
    SwitchKind::Flag("--allow-pre-commit-input"),
    SwitchKind::Flag("--disable-hang-monitor"),
    SwitchKind::Flag("--disable-ipc-flooding-protection"),
    SwitchKind::Flag("--disable-popup-blocking"),
    SwitchKind::Flag("--disable-prompt-on-repost"),
    SwitchKind::Flag("--disable-renderer-backgrounding"),
    SwitchKind::Flag("--disable-updater-scheduler"),
    SwitchKind::Flag("--force-color-profile=srgb"),
    SwitchKind::Flag("--metrics-recording-only"),
    SwitchKind::Flag("--no-first-run"),
    SwitchKind::Flag("--password-store=basic"),
    SwitchKind::Flag("--use-mock-keychain"),
    SwitchKind::Flag("--no-service-autorun"),
    SwitchKind::Flag("--export-tagged-pdf"),
    SwitchKind::Flag("--disable-search-engine-choice-screen"),
    SwitchKind::Flag("--unsafely-disable-devtools-self-xss-warnings"),
    SwitchKind::Flag("--edge-skip-compat-layer-relaunch"),
    SwitchKind::Flag("--disable-infobars"),
    SwitchKind::Flag("--disable-sync"),
    // Passed by Playwright outside `chromiumSwitches.ts` unless ignored
    SwitchKind::Flag("--enable-automation"),
];

/// Every switch in Playwright 1.64.1's `chromiumSwitches.ts` is answered once
/// in [`PLAYWRIGHT_SWITCHES`] with a written reason, and the table holds no
/// duplicate or unanswered entries (issue 2330).
#[test]
fn every_playwright_chromium_switch_is_answered_in_the_reconciliation_table() {
    let missing: Vec<SwitchKind> = PLAYWRIGHT_1_64_1_SWITCHES
        .into_iter()
        .filter(|expected| {
            !PLAYWRIGHT_SWITCHES
                .iter()
                .any(|entry| entry.switch == *expected)
        })
        .collect();
    let unexpected: Vec<SwitchKind> = PLAYWRIGHT_SWITCHES
        .iter()
        .map(|entry| entry.switch)
        .filter(|entry| !PLAYWRIGHT_1_64_1_SWITCHES.contains(entry))
        .collect();
    let empty_reasons: Vec<SwitchKind> = PLAYWRIGHT_SWITCHES
        .iter()
        .filter(|entry| entry.reason.trim().is_empty())
        .map(|entry| entry.switch)
        .collect();
    let mut seen = Vec::with_capacity(PLAYWRIGHT_SWITCHES.len());
    let mut duplicates = Vec::new();
    for entry in PLAYWRIGHT_SWITCHES {
        if seen.contains(&entry.switch) {
            duplicates.push(entry.switch);
        } else {
            seen.push(entry.switch);
        }
    }

    assert_eq!(
        (missing, unexpected, empty_reasons, duplicates),
        (Vec::new(), Vec::new(), Vec::new(), Vec::new())
    );
}

/// Every switch marked [`Disposition::Taken`] in [`PLAYWRIGHT_SWITCHES`] is
/// passed on every Chromium launch (headless and headful), with no duplicate
/// flags or features and a single `--disable-features` argument; every
/// [`Disposition::Refused`] switch stays off stock launches (issue 2330).
#[test]
fn every_taken_playwright_switch_is_passed_and_every_refused_switch_stays_off() {
    let check = |built: &Arguments| {
        let disable_features: Vec<&str> = built
            .command_line
            .iter()
            .filter_map(|argument| argument.strip_prefix("--disable-features="))
            .collect();
        let enable_features: Vec<&str> = built
            .command_line
            .iter()
            .filter_map(|argument| argument.strip_prefix("--enable-features="))
            .collect();
        let disabled: Vec<&str> = disable_features
            .last()
            .map(|list| list.split(',').collect())
            .unwrap_or_default();
        let enabled: Vec<&str> = enable_features
            .last()
            .map(|list| list.split(',').collect())
            .unwrap_or_default();

        let missing_taken: Vec<SwitchKind> = PLAYWRIGHT_SWITCHES
            .iter()
            .filter(|entry| entry.disposition == Disposition::Taken)
            .map(|entry| entry.switch)
            .filter(|switch| match *switch {
                SwitchKind::Flag(flag) => !has(built, flag),
                SwitchKind::DisabledFeature(feature) => !disabled.contains(&feature),
                SwitchKind::EnabledFeature(feature) => !enabled.contains(&feature),
            })
            .collect();
        let leaked_refused: Vec<SwitchKind> = PLAYWRIGHT_SWITCHES
            .iter()
            .filter(|entry| entry.disposition == Disposition::Refused)
            .map(|entry| entry.switch)
            .filter(|switch| match *switch {
                SwitchKind::Flag(flag) => has(built, flag),
                SwitchKind::DisabledFeature(feature) => disabled.contains(&feature),
                SwitchKind::EnabledFeature(feature) => enabled.contains(&feature),
            })
            .collect();
        let mut unique_args = Vec::with_capacity(built.command_line.len());
        let mut dup_args = Vec::new();
        for arg in &built.command_line {
            if unique_args.contains(arg) {
                dup_args.push(arg.clone());
            } else {
                unique_args.push(arg.clone());
            }
        }
        let mut unique_disabled = Vec::with_capacity(disabled.len());
        let mut dup_disabled = Vec::new();
        for feature in &disabled {
            if unique_disabled.contains(feature) {
                dup_disabled.push((*feature).to_owned());
            } else {
                unique_disabled.push(*feature);
            }
        }
        (
            disable_features.len(),
            missing_taken,
            leaked_refused,
            dup_args,
            dup_disabled,
        )
    };

    assert_eq!(
        (
            check(&arguments(&launch(true, None))),
            check(&arguments(&launch(false, None))),
        ),
        (
            (1, Vec::new(), Vec::new(), Vec::new(), Vec::new()),
            (1, Vec::new(), Vec::new(), Vec::new(), Vec::new()),
        )
    );
}
