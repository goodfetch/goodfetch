use std::net::IpAddr;
use std::path::Path;

use super::{Arguments, Launch, Warning, arguments, shared_memory_of};

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
        profile: Path::new("/runs/r/browser-profiles/default"),
        headless,
        resolve: &[],
        shared_memory,
    }
}

fn has(built: &Arguments, flag: &str) -> bool {
    built.arguments.iter().any(|argument| argument == flag)
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
                .arguments
                .iter()
                .any(|argument| argument.starts_with("--remote-debugging-port")),
            has(&built, "--user-data-dir=/runs/r/browser-profiles/default"),
            built.arguments.last().map(String::as_str),
        ),
        (true, false, true, Some("about:blank"))
    );
}

#[test]
fn resolve_reaches_chromium_as_host_resolver_rules_each_host_on_its_first_address() {
    let resolve: Vec<(String, Vec<IpAddr>)> = vec![
        (
            "catalog.test".to_owned(),
            vec!["127.0.0.1".parse().unwrap(), "127.0.0.2".parse().unwrap()],
        ),
        ("ipv6.test".to_owned(), vec!["::1".parse().unwrap()]),
        ("nowhere.test".to_owned(), vec![]),
    ];
    let built = arguments(&Launch {
        resolve: &resolve,
        ..launch(true, None)
    });

    assert!(
        has(
            &built,
            "--host-resolver-rules=MAP catalog.test 127.0.0.1, MAP ipv6.test [::1]"
        ),
        "{:?}",
        built.arguments
    );
}

#[test]
fn no_resolve_mapping_passes_no_host_resolver_rules() {
    assert!(
        !arguments(&launch(true, None))
            .arguments
            .iter()
            .any(|argument| argument.starts_with("--host-resolver-rules"))
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
