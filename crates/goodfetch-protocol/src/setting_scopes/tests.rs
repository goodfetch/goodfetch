use rstest::rstest;

use super::of;
use crate::catalog::SettingScope::{Connection, Page, Run, Session};
use crate::catalog::{Setting, SettingScope};
use crate::entries::MODULES;
use crate::page::CONTENT_EXTRACTOR;
use crate::page::access_mode::{BROWSER_ACCESS_MODE, IDENTIFIED_SIGNING_KEY, STEALTH_ITEMS};
use crate::page::proxy::{PROXY_PROVIDERS, PROXY_SERVER};
use crate::settings::{
    FILTER_LISTS_ADS, SESSION_PAGE_CHECK, SESSION_SOURCE, TIMEOUT_NAVIGATION, TIMEOUT_RUN,
};

/// Every Setting the catalog declares.
fn every_setting() -> impl Iterator<Item = &'static Setting> {
    MODULES.iter().flat_map(|entries| entries.settings.iter())
}

#[rstest]
#[case::the_extractor_applies_to_one_page(&CONTENT_EXTRACTOR, &[Connection, Run, Session, Page])]
#[case::a_clean_up_choice_applies_to_a_session(&FILTER_LISTS_ADS, &[Connection, Run, Session])]
#[case::the_page_check_applies_to_a_session(&SESSION_PAGE_CHECK, &[Connection, Run, Session])]
#[case::the_access_mode_is_chosen_as_a_session_opens(&BROWSER_ACCESS_MODE, &[Connection, Run, Session])]
#[case::an_identity_item_is_chosen_as_a_session_opens(STEALTH_ITEMS[0], &[Connection, Run, Session])]
#[case::a_signing_key_is_chosen_as_a_session_opens(&IDENTIFIED_SIGNING_KEY, &[Connection, Run, Session])]
#[case::a_session_proxy_is_chosen_as_it_opens(&PROXY_SERVER, &[Connection, Run, Session])]
#[case::a_session_source_is_its_own_argument(&SESSION_SOURCE, &[Connection, Run])]
#[case::a_runs_own_timeout_is_its_argument(&TIMEOUT_RUN, &[Connection])]
#[case::a_run_wide_setting(&TIMEOUT_NAVIGATION, &[Connection, Run])]
#[case::configuration_alone_sets_the_providers(&PROXY_PROVIDERS, &[])]
fn a_setting_applies_to_the_scopes_adr_0083_gives_it(
    #[case] setting: &Setting,
    #[case] expected: &[SettingScope],
) {
    assert_eq!(of(setting), expected, "{}", setting.name);
}

#[test]
fn a_scope_that_sets_a_setting_has_its_enclosing_scopes_set_it_too() {
    for setting in every_setting() {
        let scopes = of(setting);
        if let Some(innermost) = scopes.last() {
            let from_connection: Vec<SettingScope> = [Connection, Run, Session, Page]
                .into_iter()
                .filter(|scope| scope <= innermost)
                .collect();
            if setting.name != TIMEOUT_RUN.name {
                assert_eq!(scopes, from_connection.as_slice(), "{}", setting.name);
            }
        }
    }
}
