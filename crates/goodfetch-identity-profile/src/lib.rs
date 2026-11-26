//! A `stealth` Session's Identity profile (ADR 0035): the user agent,
//! client hints, navigator, screen, graphics card and HTTP headers one real
//! browser presents together, drawn by seed from a population built from
//! fingerprint-suite's open data (Apache-2.0), and the exit's language, time
//! zone, geolocation and WebRTC address passed through as given. Pure: the
//! same seed, constraints and exit facts give the same profile, and nothing
//! here looks the exit up.
//!
//! Its own crate because the population data it embeds is heavy and
//! rarely changes, so isolating it keeps incremental builds of the page
//! crate fast (STYLE.md, reason 3).

mod archive;
mod candidates;
mod constraints;
mod generate;
mod population;
mod profile;
mod version_fields;

#[cfg(test)]
mod tests;

pub use constraints::{Constraints, NarrowingError, NarrowingKey};
pub use generate::generate;
pub use population::{Brand, Device, EngineFamily, Os};
pub use profile::{
    BrandVersion, BrowserVersion, ClientHints, ExitFacts, Geolocation, HttpProfile,
    IdentityProfile, IdentitySeed, Navigator, Screen, VideoCard,
};
pub use version_fields::refresh_version_fields;
