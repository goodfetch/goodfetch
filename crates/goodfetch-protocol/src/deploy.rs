//! Deployment template contracts and canonical pure renderer (spec #583,
//! issue 1262; ADRs 0030, 0049, 0069, 0083, 0090, 0099).
//!
//! One pure [`render`] function serves every reader: `init --deploy`, the
//! docs generator and the deployment-target smoke runs. Each target is
//! spelled on the CLI by ADR 0099's rule, and [`InitDeployResult`] is the
//! `--json` output of `init --deploy`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::Product;
use crate::connection::ProductVersion;
use crate::daemon;
use crate::starters::StarterLanguage;

/// Default port for the Daemon's ops listener (ADR 0050).
pub const DEFAULT_OPS_PORT: u16 = 9464;

/// A Supported deployment target accepted by `init --deploy <target>`
/// (ADR 0049; spelled on the CLI by ADR 0099's rule).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DeployTarget {
    /// Docker Compose (`compose.yaml`, `operator-policy.toml`, `seccomp-profile.json`).
    Compose,
}

impl DeployTarget {
    /// Every supported deployment target, in canonical order.
    pub const ALL: [Self; 1] = [Self::Compose];

    /// Its canonical CLI spelling (ADR 0099).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Compose => "compose",
        }
    }

    /// Parses a target from its CLI name.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|target| target.as_str() == name)
    }

    /// Returns the closest known target to `unknown`, by edit distance, so
    /// a usage error always names a real suggestion (ADR 0077 G3).
    #[must_use]
    pub fn closest(unknown: &str) -> Self {
        let lower = unknown.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .min_by_key(|target| edit_distance(&lower, target.as_str()))
            .unwrap_or(Self::Compose)
    }

    /// Whether this target requires a starter project language (`lambda`,
    /// `cloud_run_job` do; `compose` does not).
    #[must_use]
    pub const fn needs_project(self) -> bool {
        match self {
            Self::Compose => false,
        }
    }
}

/// What `init --deploy <target> --json` prints (ADR 0069): the rendered
/// target and each file path written or left untouched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InitDeployResult {
    /// The deployment target rendered, such as `compose`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: DeployTarget,
    /// The files it wrote, `/`-separated relative to the output folder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub written: Vec<String>,
    /// The files it left untouched because a file, folder or link already
    /// existed at that path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub skipped: Vec<String>,
}

/// One raw template file belonging to a deployment target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeployTemplateFile<'a> {
    /// The `/`-separated output path relative to the project folder.
    pub path: &'a str,
    /// The template's UTF-8 or raw bytes.
    pub contents: &'a [u8],
}

/// One rendered deployment file ready to write or display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedDeployFile {
    /// The `/`-separated output path relative to the project folder.
    pub path: String,
    /// The rendered file bytes.
    pub contents: Vec<u8>,
}

/// Pinned OCI image coordinates for a Product's deployment templates
/// (ADRs 0030, 0049, 0090).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployImagePin {
    /// The OCI image repository, such as `ghcr.io/goodfetch/collie`.
    pub repository: String,
    /// The `sha256:<64 hex>` digest for a stable release tag.
    pub default_digest: String,
    /// The `sha256:<64 hex>` digest for the `nightly` tag.
    pub nightly_digest: String,
}

/// Why rendering a deployment template set failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DeployRenderError {
    /// No template files were supplied for the requested target.
    #[error("no deployment templates are registered for target `{}`", .0.as_str())]
    EmptyTarget(DeployTarget),
    /// A pinned image digest is not a valid `sha256:<64 lowercase hex>` digest.
    #[error("image digest `{0}` must be `sha256:` followed by 64 lowercase hexadecimal digits")]
    InvalidDigest(String),
    /// A template file is not valid UTF-8 when placeholder substitution runs.
    #[error("deployment template `{0}` is not valid UTF-8")]
    InvalidUtf8(String),
}

/// Renders `templates` for `target`, `language` and `product_version` of
/// `product`, pinning the container image by digest (stable `<version>@sha256:…`
/// or `nightly@sha256:…`, never `latest`).
///
/// # Errors
///
/// Returns [`DeployRenderError::EmptyTarget`] if `templates` is empty,
/// [`DeployRenderError::InvalidDigest`] if the pinned digest is malformed, or
/// [`DeployRenderError::InvalidUtf8`] if a template file is not valid UTF-8.
pub fn render(
    templates: &[DeployTemplateFile<'_>],
    target: DeployTarget,
    _language: Option<StarterLanguage>,
    product: Product,
    product_version: &ProductVersion,
    pin: &DeployImagePin,
) -> Result<Vec<RenderedDeployFile>, DeployRenderError> {
    if templates.is_empty() {
        return Err(DeployRenderError::EmptyTarget(target));
    }

    let (tag, digest) = if is_nightly(product_version) {
        ("nightly", pin.nightly_digest.as_str())
    } else {
        (product_version.as_str(), pin.default_digest.as_str())
    };
    validate_digest(digest)?;

    let image = format!("{}:{tag}@{digest}", pin.repository);
    let executable = product.executable_name();
    let product_name = match product {
        Product::Collie => "Goodfetch Collie",
        Product::Foxhound => "Goodfetch Foxhound",
    };
    let daemon_port = daemon::default_address(product_version).port().to_string();
    let ops_port = DEFAULT_OPS_PORT.to_string();
    let major_folder = product_version.line().to_string();

    let mut rendered = Vec::with_capacity(templates.len());
    for file in templates {
        let text = std::str::from_utf8(file.contents)
            .map_err(|_| DeployRenderError::InvalidUtf8(file.path.to_owned()))?;
        let substituted = text
            .replace("{{image}}", &image)
            .replace("{{executable}}", executable)
            .replace("{{product_name}}", product_name)
            .replace("{{product_version}}", product_version.as_str())
            .replace("{{daemon_port}}", &daemon_port)
            .replace("{{ops_port}}", &ops_port)
            .replace("{{major_folder}}", &major_folder);
        rendered.push(RenderedDeployFile {
            path: file.path.to_owned(),
            contents: substituted.into_bytes(),
        });
    }
    Ok(rendered)
}

fn is_nightly(version: &ProductVersion) -> bool {
    version.as_str().split_once('-').is_some_and(|(_, pre)| {
        pre.split('.')
            .any(|part| part.eq_ignore_ascii_case("nightly"))
    })
}

fn validate_digest(digest: &str) -> Result<(), DeployRenderError> {
    let valid = digest.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    });
    if !valid {
        return Err(DeployRenderError::InvalidDigest(digest.to_owned()));
    }
    Ok(())
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right_chars: Vec<char> = right.chars().collect();
    let mut prev: Vec<usize> = (0..=right_chars.len()).collect();
    let mut curr = vec![0; right_chars.len() + 1];
    for (i, lc) in left.chars().enumerate() {
        curr[0] = i + 1;
        for (j, &rc) in right_chars.iter().enumerate() {
            let cost = usize::from(lc != rc);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[right_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin() -> DeployImagePin {
        DeployImagePin {
            repository: "ghcr.io/goodfetch/collie".to_owned(),
            default_digest: format!("sha256:{}", "a".repeat(64)),
            nightly_digest: format!("sha256:{}", "b".repeat(64)),
        }
    }

    // Covers: story:13-11
    #[test]
    fn stable_and_nightly_versions_pin_by_digest_and_never_latest() {
        let templates = [DeployTemplateFile {
            path: "compose.yaml",
            contents: b"image: {{image}}\nexe: {{executable}}\nver: {{product_version}}\n",
        }];
        let stable = ProductVersion::parse("1.2.3").unwrap();
        let out = render(
            &templates,
            DeployTarget::Compose,
            None,
            Product::Collie,
            &stable,
            &pin(),
        )
        .unwrap();
        let text = std::str::from_utf8(&out[0].contents).unwrap();
        assert!(text.contains(&format!(
            "ghcr.io/goodfetch/collie:1.2.3@sha256:{}",
            "a".repeat(64)
        )));
        assert!(text.contains("exe: goodfetch-collie"));
        assert!(!text.contains(":latest"));

        let nightly = ProductVersion::parse("1.3.0-nightly.20261127").unwrap();
        let out_nightly = render(
            &templates,
            DeployTarget::Compose,
            None,
            Product::Collie,
            &nightly,
            &pin(),
        )
        .unwrap();
        let nightly_text = std::str::from_utf8(&out_nightly[0].contents).unwrap();
        assert!(nightly_text.contains(&format!(
            "ghcr.io/goodfetch/collie:nightly@sha256:{}",
            "b".repeat(64)
        )));
        assert!(!nightly_text.contains(":latest"));
    }

    // Covers: story:13-9
    #[test]
    fn closest_target_suggests_compose_for_unknown_target() {
        assert_eq!(DeployTarget::named("compose-v1"), None);
        assert_eq!(DeployTarget::closest("compose-v1"), DeployTarget::Compose);
    }
}
