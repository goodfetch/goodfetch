use rstest::rstest;

use super::{LABEL_SETTINGS, of};
use crate::catalog::SettingScope::{Connection, Label, Page, Run, Session};
use crate::catalog::{Setting, SettingScope};
use crate::entries::MODULES;
use crate::foxhound::fetch::{FETCH_TIER, FETCH_TIER_RACE};
use crate::foxhound::settings::POLITENESS_DOMAIN_CONCURRENCY;
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
#[case::the_extractor_applies_to_one_page(&CONTENT_EXTRACTOR, &[Connection, Run, Label, Session, Page])]
#[case::a_clean_up_choice_applies_to_a_session(&FILTER_LISTS_ADS, &[Connection, Run, Session])]
#[case::the_page_check_applies_to_a_session(&SESSION_PAGE_CHECK, &[Connection, Run, Label, Session])]
#[case::the_access_mode_is_chosen_as_a_session_opens(&BROWSER_ACCESS_MODE, &[Connection, Run, Session])]
#[case::an_identity_item_is_chosen_as_a_session_opens(STEALTH_ITEMS[0], &[Connection, Run, Session])]
#[case::a_signing_key_is_chosen_as_a_session_opens(&IDENTIFIED_SIGNING_KEY, &[Connection, Run, Session])]
#[case::a_session_proxy_is_chosen_as_it_opens(&PROXY_SERVER, &[Connection, Run, Session])]
#[case::a_session_source_is_its_own_argument(&SESSION_SOURCE, &[Connection, Run, Label])]
#[case::a_label_forces_its_own_tier(&FETCH_TIER, &[Connection, Run, Label])]
#[case::the_tier_race_is_the_runs_alone(&FETCH_TIER_RACE, &[Connection, Run])]
#[case::politeness_is_the_runs_alone(&POLITENESS_DOMAIN_CONCURRENCY, &[Connection, Run])]
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
        // A Label is a crawl's scope beside the Session and the page, not
        // around them: what it sets, its requests take.
        let scopes: Vec<SettingScope> = of(setting)
            .iter()
            .copied()
            .filter(|scope| *scope != Label)
            .collect();
        if let Some(innermost) = scopes.last() {
            let from_connection: Vec<SettingScope> = [Connection, Run, Session, Page]
                .into_iter()
                .filter(|scope| scope <= innermost)
                .collect();
            if setting.name != TIMEOUT_RUN.name {
                assert_eq!(scopes, from_connection, "{}", setting.name);
            }
        }
    }
}

#[test]
fn a_label_sets_exactly_the_label_settings_and_its_run_sets_them_too() {
    for setting in every_setting() {
        let scopes = of(setting);
        let listed = LABEL_SETTINGS
            .iter()
            .any(|label| label.name == setting.name);
        assert_eq!(scopes.contains(&Label), listed, "{}", setting.name);
        if listed {
            assert!(scopes.contains(&Run), "{}", setting.name);
        }
    }
}
