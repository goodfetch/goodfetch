//! The scopes a program's code sets each Setting at (ADR 0083): one
//! `settings` option on `connect()`, starting a Run, opening a Session and
//! opening a page, each taking the settings that apply within it. The
//! engine refuses a setting outside its scopes with
//! `setting_not_applicable`, and the SDKs type each scope's option from
//! this list.

use crate::catalog::SettingScope::{Connection, Page, Run, Session};
use crate::catalog::{Authority, Setting, SettingScope};
use crate::page::access_mode::{BROWSER_ACCESS_MODE, IDENTIFIED_ITEMS, STEALTH_ITEMS};
use crate::page::{
    BROWSER_BLOCK_RESOURCES, BROWSER_SERVICE_WORKERS, CONTENT_EXTRACTOR, LIVE_VIEW_HIDING,
};
use crate::settings::{
    FILTER_LISTS_ADS, FILTER_LISTS_ANNOYANCES, FILTER_LISTS_FIRST_PARTY_SCRIPTS,
    FILTER_LISTS_TRACKERS, SESSION_PAGE_CHECK, SESSION_SOURCE, TIMEOUT_RUN,
};

/// What starts the `proxy.*` keys, which a Session chooses as it opens.
const PROXY_PREFIX: &str = "proxy.";

/// The settings a Session's own `settings` set beside its Run's: the
/// Clean-up rules' choices, whether its sites' service workers run, its
/// page check (ADR 0121), and what it keeps for its life, which only its
/// opening sets (ADR 0021): its Access mode and the items of its Identity
/// profile. Its proxy, the `proxy.*` keys, is the rest.
const SESSION_SETTINGS: [&Setting; 9] = [
    &FILTER_LISTS_ADS,
    &FILTER_LISTS_TRACKERS,
    &FILTER_LISTS_ANNOYANCES,
    &FILTER_LISTS_FIRST_PARTY_SCRIPTS,
    &BROWSER_BLOCK_RESOURCES,
    &BROWSER_SERVICE_WORKERS,
    &LIVE_VIEW_HIDING,
    &SESSION_PAGE_CHECK,
    &BROWSER_ACCESS_MODE,
];

/// The scopes whose own `settings` may set `setting`, outermost first:
/// none for a setting only configuration or an Operator policy sets, the
/// connection alone for `timeout.run`, which starting a Run takes as its
/// `timeout` (ADR 0080), the connection and a Run for `session.source`,
/// which opening a Session takes as its `source` (ADR 0092), down to the
/// Session for what a Session sets for itself, and down to the page for
/// the content extractor; the connection and a Run for the rest.
pub fn of(setting: &Setting) -> &'static [SettingScope] {
    let outside_code = matches!(
        setting.authority,
        Authority::Configuration
            | Authority::ConfigurationProjectLimited(_)
            | Authority::OperatorPolicy
            | Authority::OperatorPolicyAlone
            | Authority::Machine
            | Authority::Fixed
    );
    let name = setting.name;
    if outside_code {
        &[]
    } else if name == TIMEOUT_RUN.name {
        &[Connection]
    } else if name == CONTENT_EXTRACTOR.name {
        &[Connection, Run, Session, Page]
    } else if name == SESSION_SOURCE.name {
        &[Connection, Run]
    } else if is_session_setting(name) {
        &[Connection, Run, Session]
    } else {
        &[Connection, Run]
    }
}

/// Whether the key `name` is one a Session's own `settings` set.
fn is_session_setting(name: &str) -> bool {
    SESSION_SETTINGS.iter().any(|setting| setting.name == name)
        || STEALTH_ITEMS.iter().any(|item| item.name == name)
        || IDENTIFIED_ITEMS.iter().any(|item| item.name == name)
        || name.starts_with(PROXY_PREFIX)
}

#[cfg(test)]
mod tests;
