//! The HTTP profile of an HTTP-only `stealth` Session (spec #579; ADRs
//! 0020 and 0035): the TLS ClientHello and HTTP/2 settings of the browser
//! its Identity profile claims, which the `wreq` transport sends so a
//! crawl over HTTP presents the same client as its browser would.
//!
//! Area 8 picks the fingerprints from captures of the Goodfetch Chromium
//! (#1175, M3). Until then the profile is one of `wreq-util`'s built-in
//! emulation profiles: Chrome 149's, the newest it has and the closest to
//! the Chrome for Testing build Collie pins (156). Its TLS and HTTP/2
//! settings alone: the request's headers stay the fetch tier's, since the
//! header order and client hints are area 8's too.

/// The TLS and HTTP/2 fingerprint an HTTP-only `stealth` Session presents:
/// its Identity profile's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpProfile {
    profile: wreq_util::Profile,
}

impl Default for HttpProfile {
    /// The pinned Chromium's own identity, until area 8's captured
    /// profiles: Chrome 149's built-in profile.
    fn default() -> Self {
        Self {
            profile: wreq_util::Profile::Chrome149,
        }
    }
}

impl HttpProfile {
    /// What `wreq` sends under this profile: its TLS, HTTP/1.1 and HTTP/2
    /// options, without the profile's headers.
    pub(crate) fn emulation(self) -> wreq::Emulation {
        wreq::IntoEmulation::into_emulation(
            wreq_util::Emulation::builder()
                .profile(self.profile)
                .headers(false)
                .build(),
        )
    }
}
