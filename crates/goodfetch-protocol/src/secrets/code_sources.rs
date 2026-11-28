//! `code_sources.<name>`, the email or SMS two-factor code sources a program
//! configures so `secrets.placeholders.<name> = { code_source, domains }` can
//! read one-time codes unattended (ADRs 0006 and 0092; ticket #734).
//!
//! Only configuration, Operator policy and the program's own run layer set
//! `code_sources`: a Recipe or a Client acting for a model receives
//! `setting_locked`. Every credential field inside a code source is a Secret
//! reference (`env:`, `keychain:`, `op://`, `caller:`, ...), never a raw value.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{Authority, ProtocolVersion, Setting, ValueKind, ValueShape};
use crate::secret_reference::SecretReference;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The default time window from a login attempt: 5 minutes, in milliseconds
/// (ADR 0080).
pub const DEFAULT_CODE_SOURCE_WINDOW_MS: u64 = 5 * 60 * 1_000;

/// The default IMAP port (IMAPS, RFC 9051).
const DEFAULT_IMAP_PORT: u16 = 993;

/// The default IMAP mailbox name.
fn default_imap_mailbox() -> String {
    "INBOX".to_owned()
}

const fn default_imap_port() -> u16 {
    DEFAULT_IMAP_PORT
}

const fn default_window_ms() -> u64 {
    DEFAULT_CODE_SOURCE_WINDOW_MS
}

/// One configured email or SMS two-factor code source (`code_sources.<name>`,
/// ADRs 0006 and 0092).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeSource {
    /// How one-time codes are fetched from this source.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: CodeSourceKind,
    /// Optional sender filter; only messages whose sender matches are read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub from: Option<String>,
    /// Optional regular expression extracting the one-time code (capture group
    /// 1 if present, else the full match). Defaults to a 4-to-8-digit code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pattern: Option<String>,
    /// How long around the login attempt a code is accepted, in whole
    /// milliseconds (ADR 0080); defaults to 5 minutes (`300000`).
    #[serde(default = "default_window_ms")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub window: u64,
}

/// The transport and credentials of a [`CodeSource`], tagged by `type`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodeSourceKind {
    /// An IMAP mailbox, reading recent messages over TCP or TLS.
    Imap {
        /// The IMAP server host.
        #[schemars(extend("arrived_in" = "0.1"))]
        host: String,
        /// The IMAP server port; defaults to 993.
        #[serde(default = "default_imap_port")]
        #[schemars(extend("arrived_in" = "0.1"))]
        port: u16,
        /// The IMAP username, as a Secret reference.
        #[schemars(extend("arrived_in" = "0.1"))]
        username: String,
        /// The IMAP password, as a Secret reference.
        #[schemars(extend("arrived_in" = "0.1"))]
        password: String,
        /// The mailbox to examine; defaults to `"INBOX"`.
        #[serde(default = "default_imap_mailbox")]
        #[schemars(extend("arrived_in" = "0.1"))]
        mailbox: String,
        /// Whether to connect over TLS; defaults to `true` except on loopback
        /// non-993 ports when unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        tls: Option<bool>,
    },
    /// Gmail API messages read using OAuth credentials.
    Gmail {
        /// The Gmail OAuth credentials (bearer token or JSON with
        /// `access_token`), as a Secret reference.
        #[schemars(extend("arrived_in" = "0.1"))]
        credentials: String,
        /// Optional Gmail API base URL override; defaults to
        /// `https://gmail.googleapis.com`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        endpoint: Option<String>,
    },
    /// Microsoft Graph mail API using client-credentials OAuth.
    MicrosoftGraph {
        /// The Microsoft Entra tenant ID or domain.
        #[schemars(extend("arrived_in" = "0.1"))]
        tenant: String,
        /// The application (client) ID, as a Secret reference.
        #[schemars(extend("arrived_in" = "0.1"))]
        client_id: String,
        /// The application client secret, as a Secret reference.
        #[schemars(extend("arrived_in" = "0.1"))]
        client_secret: String,
        /// The mailbox user principal name or ID.
        #[schemars(extend("arrived_in" = "0.1"))]
        mailbox: String,
        /// Optional Graph API base URL override; defaults to
        /// `https://graph.microsoft.com`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        endpoint: Option<String>,
        /// Optional Entra token authority base URL override; defaults to
        /// `https://login.microsoftonline.com`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        authority: Option<String>,
    },
    /// A local HTTP webhook path receiving posted email or SMS payloads.
    Webhook {
        /// The HTTP path (starting with `/`) that accepts posted code payloads.
        #[schemars(extend("arrived_in" = "0.1"))]
        path: String,
        /// Optional loopback listen address (`host:port`) for the webhook
        /// receiver.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        listen: Option<String>,
    },
    /// A custom HTTP endpoint polled for messages.
    Endpoint {
        /// The HTTP(S) URL to poll.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: String,
        /// Additional request headers, each value a Secret reference.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        #[schemars(extend("arrived_in" = "0.1"))]
        headers: BTreeMap<String, String>,
    },
    /// Twilio SMS Messages API.
    Twilio {
        /// The Twilio Account SID, as a Secret reference.
        #[schemars(extend("arrived_in" = "0.1"))]
        account_sid: String,
        /// The Twilio Auth Token, as a Secret reference.
        #[schemars(extend("arrived_in" = "0.1"))]
        auth_token: String,
        /// The destination phone number receiving the SMS.
        #[schemars(extend("arrived_in" = "0.1"))]
        to: String,
        /// Optional Twilio API base URL override; defaults to
        /// `https://api.twilio.com`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        endpoint: Option<String>,
    },
}

impl CodeSource {
    /// Parses and validates one `code_sources.<name>` entry.
    ///
    /// # Errors
    ///
    /// A message explaining what is invalid, for `setting_invalid`.
    pub fn read(value: &Value) -> Result<Self, String> {
        let source: Self = serde_json::from_value(value.clone()).map_err(|error| {
            format!("expected a code source `{{ kind, from?, pattern?, window? }}`: {error}")
        })?;
        source.validate()?;
        Ok(source)
    }

    fn validate(&self) -> Result<(), String> {
        if self.window == 0 {
            return Err("`window` must be at least 1 millisecond".to_owned());
        }
        if let Some(from) = &self.from
            && from.trim().is_empty()
        {
            return Err("`from` must not be empty when set".to_owned());
        }
        if let Some(pattern) = &self.pattern
            && pattern.is_empty()
        {
            return Err("`pattern` must not be empty when set".to_owned());
        }
        self.kind.validate()
    }
}

impl CodeSourceKind {
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Imap {
                host,
                port,
                username,
                password,
                mailbox,
                ..
            } => {
                if host.trim().is_empty() {
                    return Err("`imap.host` must not be empty".to_owned());
                }
                if *port == 0 {
                    return Err("`imap.port` must be non-zero".to_owned());
                }
                if mailbox.trim().is_empty() {
                    return Err("`imap.mailbox` must not be empty".to_owned());
                }
                check_ref("imap.username", username)?;
                check_ref("imap.password", password)
            }
            Self::Gmail {
                credentials,
                endpoint,
            } => {
                check_ref("gmail.credentials", credentials)?;
                if let Some(url) = endpoint {
                    check_http_url("gmail.endpoint", url)?;
                }
                Ok(())
            }
            Self::MicrosoftGraph {
                tenant,
                client_id,
                client_secret,
                mailbox,
                endpoint,
                authority,
            } => {
                if tenant.trim().is_empty() {
                    return Err("`microsoft_graph.tenant` must not be empty".to_owned());
                }
                if mailbox.trim().is_empty() {
                    return Err("`microsoft_graph.mailbox` must not be empty".to_owned());
                }
                check_ref("microsoft_graph.client_id", client_id)?;
                check_ref("microsoft_graph.client_secret", client_secret)?;
                if let Some(url) = endpoint {
                    check_http_url("microsoft_graph.endpoint", url)?;
                }
                if let Some(url) = authority {
                    check_http_url("microsoft_graph.authority", url)?;
                }
                Ok(())
            }
            Self::Webhook { path, .. } => {
                if !path.starts_with('/') {
                    return Err("`webhook.path` must start with `/`".to_owned());
                }
                Ok(())
            }
            Self::Endpoint { url, headers } => {
                check_http_url("endpoint.url", url)?;
                for (name, value) in headers {
                    if name.trim().is_empty() {
                        return Err("`endpoint.headers` must not contain an empty name".to_owned());
                    }
                    check_ref(&format!("endpoint.headers.{name}"), value)?;
                }
                Ok(())
            }
            Self::Twilio {
                account_sid,
                auth_token,
                to,
                endpoint,
            } => {
                check_ref("twilio.account_sid", account_sid)?;
                check_ref("twilio.auth_token", auth_token)?;
                if to.trim().is_empty() {
                    return Err("`twilio.to` must not be empty".to_owned());
                }
                if let Some(url) = endpoint {
                    check_http_url("twilio.endpoint", url)?;
                }
                Ok(())
            }
        }
    }
}

fn check_ref(field: &str, value: &str) -> Result<(), String> {
    SecretReference::parse(value).map(drop).map_err(|error| {
        format!(
            "`{field}` must be a Secret reference such as `env:NAME`: {error}; a raw value is never taken"
        )
    })
}

fn check_http_url(field: &str, url: &str) -> Result<(), String> {
    if url.starts_with("https://") || url.starts_with("http://") {
        Ok(())
    } else {
        Err(format!("`{field}` must be an `http://` or `https://` URL"))
    }
}

impl crate::catalog::SettingJson for CodeSource {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// One entry of `code_sources`.
pub const CODE_SOURCE_SHAPE: ValueShape = ValueShape {
    name: "CodeSource",
    check: |value| CodeSource::read(value).map(drop),
    schema: || {
        let schema = SchemaSettings::draft2020_12()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<CodeSource>();
        let mut schema = serde_json::to_value(schema).expect("invariant: a JSON Schema is JSON");
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            object.remove("title");
        }
        schema
    },
    examples: || {
        vec![
            json!({
                "kind": {
                    "type": "imap",
                    "host": "imap.example.test",
                    "port": 993,
                    "username": "env:IMAP_USER",
                    "password": "env:IMAP_PASS",
                    "mailbox": "INBOX",
                    "tls": true,
                },
                "from": "noreply@example.test",
                "pattern": "\\b(\\d{6})\\b",
                "window": 300_000,
            }),
            json!({
                "kind": {
                    "type": "gmail",
                    "credentials": "env:GMAIL_TOKEN",
                    "endpoint": "https://gmail.googleapis.com",
                },
            }),
            json!({
                "kind": {
                    "type": "microsoft_graph",
                    "tenant": "example-tenant",
                    "client_id": "env:GRAPH_CLIENT_ID",
                    "client_secret": "env:GRAPH_CLIENT_SECRET",
                    "mailbox": "codes@example.test",
                    "endpoint": "https://graph.microsoft.com",
                    "authority": "https://login.microsoftonline.com",
                },
            }),
            json!({
                "kind": {
                    "type": "webhook",
                    "path": "/codes/example",
                    "listen": "127.0.0.1:0",
                },
            }),
            json!({
                "kind": {
                    "type": "endpoint",
                    "url": "https://codes.example.test/latest",
                    "headers": { "Authorization": "env:CODES_AUTH" },
                },
            }),
            json!({
                "kind": {
                    "type": "twilio",
                    "account_sid": "env:TWILIO_SID",
                    "auth_token": "env:TWILIO_TOKEN",
                    "to": "+15550001111",
                    "endpoint": "https://api.twilio.com",
                },
            }),
        ]
    },
};

fn no_code_source() -> Value {
    json!({})
}

/// `code_sources` (ADRs 0006 and 0092).
pub const CODE_SOURCES: Setting = Setting {
    name: "code_sources",
    description: "Email or SMS two-factor code sources, as `code_sources.<name>`: `{ kind, from?, pattern?, window? }`, where `kind` is one of `imap`, `gmail`, `microsoft_graph`, `webhook`, `endpoint` or `twilio` with every credential a Secret reference, and `window` defaults to 5 minutes (`300000` ms) from the login attempt. Bound to a placeholder via `secrets.placeholders.<name> = { code_source, domains }`. Each matching code is used once; when none arrives, a Human pause is raised unless `session.recovery.human_pause` is `false`, in which case the call fails with `secret_unavailable`. Only configuration, Operator policy and the program's own run layer set `code_sources`: a Recipe or a Client acting for a model receives `setting_locked`.",
    kind: ValueKind::Map(&ValueKind::Shape(&CODE_SOURCE_SHAPE)),
    default: no_code_source,
    clearable: false,
    authority: Authority::NotByRecipeOrModel,
    weakens: None,
    fixed_for_run: false,
    unbuilt: None,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
