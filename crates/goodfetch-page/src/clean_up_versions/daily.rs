//! The daily channel (ADR 0047): an OCI artifact in the registry Goodfetch
//! publishes to, one per release line and format version, tagged
//! `<line>-format-<format line>`, each version also tagged by its digest,
//! `sha256-<hex>`, so a resumed Run or a `clean_up.version` fetches the one
//! it names. Each layer is one file of the version, named by its
//! `org.opencontainers.image.title` annotation. It is pulled anonymously:
//! a registry that answers `401` names the realm of its token. The channel
//! is checked at most once a day per user, as a conditional manifest check
//! whose blobs are fetched only when the manifest changed, and a version
//! older than the last one seen is refused. Its Sigstore signature is not
//! checked yet: that waits for the Plugin store's verifier (ADR 0033).

use std::time::Duration;

use goodfetch_engine::pinned_download::Digest;
use goodfetch_engine::rule_sources::{self, Fetched};
use goodfetch_protocol::clean_up_rules::RulesSource;
use goodfetch_protocol::formats::CLEAN_UP_RULES;
use serde_json::{Value, json};

use super::{FILE_LIMIT, Sources, Version};

/// The registry the channel is published to.
pub(crate) const REGISTRY: &str = "ghcr.io";

/// The channel's repository in it.
pub(crate) const REPOSITORY: &str = "goodfetch/clean-up-rules";

/// How often the channel is checked, at most (ADR 0047).
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The media type of an OCI image manifest.
const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

/// The annotation that names a layer's file.
const TITLE: &str = "org.opencontainers.image.title";

/// The most bytes a manifest or a token answer may hold.
const SMALL_LIMIT: usize = 1024 * 1024;

/// The channel's tag for this engine: its release line, and the line of
/// the manifest format it reads, each `0.y` minor counting as a major
/// (ADR 0030).
pub(crate) fn tag(sources: &Sources) -> String {
    let format = CLEAN_UP_RULES.version;
    let format_line = if format.major == 0 {
        format!("0.{}", format.minor)
    } else {
        format.major.to_string()
    };
    let product = sources.product_version.major_minor();
    format!("{}.{}-format-{format_line}", product.major, product.minor)
}

/// The channel's newest version for this engine: the one last pulled while
/// the last check is under a day old; otherwise what the conditional check
/// finds, or the last one when the check fails.
pub(super) async fn newest(sources: &Sources, now: jiff::Timestamp) -> Result<Version, String> {
    let state = sources.read_state().await;
    let daily = &state["daily"];
    let last = match daily["version"].as_str() {
        Some(digest) => sources
            .kept
            .cached(digest, now)
            .await
            .map(|(version, _)| version),
        None => None,
    };
    let checked_at = daily["checked_at"]
        .as_str()
        .and_then(|text| text.parse::<jiff::Timestamp>().ok());
    if let (Some(last), Some(checked_at)) = (&last, checked_at)
        && checked_at
            .checked_add(CHECK_EVERY)
            .is_ok_and(|due| now < due)
    {
        return Ok(last.clone());
    }
    let known = last.as_ref().and(daily["manifest"].as_str());
    let pulled = match pull(sources, &tag(sources), known).await {
        Ok(pulled) => pulled,
        Err(why) => {
            return match last {
                Some(last) => {
                    tracing::warn!(%why, "the daily channel was not checked: the Run holds to its last version");
                    Ok(last)
                }
                None => Err(why),
            };
        }
    };
    let (manifest, version) = match pulled {
        Pulled::Unchanged(manifest) => (
            manifest,
            last.ok_or("the daily channel's unchanged version is gone")?,
        ),
        Pulled::Files { manifest, files } => (
            manifest,
            sources
                .kept
                .keep(RulesSource::Daily, &files, now)
                .await
                .map_err(|why| format!("the daily channel's newest version {why}"))?,
        ),
    };
    let seen = daily["published_at"]
        .as_str()
        .and_then(|text| text.parse::<jiff::Timestamp>().ok());
    if let Some(seen) = seen
        && version.manifest.published_at < seen
    {
        return Err(format!(
            "the daily channel's newest version, {}, of {}, is older than the last one seen, of {seen}",
            version.digest, version.manifest.published_at
        ));
    }
    let mut state = state.clone();
    state["daily"] = json!({
        "checked_at": now,
        "manifest": manifest,
        "version": version.digest,
        "published_at": version.manifest.published_at,
    });
    sources.write_state(state).await;
    Ok(version)
}

/// The channel's version of `digest`, pulled by its digest's tag.
pub(super) async fn by_digest(
    sources: &Sources,
    digest: &str,
    now: jiff::Timestamp,
) -> Result<Version, String> {
    let reference = digest.replacen(':', "-", 1);
    let Pulled::Files { files, .. } = pull(sources, &reference, None).await? else {
        return Err("is not on the daily channel".to_owned());
    };
    let version = sources
        .kept
        .keep(RulesSource::Daily, &files, now)
        .await
        .map_err(|why| format!("is on the daily channel, but {why}"))?;
    if version.digest != digest {
        return Err(format!(
            "is on the daily channel as files of another digest, {}",
            version.digest
        ));
    }
    Ok(version)
}

/// What a pull found.
enum Pulled {
    /// The manifest of this digest, which the last pull fetched already.
    Unchanged(String),
    /// A manifest of this digest, and the files its layers hold.
    Files {
        manifest: String,
        files: Vec<(String, Vec<u8>)>,
    },
}

/// Pulls the channel's `reference`, a tag: its manifest, then, unless its
/// digest is `known`, the blob of each layer, checked against its digest.
async fn pull(sources: &Sources, reference: &str, known: Option<&str>) -> Result<Pulled, String> {
    let mut token = None;
    let url = format!("https://{REGISTRY}/v2/{REPOSITORY}/manifests/{reference}");
    let fetched = get(
        sources,
        &url,
        Some(MANIFEST_MEDIA_TYPE),
        &mut token,
        SMALL_LIMIT,
    )
    .await?;
    match fetched.status {
        200 => {}
        404 => return Err(format!("is not on the daily channel as `{reference}`")),
        status => return Err(format!("the daily channel answered {status}")),
    }
    let manifest_digest = Digest::of(&fetched.body).to_string();
    if known == Some(manifest_digest.as_str()) {
        return Ok(Pulled::Unchanged(manifest_digest));
    }
    let manifest: Value = serde_json::from_slice(&fetched.body)
        .map_err(|error| format!("the daily channel's manifest does not read: {error}"))?;
    let mut files = Vec::new();
    for layer in manifest["layers"].as_array().into_iter().flatten() {
        let (Some(digest), Some(name)) = (
            layer["digest"].as_str(),
            layer["annotations"][TITLE].as_str(),
        ) else {
            return Err(
                "the daily channel's manifest has a layer without its digest or file name"
                    .to_owned(),
            );
        };
        let url = format!("https://{REGISTRY}/v2/{REPOSITORY}/blobs/{digest}");
        let blob = get(sources, &url, None, &mut token, FILE_LIMIT).await?;
        if blob.status != 200 {
            return Err(format!(
                "the daily channel answered {} for `{name}`",
                blob.status
            ));
        }
        if Digest::of(&blob.body).to_string() != digest {
            return Err(format!(
                "the daily channel's `{name}` is not the bytes its digest names"
            ));
        }
        files.push((name.to_owned(), blob.body));
    }
    Ok(Pulled::Files {
        manifest: manifest_digest,
        files,
    })
}

/// GETs `url` from the registry, with its anonymous token once a `401`
/// named the token's realm.
async fn get(
    sources: &Sources,
    url: &str,
    accept: Option<&str>,
    token: &mut Option<String>,
    limit: usize,
) -> Result<Fetched, String> {
    let send = |token: Option<&String>| {
        let mut headers = Vec::new();
        if let Some(accept) = accept {
            headers.push(("accept", accept.to_owned()));
        }
        if let Some(token) = token {
            headers.push(("authorization", format!("Bearer {token}")));
        }
        async move {
            rule_sources::get(url, &headers, &sources.fetching, limit)
                .await
                .map_err(|failure| format!("the daily channel cannot be reached: {failure}"))
        }
    };
    let fetched = send(token.as_ref()).await?;
    if fetched.status != 401 || token.is_some() {
        return Ok(fetched);
    }
    let challenge = fetched
        .authenticate
        .as_deref()
        .ok_or("the daily channel refused an anonymous pull")?;
    *token = Some(anonymous_token(sources, challenge).await?);
    send(token.as_ref()).await
}

/// The anonymous token the registry's `challenge`, a `WWW-Authenticate`
/// header's `Bearer realm=…,service=…,scope=…`, names.
async fn anonymous_token(sources: &Sources, challenge: &str) -> Result<String, String> {
    let parameters = challenge
        .strip_prefix("Bearer ")
        .ok_or("the daily channel asks for another kind of authentication")?;
    let mut realm = None;
    let mut query = Vec::new();
    for parameter in parameters.split(',') {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            continue;
        };
        let value = value.trim_matches('"');
        match name {
            "realm" => realm = Some(value),
            "service" | "scope" => query.push((name, value)),
            _ => {}
        }
    }
    let realm = realm.ok_or("the daily channel names no token realm")?;
    let url = url::Url::parse_with_params(realm, &query)
        .map_err(|_| "the daily channel names a token realm that is no URL")?;
    let answer = rule_sources::get(url.as_str(), &[], &sources.fetching, SMALL_LIMIT)
        .await
        .map_err(|failure| format!("the daily channel's token cannot be had: {failure}"))?;
    let token: Value = serde_json::from_slice(&answer.body)
        .map_err(|_| "the daily channel's token answer does not read")?;
    token["token"]
        .as_str()
        .or_else(|| token["access_token"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| "the daily channel gave no token".to_owned())
}
