//! The scopes a program's code sets each Setting at (ADR 0083): one
//! `settings` option on `connect()`, starting a Run, declaring a crawl's
//! Label, opening a Session and opening a page, each taking the settings
//! that apply within it. The catalog records each setting's scopes from
//! this rule, the SDKs type each scope's option from the catalog, and the
//! engine refuses a scope's own setting outside its scopes with
//! `setting_not_applicable`, as the scope opens.

use crate::catalog::SettingScope::{Connection, Label, Page, Run, Session};
use crate::catalog::{Authority, Setting, SettingScope};
use crate::foxhound::crawl::{CRAWL_CONTENT_TYPES, CRAWL_RETRY_HANDLER_FAILED};
use crate::foxhound::fetch::{
    FETCH_ENCODING, FETCH_HTTP_FROM, FETCH_KEPT_RESPONSES_URLS, FETCH_TIER,
};
use crate::foxhound::frontier::CRAWL_KEEP_URL_FRAGMENT;
use crate::page::access_mode::{BROWSER_ACCESS_MODE, IDENTIFIED_ITEMS, STEALTH_ITEMS};
use crate::page::{
    BROWSER_BLOCK_RESOURCES, BROWSER_SERVICE_WORKERS, CONTENT_EXTRACTOR, LIVE_VIEW_HIDING,
};
use crate::settings::{
    FILTER_LISTS_ADS, FILTER_LISTS_ANNOYANCES, FILTER_LISTS_FIRST_PARTY_SCRIPTS,
    FILTER_LISTS_TRACKERS, SESSION_PAGE_CHECK, SESSION_SOURCE, TIMEOUT_HOOK, TIMEOUT_RUN,
};

/// What starts the `proxy.*` keys, which a Session chooses as it opens.
const PROXY_PREFIX: &str = "proxy.";

/// The settings a Session's own `settings` set beside its Run's, besides
/// what it keeps for its life ([`fixed_for_the_session`]): the Clean-up
/// rules' choices, whether its sites' service workers run, whether its Live
/// view hides, and its page check (ADR 0121).
const SESSION_SETTINGS: [&Setting; 8] = [
    &FILTER_LISTS_ADS,
    &FILTER_LISTS_TRACKERS,
    &FILTER_LISTS_ANNOYANCES,
    &FILTER_LISTS_FIRST_PARTY_SCRIPTS,
    &BROWSER_BLOCK_RESOURCES,
    &BROWSER_SERVICE_WORKERS,
    &LIVE_VIEW_HIDING,
    &SESSION_PAGE_CHECK,
];

/// The settings a crawl's Label sets over its Run's for its requests (ADR
/// 0093): its fetch tier, where its HTTP leaves from, its encoding, the
/// responses it keeps for its handler, the content types it takes, whether
/// it keeps its URLs' fragments (ADR 0120), whether a failed handler is
/// tried again, its main-content step, its Hooks' time, its page check and
/// its Session source, which gives it a Session pool of its own. A Label
/// is refused every other key: the Run's proxy, Politeness and bounds
/// always, and the rest of ADR 0093's Label settings until this engine
/// applies them (issue 1472).
pub const LABEL_SETTINGS: [&Setting; 11] = [
    &FETCH_TIER,
    &FETCH_HTTP_FROM,
    &FETCH_ENCODING,
    &FETCH_KEPT_RESPONSES_URLS,
    &CRAWL_CONTENT_TYPES,
    &CRAWL_KEEP_URL_FRAGMENT,
    &CRAWL_RETRY_HANDLER_FAILED,
    &CONTENT_EXTRACTOR,
    &TIMEOUT_HOOK,
    &SESSION_PAGE_CHECK,
    &SESSION_SOURCE,
];

/// The scopes whose own `settings` may set `setting`, outermost first:
/// none for a setting only configuration or an Operator policy sets, the
/// connection alone for `timeout.run`, which starting a Run takes as its
/// `timeout` (ADR 0080), the connection and a Run for `session.source`,
/// which opening a Session takes as its `source` (ADR 0092), down to the
/// Session for what a Session sets for itself, and down to the page for
/// the content extractor; the connection and a Run for the rest. A crawl's
/// Label also sets [`LABEL_SETTINGS`].
pub fn of(setting: &Setting) -> &'static [SettingScope] {
    let name = setting.name;
    let label = label_sets(name);
    if !set_by_code(setting.authority) {
        &[]
    } else if name == TIMEOUT_RUN.name {
        &[Connection]
    } else if name == CONTENT_EXTRACTOR.name {
        &[Connection, Run, Label, Session, Page]
    } else if SESSION_SETTINGS.iter().any(|setting| setting.name == name)
        || fixed_for_the_session(name)
    {
        if label {
            &[Connection, Run, Label, Session]
        } else {
            &[Connection, Run, Session]
        }
    } else if label {
        &[Connection, Run, Label]
    } else {
        &[Connection, Run]
    }
}

/// Whether a crawl's Label sets the setting `key` names (ADR 0093).
pub fn label_sets(key: &str) -> bool {
    LABEL_SETTINGS.iter().any(|setting| setting.name == key)
}

/// Whether the key `name` is one of what a Session keeps for its life,
/// which only its opening sets (ADR 0021; pitfall S05): its Access mode, an
/// item of its Identity profile, its signing key and key directory, or its
/// proxy.
pub fn fixed_for_the_session(name: &str) -> bool {
    name == BROWSER_ACCESS_MODE.name
        || STEALTH_ITEMS.iter().any(|item| item.name == name)
        || IDENTIFIED_ITEMS.iter().any(|item| item.name == name)
        || name.starts_with(PROXY_PREFIX)
}

/// Whether a program's own code may set a setting of `authority` at all,
/// rather than only configuration, an Operator policy or nothing.
fn set_by_code(authority: Authority) -> bool {
    match authority {
        Authority::Configuration
        | Authority::ConfigurationProjectLimited(_)
        | Authority::OperatorPolicy
        | Authority::OperatorPolicyAlone
        | Authority::Machine
        | Authority::ProjectFileAlone
        | Authority::Fixed => false,
        Authority::Every
        | Authority::Narrowed
        | Authority::ConfigurationExcept(_)
        | Authority::NotByRecipe
        | Authority::NotByRecipeOrModel
        | Authority::NotByModel
        | Authority::NotByModelRecipeAllowedBy(_)
        | Authority::HostsNarrowedByRecipeOrModel(_)
        | Authority::NotWeakenedByRecipeOrModel => true,
    }
}

#[cfg(test)]
mod tests;
