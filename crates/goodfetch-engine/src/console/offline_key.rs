//! `POST /offline-key` (ADR 0031; spec #592's `offline`): where a
//! signed-in page gets the key its offline cache is encrypted under, a new
//! one or the one its stored wrapped form holds, which the Daemon unwraps
//! only for the cookie of the sign-in it belongs to.

use data_encoding::BASE64URL_NOPAD;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};

use super::Site;
use crate::people::{OfflineKeyRefused, SignInRefused};

/// The longest body `POST /offline-key` reads: a wrapped key, in JSON.
const MAX_OFFLINE_KEY_BYTES: usize = 1024;

impl Site {
    /// `POST /offline-key` (ADR 0031): for a signed-in request from the
    /// Console's own origin with its CSRF token, the sign-in's offline cache
    /// key, `{"key": "…", "wrapped": "…"}` in unpadded base64url: a new one
    /// for the body `{}`, or the one `{"wrapped": "…"}` holds, which only
    /// its own sign-in's cookie unwraps (`422` for another's). `404` while
    /// `console.offline_cache` is off, so the page caches nothing.
    pub(super) async fn offline_key(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        if request.method() != Method::POST {
            return self.refused_method(&["POST"]);
        }
        if let Some(refusal) = self.foreign(request.headers()) {
            return self
                .refused(refusal.reason().as_str(), StatusCode::FORBIDDEN)
                .await;
        }
        let Some(cookie) = self.cookie_of(request.headers()).map(str::to_owned) else {
            return self
                .refused(
                    SignInRefused::TokenMissing.reason(),
                    StatusCode::UNAUTHORIZED,
                )
                .await;
        };
        if self.sign_ins.principal(&cookie).await.is_none() {
            return self
                .refused(SignInRefused::TokenWrong.reason(), StatusCode::UNAUTHORIZED)
                .await;
        }
        if let Err(reason) = self.csrf_checked(request.headers(), &cookie).await {
            return self.refused(reason, StatusCode::FORBIDDEN).await;
        }
        if !self.offline_cache().await {
            return self.plain(StatusCode::NOT_FOUND);
        }
        let Ok(body) = Limited::new(request.into_body(), MAX_OFFLINE_KEY_BYTES)
            .collect()
            .await
        else {
            return self.plain(StatusCode::PAYLOAD_TOO_LARGE);
        };
        let wrapped = match wrapped_of(&body.to_bytes()) {
            Some(KeyAsked::New) => None,
            Some(KeyAsked::Unwrapped(wrapped)) => Some(wrapped),
            None => return self.plain(StatusCode::BAD_REQUEST),
        };
        match self.sign_ins.offline_key(&cookie, wrapped.as_deref()).await {
            Ok(key) => {
                let body = serde_json::json!({
                    "key": BASE64URL_NOPAD.encode(key.key()),
                    "wrapped": BASE64URL_NOPAD.encode(key.wrapped()),
                })
                .to_string();
                let mut response = self.plain(StatusCode::OK);
                *response.body_mut() = Full::new(Bytes::from(body));
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                response
            }
            Err(OfflineKeyRefused::NotSignedIn) => {
                self.refused(SignInRefused::TokenWrong.reason(), StatusCode::UNAUTHORIZED)
                    .await
            }
            Err(OfflineKeyRefused::DoesNotOpen) => self.plain(StatusCode::UNPROCESSABLE_ENTITY),
            Err(OfflineKeyRefused::Generator(error)) => {
                tracing::error!(%error, "the OS generator gave no offline cache key");
                self.plain(StatusCode::SERVICE_UNAVAILABLE)
            }
        }
    }
}

/// What a `POST /offline-key` body asks for.
#[derive(Debug, PartialEq, Eq)]
enum KeyAsked {
    /// `{}`: a new key.
    New,
    /// `{"wrapped": "<unpadded base64url>"}`: the key these bytes wrap.
    Unwrapped(Vec<u8>),
}

/// What a `POST /offline-key` body asks for; none for a body that is
/// neither form.
fn wrapped_of(body: &[u8]) -> Option<KeyAsked> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Body {
        wrapped: Option<String>,
    }
    let read: Body = serde_json::from_slice(body).ok()?;
    match read.wrapped {
        None => Some(KeyAsked::New),
        Some(wrapped) => BASE64URL_NOPAD
            .decode(wrapped.as_bytes())
            .ok()
            .map(KeyAsked::Unwrapped),
    }
}

/// The fuzz target `console_sign_in`'s check of a `POST /offline-key`
/// body: a wrapped key it names reads back as itself.
pub(super) fn fuzz_body(body: &[u8]) {
    if let Some(KeyAsked::Unwrapped(wrapped)) = wrapped_of(body) {
        let encoded = BASE64URL_NOPAD.encode(&wrapped);
        assert_eq!(
            wrapped_of(format!("{{\"wrapped\":\"{encoded}\"}}").as_bytes()),
            Some(KeyAsked::Unwrapped(wrapped)),
            "a wrapped key does not read back as itself"
        );
    }
}
