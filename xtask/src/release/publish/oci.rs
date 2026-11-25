//! The release plan's OCI steps: each Product's image tags and Helm chart,
//! and each first-party Plugin's artifact (ADRs 0033, 0049, 0094), read
//! through the OCI Distribution interface of the registry they go to.
//!
//! - **An image** gets ADR 0049's five tags, `<v>`, `<v>-slim`, `<v>-full`,
//!   `<v>-node` and `<v>-python`, then `latest` for a stable version no
//!   higher stable tag of the image passes; a nightly gets `<v>` and
//!   `nightly` only (ADR 0030).
//! - **A moving tag** (`latest`, `nightly`) is out once it points at the
//!   manifest `<v>` points at.
//! - **The Helm chart** (`goodfetch/charts/<product>`) and **each Plugin**
//!   (`goodfetch/goodfetch-plugin-*`, the workspace crates of that name) get
//!   `<v>`, for a stable version and a release candidate.
//! - A repository the registry does not have yet is planned as `publish`:
//!   the registry makes it on the first push, so no first publish by hand.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use ureq::http::Response;
use ureq::{Agent, Body};

use super::super::{ProductVersion, Stage, Tree, name, toml};
use super::{PackageState, Step};
use crate::products::Product;

/// GitHub's container registry, which holds every image, chart and Plugin
/// (ADR 0090).
pub(super) const REGISTRY: &str = "https://ghcr.io";

/// ADR 0049's tags of each image, after its version.
const VARIANTS: [&str; 5] = ["", "-slim", "-full", "-node", "-python"];

/// The first-party Plugins' crate names start with it (ADR 0033), and so do
/// their repositories' names.
const PLUGIN_PREFIX: &str = "goodfetch-plugin-";
/// The workspace folders that hold crates, Plugins' included.
const CRATE_FOLDERS: [&str; 2] = ["crates", "crates/foxhound"];

/// The manifest kinds a tag's digest is asked for: an image's
/// multi-platform index, a chart's or a Plugin's manifest.
const MANIFEST_TYPES: &str = "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json";

const USER_AGENT: &str = "goodfetch-xtask (https://github.com/goodfetch/goodfetch)";

/// Each OCI step of `version`, each Product's image and chart in turn, then
/// each first-party Plugin.
pub(super) fn steps(
    root: &Path,
    registry: &Registry<'_>,
    version: &ProductVersion,
) -> Result<Vec<Step>> {
    let semver = version.semver();
    let nightly = matches!(version.stage, Stage::Nightly(_));
    let mut steps = Vec::new();
    for product in Product::ALL {
        let image = repository(product.names().image)?;
        let tags = registry.tags(image)?.unwrap_or_default();
        let variants = if nightly {
            &VARIANTS[..1]
        } else {
            &VARIANTS[..]
        };
        for variant in variants {
            steps.push(step(image, &format!("{semver}{variant}"), &tags));
        }
        let moving = match version.stage {
            Stage::Nightly(_) => Some("nightly"),
            Stage::Stable if !higher_stable(&tags, version) => Some("latest"),
            Stage::Stable | Stage::Candidate(_) => None,
        };
        if let Some(moving) = moving {
            let out = tags.contains(moving)
                && tags.contains(&semver)
                && registry.digest(image, moving)? == registry.digest(image, &semver)?;
            steps.push(Step::Package {
                registry: "oci",
                name: image.to_owned(),
                version: moving.to_owned(),
                state: if out {
                    PackageState::Published
                } else {
                    PackageState::Publish
                },
            });
        }
        if !nightly {
            let chart = chart(image)?;
            let tags = registry.tags(&chart)?.unwrap_or_default();
            steps.push(step(&chart, &semver, &tags));
        }
    }
    if !nightly {
        for plugin in plugins(root)? {
            let repository = format!("goodfetch/{plugin}");
            let tags = registry.tags(&repository)?.unwrap_or_default();
            steps.push(step(&repository, &semver, &tags));
        }
    }
    Ok(steps)
}

fn step(repository: &str, tag: &str, tags: &BTreeSet<String>) -> Step {
    Step::Package {
        registry: "oci",
        name: repository.to_owned(),
        version: tag.to_owned(),
        state: if tags.contains(tag) {
            PackageState::Published
        } else {
            PackageState::Publish
        },
    }
}

/// An image's repository on its registry: `goodfetch/collie` of
/// `ghcr.io/goodfetch/collie`.
fn repository(image: &str) -> Result<&str> {
    image
        .split_once('/')
        .map(|(_, repository)| repository)
        .with_context(|| format!("the image {image} names no registry"))
}

/// The Helm chart's repository of an image's: `goodfetch/charts/collie`
/// (ADR 0094).
fn chart(image: &str) -> Result<String> {
    let (namespace, name) = image
        .split_once('/')
        .with_context(|| format!("the repository {image} has no namespace"))?;
    Ok(format!("{namespace}/charts/{name}"))
}

/// Whether a stable version above `version` is among an image's tags, so
/// `latest` stays where it is.
fn higher_stable(tags: &BTreeSet<String>, version: &ProductVersion) -> bool {
    let core = |version: &ProductVersion| (version.major, version.minor, version.patch);
    tags.iter()
        .filter_map(|tag| tag.parse::<ProductVersion>().ok())
        .any(|tag| tag.stage == Stage::Stable && core(&tag) > core(version))
}

/// The first-party Plugins' crate names, in name order.
fn plugins(root: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for folder in CRATE_FOLDERS {
        let Ok(entries) = std::fs::read_dir(root.join(folder)) else {
            continue;
        };
        for entry in entries {
            let entry = entry.with_context(|| format!("reading {folder}"))?;
            let file_name = entry.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            if !file_name.starts_with(PLUGIN_PREFIX) {
                continue;
            }
            let path = format!("{folder}/{file_name}/Cargo.toml");
            let manifest = toml(Tree::Workspace(root), &path)?;
            let package = manifest
                .get("package")
                .and_then(toml_edit::Item::as_table)
                .with_context(|| format!("{path} has no [package] table"))?;
            names.push(name(package, &path)?);
        }
    }
    names.sort();
    Ok(names)
}

/// An OCI registry's read interface: `https://ghcr.io`, or a stand-in's
/// address.
pub(super) struct Registry<'a> {
    url: &'a str,
    /// The GitHub token the pull token is asked with; anonymous without.
    token: Option<&'a str>,
    agent: Agent,
}

#[derive(Deserialize)]
struct TagList {
    /// `null` once a repository's every tag is deleted.
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct PullToken {
    token: Option<String>,
    access_token: Option<String>,
}

impl<'a> Registry<'a> {
    pub(super) fn new(url: &'a str, token: Option<&'a str>) -> Self {
        Self {
            url: url.trim_end_matches('/'),
            token,
            agent: Agent::config_builder()
                .http_status_as_error(false)
                .build()
                .into(),
        }
    }

    /// The tags of `repository`, every page of them; `None` when the
    /// registry does not have it.
    fn tags(&self, repository: &str) -> Result<Option<BTreeSet<String>>> {
        let mut url = format!("{}/v2/{repository}/tags/list", self.url);
        let mut tags = BTreeSet::new();
        loop {
            let Some(mut response) = self.read(&url, repository, false)? else {
                return Ok(None);
            };
            let next = response
                .headers()
                .get("link")
                .and_then(|link| link.to_str().ok())
                .and_then(next_page)
                .map(|next| {
                    if next.starts_with("http") {
                        next.to_owned()
                    } else {
                        format!("{}{next}", self.url)
                    }
                });
            let page: TagList = response
                .body_mut()
                .read_json()
                .with_context(|| format!("reading {url}"))?;
            tags.extend(page.tags.unwrap_or_default());
            match next {
                Some(next) => url = next,
                None => return Ok(Some(tags)),
            }
        }
    }

    /// The digest of the manifest `tag` points at; `None` when there is no
    /// such tag.
    fn digest(&self, repository: &str, tag: &str) -> Result<Option<String>> {
        let url = format!("{}/v2/{repository}/manifests/{tag}", self.url);
        let Some(response) = self.read(&url, repository, true)? else {
            return Ok(None);
        };
        response
            .headers()
            .get("docker-content-digest")
            .and_then(|digest| digest.to_str().ok())
            .map(|digest| Some(digest.to_owned()))
            .with_context(|| format!("{url} answered no Docker-Content-Digest"))
    }

    /// GETs, or HEADs, `url`, answering a `401`'s Bearer challenge once
    /// with a pull token of `repository`, as GHCR asks even of anonymous
    /// reads; `None` on a 404.
    fn read(&self, url: &str, repository: &str, head: bool) -> Result<Option<Response<Body>>> {
        let mut response = self.send(url, head, None)?;
        if response.status() == 401 {
            let challenge = response
                .headers()
                .get("www-authenticate")
                .and_then(|value| value.to_str().ok())
                .and_then(bearer_challenge)
                .with_context(|| format!("{url} answered 401 with no Bearer challenge"))?;
            let token = self.pull_token(&challenge, repository)?;
            response = self.send(url, head, Some(&token))?;
        }
        match response.status().as_u16() {
            200 => Ok(Some(response)),
            404 => Ok(None),
            status => bail!("{url} answered {status}"),
        }
    }

    fn send(&self, url: &str, head: bool, bearer: Option<&str>) -> Result<Response<Body>> {
        let request = if head {
            self.agent.head(url)
        } else {
            self.agent.get(url)
        };
        let request = request.header("User-Agent", USER_AGENT).header(
            "Accept",
            if head {
                MANIFEST_TYPES
            } else {
                "application/json"
            },
        );
        let request = match bearer {
            Some(token) => request.header("Authorization", format!("Bearer {token}")),
            None => request,
        };
        request.call().with_context(|| format!("reading {url}"))
    }

    /// A pull token of `repository` from the challenge's realm, asked with
    /// the GitHub token when there is one.
    fn pull_token(&self, challenge: &Challenge, repository: &str) -> Result<String> {
        let scope = challenge
            .scope
            .clone()
            .unwrap_or_else(|| format!("repository:{repository}:pull"));
        let request = self
            .agent
            .get(&challenge.realm)
            .query("scope", &scope)
            .header("User-Agent", USER_AGENT);
        let request = match &challenge.service {
            Some(service) => request.query("service", service),
            None => request,
        };
        let request = match self.token {
            Some(token) => request.header(
                "Authorization",
                format!(
                    "Basic {}",
                    data_encoding::BASE64.encode(format!("x-access-token:{token}").as_bytes())
                ),
            ),
            None => request,
        };
        let mut response = request
            .call()
            .with_context(|| format!("asking {} for a pull token", challenge.realm))?;
        let status = response.status();
        if status != 200 {
            bail!(
                "{} answered {status} to a pull token of {repository}",
                challenge.realm
            );
        }
        let answer: PullToken = response
            .body_mut()
            .read_json()
            .with_context(|| format!("reading {}'s pull token", challenge.realm))?;
        answer
            .token
            .or(answer.access_token)
            .with_context(|| format!("{} answered no token", challenge.realm))
    }
}

/// A `WWW-Authenticate: Bearer …` challenge's parameters.
#[derive(Debug, PartialEq, Eq)]
struct Challenge {
    realm: String,
    service: Option<String>,
    scope: Option<String>,
}

/// The parameters of a Bearer challenge, `Bearer realm="…",service="…",
/// scope="…"`; a quoted value may hold commas.
fn bearer_challenge(header: &str) -> Option<Challenge> {
    let (scheme, mut rest) = header.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let (mut realm, mut service, mut scope) = (None, None, None);
    loop {
        rest = rest.trim_start_matches([',', ' ']);
        let Some((key, after)) = rest.split_once('=') else {
            break;
        };
        let (value, after) = match after.strip_prefix('"') {
            Some(quoted) => {
                let (value, after) = quoted.split_once('"')?;
                (value, after)
            }
            None => after.split_once(',').unwrap_or((after, "")),
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "realm" => realm = Some(value.to_owned()),
            "service" => service = Some(value.to_owned()),
            "scope" => scope = Some(value.to_owned()),
            _ => {}
        }
        rest = after;
    }
    Some(Challenge {
        realm: realm?,
        service,
        scope,
    })
}

/// The target of a `Link` header's `rel="next"` entry, which a registry
/// sends while more tags are left.
fn next_page(header: &str) -> Option<&str> {
    header.split(',').find_map(|link| {
        let (target, parameters) = link.split_once(';')?;
        parameters
            .split(';')
            .any(|parameter| parameter.trim() == r#"rel="next""#)
            .then(|| target.trim().trim_start_matches('<').trim_end_matches('>'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bearer_challenge_gives_its_realm_service_and_scope() {
        assert_eq!(
            bearer_challenge(
                r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:goodfetch/collie:pull,push""#
            ),
            Some(Challenge {
                realm: "https://ghcr.io/token".to_owned(),
                service: Some("ghcr.io".to_owned()),
                scope: Some("repository:goodfetch/collie:pull,push".to_owned()),
            })
        );
        assert_eq!(bearer_challenge(r#"Basic realm="registry""#), None);
        assert_eq!(bearer_challenge(r#"Bearer service="ghcr.io""#), None);
    }

    #[test]
    fn the_next_page_is_the_link_whose_rel_is_next() {
        assert_eq!(
            next_page(r#"</v2/goodfetch/collie/tags/list?last=1.4.0&n=100>; rel="next""#),
            Some("/v2/goodfetch/collie/tags/list?last=1.4.0&n=100")
        );
        assert_eq!(
            next_page(
                r#"<https://a.example/first>; rel="prev", <https://a.example/next>; rel="next""#
            ),
            Some("https://a.example/next")
        );
        assert_eq!(next_page(r#"</v2/x/tags/list>; rel="prev""#), None);
    }
}
