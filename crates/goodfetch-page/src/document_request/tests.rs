#![expect(
    clippy::disallowed_methods,
    reason = "the tests write stand-in browsers to a temporary folder and read what they left, outside any engine (ADR 0059)"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_engine::browser_cache::BuildPlatform;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::run::warning::BROWSER_VERSION_UNREAD;
use goodfetch_test_support::{
    CHROME_FOR_TESTING_BUILD, TestEnv, browser_cache, chrome_for_testing,
};
use rstest::rstest;
use url::Url;

use super::{DocumentRequest, Unread};

/// The value of `name` among the headers `request` sends to `url`.
fn header(request: &DocumentRequest, url: &str, name: &str) -> Option<String> {
    request
        .headers(&Url::parse(url).unwrap())
        .iter()
        .find(|(header, _)| header == name)
        .map(|(_, value)| value.clone())
}

/// The brands Chromium 124 and 131 sent, as sites recorded them: the
/// made-up brand's name, version and place each follow the major.
#[rstest]
#[case::an_even_major("124.0.6367.60-linux64", r#""Not-A.Brand";v="99", "Chromium";v="124""#)]
#[case::an_odd_major("131.0.6778.33-linux64", r#""Chromium";v="131", "Not_A Brand";v="24""#)]
fn the_brands_follow_the_builds_major(#[case] build: &str, #[case] brands: &str) {
    let request = DocumentRequest::of_build(build, BuildPlatform::LinuxX64).unwrap();

    assert_eq!(
        header(&request, "https://example.com/", "sec-ch-ua").as_deref(),
        Some(brands)
    );
}

#[rstest]
#[case::linux(
    BuildPlatform::LinuxX64,
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
    "\"Linux\""
)]
#[case::mac(
    BuildPlatform::MacArm64,
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
    "\"macOS\""
)]
#[case::windows(
    BuildPlatform::WindowsX64,
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
    "\"Windows\""
)]
fn the_user_agent_is_the_reduced_one_of_the_builds_platform(
    #[case] platform: BuildPlatform,
    #[case] user_agent: &str,
    #[case] hint: &str,
) {
    let request = DocumentRequest::of_build("141.0.7390.54-linux64", platform).unwrap();

    assert_eq!(
        (
            header(&request, "http://example.com/", "user-agent").as_deref(),
            header(&request, "https://example.com/", "sec-ch-ua-platform").as_deref()
        ),
        (Some(user_agent), Some(hint))
    );
}

/// Client hints and fetch metadata go only to a potentially trustworthy
/// origin, and only there does the browser ask for `br` and `zstd`.
#[rstest]
#[case::https("https://example.com/", true, "gzip, deflate, br, zstd")]
#[case::localhost("http://localhost:8080/", true, "gzip, deflate, br, zstd")]
#[case::a_name_under_localhost("http://shop.localhost/", true, "gzip, deflate, br, zstd")]
#[case::loopback("http://127.0.0.1/", true, "gzip, deflate, br, zstd")]
#[case::loopback_ipv6("http://[::1]/", true, "gzip, deflate, br, zstd")]
#[case::plain_http("http://example.com/", false, "gzip, deflate")]
#[case::a_name_ending_in_localhost("http://notlocalhost/", false, "gzip, deflate")]
fn client_hints_go_to_trustworthy_origins_only(
    #[case] url: &str,
    #[case] trustworthy: bool,
    #[case] codings: &str,
) {
    let request =
        DocumentRequest::of_build("141.0.7390.54-linux64", BuildPlatform::LinuxX64).unwrap();

    assert_eq!(
        (
            header(&request, url, "sec-ch-ua").is_some(),
            header(&request, url, "sec-fetch-dest").is_some(),
            header(&request, url, "accept-encoding").as_deref(),
        ),
        (trustworthy, trustworthy, Some(codings))
    );
}

#[test]
fn a_build_id_with_no_major_gives_no_request() {
    assert_eq!(
        DocumentRequest::of_build("stable", BuildPlatform::LinuxX64),
        None
    );
}

/// The platform whose user agent and `Sec-CH-UA-Platform` a browser on
/// this host presents.
fn hosts_platform() -> BuildPlatform {
    if cfg!(target_os = "macos") {
        BuildPlatform::MacArm64
    } else if cfg!(windows) {
        BuildPlatform::WindowsX64
    } else {
        BuildPlatform::LinuxX64
    }
}

/// An executable at `folder`/`name` that runs the shell `script`.
#[cfg(unix)]
fn executable(folder: &Path, name: &str, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = folder.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
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
#[cfg(unix)]
#[tokio::test]
async fn an_installed_browsers_request_is_that_of_the_version_it_prints() {
    let folder = tempfile::tempdir().unwrap();
    let chromium = executable(
        folder.path(),
        "chromium",
        "echo 'Chromium 141.0.7390.54 built on Debian 12.7, running on Debian 12.7'",
    );

    let request = of_installed(&[folder.path().join("absent"), chromium]).await;

    assert_eq!(
        request,
        Ok(DocumentRequest::of_build("141.0.7390.54", hosts_platform()).unwrap())
    );
}

/// An executable that prints no version, or exits with an error, and a
/// browser installed at none of the paths: no request, and why, which the
/// Run's warning names.
#[cfg(unix)]
#[rstest]
#[case::no_version("echo 'Usage: chromium [OPTION]... [URL]...'")]
#[case::an_error("echo 'Chromium 141.0.7390.54'; exit 1")]
#[tokio::test]
async fn an_installed_browser_that_prints_no_version_gives_no_request(#[case] script: &str) {
    let folder = tempfile::tempdir().unwrap();
    let chromium = executable(folder.path(), "chromium", script);

    let request = of_installed(std::slice::from_ref(&chromium)).await;

    assert_eq!(
        request,
        Err(Unread::NoVersion {
            executable: chromium
        })
    );
}

#[tokio::test]
async fn no_installed_browser_gives_no_request() {
    let folder = tempfile::tempdir().unwrap();
    let looked_in = vec![folder.path().join("chromium"), folder.path().join("chrome")];

    let request = of_installed(&looked_in).await;

    assert_eq!(request, Err(Unread::NotInstalled { looked_in }));
}

/// The warning a Run gets for each reason names the fix.
#[rstest]
#[case::not_installed(
    Unread::NotInstalled { looked_in: vec![PathBuf::from("/usr/bin/chromium")] },
    "No browser is installed at /usr/bin/chromium, so a `standard` Session's own HTTP transport presents no browser identity: install the distribution's Chromium package."
)]
#[case::no_version(
    Unread::NoVersion { executable: PathBuf::from("/usr/bin/chromium") },
    "The browser at /usr/bin/chromium printed no version for `--version`, so a `standard` Session's own HTTP transport presents no browser identity: reinstall the distribution's Chromium package."
)]
fn the_runs_warning_names_why_and_the_fix(#[case] unread: Unread, #[case] message: &str) {
    let warning = unread.warning();

    assert_eq!(
        (warning.code.as_str(), warning.message.as_str()),
        (BROWSER_VERSION_UNREAD, message)
    );
}

/// A browser that leaves a process behind when it prints its version:
/// none outlives the read.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn no_process_outlives_the_version_read() {
    let folder = tempfile::tempdir().unwrap();
    let left = folder.path().join("left");
    let chromium = executable(
        folder.path(),
        "chromium",
        &format!(
            "sleep 600 &\necho $! > '{}'\necho 'Chromium 141.0.7390.54'",
            left.display()
        ),
    );

    let request = of_installed(&[chromium]).await;

    assert!(request.is_ok(), "{request:?}");
    let process = std::fs::read_to_string(&left).unwrap();
    let alive = Path::new("/proc").join(process.trim());
    let mut waited = Duration::ZERO;
    while alive.exists() && waited < Duration::from_secs(10) {
        tokio::time::sleep(Duration::from_millis(50)).await;
        waited += Duration::from_millis(50);
    }
    assert!(!alive.exists(), "{} outlived the read", process.trim());
}

/// Chrome for Testing's own executable, read as an installed browser,
/// prints its version and exits, and its request is the pinned build's:
/// the one Foxhound's test compares with the build's own navigation. On
/// Unix only: an installed browser is read only where the platform has no
/// pinned build, Linux arm64 and musl, and Windows has one.
#[cfg(unix)]
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
