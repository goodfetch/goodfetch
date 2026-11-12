//! The request a `standard` Session's browser sends for a document (ADR
//! 0021; issue 2315): its `User-Agent` and the headers of a navigation, in
//! the browser's order, for a client that sends a Session's HTTP without
//! its browser and must present the same identity.
//!
//! It is read from the build the Session's browser would launch, never
//! from a launched browser, so a Session that only ever sends HTTP starts
//! none. Chrome for Testing fixes all of it by its major version and its
//! platform:
//!
//! - **The user agent** is Chrome's reduced one, the window's (the
//!   `driver`'s `identity` module gives a headless page the same): its
//!   platform part is one constant for each operating system and its
//!   version is the major alone.
//! - **The brands** of `Sec-CH-UA` are `Chromium` and one made-up brand,
//!   whose name, version and place Chromium derives from the major (its
//!   `GenerateBrandVersionList`).
//! - **The client hints and fetch metadata** go only to a potentially
//!   trustworthy origin, `https:` or loopback; elsewhere the browser also
//!   asks for `gzip` and `deflate` alone.
//!
//! `Accept-Language` is what the build sends under its default locale,
//! `en-US`. Foxhound's test of a `standard` Session's HTTP tier compares
//! these headers with the pinned build's own navigation, so a pin whose
//! browser sends another request fails it.

use goodfetch_engine::browser_cache::{
    BrowserChoice, BrowserSource, BuildPlatform, PinnedArtifact, Platform, browser_source,
};
use goodfetch_protocol::browser_cache::ArtifactKind;
use std::collections::BTreeMap;
use url::{Host, Url};

/// What Chrome accepts for a document.
const ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,image/jxl,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7";

/// What Chrome sends under its default locale.
const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";

/// The characters Chromium draws the made-up brand's two separators from.
const BRAND_SEPARATORS: [&str; 11] = [" ", "(", ":", "-", ".", "/", ")", ";", "=", "?", "_"];

/// The versions Chromium draws the made-up brand's from.
const BRAND_VERSIONS: [&str; 3] = ["8", "99", "24"];

/// The document request of one browser build on one platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentRequest {
    /// The headers a potentially trustworthy origin receives.
    trustworthy: Vec<(String, String)>,
    /// The headers every other origin receives.
    elsewhere: Vec<(String, String)>,
}

impl DocumentRequest {
    /// The document request of the build a Session's browser launches on
    /// this platform, among `pins`: none where the platform has no pinned
    /// build and the browser is an installed one, whose version only a
    /// launch tells.
    pub fn of_pinned(pins: &[PinnedArtifact]) -> Option<Self> {
        let BrowserSource::Pinned { name, platform, .. } = browser_source(
            Platform::current(),
            &BrowserChoice::Default,
            &BTreeMap::new(),
        ) else {
            return None;
        };
        let pinned = pins.iter().find(|artifact| {
            artifact.id.kind == ArtifactKind::Browser
                && artifact.id.name == name
                && artifact.platform == Some(platform)
        })?;
        Self::of_build(&pinned.id.build, platform)
    }

    /// The document request of the Chrome for Testing build `build`, such
    /// as `141.0.7390.54-linux64`, on `platform`: none for a build ID that
    /// does not start with a major version.
    pub fn of_build(build: &str, platform: BuildPlatform) -> Option<Self> {
        let major: usize = build.split('.').next()?.parse().ok()?;
        let (system, name) = match platform {
            BuildPlatform::LinuxX64 | BuildPlatform::LinuxArm64 => ("X11; Linux x86_64", "Linux"),
            BuildPlatform::MacArm64 | BuildPlatform::MacX64 => {
                ("Macintosh; Intel Mac OS X 10_15_7", "macOS")
            }
            BuildPlatform::WindowsX64 => ("Windows NT 10.0; Win64; x64", "Windows"),
        };
        let user_agent = format!(
            "Mozilla/5.0 ({system}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
        );
        let brands = brands(major);
        let platform = format!("\"{name}\"");
        let asked = [
            ("upgrade-insecure-requests", "1"),
            ("user-agent", user_agent.as_str()),
            ("accept", ACCEPT),
        ];
        let hints = [
            ("sec-ch-ua", brands.as_str()),
            ("sec-ch-ua-mobile", "?0"),
            ("sec-ch-ua-platform", platform.as_str()),
        ];
        let metadata = [
            ("sec-fetch-site", "none"),
            ("sec-fetch-mode", "navigate"),
            ("sec-fetch-user", "?1"),
            ("sec-fetch-dest", "document"),
        ];
        let owned = |headers: &[&[(&str, &str)]]| {
            headers
                .concat()
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect()
        };
        Some(Self {
            trustworthy: owned(&[
                &hints,
                &asked,
                &metadata,
                &[
                    ("accept-encoding", "gzip, deflate, br, zstd"),
                    ("accept-language", ACCEPT_LANGUAGE),
                ],
            ]),
            elsewhere: owned(&[
                &asked,
                &[
                    ("accept-encoding", "gzip, deflate"),
                    ("accept-language", ACCEPT_LANGUAGE),
                ],
            ]),
        })
    }

    /// The headers of the request for the document at `url`, their names
    /// in lowercase, in the browser's order.
    #[must_use]
    pub fn headers(&self, url: &Url) -> &[(String, String)] {
        if potentially_trustworthy(url) {
            &self.trustworthy
        } else {
            &self.elsewhere
        }
    }
}

/// The `Sec-CH-UA` brands of Chromium `major`, as its
/// `GenerateBrandVersionList` orders and spells them for a build with no
/// brand of its own: the made-up brand first on an even major.
fn brands(major: usize) -> String {
    let made_up = format!(
        "\"Not{}A{}Brand\";v=\"{}\"",
        BRAND_SEPARATORS[major % BRAND_SEPARATORS.len()],
        BRAND_SEPARATORS[(major + 1) % BRAND_SEPARATORS.len()],
        BRAND_VERSIONS[major % BRAND_VERSIONS.len()],
    );
    let chromium = format!("\"Chromium\";v=\"{major}\"");
    if major.is_multiple_of(2) {
        format!("{made_up}, {chromium}")
    } else {
        format!("{chromium}, {made_up}")
    }
}

/// Whether `url`'s origin is potentially trustworthy (Secure Contexts):
/// `https:`, `localhost`, a name under it, or a loopback address.
fn potentially_trustworthy(url: &Url) -> bool {
    url.scheme() == "https"
        || match url.host() {
            Some(Host::Domain(name)) => {
                let name = name.trim_end_matches('.');
                name.eq_ignore_ascii_case("localhost")
                    || name.to_ascii_lowercase().ends_with(".localhost")
            }
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            None => false,
        }
}

#[cfg(test)]
mod tests;
