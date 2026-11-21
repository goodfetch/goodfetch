//! The site of the crawls whose pages do not matter to what they prove:
//! the Proving ground's plain hosts, where every path is a small page,
//! fetched on a forced `http` tier, so a test with no browser
//! crawls pages that answer (issue 1602: with nothing set, `auto` serves
//! every request on a tier, and no test crawls an unreachable host).

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

/// The Proving ground's seed.
const SEED: u64 = 1602;

/// A running Proving ground, and the engines that crawl it.
pub(crate) struct Site(ProvingGround);

impl Site {
    /// Starts the site.
    pub(crate) fn start(env: &TestEnv) -> Self {
        Self(ProvingGround::start(env, SEED).expect("the Proving ground starts"))
    }

    /// The URL of the page at `path` on `host`, one of the Proving
    /// ground's plain hosts, such as `shop.test`: over plain HTTP, on the
    /// port the Proving ground listens on.
    pub(crate) fn url(&self, host: &str, path: &str) -> String {
        self.0.url(host, path)
    }

    /// The URL of the page at `path` on `shop.test`.
    pub(crate) fn page(&self, path: &str) -> String {
        self.url("shop.test", path)
    }

    /// Fetches `url`, a page of the site, from outside any engine.
    pub(crate) fn fetch(&self, url: &str) -> crate::frontier::Fetched {
        crate::frontier::fetch(&self.0, url)
    }

    /// The settings of a crawl that reaches the site on a forced `http`
    /// tier.
    pub(crate) fn settings(&self) -> Value {
        crate::fetch_tiers::settings(&self.0, &json!({ "fetch": { "tier": "http" } }))
    }

    /// Foxhound's Attached engine as [`crate::attached`] starts it, whose
    /// crawls take [`Site::settings`] where they set none of their own.
    pub(crate) fn engine(&self, env: &TestEnv) -> Attached {
        engine_on(env, &self.0)
    }

    /// [`Site::engine`], on a Connection whose `hello` sets `settings`.
    pub(crate) fn engine_setting(&self, env: &TestEnv, settings: &Value) -> Attached {
        crate::attached_setting(env, settings).settings_by_default(&self.settings())
    }
}

/// Foxhound's Attached engine as [`crate::attached`] starts it, whose
/// crawls reach every host of `proving_ground` on a forced `http` tier
/// where they set none of their own: a handler that reads no page still
/// gets one, which a test with no browser has the `http` tier serve.
pub(crate) fn engine_on(env: &TestEnv, proving_ground: &ProvingGround) -> Attached {
    let settings =
        crate::fetch_tiers::settings(proving_ground, &json!({ "fetch": { "tier": "http" } }));
    crate::attached(env).settings_by_default(&settings)
}

/// The path of `url`, a page of the site, with its query: what a handler
/// tells its pages apart by, whatever port the site listens on.
pub(crate) fn path_of(url: &Value) -> &str {
    url.as_str()
        .and_then(goodfetch_test_support::path_of)
        .unwrap_or_default()
}

/// The HTML of the page the engine fetched for the request `handle`
/// hands over, as the site sent it: what a handler reads its links from,
/// sending nothing of its own.
pub(crate) fn html_of(inside: &mut crate::frontier::Inside<'_>, handle: &Value) -> String {
    let reply = inside.call(
        "page.formats",
        &json!({ "page_id": handle["page_id"], "formats": { "html": { "kind": "raw" } } }),
    );
    reply["result"]["html"]
        .as_str()
        .unwrap_or_else(|| panic!("no HTML: {reply}"))
        .to_owned()
}
