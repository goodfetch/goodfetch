//! The network errors the browser reports for a request or a navigation
//! (`Network.loadingFailed`'s and `Page.navigate`'s `errorText`), as typed
//! variants (ADR 0127): only here is that text read. The errors the engine
//! tells apart have their own variants; any other the browser names keeps
//! its name, `net::ERR_*`, which a failure reports as the browser's own
//! (`navigation_failed`'s `error`); and a text that names none is
//! unrecognized, logged at `warn`, and reported as `net::ERR_FAILED`,
//! Chromium's generic network error.

use crate::failure::PartyText;

/// A network error of the browser's.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetError {
    /// The browser cancelled the request or navigation: another superseded
    /// it, it was stopped, or its answer has no document.
    Aborted,
    /// A navigation went past the browser's 19 redirects.
    TooManyRedirects,
    /// A body's listed content coding did not decode.
    ContentDecodingFailed,
    /// A zstd body asked for a window past RFC 9659's 8 MB, which the
    /// browser refuses to decode.
    ZstdWindowSizeTooBig,
    /// A navigation's response, an error status with an empty body, which
    /// the browser shows as an error page of its own: the response is still
    /// the navigation's answer.
    HttpResponseCodeFailure,
    /// A blocker failed the request: the interception's Filter lists, or
    /// the page's own.
    BlockedByClient,
    /// Any other error the browser names, kept by its `net::ERR_*` name.
    Other(String),
    /// A text that names no network error, kept for diagnosis.
    Unrecognized(PartyText),
}

/// The prefix of every network error's name.
const PREFIX: &str = "net::ERR_";

/// The suffix the browser adds to a network error its debugging client
/// caused, such as the interception's block: no blocker in a person's
/// browser gives it, so the error is named without it.
const INSPECTOR: &str = ".Inspector";

/// The names of the errors with their own variant.
const NAMED: [(&str, NetError); 6] = [
    ("net::ERR_ABORTED", NetError::Aborted),
    ("net::ERR_TOO_MANY_REDIRECTS", NetError::TooManyRedirects),
    (
        "net::ERR_CONTENT_DECODING_FAILED",
        NetError::ContentDecodingFailed,
    ),
    (
        "net::ERR_ZSTD_WINDOW_SIZE_TOO_BIG",
        NetError::ZstdWindowSizeTooBig,
    ),
    (
        "net::ERR_HTTP_RESPONSE_CODE_FAILURE",
        NetError::HttpResponseCodeFailure,
    ),
    ("net::ERR_BLOCKED_BY_CLIENT", NetError::BlockedByClient),
];

/// The name an unrecognized error is reported with.
const FAILED: &str = "net::ERR_FAILED";

impl NetError {
    /// The network error `text`, the browser's `errorText`, reports.
    #[must_use]
    pub fn of(text: &str) -> Self {
        let name = text.strip_suffix(INSPECTOR).unwrap_or(text);
        if let Some((_, error)) = NAMED.iter().find(|(named, _)| *named == name) {
            return error.clone();
        }
        let well_formed = name.strip_prefix(PREFIX).is_some_and(|code| {
            !code.is_empty()
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        });
        if well_formed {
            return Self::Other(name.to_owned());
        }
        tracing::warn!(text, "an unrecognized network error of the browser's");
        Self::Unrecognized(PartyText::new(text))
    }

    /// Whether the error is a body's content coding that did not decode, a
    /// zstd window too big for the browser among them: after a response
    /// arrived, a failure of its own, never a cut body (ADR 0118's #699
    /// note).
    #[must_use]
    pub fn fails_to_decode(&self) -> bool {
        matches!(
            self,
            Self::ContentDecodingFailed | Self::ZstdWindowSizeTooBig
        )
    }

    /// Its name as the browser gives it, `net::ERR_*`, which a failure
    /// reports as the browser's network error.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Other(name) => name,
            Self::Unrecognized(_) => FAILED,
            named => NAMED
                .iter()
                .find(|(_, error)| error == named)
                .map_or(FAILED, |(name, _)| name),
        }
    }
}

#[cfg(test)]
mod tests;
