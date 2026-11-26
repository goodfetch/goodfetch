//! The Secret placeholders a `page.request` carries in its header values
//! and its JSON body's strings (ADR 0092): each bound when the call starts,
//! through the nearest scope's `secrets.placeholders`, and its value added
//! at step 7 of the page's interception, at each hop of the request whose
//! host is on the placeholder's bound domains, after every page script and
//! service worker. A hop outside them is never sent: the request fails with
//! `secret_domain_not_allowed`, so a redirect to another domain carries no
//! value there. A request with no placeholder costs nothing more.

use std::collections::BTreeMap;

use goodfetch_engine::hostile_input::check_header_value;
use goodfetch_engine::run::secret_error;
use goodfetch_engine::run_identity::RunIdentity;
use goodfetch_engine::runs::RunKeys;
use goodfetch_engine::secrets::{self, Purpose, SecretError};
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::{CallError, named};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::request::{JsonSecret, PageRequestParams, SecretHeader};
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::placeholders::{PlaceholderSource, SECRETS_PLACEHOLDERS};
use goodfetch_protocol::settings::SettingsLayer;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::cdp::{self, ContinueRequest, RequestHeaders};

/// The Secret placeholders of one `page.request`, bound.
pub(crate) struct Placeholders {
    /// The Run whose Secrets resolve their values.
    run: RunIdentity,
    /// Its Secrets.
    keys: RunKeys,
    /// The headers whose value holds one.
    headers: Vec<Header>,
    /// The JSON body whose strings hold them, when any does.
    json: Option<JsonBody>,
}

/// A header whose value holds a placeholder.
struct Header {
    name: String,
    form: Form,
    bound: Bound,
}

/// How a header's value is built from the placeholder's.
enum Form {
    /// The value after a prefix.
    Value { prefix: String },
    /// HTTP Basic credentials, the value the password (RFC 7617).
    Basic { username: String },
}

/// The JSON body, and the strings of it that hold a placeholder's value.
struct JsonBody {
    body: Value,
    strings: Vec<(String, Bound)>,
}

/// A placeholder bound to a Secret reference.
struct Bound {
    placeholder: String,
    reference: SecretReference,
    domains: Vec<String>,
    layer: Option<SettingsLayer>,
}

impl Placeholders {
    /// The placeholders `params` carries, bound through `settings`, the
    /// page's Session's beneath its Run's, their values resolved later
    /// through the Secrets of `keys` for the Run `run`: none when it
    /// carries none, so nothing more is done for it.
    ///
    /// # Errors
    ///
    /// `secret_unbound` for a placeholder no scope binds;
    /// `header_value_invalid` for a prefix or a username holding a CR, an
    /// LF or a NUL; `invalid_params` for a placeholder bound to a TOTP seed
    /// or a code source, whose values are only typed, a header also named
    /// in `headers` or named twice, a username holding a colon, and a JSON
    /// placeholder without a JSON body holding its pointer's parent.
    pub(crate) fn bind(
        (run, keys): (&RunIdentity, &RunKeys),
        settings: Option<&Resolved>,
        params: &PageRequestParams,
    ) -> Result<Option<Self>, CallError> {
        let secret_headers = params.secret_headers.as_ref().filter(|map| !map.is_empty());
        let json_secrets = params.json_secrets.as_ref().filter(|list| !list.is_empty());
        if secret_headers.is_none() && json_secrets.is_none() {
            return Ok(None);
        }
        let headers = secret_headers
            .into_iter()
            .flatten()
            .map(|(name, header)| {
                let named_twice = params
                    .headers
                    .iter()
                    .flat_map(BTreeMap::keys)
                    .chain(secret_headers.into_iter().flat_map(BTreeMap::keys))
                    .filter(|other| other.eq_ignore_ascii_case(name))
                    .count()
                    > 1;
                if named_twice {
                    return Err(invalid(format!(
                        "the header `{name}` is named twice: once in `{}` and again in `{}` or `{}`",
                        named::parameter("secret_headers"),
                        named::parameter("headers"),
                        named::parameter("secret_headers"),
                    )));
                }
                header_of(name, header, |placeholder| bound(settings, placeholder))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let json = json_secrets
            .map(|strings| json_body(params.post_data.as_deref(), strings, settings))
            .transpose()?;
        Ok(Some(Self {
            run: run.clone(),
            keys: keys.clone(),
            headers,
            json,
        }))
    }

    /// Whether the hop to `url` may carry the placeholders' values, before
    /// any other step judges it and before any value resolves;
    /// `carries_body` when the hop sends the request's body, as its first
    /// does and a 307 or a 308 resends it.
    ///
    /// # Errors
    ///
    /// `secret_domain_not_allowed` when the hop's host, its userinfo left
    /// out, is outside the bound domains of a placeholder it would carry.
    /// The hop is then not sent.
    pub(crate) fn admit(&self, url: &str, carries_body: bool) -> Result<(), CallError> {
        let host = host_of(url);
        let body = self.json.as_ref().filter(|_| carries_body);
        let used = self.headers.iter().map(|header| &header.bound).chain(
            body.into_iter()
                .flat_map(|json| json.strings.iter().map(|(_, bound)| bound)),
        );
        for bound in used {
            if !secrets::allowed(&host, &bound.domains) {
                return Err(secret_error(&SecretError::DomainNotAllowed {
                    placeholder: bound.placeholder.clone(),
                    reference: bound.reference.clone(),
                    domain: host,
                    domains: bound.domains.clone(),
                }));
            }
        }
        Ok(())
    }

    /// Adds the placeholders' values to `command`, the hop to `url` that
    /// [`Self::admit`] admitted and the browser sends with `sent`, its own
    /// headers; `carries_body` as there. Each header's value replaces what
    /// the hop carries of that name, and the body, with its strings set,
    /// what it sends.
    ///
    /// # Errors
    ///
    /// The codes of a resolution that fails; `header_value_invalid` for a
    /// value built holding a CR, an LF or a NUL. The hop is then not sent.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future abandons the resolutions; a value resolved
    /// stays held by the Run, and `command` may hold part of the values.
    pub(crate) async fn fill(
        &self,
        url: &str,
        carries_body: bool,
        sent: RequestHeaders<'_>,
        command: &mut ContinueRequest,
    ) -> Result<(), CallError> {
        let host = host_of(url);
        let body = self.json.as_ref().filter(|_| carries_body);
        if !self.headers.is_empty() {
            let mut entries = command.headers.take().unwrap_or_else(|| {
                sent.iter()
                    .map(|(name, value)| cdp::Header {
                        name: name.to_owned(),
                        value: value.to_owned(),
                    })
                    .collect()
            });
            entries.retain(|entry| {
                !self
                    .headers
                    .iter()
                    .any(|header| header.name.eq_ignore_ascii_case(&entry.name))
            });
            for header in &self.headers {
                let value = self.resolve(&header.bound, &host).await?;
                let built = header.form.build(&value);
                check_header_value(&header.name, built.expose_secret())
                    .map_err(|refused| refused.error())?;
                entries.push(cdp::Header {
                    name: header.name.clone(),
                    value: built.expose_secret().to_owned(),
                });
            }
            command.headers = Some(entries);
        }
        if let Some(json) = body {
            let mut filled = json.body.clone();
            for (pointer, bound) in &json.strings {
                let value = self.resolve(bound, &host).await?;
                set(
                    &mut filled,
                    pointer,
                    Value::String(value.expose_secret().to_owned()),
                );
            }
            let bytes = serde_json::to_vec(&filled)
                .expect("invariant: a JSON value with string leaves serializes");
            command.post_data = Some(data_encoding::BASE64.encode(&bytes));
        }
        Ok(())
    }

    /// The value of `bound`'s reference, for the hop to `host`.
    async fn resolve(&self, bound: &Bound, host: &str) -> Result<SecretString, CallError> {
        let purpose = Purpose {
            placeholder: &bound.placeholder,
            domain: Some(host),
            domains: &bound.domains,
            layer: bound.layer,
            run_id: Some(self.run.id()),
        };
        self.keys
            .secrets()
            .resolve(&bound.reference, &purpose)
            .await
            .map_err(|error| secret_error(&error))
    }
}

impl Form {
    /// The header's value with `value` in it.
    fn build(&self, value: &SecretString) -> SecretString {
        match self {
            Self::Value { prefix } => format!("{prefix}{}", value.expose_secret()).into(),
            Self::Basic { username } => format!(
                "Basic {}",
                data_encoding::BASE64
                    .encode(format!("{username}:{}", value.expose_secret()).as_bytes())
            )
            .into(),
        }
    }
}

/// The host of `url`, without its userinfo or port; empty for a URL that
/// has none.
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The header `name`, whose value `header` builds, its placeholder bound by
/// `bind`.
fn header_of(
    name: &str,
    header: &SecretHeader,
    bind: impl Fn(&str) -> Result<Bound, CallError>,
) -> Result<Header, CallError> {
    let form = match header {
        SecretHeader::Value { prefix, .. } => {
            let prefix = prefix.clone().unwrap_or_default();
            check_header_value(name, &prefix).map_err(|refused| refused.error())?;
            Form::Value { prefix }
        }
        SecretHeader::Basic { username, .. } => {
            check_header_value(name, username).map_err(|refused| refused.error())?;
            if username.contains(':') {
                return Err(invalid(format!(
                    "the username of the header `{name}` holds a colon, which HTTP Basic credentials put between it and the password (RFC 7617)"
                )));
            }
            Form::Basic {
                username: username.clone(),
            }
        }
    };
    Ok(Header {
        name: name.to_owned(),
        form,
        bound: bind(header.placeholder())?,
    })
}

/// The JSON body `post_data`, in Base64, whose `strings` hold placeholders
/// bound through `settings`.
fn json_body(
    post_data: Option<&str>,
    strings: &[JsonSecret],
    settings: Option<&Resolved>,
) -> Result<JsonBody, CallError> {
    let no_json = || {
        invalid(format!(
            "`{}` needs `{}` to hold a JSON body, in Base64",
            named::parameter("json_secrets"),
            named::parameter("post_data"),
        ))
    };
    let bytes = post_data
        .and_then(|post_data| data_encoding::BASE64.decode(post_data.as_bytes()).ok())
        .ok_or_else(no_json)?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| no_json())?;
    let strings = strings
        .iter()
        .map(|string| {
            if !settable(&body, &string.pointer) {
                return Err(invalid(format!(
                    "the JSON Pointer `{}` of `{}` names neither a member of an object nor an element an array holds in the body",
                    string.pointer.escape_debug(),
                    named::parameter("json_secrets"),
                )));
            }
            Ok((string.pointer.clone(), bound(settings, &string.placeholder)?))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(JsonBody { body, strings })
}

/// `pointer` split into its parent's pointer and its last reference
/// token, unescaped (RFC 6901); none for the whole document's.
fn split(pointer: &str) -> Option<(&str, String)> {
    let (parent, token) = pointer.rsplit_once('/')?;
    Some((parent, token.replace("~1", "/").replace("~0", "~")))
}

/// Whether `pointer` names, in `body`, a member of an object, which a
/// string is added as or replaces, or an element an array holds.
fn settable(body: &Value, pointer: &str) -> bool {
    let Some((parent, token)) = split(pointer) else {
        return false;
    };
    match body.pointer(parent) {
        Some(Value::Object(_)) => true,
        Some(Value::Array(elements)) => index(&token).is_some_and(|at| at < elements.len()),
        _ => false,
    }
}

/// An array index token: digits, without a leading zero but for `0`.
fn index(token: &str) -> Option<usize> {
    let digits = !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_digit());
    (digits && (token == "0" || !token.starts_with('0')))
        .then(|| token.parse().ok())
        .flatten()
}

/// Sets `value` at `pointer` in `body`, which [`settable`] checked.
fn set(body: &mut Value, pointer: &str, value: Value) {
    let (parent, token) = split(pointer).expect("invariant: the pointer was checked settable");
    match body.pointer_mut(parent) {
        Some(Value::Object(members)) => {
            members.insert(token, value);
        }
        Some(Value::Array(elements)) => {
            let at = index(&token).expect("invariant: the pointer was checked settable");
            elements[at] = value;
        }
        _ => unreachable!("invariant: the pointer was checked settable"),
    }
}

/// The binding `settings` give `placeholder`.
///
/// # Errors
///
/// `secret_unbound` when no scope binds it, and `invalid_params` when it
/// is bound to a TOTP seed or a code source, whose values are only typed.
fn bound(settings: Option<&Resolved>, placeholder: &str) -> Result<Bound, CallError> {
    let Some(settings) = settings else {
        return Err(secret_error(&SecretError::Unbound {
            placeholder: placeholder.to_owned(),
            bound: Vec::new(),
        }));
    };
    let binding = secrets::binding(settings, placeholder).map_err(|error| secret_error(&error))?;
    match binding.source {
        PlaceholderSource::Reference {
            reference,
            totp: false,
        } => Ok(Bound {
            placeholder: placeholder.to_owned(),
            reference,
            domains: binding.domains,
            layer: settings
                .effective(&SECRETS_PLACEHOLDERS)
                .map(|effective| effective.layer),
        }),
        PlaceholderSource::Reference { totp: true, .. } | PlaceholderSource::CodeSource(_) => {
            Err(invalid(format!(
                "the Secret placeholder `{placeholder}` is bound to a TOTP seed or a code source, whose values are only typed, never sent in a header or a JSON body"
            )))
        }
    }
}

/// `invalid_params`, saying `message`.
fn invalid(message: String) -> CallError {
    CallError::protocol(ProtocolFailure::InvalidParams, message)
}

#[cfg(test)]
mod tests;
