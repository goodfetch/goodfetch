//! Azure Blob Storage Block Blob primitives (`SharedKey` HMAC-SHA256 signing,
//! SAS token auth, `PutBlob` / `PutBlock` / `PutBlockList`, GET, and XML error
//! translation).

use std::time::SystemTime;

use hmac::{Hmac, Mac as _};
use hyper::body::Bytes;
use hyper::header::HeaderValue;
use hyper::{Method, Request};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::Sha256;
use url::Url;

use super::s3::xml_tag;
use super::url::{ObjectLocation, encode_component, encode_path};
use super::{StoreError, StoreKind};

const AZURE_STORAGE_VERSION: &str = "2020-10-02";

/// Azure Blob authentication resolved from Secret references.
#[derive(Clone)]
pub(super) enum AzureAuth {
    /// Account name + Base64-encoded `SharedKey`.
    SharedKey { account: String, key: SecretString },
    /// SAS query token (without leading `?`).
    SasToken(SecretString),
}

/// Azure Blob configuration resolved from `checkpoint.object_storage.az.*`.
#[derive(Clone)]
pub(super) struct AzureConfig {
    pub(super) endpoint: Option<String>,
    pub(super) account: Option<String>,
    pub(super) auth: Option<AzureAuth>,
}

impl std::fmt::Debug for AzureConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AzureConfig")
            .field("endpoint", &self.endpoint)
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

impl AzureConfig {
    pub(super) fn blob_url(
        &self,
        object: &ObjectLocation,
        extra_query: &[(&str, &str)],
    ) -> Result<Url, StoreError> {
        let base = match (&self.endpoint, &self.account) {
            (Some(endpoint), _) => endpoint.trim_end_matches('/').to_owned(),
            (None, Some(account)) => format!("https://{account}.blob.core.windows.net"),
            (None, None) => {
                return Err(StoreError::InvalidConfig(
                    "Azure Checkpoint storage requires `checkpoint.object_storage.az.endpoint` or `checkpoint.object_storage.az.account`".to_owned(),
                ));
            }
        };
        let container = encode_component(&object.bucket);
        let blob = encode_path(&object.key);
        let mut raw = format!("{base}/{container}/{blob}");
        let mut sep = '?';
        for (k, v) in extra_query {
            use std::fmt::Write as _;
            let _ = write!(raw, "{sep}{}={}", encode_component(k), encode_component(v));
            sep = '&';
        }
        if let Some(AzureAuth::SasToken(sas)) = &self.auth {
            let trimmed = sas.expose_secret().trim_start_matches('?');
            if !trimmed.is_empty() {
                raw.push(sep);
                raw.push_str(trimmed);
            }
        }
        Url::parse(&raw).map_err(|error| StoreError::InvalidConfig(format!("Azure URL: {error}")))
    }

    pub(super) fn signed_request(
        &self,
        method: &Method,
        url: &Url,
        extra_headers: &[(&str, &str)],
        body: Bytes,
        now: SystemTime,
    ) -> Result<Request<Bytes>, StoreError> {
        let date = httpdate::fmt_http_date(now);
        let content_length = if body.is_empty() {
            String::new()
        } else {
            body.len().to_string()
        };
        let mut ms_headers: Vec<(String, String)> = vec![
            ("x-ms-date".to_owned(), date.clone()),
            ("x-ms-version".to_owned(), AZURE_STORAGE_VERSION.to_owned()),
        ];
        let mut content_type = String::new();
        for (name, value) in extra_headers {
            let lower = name.to_ascii_lowercase();
            if lower == "content-type" {
                (*value).clone_into(&mut content_type);
            } else if lower.starts_with("x-ms-") {
                ms_headers.push((lower, (*value).to_owned()));
            }
        }
        ms_headers.sort_by(|a, b| a.0.cmp(&b.0));

        let mut builder = Request::builder()
            .method(method.clone())
            .uri(url.as_str())
            .header("x-ms-date", &date)
            .header("x-ms-version", AZURE_STORAGE_VERSION);
        if !body.is_empty() {
            builder = builder.header("content-length", &content_length);
        }
        for (name, value) in extra_headers {
            builder = builder.header(*name, *value);
        }

        if let Some(AzureAuth::SharedKey { account, key }) = &self.auth {
            let auth_header = shared_key_authorization(
                method.as_str(),
                &content_length,
                &content_type,
                &ms_headers,
                account,
                url,
                key.expose_secret(),
            )?;
            let mut val = HeaderValue::from_str(&auth_header).map_err(|_| {
                StoreError::InvalidConfig("invalid Azure Authorization header".to_owned())
            })?;
            val.set_sensitive(true);
            builder = builder.header("authorization", val);
        }

        builder
            .body(body)
            .map_err(|error| StoreError::InvalidConfig(error.to_string()))
    }
}

/// Computes `Authorization: SharedKey <account>:<signature>` per Azure Blob Storage REST API.
fn shared_key_authorization(
    method: &str,
    content_length: &str,
    content_type: &str,
    ms_headers: &[(String, String)],
    account: &str,
    url: &Url,
    base64_key: &str,
) -> Result<String, StoreError> {
    let decoded_key = data_encoding::BASE64
        .decode(base64_key.trim().as_bytes())
        .map_err(|_| {
            StoreError::InvalidConfig("the Azure account key is not valid Base64".to_owned())
        })?;
    let mut canonicalized_headers = String::new();
    for (name, value) in ms_headers {
        canonicalized_headers.push_str(name);
        canonicalized_headers.push(':');
        canonicalized_headers.push_str(value.trim());
        canonicalized_headers.push('\n');
    }
    let canonicalized_path = format!("/{account}{}", url.path());
    let mut query_pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.into_owned()))
        .collect();
    query_pairs.sort();
    let mut canonicalized_resource = canonicalized_path;
    for (k, v) in query_pairs {
        canonicalized_resource.push('\n');
        canonicalized_resource.push_str(&k);
        canonicalized_resource.push(':');
        canonicalized_resource.push_str(&v);
    }

    // StringToSign for Blob Service (version 2009-09-19 and later):
    // VERB\nContent-Encoding\nContent-Language\nContent-Length\nContent-MD5\n
    // Content-Type\nDate\nIf-Modified-Since\nIf-Match\nIf-None-Match\n
    // If-Unmodified-Since\nRange\nCanonicalizedHeaders + CanonicalizedResource
    let string_to_sign = format!(
        "{method}\n\n\n{content_length}\n\n{content_type}\n\n\n\n\n\n\n{canonicalized_headers}{canonicalized_resource}"
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(&decoded_key)
        .expect("invariant: HMAC accepts any key length");
    mac.update(string_to_sign.as_bytes());
    let signature = data_encoding::BASE64.encode(&mac.finalize().into_bytes());
    Ok(format!("SharedKey {account}:{signature}"))
}

/// Builds the `<BlockList>` XML body for committing uploaded blocks.
pub(super) fn block_list_xml(block_ids: &[String]) -> Bytes {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>");
    for id in block_ids {
        use std::fmt::Write as _;
        let _ = write!(xml, "<Latest>{id}</Latest>");
    }
    xml.push_str("</BlockList>");
    Bytes::from(xml)
}

/// Translates an Azure Blob Storage error response at one translation point (ADR 0127).
pub(crate) fn translate_error(status: u16, body: &str, header_code: Option<&str>) -> StoreError {
    let code = header_code
        .map(str::to_owned)
        .or_else(|| xml_tag(body, "Code"));
    match (status, code.as_deref()) {
        (_, Some("BlobNotFound" | "ResourceNotFound")) | (404, None) => StoreError::NotFound {
            store: StoreKind::Azure,
        },
        (_, Some("ContainerNotFound")) => StoreError::BucketNotFound {
            store: StoreKind::Azure,
        },
        (_, Some("AuthenticationFailed" | "InvalidAuthenticationInfo")) | (401, _) => {
            StoreError::InvalidCredentials {
                store: StoreKind::Azure,
            }
        }
        (
            _,
            Some(
                "AuthorizationFailure"
                | "AuthorizationPermissionMismatch"
                | "InsufficientAccountPermissions",
            ),
        )
        | (403, None) => StoreError::AccessDenied {
            store: StoreKind::Azure,
        },
        (_, Some("ServerBusy" | "OperationTimedOut")) | (429, _) => StoreError::Throttled {
            store: StoreKind::Azure,
        },
        (_, Some("InternalError")) | (500 | 502 | 503 | 504, None) => StoreError::Transient {
            store: StoreKind::Azure,
            status,
        },
        _ => StoreError::Unrecognized {
            store: StoreKind::Azure,
            status,
            code,
        },
    }
}
