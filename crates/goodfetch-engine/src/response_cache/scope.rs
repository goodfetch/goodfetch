//! Wall-allowance and consent-acceptance scoping for Response cache entries
//! (ADRs 0046, 0048 and 0052, issue 803): what a person approved for one Run
//! stays with Runs whose settings cover it, never leaks to a denied domain,
//! and never travels with a Saved identity outside the saving Run's
//! `consent.domains` or Wall allowance.

use goodfetch_protocol::settings::{WALL_DENIED_DOMAINS, WALL_DOMAINS, WALL_REFETCH_DOMAINS};
use serde_json::Value;

use crate::host_name;
use crate::settings::Resolved;

/// Normalized registrable-domain allow/deny list (`host_name::lookup_key` or `*`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainList(Vec<String>);

impl DomainList {
    /// Parses a setting's JSON array of domains.
    #[must_use]
    pub fn of(value: &Value) -> Self {
        Self(
            value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(host_name::lookup_key)
                .collect(),
        )
    }

    /// Whether `domain` is listed or covered by `*`.
    #[must_use]
    pub fn covers(&self, domain: &str) -> bool {
        let key = host_name::lookup_key(domain);
        self.0.iter().any(|listed| listed == "*" || listed == &key)
    }
}

/// A Run's Wall allowance as the Response cache checks it (ADR 0052): built
/// once from the Run's resolved settings so every lookup checks in memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WallAllowance {
    wall: DomainList,
    refetch: DomainList,
    denied: DomainList,
}

impl WallAllowance {
    /// Builds the allowance from a Run's resolved settings.
    #[must_use]
    pub fn of(settings: &Resolved) -> Self {
        Self {
            wall: DomainList::of(settings.value(&WALL_DOMAINS)),
            refetch: DomainList::of(settings.value(&WALL_REFETCH_DOMAINS)),
            denied: DomainList::of(settings.value(&WALL_DENIED_DOMAINS)),
        }
    }

    /// Builds an allowance directly from domain lists, normalizing each host
    /// as `host_name::lookup_key` does.
    #[must_use]
    pub fn from_lists(wall: &[&str], refetch: &[&str], denied: &[&str]) -> Self {
        let to_domains = |list: &[&str]| {
            DomainList::of(&Value::Array(
                list.iter()
                    .map(|d| Value::String((*d).to_owned()))
                    .collect(),
            ))
        };
        Self {
            wall: to_domains(wall),
            refetch: to_domains(refetch),
            denied: to_domains(denied),
        }
    }

    /// Whether this Run may be served a cached entry fetched under a Wall
    /// allowance for `domain` (and, when `refetch` is true, by a Wall refetch).
    /// A denied domain is never served, even under `*`.
    #[must_use]
    pub fn allows(&self, domain: &str, refetch: bool) -> bool {
        if self.denied.covers(domain) {
            return false;
        }
        if !self.wall.covers(domain) {
            return false;
        }
        if refetch && !self.refetch.covers(domain) {
            return false;
        }
        true
    }
}

/// Approval tags carried by a Response cache entry (ADRs 0048 and 0052).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryScope {
    /// Registrable domain when fetched under a Wall allowance (`wall.domains`).
    pub wall_domain: Option<String>,
    /// True when fetched by a Wall refetch (`wall.refetch_domains`).
    pub wall_refetch: bool,
    /// Registrable domain when fetched under a consent acceptance (`consent.domains`).
    pub consent_domain: Option<String>,
}

impl EntryScope {
    /// An entry fetched under a Wall allowance for `domain`.
    #[must_use]
    pub fn wall(domain: &str) -> Self {
        Self {
            wall_domain: Some(host_name::lookup_key(domain)),
            wall_refetch: false,
            consent_domain: None,
        }
    }

    /// An entry fetched by a Wall refetch for `domain`.
    #[must_use]
    pub fn wall_refetch(domain: &str) -> Self {
        Self {
            wall_domain: Some(host_name::lookup_key(domain)),
            wall_refetch: true,
            consent_domain: None,
        }
    }

    /// An entry fetched under a consent acceptance for `domain`.
    #[must_use]
    pub fn consent(domain: &str) -> Self {
        Self {
            wall_domain: None,
            wall_refetch: false,
            consent_domain: Some(host_name::lookup_key(domain)),
        }
    }

    /// Whether a Run with `allowance` may be served this entry.
    #[must_use]
    pub(super) fn allowed_by(&self, allowance: &WallAllowance) -> bool {
        self.wall_domain
            .as_deref()
            .is_none_or(|domain| allowance.allows(domain, self.wall_refetch))
    }

    /// Whether this entry survives saving a Saved identity under a Run whose
    /// `consent.domains` is `consent` and whose Wall allowance is `wall`
    /// (ADRs 0048 and 0052).
    #[must_use]
    pub(super) fn survives_identity_save(
        &self,
        consent: &DomainList,
        wall: &WallAllowance,
    ) -> bool {
        if let Some(domain) = self.consent_domain.as_deref()
            && !consent.covers(domain)
        {
            return false;
        }
        self.allowed_by(wall)
    }
}
