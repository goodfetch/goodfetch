//! The kinds of browser the population holds, and what a population user
//! agent says it is.

use std::fmt;

/// The engine a Session's browser runs on; `WebKit` has no stealth mode,
/// so no population.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EngineFamily {
    Chromium,
    Firefox,
}

impl fmt::Display for EngineFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Chromium => "Chromium",
            Self::Firefox => "Firefox",
        })
    }
}

/// An operating system of the population, by its `stealth.identity.os`
/// word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Os {
    Windows,
    Macos,
    Linux,
    Android,
}

/// A device of the population, by its `stealth.identity.device` word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Device {
    Desktop,
    Mobile,
}

/// A browser brand of the population, by its `stealth.identity.brand`
/// word: Chrome, Edge and Android Chrome on Chromium, Firefox on Firefox.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Brand {
    Chrome,
    Edge,
    AndroidChrome,
    Firefox,
}

impl Os {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Android => "android",
        }
    }
}

impl Device {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Mobile => "mobile",
        }
    }
}

impl Brand {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "edge",
            Self::AndroidChrome => "android_chrome",
            Self::Firefox => "firefox",
        }
    }

    pub fn family(self) -> EngineFamily {
        match self {
            Self::Chrome | Self::Edge | Self::AndroidChrome => EngineFamily::Chromium,
            Self::Firefox => EngineFamily::Firefox,
        }
    }
}

/// What a population user agent says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kind {
    pub(crate) os: Os,
    pub(crate) device: Device,
    pub(crate) brand: Brand,
    pub(crate) major: u32,
}

/// The kind `user_agent` says it is, when it is a Chrome, Edge or Firefox
/// user agent with a reduced version, on a desktop or a phone of the four
/// operating systems; tablets, iOS, `ChromeOS` and full-version user agents
/// are left out of the population.
pub(crate) fn kind(user_agent: &str) -> Option<Kind> {
    let inside = user_agent.strip_prefix("Mozilla/5.0 (")?;
    let (platform, mobile, brand, major) = chromium(inside).or_else(|| firefox(inside))?;
    let os = if platform.contains("Android") {
        Os::Android
    } else if platform.contains("Windows NT") {
        Os::Windows
    } else if platform.contains("Macintosh") {
        Os::Macos
    } else if platform.contains("Linux") && !platform.contains("CrOS") {
        Os::Linux
    } else {
        return None;
    };
    let device = match (os, mobile) {
        (Os::Android, true) => Device::Mobile,
        (Os::Android, false) | (_, true) => return None,
        (_, false) => Device::Desktop,
    };
    let brand = match brand {
        Brand::Chrome if os == Os::Android => Brand::AndroidChrome,
        brand => brand,
    };
    Some(Kind {
        os,
        device,
        brand,
        major,
    })
}

/// A Chrome or Edge user agent's platform, whether it says `Mobile`, its
/// brand and its major version.
fn chromium(inside: &str) -> Option<(&str, bool, Brand, u32)> {
    let (platform, rest) = inside.split_once(") AppleWebKit/537.36 (KHTML, like Gecko) Chrome/")?;
    let (major, rest) = rest.split_once(".0.0.0 ")?;
    let (mobile, rest) = match rest.strip_prefix("Mobile ") {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    let brand = match rest.strip_prefix("Safari/537.36")? {
        "" => Brand::Chrome,
        edge if edge.starts_with(" Edg/") || edge.starts_with(" EdgA/") => Brand::Edge,
        _ => return None,
    };
    Some((platform, mobile, brand, major.parse().ok()?))
}

/// A Firefox user agent's platform, whether it says `Mobile` and its major
/// version, which its `rv:` repeats.
fn firefox(inside: &str) -> Option<(&str, bool, Brand, u32)> {
    let (inside, rest) = inside.split_once(") Gecko/")?;
    let (platform, revision) = inside.rsplit_once("; rv:")?;
    let (_, version) = rest.split_once(" Firefox/")?;
    let major = version.strip_suffix(".0")?;
    if revision.strip_suffix(".0")? != major {
        return None;
    }
    let mobile = platform.split("; ").any(|part| part == "Mobile");
    Some((platform, mobile, Brand::Firefox, major.parse().ok()?))
}
