use std::path::PathBuf;

use goodfetch_engine::browser_cache::BuildPlatform;
use rstest::rstest;
use url::Url;

use super::{DocumentRequest, Unread};

mod installed;

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

/// Each reason's message, the warning a Run gets, names why and the fix.
#[rstest]
#[case::not_installed(
    Unread::NotInstalled { looked_in: vec![PathBuf::from("/usr/bin/chromium"), PathBuf::from("/usr/bin/chromium-browser")] },
    "No browser is installed at /usr/bin/chromium, /usr/bin/chromium-browser, so a `standard` Session's own HTTP transport presents no browser identity: install the distribution's Chromium package."
)]
#[case::unstarted(
    Unread::Unstarted { executable: PathBuf::from("/usr/bin/chromium"), error: "Permission denied (os error 13)".to_owned() },
    "The browser at /usr/bin/chromium could not be started for `--version` (Permission denied (os error 13)), so a `standard` Session's own HTTP transport presents no browser identity: let the engine's user run it, or reinstall the distribution's Chromium package."
)]
#[case::no_version(
    Unread::NoVersion { executable: PathBuf::from("/usr/bin/chromium") },
    "The browser at /usr/bin/chromium printed no version for `--version`, so a `standard` Session's own HTTP transport presents no browser identity: reinstall the distribution's Chromium package."
)]
fn the_message_names_why_and_the_fix(#[case] unread: Unread, #[case] message: &str) {
    assert_eq!(unread.to_string(), message);
}
