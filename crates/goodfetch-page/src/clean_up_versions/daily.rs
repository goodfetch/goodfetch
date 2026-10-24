//! The daily channel (ADR 0047): an OCI artifact in the registry Goodfetch
//! publishes to, or a mirror `clean_up.channel` names, each version tagged
//! by its digest, `sha256-<hex>`, so a resumed Run or a `clean_up.version`
//! fetches the one it names. Each layer is one file of the version, named
//! by its `org.opencontainers.image.title` annotation, and checked against
//! its digest; the version's files are then checked against the digest
//! that named it. It is pulled anonymously: a registry that answers `401`
//! names the realm of its token.
//!
//! Only a version pinned by its digest is pulled: `clean_up.update = daily`,
//! which would take the channel's newest version, is refused until its
//! Sigstore signature is checked, which waits for the Plugin store's
//! verifier (ADR 0033).

use goodfetch_engine::browser_cache::HeldArtifact;
use goodfetch_engine::pinned_download::Digest;
use goodfetch_engine::rule_sources::{self, Fetched};
use goodfetch_protocol::clean_up_rules::RulesSource;
use serde_json::Value;

use super::failures::Unobtained;
use super::{FILE_LIMIT, Sources, Version};

/// The media type of an OCI image manifest.
const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

/// The annotation that names a layer's file.
const TITLE: &str = "org.opencontainers.image.title";

/// The most bytes a manifest or a token answer may hold.
const SMALL_LIMIT: usize = 1024 * 1024;

/// The channel's version of `digest`, pulled by its digest's tag, kept and
/// held.
pub(super) async fn by_digest(
    sources: &Sources,
    digest: &str,
    now: jiff::Timestamp,
) -> Result<(Version, HeldArtifact), Unobtained> {
    let reference = digest.replacen(':', "-", 1);
    let files = pull(sources, &reference).await?;
    let (version, held) = sources.kept.keep(RulesSource::Daily, files, now).await?;
    if version.digest != digest {
        return Err(Unobtained::OtherDigest {
            host: registry(sources).to_owned(),
            found: version.digest,
        });
    }
    Ok((version, held))
}

/// The registry `clean_up.channel` names.
fn registry(sources: &Sources) -> &str {
    sources
        .channel
        .split_once('/')
        .map_or(sources.channel.as_str(), |(registry, _)| registry)
}

/// Pulls the channel's `reference`, a tag: its manifest, then the blob of
/// each layer, checked against its digest.
async fn pull(sources: &Sources, reference: &str) -> Result<Vec<(String, Vec<u8>)>, Unobtained> {
    let mut token = None;
    let (registry, repository) = sources.channel.split_once('/').ok_or_else(|| {
        Unobtained::Unreadable(
            "`clean_up.channel` names no repository `host[:port]/path`".to_owned(),
        )
    })?;
    let url = format!("https://{registry}/v2/{repository}/manifests/{reference}");
    let fetched = get(
        sources,
        &url,
        Some(MANIFEST_MEDIA_TYPE),
        &mut token,
        SMALL_LIMIT,
    )
    .await?;
    if fetched.status != 200 {
        return Err(Unobtained::Refused {
            host: registry.to_owned(),
            status: fetched.status,
        });
    }
    let manifest: Value = serde_json::from_slice(&fetched.body).map_err(|error| {
        Unobtained::Unreadable(format!(
            "the daily channel's manifest does not read: {error}"
        ))
    })?;
    let mut files = Vec::new();
    for layer in manifest["layers"].as_array().into_iter().flatten() {
        let (Some(digest), Some(name)) = (
            layer["digest"].as_str(),
            layer["annotations"][TITLE].as_str(),
        ) else {
            return Err(Unobtained::Unreadable(
                "the daily channel's manifest has a layer without its digest or file name"
                    .to_owned(),
            ));
        };
        let url = format!("https://{registry}/v2/{repository}/blobs/{digest}");
        let blob = get(sources, &url, None, &mut token, FILE_LIMIT).await?;
        if blob.status != 200 {
            return Err(Unobtained::Refused {
                host: registry.to_owned(),
                status: blob.status,
            });
        }
        if Digest::of(&blob.body).to_string() != digest {
            return Err(Unobtained::Tampered {
                host: registry.to_owned(),
                file: name.to_owned(),
            });
        }
        files.push((name.to_owned(), blob.body));
    }
    Ok(files)
}

/// GETs `url` from the registry, with its anonymous token once a `401`
/// named the token's realm.
async fn get(
    sources: &Sources,
    url: &str,
    accept: Option<&str>,
    token: &mut Option<String>,
    limit: usize,
) -> Result<Fetched, Unobtained> {
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
                .map_err(|failure| Unobtained::of(failure, &sources.fetching))
        }
    };
    let fetched = send(token.as_ref()).await?;
    if fetched.status != 401 || token.is_some() {
        return Ok(fetched);
    }
    let refused = || Unobtained::Refused {
        host: registry(sources).to_owned(),
        status: 401,
    };
    let challenge = fetched.authenticate.as_deref().ok_or_else(refused)?;
    *token = Some(
        anonymous_token(sources, challenge)
            .await
            .ok_or_else(refused)?,
    );
    send(token.as_ref()).await
}

/// The anonymous token the registry's `challenge`, a `WWW-Authenticate`
/// header's `Bearer realm=…,service=…,scope=…`, names; none when it names
/// no realm or gives no token.
async fn anonymous_token(sources: &Sources, challenge: &str) -> Option<String> {
    let parameters = challenge.strip_prefix("Bearer ")?;
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
    let url = url::Url::parse_with_params(realm?, &query).ok()?;
    let answer = rule_sources::get(url.as_str(), &[], &sources.fetching, SMALL_LIMIT)
        .await
        .ok()?;
    let token: Value = serde_json::from_slice(&answer.body).ok()?;
    token["token"]
        .as_str()
        .or_else(|| token["access_token"].as_str())
        .map(str::to_owned)
}
