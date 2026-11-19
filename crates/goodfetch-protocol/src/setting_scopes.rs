//! The scopes a program's code sets each Setting at (ADR 0083): one
//! `settings` option on `connect()`, starting a Run, opening a Session and
//! opening a page, each taking the settings that apply within it. The
//! catalog records each setting's scopes from this rule, the SDKs type each
//! scope's option from the catalog, and the engine refuses a Session's or a
//! page's own setting outside its scopes with `setting_not_applicable`.
//! Starting a Run does not check it yet: issue 1245 brings that with the
//! connection's own settings.

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

/// The scopes whose own `settings` may set `setting`, outermost first:
/// none for a setting only configuration or an Operator policy sets, the
/// connection alone for `timeout.run`, which starting a Run takes as its
/// `timeout` (ADR 0080), the connection and a Run for `session.source`,
/// which opening a Session takes as its `source` (ADR 0092), down to the
/// Session for what a Session sets for itself, and down to the page for
/// the content extractor; the connection and a Run for the rest.
pub fn of(setting: &Setting) -> &'static [SettingScope] {
    let name = setting.name;
    if !set_by_code(setting.authority) {
        &[]
    } else if name == TIMEOUT_RUN.name {
        &[Connection]
    } else if name == CONTENT_EXTRACTOR.name {
        &[Connection, Run, Session, Page]
    } else if name == SESSION_SOURCE.name {
        &[Connection, Run]
    } else if SESSION_SETTINGS.iter().any(|setting| setting.name == name)
        || fixed_for_the_session(name)
    {
        &[Connection, Run, Session]
    } else {
        &[Connection, Run]
    }
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
