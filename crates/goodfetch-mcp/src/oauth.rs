//! OAuth 2.1 resource-server metadata (RFC 9728) and JWT bearer token
//! validation for remote Streamable HTTP MCP servers (ADRs 0017, 0067;
//! ticket #1367).
//!
//! The MCP server is strictly a resource server: it publishes protected-resource
//! metadata naming `mcp.oauth.issuer`, discovers the issuer's JWKS, verifies
//! asymmetric JWT signatures (`EdDSA`, `ES256`, `RS256`), validates `iss`,
//! `aud` (`mcp.oauth.audience`), `exp`, and `nbf`, and never forwards any
//! incoming token to the engine, a Daemon, or any downstream service.

use std::fmt;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use data_encoding::BASE64URL_NOPAD;
use ed25519_dalek::{Signature as Ed25519Signature, VerifyingKey as Ed25519VerifyingKey};
use goodfetch_protocol::mcp::{OAUTH_AUDIENCE, OAUTH_ISSUER};
use p256::ecdsa::signature::Verifier as _;
use p256::ecdsa::{Signature as P256Signature, VerifyingKey as P256VerifyingKey};
use rustls::SignatureScheme;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use url::{Host, Url};

/// Base RFC 9728 well-known path for OAuth 2.0 Protected Resource Metadata.
pub const PROTECTED_RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource";

/// Default Streamable HTTP MCP path appended for path-scoped RFC 9728 discovery.
const MCP_RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource/mcp";

/// How long a cached JWKS document stays fresh before automatic re-fetch.
const JWKS_CACHE_TTL_SECS: u64 = 300;

/// Minimum interval between forced JWKS refreshes on an unknown `kid` to guard
/// against unauthenticated cache-busting floods.
const JWKS_MIN_REFRESH_INTERVAL_SECS: u64 = 10;

/// Invalid `mcp.oauth.*` configuration, naming the exact catalog key.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid `{key}`: {reason}")]
pub struct OAuthConfigError {
    /// The setting key (`mcp.oauth.issuer` or `mcp.oauth.audience`).
    pub key: &'static str,
    /// Human-readable reason for `setting_invalid`.
    pub reason: String,
}

/// Validated OAuth 2.1 resource-server settings (`mcp.oauth.issuer` and
/// `mcp.oauth.audience`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthConfig {
    issuer: String,
    issuer_url: Url,
    audience: String,
    resource_metadata_url: String,
    audience_metadata_path: Option<String>,
    public_authority: Option<String>,
}

impl OAuthConfig {
    /// Validates `mcp.oauth.issuer` and `mcp.oauth.audience` at the
    /// configuration boundary.
    ///
    /// # Errors
    ///
    /// Returns [`OAuthConfigError`] naming the invalid key.
    pub fn new(issuer: &str, audience: &str) -> Result<Self, OAuthConfigError> {
        let issuer = issuer.trim();
        if issuer.is_empty() {
            return Err(OAuthConfigError {
                key: OAUTH_ISSUER.name,
                reason: "must be a non-empty OAuth 2.1 issuer URL".to_owned(),
            });
        }
        let issuer_url = validate_endpoint_url(issuer, OAUTH_ISSUER.name)?;

        let audience = audience.trim();
        if audience.is_empty() {
            return Err(OAuthConfigError {
                key: OAUTH_AUDIENCE.name,
                reason: "must be a non-empty OAuth 2.1 resource identifier (audience)".to_owned(),
            });
        }
        let audience_url = Url::parse(audience).map_err(|err| OAuthConfigError {
            key: OAUTH_AUDIENCE.name,
            reason: format!("`{audience}` is not a valid absolute URI: {err}"),
        })?;
        if audience_url.fragment().is_some() {
            return Err(OAuthConfigError {
                key: OAUTH_AUDIENCE.name,
                reason: "an OAuth 2.1 resource identifier must not contain a `#fragment`"
                    .to_owned(),
            });
        }

        let (resource_metadata_url, audience_metadata_path, public_authority) =
            derive_metadata_location(&audience_url);

        Ok(Self {
            issuer: issuer.to_owned(),
            issuer_url,
            audience: audience.to_owned(),
            resource_metadata_url,
            audience_metadata_path,
            public_authority,
        })
    }

    /// Configured `mcp.oauth.issuer`.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Configured `mcp.oauth.audience`.
    #[must_use]
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// RFC 9728 `resource_metadata` URL advertised in `WWW-Authenticate` challenges.
    #[must_use]
    pub fn resource_metadata_url(&self) -> &str {
        &self.resource_metadata_url
    }

    /// Public `host[:port]` authority derived from `mcp.oauth.audience` when it
    /// is an HTTP(S) URL, admitted by `Host`/`Origin` checks.
    #[must_use]
    pub fn public_authority(&self) -> Option<&str> {
        self.public_authority.as_deref()
    }

    /// Whether `path` matches an RFC 9728 protected-resource metadata endpoint
    /// served by this resource server.
    #[must_use]
    pub fn matches_metadata_path(&self, path: &str) -> bool {
        path == PROTECTED_RESOURCE_METADATA_PATH
            || path == MCP_RESOURCE_METADATA_PATH
            || self
                .audience_metadata_path
                .as_deref()
                .is_some_and(|expected| path == expected)
    }

    /// Builds the RFC 9728 Protected Resource Metadata document.
    #[must_use]
    pub fn protected_resource_metadata(&self) -> ProtectedResourceMetadata {
        ProtectedResourceMetadata {
            resource: self.audience.clone(),
            authorization_servers: vec![self.issuer.clone()],
            bearer_methods_supported: vec!["header".to_owned()],
        }
    }

    /// Candidate discovery URLs for fetching the authorization server's
    /// metadata (RFC 8414 Section 3.1 + OIDC Discovery 1.0).
    #[must_use]
    pub fn discovery_urls(&self) -> Vec<String> {
        let origin = format!(
            "{}://{}",
            self.issuer_url.scheme(),
            format_authority(&self.issuer_url)
        );
        let path = self.issuer_url.path().trim_end_matches('/');
        let mut urls = Vec::with_capacity(3);
        // RFC 8414 path-inserted well-known URL.
        urls.push(format!(
            "{origin}/.well-known/oauth-authorization-server{path}"
        ));
        // OIDC Discovery path-appended well-known URL.
        urls.push(format!("{origin}{path}/.well-known/openid-configuration"));
        if !path.is_empty() {
            urls.push(format!(
                "{origin}{path}/.well-known/oauth-authorization-server"
            ));
        }
        urls
    }

    /// Whether `issuer` is a loopback HTTP URL (used by local authorization
    /// server stubs in tests).
    fn allows_loopback_http(&self) -> bool {
        self.issuer_url.scheme() == "http" && is_loopback_url(&self.issuer_url)
    }
}

/// RFC 9728 Protected Resource Metadata JSON document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedResourceMetadata {
    /// The protected resource's canonical URI (`mcp.oauth.audience`).
    pub resource: String,
    /// Authorization server issuer identifiers trusted by this resource server
    /// (`mcp.oauth.issuer`).
    pub authorization_servers: Vec<String>,
    /// Supported methods for presenting OAuth 2.1 bearer tokens (`["header"]`).
    pub bearer_methods_supported: Vec<String>,
}

/// Async JSON fetcher used to retrieve authorization server discovery metadata
/// and JWKS documents.
pub trait OAuthMetadataFetcher: Send + Sync + 'static {
    /// Performs an HTTP(S) `GET` to `url` and parses its JSON response body.
    fn fetch_json(
        &self,
        url: String,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + Send + '_>>;
}

/// Newtype hiding dynamic dispatch for an [`OAuthMetadataFetcher`] (STYLE.md).
#[derive(Clone)]
pub struct SharedOAuthMetadataFetcher(Arc<dyn OAuthMetadataFetcher>);

impl SharedOAuthMetadataFetcher {
    /// Wraps `fetcher` in a cheaply cloneable [`SharedOAuthMetadataFetcher`].
    #[must_use]
    pub fn new(fetcher: impl OAuthMetadataFetcher) -> Self {
        Self(Arc::new(fetcher))
    }

    /// Fetches and parses JSON from `url` via the wrapped fetcher.
    ///
    /// # Errors
    ///
    /// Returns a human-readable description if fetching or JSON parsing fails.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels the in-flight HTTP request.
    pub async fn fetch_json(&self, url: String) -> Result<serde_json::Value, String> {
        self.0.fetch_json(url).await
    }
}

/// Time source returning Unix epoch seconds (avoids calling `SystemTime::now`
/// directly, ADR 0059; newtype hiding `Arc<dyn Fn>` per STYLE.md).
#[derive(Clone)]
pub struct EpochSecondsClock(Arc<dyn Fn() -> u64 + Send + Sync>);

impl EpochSecondsClock {
    /// Wraps a closure returning Unix epoch seconds.
    #[must_use]
    pub fn new(now: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    /// Returns the current Unix epoch timestamp in seconds.
    #[must_use]
    pub fn now(&self) -> u64 {
        (self.0)()
    }
}

/// Verified claims extracted from an accepted OAuth 2.1 JWT access token.
/// Never carries the raw bearer token string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedToken {
    /// Verified issuer (`iss`).
    pub issuer: String,
    /// Optional subject (`sub`).
    pub subject: Option<String>,
    /// Verified expiration timestamp (`exp`, Unix epoch seconds).
    pub expires_at: u64,
}

/// Why an incoming OAuth 2.1 bearer token was rejected. Never quotes the token.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TokenRejection {
    /// The bearer string is not a compact 3-segment JWS or fails base64url/JSON decoding.
    #[error("malformed JWT access token")]
    Malformed,
    /// The JWS header specifies an unsupported or forbidden algorithm (`none`, `HS*`, etc.).
    #[error("unsupported JWT signing algorithm")]
    UnsupportedAlgorithm,
    /// The `iss` claim is missing or does not match `mcp.oauth.issuer`.
    #[error("token issuer does not match configured issuer")]
    IssuerMismatch,
    /// The `aud` claim is missing or does not contain `mcp.oauth.audience`.
    #[error("token audience does not match configured audience")]
    AudienceMismatch,
    /// The `exp` claim is missing or in the past (`now >= exp`).
    #[error("token is expired")]
    Expired,
    /// The `nbf` claim is in the future (`now < nbf`).
    #[error("token is not yet valid")]
    NotYetValid,
    /// No signing key in the issuer's JWKS verified the token signature.
    #[error("token signature verification failed")]
    InvalidSignature,
    /// The issuer's discovery metadata or JWKS could not be fetched or validated.
    #[error("authorization server keys unavailable")]
    JwksUnavailable,
}

impl TokenRejection {
    /// Safe `error_description` value for `WWW-Authenticate: Bearer` (RFC 6750).
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::Malformed => "malformed JWT access token",
            Self::UnsupportedAlgorithm => "unsupported JWT signing algorithm",
            Self::IssuerMismatch => "token issuer does not match configured issuer",
            Self::AudienceMismatch => "token audience does not match configured audience",
            Self::Expired => "token is expired",
            Self::NotYetValid => "token is not yet valid",
            Self::InvalidSignature => "token signature verification failed",
            Self::JwksUnavailable => "authorization server keys unavailable",
        }
    }
}

/// OAuth 2.1 resource server handling RFC 9728 metadata publishing and JWT
/// bearer token verification against `mcp.oauth.issuer`'s JWKS.
pub struct OAuthResourceServer {
    config: OAuthConfig,
    metadata_json: Bytes,
    fetcher: SharedOAuthMetadataFetcher,
    now: EpochSecondsClock,
    cache: RwLock<Option<CachedJwks>>,
    refresh_lock: Mutex<()>,
}

impl fmt::Debug for OAuthResourceServer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthResourceServer")
            .field("issuer", &self.config.issuer)
            .field("audience", &self.config.audience)
            .finish_non_exhaustive()
    }
}

struct CachedJwks {
    keys: Vec<Jwk>,
    fetched_at: u64,
}

impl OAuthResourceServer {
    /// Creates an OAuth 2.1 resource server from a validated [`OAuthConfig`],
    /// metadata/JWKS fetcher, and clock returning Unix epoch seconds.
    #[must_use]
    pub fn new(
        config: OAuthConfig,
        fetcher: SharedOAuthMetadataFetcher,
        now: EpochSecondsClock,
    ) -> Self {
        let metadata = config.protected_resource_metadata();
        let metadata_json = Bytes::from(
            serde_json::to_vec(&metadata)
                .expect("invariant: ProtectedResourceMetadata always serializes to JSON"),
        );
        Self {
            config,
            metadata_json,
            fetcher,
            now,
            cache: RwLock::new(None),
            refresh_lock: Mutex::new(()),
        }
    }

    /// Public `Host` authority derived from the configured OAuth audience, if any.
    #[must_use]
    pub fn public_authority(&self) -> Option<&str> {
        self.config.public_authority()
    }

    /// Serialized RFC 9728 Protected Resource Metadata JSON bytes.
    #[must_use]
    pub fn metadata_json(&self) -> Bytes {
        self.metadata_json.clone()
    }

    /// Whether `path` should be answered with RFC 9728 protected-resource metadata.
    #[must_use]
    pub fn matches_metadata_path(&self, path: &str) -> bool {
        self.config.matches_metadata_path(path)
    }

    /// Formats the `WWW-Authenticate` header value for a `401 Unauthorized`
    /// response (RFC 9728 Section 5.1 & RFC 6750 Section 3).
    #[must_use]
    pub fn www_authenticate(&self, rejection: Option<&TokenRejection>) -> String {
        let metadata = self.config.resource_metadata_url();
        match rejection {
            None => format!("Bearer resource_metadata=\"{metadata}\""),
            Some(reason) => format!(
                "Bearer error=\"invalid_token\", error_description=\"{}\", resource_metadata=\"{metadata}\"",
                reason.description()
            ),
        }
    }

    /// Validates an OAuth 2.1 JWT bearer token: format, algorithm, `iss`,
    /// `aud`, `exp`, `nbf`, and cryptographic signature against the issuer's
    /// JWKS.
    ///
    /// # Errors
    ///
    /// Returns [`TokenRejection`] describing why the token was refused.
    pub async fn verify_bearer(&self, token: &str) -> Result<VerifiedToken, TokenRejection> {
        let parsed = parse_jwt(token)?;

        // Claims validation before touching network or crypto (fail closed fast).
        if parsed.claims.iss.as_deref() != Some(self.config.issuer.as_str()) {
            return Err(TokenRejection::IssuerMismatch);
        }
        if !parsed
            .claims
            .aud
            .as_ref()
            .is_some_and(|aud| aud.contains(&self.config.audience))
        {
            return Err(TokenRejection::AudienceMismatch);
        }
        let now = self.now.now();
        let exp = parsed.claims.exp.ok_or(TokenRejection::Expired)?;
        if now >= exp {
            return Err(TokenRejection::Expired);
        }
        if let Some(nbf) = parsed.claims.nbf
            && now < nbf
        {
            return Err(TokenRejection::NotYetValid);
        }

        // Verify signature against cached JWKS; refresh once on cache miss or unknown kid.
        let keys = self.current_keys(now, false).await?;
        if verify_against_keys(&parsed, &keys) {
            return Ok(VerifiedToken {
                issuer: self.config.issuer.clone(),
                subject: parsed.claims.sub,
                expires_at: exp,
            });
        }

        // Refresh once if the token's key may have been rotated.
        let refreshed = self.current_keys(now, true).await?;
        if verify_against_keys(&parsed, &refreshed) {
            return Ok(VerifiedToken {
                issuer: self.config.issuer.clone(),
                subject: parsed.claims.sub,
                expires_at: exp,
            });
        }

        Err(TokenRejection::InvalidSignature)
    }

    async fn current_keys(
        &self,
        now: u64,
        force_refresh: bool,
    ) -> Result<Vec<Jwk>, TokenRejection> {
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                let age = now.saturating_sub(cached.fetched_at);
                let fresh = age < JWKS_CACHE_TTL_SECS;
                let in_cooldown = age < JWKS_MIN_REFRESH_INTERVAL_SECS;
                if (!force_refresh && fresh) || (force_refresh && in_cooldown) {
                    return Ok(cached.keys.clone());
                }
            }
        }

        let _refresh_guard = self.refresh_lock.lock().await;
        // Re-check after acquiring refresh lock to coalesce concurrent refreshes.
        {
            let guard = self.cache.read().await;
            if let Some(cached) = guard.as_ref() {
                let age = now.saturating_sub(cached.fetched_at);
                if (!force_refresh && age < JWKS_CACHE_TTL_SECS)
                    || (force_refresh && age < JWKS_MIN_REFRESH_INTERVAL_SECS)
                {
                    return Ok(cached.keys.clone());
                }
            }
        }

        let keys = self.discover_and_fetch_jwks().await?;
        {
            let mut guard = self.cache.write().await;
            *guard = Some(CachedJwks {
                keys: keys.clone(),
                fetched_at: now,
            });
        }
        Ok(keys)
    }

    async fn discover_and_fetch_jwks(&self) -> Result<Vec<Jwk>, TokenRejection> {
        let mut last_error = None;
        let mut metadata: Option<AuthorizationServerMetadata> = None;
        for url in self.config.discovery_urls() {
            match self.fetcher.fetch_json(url).await {
                Ok(value) => match serde_json::from_value::<AuthorizationServerMetadata>(value) {
                    Ok(parsed) => {
                        metadata = Some(parsed);
                        break;
                    }
                    Err(err) => last_error = Some(err.to_string()),
                },
                Err(err) => last_error = Some(err),
            }
        }

        let Some(metadata) = metadata else {
            tracing::warn!(
                issuer = %self.config.issuer,
                error = last_error.as_deref().unwrap_or("no discovery endpoint answered"),
                "failed to fetch OAuth 2.1 authorization server metadata"
            );
            return Err(TokenRejection::JwksUnavailable);
        };

        // RFC 8414 Section 3.3: discovered `issuer` MUST match configured issuer exactly.
        if metadata.issuer != self.config.issuer {
            tracing::warn!(
                configured = %self.config.issuer,
                discovered = %metadata.issuer,
                "authorization server metadata issuer mismatch"
            );
            return Err(TokenRejection::JwksUnavailable);
        }

        let Ok(jwks_url) = Url::parse(&metadata.jwks_uri) else {
            tracing::warn!(jwks_uri = %metadata.jwks_uri, "invalid jwks_uri in discovery metadata");
            return Err(TokenRejection::JwksUnavailable);
        };
        let scheme_ok = jwks_url.scheme() == "https"
            || (self.config.allows_loopback_http()
                && jwks_url.scheme() == "http"
                && is_loopback_url(&jwks_url));
        if !scheme_ok {
            tracing::warn!(jwks_uri = %metadata.jwks_uri, "refused non-HTTPS jwks_uri");
            return Err(TokenRejection::JwksUnavailable);
        }

        let jwks_json = self
            .fetcher
            .fetch_json(metadata.jwks_uri)
            .await
            .map_err(|err| {
                tracing::warn!(%err, "failed to fetch OAuth 2.1 JWKS");
                TokenRejection::JwksUnavailable
            })?;
        let jwks: JwksDocument = serde_json::from_value(jwks_json).map_err(|err| {
            tracing::warn!(%err, "invalid OAuth 2.1 JWKS JSON");
            TokenRejection::JwksUnavailable
        })?;
        Ok(jwks.keys)
    }
}

#[derive(Deserialize)]
struct AuthorizationServerMetadata {
    issuer: String,
    jwks_uri: String,
}

#[derive(Deserialize)]
struct JwksDocument {
    keys: Vec<Jwk>,
}

/// One public JSON Web Key from an authorization server's JWKS.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Jwk {
    /// Key type (`OKP`, `EC`, `RSA`).
    pub kty: String,
    /// Optional key ID (`kid`).
    #[serde(default)]
    pub kid: Option<String>,
    /// Optional public key use (`sig` expected when present).
    #[serde(default, rename = "use")]
    pub key_use: Option<String>,
    /// Optional algorithm restriction (`EdDSA`, `ES256`, `RS256`).
    #[serde(default)]
    pub alg: Option<String>,
    /// Elliptic / Edwards curve (`Ed25519`, `P-256`).
    #[serde(default)]
    pub crv: Option<String>,
    /// Public key `x` coordinate (OKP or EC, base64url without padding).
    #[serde(default)]
    pub x: Option<String>,
    /// Public key `y` coordinate (EC, base64url without padding).
    #[serde(default)]
    pub y: Option<String>,
    /// RSA modulus `n` (base64url without padding).
    #[serde(default)]
    pub n: Option<String>,
    /// RSA public exponent `e` (base64url without padding).
    #[serde(default)]
    pub e: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JwtAlg {
    EdDsa,
    Es256,
    Rs256,
}

impl JwtAlg {
    fn parse(raw: &str) -> Result<Self, TokenRejection> {
        match raw {
            "EdDSA" => Ok(Self::EdDsa),
            "ES256" => Ok(Self::Es256),
            "RS256" => Ok(Self::Rs256),
            _ => Err(TokenRejection::UnsupportedAlgorithm),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::EdDsa => "EdDSA",
            Self::Es256 => "ES256",
            Self::Rs256 => "RS256",
        }
    }
}

#[derive(Deserialize)]
struct JwtHeader {
    alg: String,
    #[serde(default)]
    kid: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AudienceClaim {
    One(String),
    Many(Vec<String>),
}

impl AudienceClaim {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(value) => value == expected,
            Self::Many(values) => values.iter().any(|value| value == expected),
        }
    }
}

#[derive(Deserialize)]
struct JwtClaims {
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    aud: Option<AudienceClaim>,
    #[serde(default)]
    exp: Option<u64>,
    #[serde(default)]
    nbf: Option<u64>,
    #[serde(default)]
    sub: Option<String>,
}

struct ParsedJwt {
    alg: JwtAlg,
    kid: Option<String>,
    claims: JwtClaims,
    signing_input: Vec<u8>,
    signature: Vec<u8>,
}

fn parse_jwt(token: &str) -> Result<ParsedJwt, TokenRejection> {
    let mut segments = token.split('.');
    let Some(header_b64) = segments.next() else {
        return Err(TokenRejection::Malformed);
    };
    let Some(payload_b64) = segments.next() else {
        return Err(TokenRejection::Malformed);
    };
    let Some(sig_b64) = segments.next() else {
        return Err(TokenRejection::Malformed);
    };
    if segments.next().is_some()
        || header_b64.is_empty()
        || payload_b64.is_empty()
        || sig_b64.is_empty()
    {
        return Err(TokenRejection::Malformed);
    }

    let header_bytes = BASE64URL_NOPAD
        .decode(header_b64.as_bytes())
        .map_err(|_| TokenRejection::Malformed)?;
    let header: JwtHeader =
        serde_json::from_slice(&header_bytes).map_err(|_| TokenRejection::Malformed)?;
    let alg = JwtAlg::parse(&header.alg)?;

    let payload_bytes = BASE64URL_NOPAD
        .decode(payload_b64.as_bytes())
        .map_err(|_| TokenRejection::Malformed)?;
    let claims: JwtClaims =
        serde_json::from_slice(&payload_bytes).map_err(|_| TokenRejection::Malformed)?;

    let signature = BASE64URL_NOPAD
        .decode(sig_b64.as_bytes())
        .map_err(|_| TokenRejection::Malformed)?;
    let signing_input = format!("{header_b64}.{payload_b64}").into_bytes();

    Ok(ParsedJwt {
        alg,
        kid: header.kid,
        claims,
        signing_input,
        signature,
    })
}

fn verify_against_keys(parsed: &ParsedJwt, keys: &[Jwk]) -> bool {
    keys.iter().any(|jwk| {
        if jwk.key_use.as_deref().is_some_and(|u| u != "sig") {
            return false;
        }
        if jwk
            .alg
            .as_deref()
            .is_some_and(|alg| alg != parsed.alg.as_str())
        {
            return false;
        }
        if let Some(expected_kid) = parsed.kid.as_deref()
            && jwk.kid.as_deref() != Some(expected_kid)
        {
            return false;
        }
        verify_signature_with_jwk(parsed.alg, &parsed.signing_input, &parsed.signature, jwk)
    })
}

fn verify_signature_with_jwk(
    alg: JwtAlg,
    signing_input: &[u8],
    signature: &[u8],
    jwk: &Jwk,
) -> bool {
    match alg {
        JwtAlg::EdDsa => verify_eddsa(signing_input, signature, jwk),
        JwtAlg::Es256 => verify_es256(signing_input, signature, jwk),
        JwtAlg::Rs256 => verify_rs256(signing_input, signature, jwk),
    }
}

fn verify_eddsa(signing_input: &[u8], signature: &[u8], jwk: &Jwk) -> bool {
    if jwk.kty != "OKP" || jwk.crv.as_deref() != Some("Ed25519") {
        return false;
    }
    let Some(x_b64) = jwk.x.as_deref() else {
        return false;
    };
    let Ok(x_bytes) = BASE64URL_NOPAD.decode(x_b64.as_bytes()) else {
        return false;
    };
    let Ok(key_bytes): Result<[u8; 32], _> = x_bytes.try_into() else {
        return false;
    };
    let Ok(verifying_key) = Ed25519VerifyingKey::from_bytes(&key_bytes) else {
        return false;
    };
    let Ok(sig) = Ed25519Signature::from_slice(signature) else {
        return false;
    };
    verifying_key.verify_strict(signing_input, &sig).is_ok()
}

fn verify_es256(signing_input: &[u8], signature: &[u8], jwk: &Jwk) -> bool {
    if jwk.kty != "EC" || jwk.crv.as_deref() != Some("P-256") {
        return false;
    }
    let (Some(x_b64), Some(y_b64)) = (jwk.x.as_deref(), jwk.y.as_deref()) else {
        return false;
    };
    let (Ok(x), Ok(y)) = (
        BASE64URL_NOPAD.decode(x_b64.as_bytes()),
        BASE64URL_NOPAD.decode(y_b64.as_bytes()),
    ) else {
        return false;
    };
    if x.len() != 32 || y.len() != 32 {
        return false;
    }
    let mut sec1 = [0u8; 65];
    sec1[0] = 0x04;
    sec1[1..33].copy_from_slice(&x);
    sec1[33..65].copy_from_slice(&y);
    let Ok(verifying_key) = P256VerifyingKey::from_sec1_bytes(&sec1) else {
        return false;
    };
    let Ok(sig) = P256Signature::from_slice(signature) else {
        return false;
    };
    verifying_key.verify(signing_input, &sig).is_ok()
}

fn verify_rs256(signing_input: &[u8], signature: &[u8], jwk: &Jwk) -> bool {
    if jwk.kty != "RSA" {
        return false;
    }
    let (Some(n_b64), Some(e_b64)) = (jwk.n.as_deref(), jwk.e.as_deref()) else {
        return false;
    };
    let (Ok(n), Ok(e)) = (
        BASE64URL_NOPAD.decode(n_b64.as_bytes()),
        BASE64URL_NOPAD.decode(e_b64.as_bytes()),
    ) else {
        return false;
    };
    if n.is_empty() || e.is_empty() {
        return false;
    }
    let rsa_der = encode_rsa_public_key_der(&n, &e);
    let provider = rustls::crypto::ring::default_provider();
    let Some((_, algs)) = provider
        .signature_verification_algorithms
        .mapping
        .iter()
        .find(|(scheme, _)| *scheme == SignatureScheme::RSA_PKCS1_SHA256)
    else {
        return false;
    };
    algs.iter().any(|alg| {
        alg.verify_signature(&rsa_der, signing_input, signature)
            .is_ok()
    })
}

fn encode_rsa_public_key_der(n: &[u8], e: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(n.len() + e.len() + 16);
    push_der_integer(&mut body, n);
    push_der_integer(&mut body, e);
    let mut seq = Vec::with_capacity(body.len() + 6);
    seq.push(0x30);
    push_der_length(&mut seq, body.len());
    seq.extend_from_slice(&body);
    seq
}

fn push_der_integer(out: &mut Vec<u8>, bytes: &[u8]) {
    let first_nonzero = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    let trimmed = &bytes[first_nonzero..];
    out.push(0x02);
    if trimmed.is_empty() {
        out.push(0x01);
        out.push(0x00);
        return;
    }
    let needs_pad = (trimmed[0] & 0x80) != 0;
    push_der_length(out, trimmed.len() + usize::from(needs_pad));
    if needs_pad {
        out.push(0x00);
    }
    out.extend_from_slice(trimmed);
}

fn push_der_length(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(u8::try_from(len).expect("invariant: len < 128"));
    } else if len <= 0xff {
        out.push(0x81);
        out.push(u8::try_from(len).expect("invariant: len <= 255"));
    } else {
        out.push(0x82);
        let hi = u8::try_from((len >> 8) & 0xff).expect("invariant: masked byte");
        let lo = u8::try_from(len & 0xff).expect("invariant: masked byte");
        out.push(hi);
        out.push(lo);
    }
}

fn validate_endpoint_url(raw: &str, key: &'static str) -> Result<Url, OAuthConfigError> {
    let parsed = Url::parse(raw).map_err(|err| OAuthConfigError {
        key,
        reason: format!("`{raw}` is not a valid URL: {err}"),
    })?;
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(OAuthConfigError {
            key,
            reason: "issuer URL must not contain a query string or fragment".to_owned(),
        });
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(OAuthConfigError {
            key,
            reason: "issuer URL must not embed credentials".to_owned(),
        });
    }
    match parsed.scheme() {
        "https" => {}
        "http" if is_loopback_url(&parsed) => {}
        other => {
            return Err(OAuthConfigError {
                key,
                reason: format!(
                    "expected an `https://` issuer URL (plain `http://` is only accepted for a loopback test stub), got `{other}://`"
                ),
            });
        }
    }
    Ok(parsed)
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

fn format_authority(url: &Url) -> String {
    let host = match url.host() {
        Some(Host::Domain(d)) => d.to_owned(),
        Some(Host::Ipv4(ip)) => ip.to_string(),
        Some(Host::Ipv6(ip)) => format!("[{ip}]"),
        None => String::new(),
    };
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

fn derive_metadata_location(audience_url: &Url) -> (String, Option<String>, Option<String>) {
    if !matches!(audience_url.scheme(), "http" | "https") {
        return (PROTECTED_RESOURCE_METADATA_PATH.to_owned(), None, None);
    }
    let authority = format_authority(audience_url);
    let path = audience_url.path().trim_end_matches('/');
    let metadata_path = if path.is_empty() {
        PROTECTED_RESOURCE_METADATA_PATH.to_owned()
    } else {
        format!("{PROTECTED_RESOURCE_METADATA_PATH}{path}")
    };
    let metadata_url = format!("{}://{authority}{metadata_path}", audience_url.scheme());
    (metadata_url, Some(metadata_path), Some(authority))
}
