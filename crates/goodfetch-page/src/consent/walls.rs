//! Consent walls (ADR 0048): who approved accepting one, and the walls the
//! runner left, which a page's next call raises as `blocked` with
//! `consent_required`. Pure, so a domain's approval is tested without a
//! browser.
#![expect(
    clippy::disallowed_types,
    reason = "the walls table's one lock guards one insert or removal, never held across another lock or an `.await`, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use goodfetch_protocol::page::PageId;
use goodfetch_protocol::settings::SettingsLayer;
use serde_json::Value;

use crate::interception::domain_key;

/// The approval to accept a consent wall: `consent.domains`, as the Run
/// resolved it, and the layer that set it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Approval {
    /// The registrable domains, lowercase, or `*`, written explicitly.
    domains: Vec<String>,
    /// The layer whose value the Run resolved.
    pub(crate) layer: SettingsLayer,
}

impl Approval {
    /// The approval `value`, `consent.domains` as `layer` set it; none for
    /// an empty list, which approves nothing.
    pub(crate) fn of(value: &Value, layer: SettingsLayer) -> Option<Self> {
        let domains: Vec<String> = value
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(|domain| domain.trim_end_matches('.').to_ascii_lowercase())
            .collect();
        (!domains.is_empty()).then_some(Self { domains, layer })
    }

    /// The domain of the document at `url`, as ADR 0004 keys it, when the
    /// approval covers it.
    pub(crate) fn covers(&self, url: &str) -> Option<String> {
        let domain = domain_key(url)?;
        self.domains
            .iter()
            .any(|approved| approved == "*" || *approved == domain)
            .then_some(domain)
    }
}

/// A consent wall the runner left in a page's main frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Wall {
    /// The browser's ID of the load of the document it is in.
    pub(crate) loader: String,
    /// The wall test's rule that found it.
    pub(crate) rule: &'static str,
}

/// The walls each page's runner left for its next call. Clones share them.
#[derive(Clone, Default)]
pub(crate) struct Walls(Arc<Mutex<BTreeMap<PageId, Wall>>>);

impl Walls {
    /// The page `page` shows `wall`.
    pub(crate) fn put(&self, page: PageId, wall: Wall) {
        self.lock().insert(page, wall);
    }

    /// The wall the page `page` showed, taken, so it is raised once.
    pub(crate) fn take(&self, page: &PageId) -> Option<Wall> {
        self.lock().remove(page)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<PageId, Wall>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests;
