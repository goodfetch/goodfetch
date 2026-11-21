//! `session.page_check`'s value (ADRs 0006 and 0092), defined once: the
//! login page and the expected page every page of a Session is judged
//! against, each optional and each a page match the engine checks on the
//! document a navigation reached. A page matching its login page is a lost
//! login; one matching neither, when an expected page is given, fails its
//! Page check. Left unset, or `null`, ADR 0006's built-in detector judges
//! the Session's pages, which are weakly checked.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::catalog::ValueShape;

/// One page match (ADR 0092), named by `kind`: the document's URL against
/// a glob or a regular expression, as `page.route` takes them (ADR 0081),
/// an element a CSS or `XPath` selector finds in it, or a text its body
/// shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PageMatch {
    /// The document's URL matches a glob, as Playwright reads one.
    Glob {
        /// The glob.
        #[schemars(extend("arrived_in" = "0.1"))]
        pattern: String,
    },
    /// The document's URL matches a regular expression, in the `regex`
    /// crate's syntax, anywhere in it.
    Regex {
        /// The expression's source.
        #[schemars(extend("arrived_in" = "0.1"))]
        pattern: String,
    },
    /// An element of the main frame's document matches a CSS selector.
    Css {
        /// The selector.
        #[schemars(extend("arrived_in" = "0.1"))]
        selector: String,
    },
    /// An element of the main frame's document matches an `XPath`
    /// expression.
    Xpath {
        /// The expression.
        #[schemars(extend("arrived_in" = "0.1"))]
        selector: String,
    },
    /// The main frame's body shows a text, its runs of white space read as
    /// one space, case and all.
    Text {
        /// The text.
        #[schemars(extend("arrived_in" = "0.1"))]
        text: String,
    },
}

/// What `session.page_check` holds (ADR 0092): the Session's login page
/// and its expected page, each optional. Without a login page no page is
/// judged a login page; without an expected page every other page passes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageCheck {
    /// The page a site shows instead of the expected page once the
    /// Session's login is lost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub login_page: Option<PageMatch>,
    /// The page the Session's navigations should reach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub expected_page: Option<PageMatch>,
}

impl crate::catalog::SettingJson for PageCheck {
    fn into_setting_json(self) -> serde_json::Value {
        crate::catalog::shape_json(self)
    }
}

/// `session.page_check`'s values: a [`PageCheck`].
pub const PAGE_CHECK_SHAPE: ValueShape = ValueShape {
    name: "PageCheck",
    check: |value| {
        serde_json::from_value::<PageCheck>(value.clone())
            .map(drop)
            .map_err(|error| {
                format!(
                    "expected a Page check, `login_page` and `expected_page` each a page match of one `kind`: {error}"
                )
            })
    },
    schema: || {
        let schema = SchemaSettings::draft2020_12()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<PageCheck>();
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
                "login_page": { "kind": "css", "selector": "form#login" },
                "expected_page": { "kind": "glob", "pattern": "**/reports*" },
            }),
            json!({
                "login_page": { "kind": "text", "text": "Sign in" },
                "expected_page": { "kind": "xpath", "selector": "//main[@id='records']" },
            }),
            json!({
                "login_page": { "kind": "regex", "pattern": "/login$" },
                "expected_page": { "kind": "text", "text": "Your reports" },
            }),
        ]
    },
};
