//! An Identity profile and what it is drawn with: the seed, the exit's
//! facts and, for a refresh, the browser's version.

use std::net::IpAddr;

use crate::population::{Brand, Device, EngineFamily, Os};

/// The seed an Identity profile is drawn with, from the `Entropy` handle
/// (ADR 0059), kept by its Session or Saved identity and never re-drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IdentitySeed(pub u64);

/// A browser's version: its major and its full dotted version, such as
/// `156` and `156.0.8078.80`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserVersion {
    major: u32,
    full: String,
}

impl BrowserVersion {
    /// `dotted`'s version, when it is one to four dot-separated numbers.
    pub fn parse(dotted: &str) -> Option<Self> {
        let parts: Vec<&str> = dotted.split('.').collect();
        let numeric = |part: &&str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        if parts.len() > 4 || !parts.iter().all(numeric) {
            return None;
        }
        Some(Self {
            major: parts.first()?.parse().ok()?,
            full: dotted.to_owned(),
        })
    }

    pub fn major(&self) -> u32 {
        self.major
    }

    pub fn full(&self) -> &str {
        &self.full
    }
}

/// What the proxy exit says of where the traffic comes from, passed
/// through to the profile as given: nothing here looks the exit up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExitFacts {
    /// The exit's country, as an ISO 3166-1 alpha-2 code.
    pub country: Option<String>,
    /// The exit's language, as a BCP 47 tag such as `fr-FR`; `en-US` when
    /// unknown.
    pub language: Option<String>,
    /// The exit's IANA time zone, such as `Europe/Paris`.
    pub time_zone: Option<String>,
    pub geolocation: Option<Geolocation>,
    /// The address WebRTC shows: the exit's, never the machine's.
    pub webrtc_address: Option<IpAddr>,
}

/// Where the exit is, in degrees, and how sure, in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geolocation {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
}

/// One browser's identity: values a real browser of the population
/// presents together, with the exit's facts. Only [`crate::generate`]
/// makes one.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct IdentityProfile {
    pub family: EngineFamily,
    pub os: Os,
    pub device: Device,
    pub brand: Brand,
    pub version: BrowserVersion,
    pub user_agent: String,
    /// `navigator.userAgentData` and the `Sec-CH-UA` hints; Firefox has
    /// none.
    pub client_hints: Option<ClientHints>,
    pub navigator: Navigator,
    pub screen: Screen,
    pub video_card: VideoCard,
    pub http: HttpProfile,
    pub exit: ExitFacts,
}

/// A Chromium browser's client hints.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientHints {
    /// The brands and their major versions, with the GREASE brand, in the
    /// order Chromium writes them for that major.
    pub brands: Vec<BrandVersion>,
    /// The same brands with their full versions.
    pub full_version_list: Vec<BrandVersion>,
    pub mobile: bool,
    /// `Windows`, `macOS`, `Linux` or `Android`.
    pub platform: String,
    pub platform_version: String,
    pub architecture: String,
    pub bitness: String,
    pub model: String,
    pub ua_full_version: String,
}

/// One brand of a client hints list and its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrandVersion {
    pub brand: String,
    pub version: String,
}

/// What `navigator` shows.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Navigator {
    pub platform: String,
    /// `Google Inc.` on Chromium, empty on Firefox.
    pub vendor: String,
    pub product_sub: String,
    pub app_version: String,
    /// Firefox's alone.
    pub oscpu: Option<String>,
    pub hardware_concurrency: u32,
    /// In gigabytes; Chromium's alone.
    pub device_memory: Option<f64>,
    pub max_touch_points: u32,
    /// The exit's language, then its primary subtag.
    pub languages: Vec<String>,
}

/// What `screen` and `devicePixelRatio` show, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub avail_width: u32,
    pub avail_height: u32,
    pub color_depth: u32,
    pub pixel_depth: u32,
    pub device_pixel_ratio: f64,
}

/// The WebGL graphics card the browser reports, unmasked.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct VideoCard {
    pub vendor: String,
    pub renderer: String,
}

/// The headers the browser sends a navigation over HTTP/2 that come from
/// its identity, lower-case, in the order it sends them.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HttpProfile {
    pub headers: Vec<(String, String)>,
}
