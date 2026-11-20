//! The document request of an installed browser (issue 2532) where the
//! installed browser is the Browser cache's Chrome for Testing: the version
//! its executable prints for `--version`, read through the installed branch
//! of `DocumentRequest::of_source`, the one a Session's browser takes on a
//! platform with no pinned build. The stand-in executables' cases are the
//! `document_request` module's unit tests.

#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test runs on `#[tokio::test]`'s own runtime with real time, as the real browser it reads answers in real time (ADR 0060)"
)]

use std::path::PathBuf;

use goodfetch_engine::browser_cache::{BrowserSource, BuildPlatform};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_page::document_request::{DocumentRequest, Unread};
use goodfetch_test_support::{
    CHROME_FOR_TESTING_BUILD, TestEnv, browser_cache, chrome_for_testing,
};

/// The platform whose user agent and `Sec-CH-UA-Platform` a browser on
/// this host presents.
fn hosts_platform() -> BuildPlatform {
    if cfg!(target_os = "macos") {
        BuildPlatform::MacArm64
    } else {
        BuildPlatform::LinuxX64
    }
}

/// The document request of a Session's browser where it is the one
/// installed among `candidates`, as on a platform with no pinned build: no
/// pin is offered, so a request can only come from the version read.
async fn of_installed(candidates: &[PathBuf]) -> Result<Option<DocumentRequest>, Unread> {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let source = BrowserSource::Installed {
        candidates: candidates.to_vec(),
        notice: None,
    };
    DocumentRequest::of_source(source, &[], &owner.blocking_threads(), &Clock::system()).await
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
        Ok(Some(
            DocumentRequest::of_build(CHROME_FOR_TESTING_BUILD, hosts_platform()).unwrap()
        ))
    );
}
