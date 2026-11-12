//! The document request of an installed browser (issue 2532): the version
//! its executable prints for `--version`, read through the crate's public
//! interface from stand-in executables in the test's own folder and from
//! the Browser cache's Chrome for Testing.

#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write stand-in browsers to the test's own folder and read what they left, outside any engine (ADR 0059)"
)]

use std::path::{Path, PathBuf};

use goodfetch_engine::browser_cache::BuildPlatform;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_page::document_request::{DocumentRequest, Unread};
use goodfetch_test_support::{
    CHROME_FOR_TESTING_BUILD, TestEnv, browser_cache, chrome_for_testing,
};
use rstest::rstest;

/// The platform whose user agent and `Sec-CH-UA-Platform` a browser on
/// this host presents.
fn hosts_platform() -> BuildPlatform {
    if cfg!(target_os = "macos") {
        BuildPlatform::MacArm64
    } else {
        BuildPlatform::LinuxX64
    }
}

/// An executable at `folder`/`name` that runs the shell `script`.
fn executable(folder: &Path, name: &str, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = folder.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n"))
        .expect("the test's folder takes a stand-in browser");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("the stand-in browser is the test's own");
    path
}

/// The document request of the installed browser among `candidates`.
async fn of_installed(candidates: &[PathBuf]) -> Result<DocumentRequest, Unread> {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    DocumentRequest::of_installed(candidates, &owner.blocking_threads(), &Clock::system()).await
}

/// The distribution's Chromium, the first of the paths that exists,
/// prints its version for `--version`: the request is that major's, on
/// the host's platform, as a pinned build's of the same major.
#[tokio::test]
async fn an_installed_browsers_request_is_that_of_the_version_it_prints() {
    let env = TestEnv::new();
    let chromium = executable(
        env.folder(),
        "chromium",
        "echo 'Chromium 141.0.7390.54 built on Debian 12.7, running on Debian 12.7'",
    );

    let request = of_installed(&[env.folder().join("absent"), chromium]).await;

    assert_eq!(
        request,
        Ok(DocumentRequest::of_build("141.0.7390.54", hosts_platform()).unwrap())
    );
}

/// An executable that prints no version, or exits with an error: no
/// request, and why.
#[rstest]
#[case::no_version("echo 'Usage: chromium [OPTION]... [URL]...'")]
#[case::an_error("echo 'Chromium 141.0.7390.54'; exit 1")]
#[tokio::test]
async fn an_installed_browser_that_prints_no_version_gives_no_request(#[case] script: &str) {
    let env = TestEnv::new();
    let chromium = executable(env.folder(), "chromium", script);

    let request = of_installed(std::slice::from_ref(&chromium)).await;

    assert_eq!(
        request,
        Err(Unread::NoVersion {
            executable: chromium
        })
    );
}

/// An executable that does not exit within its 10 seconds gives no
/// request, on the paused clock.
#[tokio::test(start_paused = true)]
async fn an_installed_browser_that_does_not_exit_in_time_gives_no_request() {
    let env = TestEnv::new();
    let chromium = executable(env.folder(), "chromium", "exec sleep 600");

    let request = of_installed(std::slice::from_ref(&chromium)).await;

    assert_eq!(
        request,
        Err(Unread::NoVersion {
            executable: chromium
        })
    );
}

/// A file the engine's user may not run is not started, and the reason
/// says so.
#[tokio::test]
async fn an_installed_browser_that_cannot_start_gives_no_request() {
    use std::os::unix::fs::PermissionsExt as _;
    let env = TestEnv::new();
    let chromium = executable(env.folder(), "chromium", "echo 'Chromium 141.0.7390.54'");
    std::fs::set_permissions(&chromium, std::fs::Permissions::from_mode(0o644)).unwrap();

    let request = of_installed(std::slice::from_ref(&chromium)).await;

    assert!(
        matches!(&request, Err(Unread::Unstarted { executable, .. }) if *executable == chromium),
        "{request:?}"
    );
}

#[tokio::test]
async fn no_installed_browser_gives_no_request() {
    let env = TestEnv::new();
    let looked_in = vec![env.folder().join("chromium"), env.folder().join("chrome")];

    let request = of_installed(&looked_in).await;

    assert_eq!(request, Err(Unread::NotInstalled { looked_in }));
}

/// A browser that leaves a process behind when it prints its version:
/// none outlives the read. The process held the output open, so the read
/// ended only once it had exited: it is gone, or a zombie its new parent
/// has yet to reap.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn no_process_outlives_the_version_read() {
    let env = TestEnv::new();
    let left = env.folder().join("left");
    let chromium = executable(
        env.folder(),
        "chromium",
        &format!(
            "sleep 600 &\necho $! > '{}'\necho 'Chromium 141.0.7390.54'",
            left.display()
        ),
    );

    let request = of_installed(&[chromium]).await;

    assert!(request.is_ok(), "{request:?}");
    let process = std::fs::read_to_string(&left).unwrap();
    let state = std::fs::read_to_string(Path::new("/proc").join(process.trim()).join("stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().next().map(str::to_owned))
        });
    assert!(
        matches!(state.as_deref(), None | Some("Z" | "X")),
        "{} outlived the read in state {state:?}",
        process.trim()
    );
}

/// Chrome for Testing's own executable, read as an installed browser,
/// prints its version and exits, and its request is the pinned build's:
/// the one Foxhound's test compares with the build's own navigation.
#[tokio::test]
async fn chrome_for_testing_read_as_an_installed_browser_presents_its_pinned_request() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();

    let request = of_installed(&[chrome_for_testing(&cache)]).await;

    assert_eq!(
        request,
        Ok(DocumentRequest::of_build(CHROME_FOR_TESTING_BUILD, hosts_platform()).unwrap())
    );
}
