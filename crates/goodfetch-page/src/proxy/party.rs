//! The proxy-or-site rule (spec #577, **(shape)**): who answered a request
//! that went through the Session's proxy. A response is the proxy's when the
//! proxy answers the tunnel's `CONNECT` itself, or when the proxy forwarded
//! the request itself (an `http:` URL sent to an HTTP or HTTPS proxy, not
//! around it by `proxy.bypass`) and the last member of its `Proxy-Status`
//! header (RFC 9209), the member of the intermediary nearest the browser,
//! names an `error`, the error that intermediary met. Any other response is
//! the site's: over a tunnel or a SOCKS proxy only the site and its own
//! intermediaries can write the header. So a site's 403 or 429 stays the
//! site's and a proxy's own refusal never becomes a site's Block cause
//! (ADRs 0053 and 0088). Chromium reports the
//! proxy's answer to a `CONNECT`, a proxy it cannot reach and a SOCKS
//! refusal only as a network error of its own, so a request that failed is
//! the proxy's when its error is one of those, and the site's otherwise.

/// The header an intermediary names its own errors in (RFC 9209), as the
/// browser gives a response's headers, lower-cased.
pub(crate) const PROXY_STATUS: &str = "proxy-status";

/// Who answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Party {
    /// The proxy itself.
    Proxy,
    /// The site, through the proxy.
    Site,
}

/// What a request through the proxy met.
#[derive(Clone, Copy, Debug)]
pub enum Answer<'a> {
    /// No response: the browser's network error, such as
    /// `net::ERR_TUNNEL_CONNECTION_FAILED` for a tunnel the proxy refused.
    Failure {
        /// The error.
        error: &'a str,
    },
    /// A response, with its `Proxy-Status` header if it carries one.
    Response {
        /// The header's value.
        proxy_status: Option<&'a str>,
        /// Whether the proxy forwarded the request itself: an `http:` URL
        /// sent to an HTTP or HTTPS proxy, which `proxy.bypass` does not
        /// send around it. Only then can the proxy add its own member to
        /// the response; through a tunnel or a SOCKS proxy, the field is
        /// the site's.
        forwarded: bool,
    },
}

/// The name the Session's own loopback relay gives its member of a
/// `Proxy-Status` field.
pub(crate) const RELAY: &str = "goodfetch-relay";

/// The relay's answer to the browser when an HTTP proxy cannot be reached:
/// its own `502`, whose `Proxy-Status` member names the relay, so it stays
/// the proxy's failure yet is no refusal by the proxy.
pub(crate) fn relay_unreachable() -> String {
    format!(
        "HTTP/1.1 502 Bad Gateway\r\nProxy-Status: {RELAY}; error=connection_refused\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

/// Whether the nearest member of the `Proxy-Status` field `field` is the
/// Session's own relay's.
pub(crate) fn relays(field: &str) -> bool {
    nearest(field).map(name) == Some(RELAY)
}

/// Who gave `answer`.
pub fn party(answer: Answer<'_>) -> Party {
    match answer {
        Answer::Failure { error } if tunnel_failure(error) => Party::Proxy,
        Answer::Response {
            proxy_status: Some(field),
            forwarded: true,
        } if proxy_error(field).is_some() => Party::Proxy,
        Answer::Failure { .. } | Answer::Response { .. } => Party::Site,
    }
}

/// Whether `answer` is the proxy refusing a request its plan does not
/// admit, which lowers its learned ceiling (ADR 0053; spec #577): its own
/// 429 or 502, or a tunnel it refused, which Chromium reports only as
/// `ERR_TUNNEL_CONNECTION_FAILED`. Its 407, a site's answer of any status,
/// a proxy that cannot be reached and the Session's relay answering for a
/// proxy it cannot reach are no such refusal.
pub(crate) fn refusal(answer: Answer<'_>, status: Option<u16>) -> bool {
    match answer {
        Answer::Failure { error } => {
            error.strip_prefix("net::").unwrap_or(error) == "ERR_TUNNEL_CONNECTION_FAILED"
        }
        Answer::Response { proxy_status, .. } => {
            matches!(status, Some(429 | 502))
                && party(answer) == Party::Proxy
                && proxy_status.and_then(nearest).map(name) != Some(RELAY)
        }
    }
}

/// The error the last member of a `Proxy-Status` field, the nearest
/// intermediary's, names, such as `destination_ip_prohibited`.
pub(crate) fn proxy_error(field: &str) -> Option<String> {
    parameter(nearest(field)?, "error")
}

/// The last member of a `Proxy-Status` field: the intermediary nearest the
/// browser appends its own (RFC 9209).
fn nearest(field: &str) -> Option<&str> {
    split_outside_quotes(field, ',').pop()
}

/// A member's intermediary name, its item before any parameter.
fn name(member: &str) -> &str {
    split_outside_quotes(member, ';')
        .first()
        .map_or("", |item| item.trim())
}

/// Whether Chromium's network error `error` is the proxy's own: one it
/// gives when the proxy cannot be reached, its TLS fails, or it answers the
/// tunnel's `CONNECT` or the SOCKS handshake with a refusal.
fn tunnel_failure(error: &str) -> bool {
    let name = error.strip_prefix("net::").unwrap_or(error);
    [
        "ERR_PROXY_",
        "ERR_TUNNEL_",
        "ERR_SOCKS_",
        "ERR_HTTPS_PROXY_",
        "ERR_MANDATORY_PROXY_",
        "ERR_NO_SUPPORTED_PROXIES",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

/// The value of the parameter `name` of one member of a structured field
/// list (RFC 8941): its item, then `;key=value` parameters, a value a token
/// or a quoted string.
fn parameter(member: &str, name: &str) -> Option<String> {
    split_outside_quotes(member, ';')
        .into_iter()
        .skip(1)
        .find_map(|parameter| {
            let (key, value) = parameter.split_once('=')?;
            (key.trim() == name).then(|| unquoted(value.trim()))
        })
        .filter(|value| !value.is_empty())
}

/// `text` split at each `separator` outside a quoted string.
fn split_outside_quotes(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (at, character) in text.char_indices() {
        match character {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            _ if character == separator && !quoted => {
                parts.push(&text[start..at]);
                start = at + character.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// A token as it is, or a quoted string without its quotes and escapes.
fn unquoted(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return value.to_owned();
    };
    let mut text = String::with_capacity(inner.len());
    let mut escaped = false;
    for character in inner.chars() {
        if escaped || character != '\\' {
            text.push(character);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    text
}

#[cfg(test)]
mod tests;
