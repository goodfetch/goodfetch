//! `compat/releases.toml`, the record of what each Product version shipped
//! (ADR 0066): one entry per Product version, which both Products share
//! (ADRs 0090 and 0094), with each Product's executables per target, its npm
//! and Python packages, its crates, its Helm chart and OCI image, the Collie
//! client packages, the Plugin artifacts, each with its SHA-256 digest and
//! attestation bundle, and the version of every registered format it shipped.
//!
//! The release's pull request to `main` appends the entry. `xtask compat
//! check` reads the format versions to know which frozen sets must exist, the
//! CI's pinned artifact mirror takes every `url`, and the Release suite's skew
//! and upgrade jobs fetch the artifacts by digest. Its JSON Schema is the
//! bundle's `files."compat/releases.toml"`.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// The whole file: every release, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleasesFile {
    /// One entry per stable release or patch; release candidates and
    /// nightlies have none.
    #[serde(default)]
    pub release: Vec<Release>,
}

/// What one Product version shipped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Release {
    /// The Product version both Products carry (ADR 0090).
    pub version: ProductVersion,
    /// The version of every registered format the release shipped, read from
    /// its tag's format registry.
    pub formats: BTreeMap<FormatName, FormatVersion>,
    /// Collie's files.
    pub collie: ProductArtifacts,
    /// Foxhound's files.
    pub foxhound: ProductArtifacts,
    /// The Collie client packages both Products' SDKs build on (ADR 0095).
    pub collie_client: ClientPackages,
    /// The first-party Plugins, one OCI artifact per target (ADR 0094).
    #[serde(default)]
    pub plugins: Vec<TargetArtifact>,
}

/// One Product's files at one release (ADR 0094).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductArtifacts {
    /// Its executable's archive for each target.
    #[serde(default)]
    pub executables: Vec<TargetArtifact>,
    /// Its npm package and its platform packages.
    #[serde(default)]
    pub npm: Vec<Artifact>,
    /// Its Python wheels on the Python Package Index.
    #[serde(default)]
    pub pypi: Vec<Artifact>,
    /// Its crates on crates.io.
    #[serde(default)]
    pub crates_io: Vec<Artifact>,
    /// Its Helm chart, an OCI artifact.
    pub helm_chart: Artifact,
    /// Its OCI image.
    pub image: Artifact,
}

/// The Collie client packages, one per language (ADR 0095).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClientPackages {
    /// `@goodfetch/collie-client`.
    pub npm: Artifact,
    /// `goodfetch-collie-client`'s wheel.
    pub pypi: Artifact,
    /// The `goodfetch-collie-client` crate.
    pub crates_io: Artifact,
}

/// A published file or package.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    /// Its name as published: a package or crate name, a file name, or an
    /// OCI reference without its tag.
    pub name: String,
    /// Where it is downloaded from.
    pub url: HttpsUrl,
    /// The SHA-256 digest of what `url` serves.
    pub sha256: Sha256,
    /// Its Sigstore attestation bundle.
    pub attestation: Attestation,
}

/// A published file built for one target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetArtifact {
    /// The Rust target triple it was built for.
    pub target: String,
    /// Its name as published.
    pub name: String,
    /// Where it is downloaded from.
    pub url: HttpsUrl,
    /// The SHA-256 digest of what `url` serves.
    pub sha256: Sha256,
    /// Its Sigstore attestation bundle.
    pub attestation: Attestation,
}

/// A Sigstore attestation bundle, checked offline with `gh attestation
/// verify --bundle` (ADR 0066).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Attestation {
    /// Where the bundle is downloaded from.
    pub url: HttpsUrl,
    /// The SHA-256 digest of the bundle.
    pub sha256: Sha256,
}

/// Why a value of `compat/releases.toml` was refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{value}` is not {what}")]
#[non_exhaustive]
pub struct Malformed {
    what: &'static str,
    value: String,
}

fn malformed(what: &'static str, value: &str) -> Malformed {
    Malformed {
        what,
        value: value.to_owned(),
    }
}

/// A Product version of a stable release or patch, `X.Y.Z` (ADR 0030).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProductVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl ProductVersion {
    /// The version `major.minor.patch`.
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl TryFrom<String> for ProductVersion {
    type Error = Malformed;

    fn try_from(text: String) -> Result<Self, Malformed> {
        let parts = numbers::<3>(&text).ok_or_else(|| malformed("a version `X.Y.Z`", &text))?;
        Ok(Self::new(parts[0], parts[1], parts[2]))
    }
}

impl From<ProductVersion> for String {
    fn from(version: ProductVersion) -> Self {
        version.to_string()
    }
}

impl fmt::Display for ProductVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl JsonSchema for ProductVersion {
    fn schema_name() -> Cow<'static, str> {
        "ProductVersion".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A Product version of a stable release or patch, `X.Y.Z` (ADR 0030).",
            "type": "string",
            "pattern": r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$",
        })
    }
}

/// A registered format's version, `major.minor` (ADRs 0030 and 0066).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FormatVersion {
    major: u32,
    minor: u32,
}

impl FormatVersion {
    /// The version `major.minor`.
    pub fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }
}

impl TryFrom<String> for FormatVersion {
    type Error = Malformed;

    fn try_from(text: String) -> Result<Self, Malformed> {
        Self::try_from(text.as_str())
    }
}

impl TryFrom<&str> for FormatVersion {
    type Error = Malformed;

    fn try_from(text: &str) -> Result<Self, Malformed> {
        let parts =
            numbers::<2>(text).ok_or_else(|| malformed("a format version `major.minor`", text))?;
        Ok(Self::new(parts[0], parts[1]))
    }
}

impl From<FormatVersion> for String {
    fn from(version: FormatVersion) -> Self {
        version.to_string()
    }
}

impl fmt::Display for FormatVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl JsonSchema for FormatVersion {
    fn schema_name() -> Cow<'static, str> {
        "FormatVersion".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A registered format's version, `major.minor` (ADRs 0030 and 0066).",
            "type": "string",
            "pattern": r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$",
        })
    }
}

/// `count` decimal numbers joined by dots, without leading zeros or signs.
fn numbers<const COUNT: usize>(text: &str) -> Option<[u32; COUNT]> {
    let mut parts = [0; COUNT];
    let mut pieces = text.split('.');
    for part in &mut parts {
        let piece = pieces.next()?;
        let canonical = piece == "0"
            || (!piece.is_empty()
                && !piece.starts_with('0')
                && piece.bytes().all(|byte| byte.is_ascii_digit()));
        if !canonical {
            return None;
        }
        *part = piece.parse().ok()?;
    }
    pieces.next().is_none().then_some(parts)
}

/// A registered format's name: lowercase words joined by hyphens, as its
/// `compat/<format>/` folder is named (ADR 0066).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FormatName(String);

impl FormatName {
    /// The name as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FormatName {
    type Error = Malformed;

    fn try_from(text: String) -> Result<Self, Malformed> {
        let valid = !text.is_empty()
            && text.split('-').all(|word| {
                !word.is_empty()
                    && word
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            });
        if valid {
            Ok(Self(text))
        } else {
            Err(malformed(
                "a format name of lowercase words joined by hyphens",
                &text,
            ))
        }
    }
}

impl From<FormatName> for String {
    fn from(name: FormatName) -> Self {
        name.0
    }
}

impl fmt::Display for FormatName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl JsonSchema for FormatName {
    fn schema_name() -> Cow<'static, str> {
        "FormatName".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A registered format's name: lowercase words joined by hyphens (ADR 0066).",
            "type": "string",
            "pattern": "^[a-z0-9]+(-[a-z0-9]+)*$",
        })
    }
}

/// A lowercase hexadecimal SHA-256 digest.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256(String);

impl Sha256 {
    /// The digest's 64 hexadecimal digits.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Sha256 {
    type Error = Malformed;

    fn try_from(text: String) -> Result<Self, Malformed> {
        let valid = text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if valid {
            Ok(Self(text))
        } else {
            Err(malformed(
                "a SHA-256 digest of 64 lowercase hexadecimal digits",
                &text,
            ))
        }
    }
}

impl From<Sha256> for String {
    fn from(digest: Sha256) -> Self {
        digest.0
    }
}

impl JsonSchema for Sha256 {
    fn schema_name() -> Cow<'static, str> {
        "Sha256".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A SHA-256 digest of 64 lowercase hexadecimal digits.",
            "type": "string",
            "pattern": "^[0-9a-f]{64}$",
        })
    }
}

/// An `https://` URL with a host.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HttpsUrl(String);

impl HttpsUrl {
    /// The URL as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for HttpsUrl {
    type Error = Malformed;

    fn try_from(text: String) -> Result<Self, Malformed> {
        let host = text
            .strip_prefix("https://")
            .and_then(|rest| rest.split(['/', '?', '#']).next())
            .unwrap_or_default();
        if host.is_empty() || text.contains(char::is_whitespace) {
            Err(malformed("an https:// URL with a host", &text))
        } else {
            Ok(Self(text))
        }
    }
}

impl From<HttpsUrl> for String {
    fn from(url: HttpsUrl) -> Self {
        url.0
    }
}

impl JsonSchema for HttpsUrl {
    fn schema_name() -> Cow<'static, str> {
        "HttpsUrl".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "An https:// URL with a host.",
            "type": "string",
            "pattern": r"^https://[^/?#\s]+[^\s]*$",
        })
    }
}

#[cfg(test)]
mod tests;
