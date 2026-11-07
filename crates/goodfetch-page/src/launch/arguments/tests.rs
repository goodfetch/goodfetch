#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::path::Path;

use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::pinned_download::{CHROMIUM_BACKGROUND_TRAFFIC_OFF, ChromiumSwitch};

#[cfg(target_os = "linux")]
use super::shared_memory_of;
use super::{Arguments, Launch, Warning, arguments};

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
        resolve: Box::leak(Box::default()),
        shared_memory,
        proxy_script: None,
        sandbox: true,
        relay_authority: None,
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
