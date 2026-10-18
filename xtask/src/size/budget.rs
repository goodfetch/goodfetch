//! Each Product's Size budget as constants: the ceilings and the increase
//! thresholds of ADR 0057 (Foxhound's) and ADR 0087 (Collie's). Only the
//! owner moves a ceiling, by changing the ADR that sets it.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A size in bytes. The ADRs' "MB" is 1,000,000 bytes and "MiB" 1,048,576.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct Bytes(pub(crate) u64);

const MB: u64 = 1_000_000;
const MIB: u64 = 1_048_576;

/// ADRs 0057 and 0087: +1 MB per compressed release file, for both Products.
const COMPRESSED_INCREASE: Bytes = Bytes(MB);
/// ADRs 0057 and 0087: +3 MB compressed per OCI image tag, for both Products.
const OCI_IMAGE_INCREASE: Bytes = Bytes(3 * MB);
/// ADRs 0057 and 0087: 8 MiB per published crate's tarball, 2 MiB under
/// crates.io's limit.
const CRATE_CEILING: Bytes = Bytes(8 * MIB);
/// ADR 0057: Foxhound's compressed release file per platform.
const FOXHOUND_COMPRESSED_CEILING: Bytes = Bytes(60 * MB);
/// ADR 0057: Foxhound's raw executable per platform.
const FOXHOUND_EXECUTABLE_CEILING: Bytes = Bytes(160 * MB);
/// ADR 0087: Collie's compressed release file per platform.
const COLLIE_COMPRESSED_CEILING: Bytes = Bytes(50 * MB);
/// ADR 0087: Collie's raw executable per platform.
const COLLIE_EXECUTABLE_CEILING: Bytes = Bytes(120 * MB);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Product {
    Collie,
    Foxhound,
}

/// What a built file is, which decides the rule it is held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Kind {
    /// A compressed release file: an npm platform package.
    NpmPackage,
    /// A compressed release file: a wheel.
    Wheel,
    /// A compressed release file: a GitHub Release archive.
    ReleaseArchive,
    /// The raw executable.
    Executable,
    /// A published crate's `.crate` tarball, which has no target.
    Crate,
    /// An OCI image tag, measured as the compressed sizes its manifest lists.
    OciImage,
}

/// What a file is held to in one mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Limit {
    /// Growth over `main`'s figure that trips the increase check.
    Increase(Bytes),
    /// The largest size the file may reach.
    Ceiling(Bytes),
    /// Measured and reported only.
    None,
}

/// What each file is held to in the increase check: a compressed release
/// file and an OCI image tag to their threshold over `main`, a crate tarball
/// to its ceiling, which has no threshold (ADR 0057).
pub(crate) fn increase(kind: Kind) -> Limit {
    match kind {
        Kind::NpmPackage | Kind::Wheel | Kind::ReleaseArchive => {
            Limit::Increase(COMPRESSED_INCREASE)
        }
        Kind::OciImage => Limit::Increase(OCI_IMAGE_INCREASE),
        Kind::Crate => Limit::Ceiling(CRATE_CEILING),
        Kind::Executable => Limit::None,
    }
}

/// The ceiling of each file on its release-profile build.
pub(crate) fn ceiling(product: Product, kind: Kind) -> Limit {
    match (kind, product) {
        (Kind::NpmPackage | Kind::Wheel | Kind::ReleaseArchive, Product::Collie) => {
            Limit::Ceiling(COLLIE_COMPRESSED_CEILING)
        }
        (Kind::NpmPackage | Kind::Wheel | Kind::ReleaseArchive, Product::Foxhound) => {
            Limit::Ceiling(FOXHOUND_COMPRESSED_CEILING)
        }
        (Kind::Executable, Product::Collie) => Limit::Ceiling(COLLIE_EXECUTABLE_CEILING),
        (Kind::Executable, Product::Foxhound) => Limit::Ceiling(FOXHOUND_EXECUTABLE_CEILING),
        (Kind::Crate, _) => Limit::Ceiling(CRATE_CEILING),
        (Kind::OciImage, _) => Limit::None,
    }
}

impl fmt::Display for Product {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Collie => "Collie",
            Self::Foxhound => "Foxhound",
        })
    }
}

impl Product {
    /// The Product's name in a file's identifier, as the manifest spells it.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Collie => "collie",
            Self::Foxhound => "foxhound",
        }
    }
}

impl Bytes {
    /// In MB with two decimals, rounded to the nearest hundredth.
    pub(crate) fn megabytes(self) -> String {
        let hundredths = (self.0 + MB / 200) / (MB / 100);
        format!("{}.{:02} MB", hundredths / 100, hundredths % 100)
    }
}

impl fmt::Display for Limit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Increase(bytes) => write!(formatter, "+{}", whole(*bytes)),
            Self::Ceiling(bytes) => formatter.write_str(&whole(*bytes)),
            Self::None => formatter.write_str("—"),
        }
    }
}

/// A limit as the ADRs write it: whole MiB or MB.
fn whole(bytes: Bytes) -> String {
    if bytes.0.is_multiple_of(MIB) {
        format!("{} MiB", bytes.0 / MIB)
    } else if bytes.0.is_multiple_of(MB) {
        format!("{} MB", bytes.0 / MB)
    } else {
        bytes.megabytes()
    }
}
