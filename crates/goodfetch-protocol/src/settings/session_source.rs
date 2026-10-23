//! `session.source`'s tagged union (ADRs 0056 and 0092), defined once: where
//! a Session's login comes from, exactly one kind, named by `kind`. Left
//! unset, or `null`, a Session opens fresh. Each kind's `domains` are
//! registrable domains. `client_browser` is `run.borrow()`'s and never
//! written here. The engine serves each kind once the ticket of that kind
//! builds its reader; until then, a Session opening from it raises
//! `capability_unavailable` naming the kind.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::catalog::ValueShape;

/// A browser family, as ADR 0056 names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserName {
    /// Google Chrome.
    Chrome,
    /// Microsoft Edge.
    Edge,
    /// Brave.
    Brave,
    /// Chromium.
    Chromium,
    /// Firefox.
    Firefox,
    /// Safari.
    Safari,
}

/// The format of a `state_file`, detected when left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StateFileFormat {
    /// Playwright's `storageState`.
    StorageState,
    /// A Netscape `cookies.txt`.
    CookiesTxt,
    /// A CDP `Network.Cookie` array.
    CdpCookies,
    /// A session export bundle, opened with `key` (ADR 0015).
    SessionExport,
}

/// Where a Session's login comes from (ADR 0056): exactly one kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionSource {
    /// A Saved identity, by its name.
    SavedIdentity {
        /// Its name.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
    },
    /// A file of cookies and storage, read on the program's machine and
    /// sent over the connection.
    StateFile {
        /// The file.
        #[schemars(extend("arrived_in" = "0.1"))]
        path: String,
        /// Its format, detected when left out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        format: Option<StateFileFormat>,
        /// The Secret reference that opens a session export bundle.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        key: Option<String>,
        /// The registrable domains its navigation is bound to.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        domains: Option<Vec<String>>,
    },
    /// A `Cookie` header's cookies, for one domain.
    CookieHeader {
        /// The header's value.
        #[schemars(extend("arrived_in" = "0.1"))]
        header: String,
        /// Its domain.
        #[schemars(extend("arrived_in" = "0.1"))]
        domain: String,
    },
    /// A Profile directory, used in place.
    ProfileDirectory {
        /// The directory.
        #[schemars(extend("arrived_in" = "0.1"))]
        path: String,
        /// Its browser family.
        #[schemars(extend("arrived_in" = "0.1"))]
        browser: BrowserName,
        /// A Chromium-family profile within it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        chromium_profile: Option<String>,
        /// The registrable domains its navigation is bound to.
        #[schemars(extend("arrived_in" = "0.1"))]
        domains: Vec<String>,
    },
    /// The data of a browser the person installed, read once.
    InstalledBrowser {
        /// Its browser family.
        #[schemars(extend("arrived_in" = "0.1"))]
        browser: BrowserName,
        /// A Chromium-family profile of it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        chromium_profile: Option<String>,
        /// The registrable domains its data is read for.
        #[schemars(extend("arrived_in" = "0.1"))]
        domains: Vec<String>,
    },
    /// A browser the person runs, reached through its consented connection.
    RunningBrowser {
        /// Its browser family.
        #[schemars(extend("arrived_in" = "0.1"))]
        browser: BrowserName,
        /// The registrable domains its navigation is bound to.
        #[schemars(extend("arrived_in" = "0.1"))]
        domains: Vec<String>,
    },
    /// A browser at a CDP address (ADR 0115).
    CdpUrl {
        /// The address.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: String,
        /// Headers its connection sends, each value a Secret reference.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        headers: Option<BTreeMap<String, String>>,
        /// The registrable domains its navigation is bound to.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        domains: Option<Vec<String>>,
        /// Whether nothing else drives the browser (ADR 0114).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        exclusive: Option<bool>,
    },
}

impl SessionSource {
    /// Its kind's name, as `kind` writes it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::SavedIdentity { .. } => "saved_identity",
            Self::StateFile { .. } => "state_file",
            Self::CookieHeader { .. } => "cookie_header",
            Self::ProfileDirectory { .. } => "profile_directory",
            Self::InstalledBrowser { .. } => "installed_browser",
            Self::RunningBrowser { .. } => "running_browser",
            Self::CdpUrl { .. } => "cdp_url",
        }
    }
}

/// `session.source`'s values: a [`SessionSource`].
pub const SESSION_SOURCE_SHAPE: ValueShape = ValueShape {
    name: "SessionSource",
    check: |value| {
        serde_json::from_value::<SessionSource>(value.clone())
            .map(drop)
            .map_err(|error| {
                format!("expected a Session source, one `kind` of ADR 0056's: {error}")
            })
    },
    schema: || {
        let schema = SchemaSettings::draft2020_12()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<SessionSource>();
        let mut schema = serde_json::to_value(schema).expect("invariant: a JSON Schema is JSON");
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            object.remove("title");
        }
        schema
    },
    examples: || {
        vec![
            json!({ "kind": "saved_identity", "name": "example" }),
            json!({
                "kind": "state_file",
                "path": "state.json",
                "format": "storage_state",
                "key": "env:EXAMPLE",
                "domains": ["example.com"],
            }),
            json!({ "kind": "cookie_header", "header": "id=1", "domain": "example.com" }),
            json!({
                "kind": "profile_directory",
                "path": "profile",
                "browser": "chrome",
                "chromium_profile": "Default",
                "domains": ["example.com"],
            }),
            json!({
                "kind": "cdp_url",
                "url": "ws://127.0.0.1:9222/devtools/browser/example",
                "headers": { "authorization": "env:EXAMPLE" },
                "domains": ["example.com"],
                "exclusive": true,
            }),
        ]
    },
};
