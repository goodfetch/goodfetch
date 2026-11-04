//! What the Kennel does once its settings are read at launch: the
//! hidden-icon check (macOS), which opens the details window with a note
//! when the icon is hidden, and start at login, which follows its setting.

use super::super::{Effect, IconCheck, Input, State};
use crate::settings::{Settings, SettingsError};
use crate::window::Opening;

fn read(settings: Settings) -> Input {
    Input::Settings(Ok(settings))
}

#[test]
fn once_the_settings_are_read_the_kennel_checks_its_icon_once() {
    let mut state = State::new();

    let first = state.apply(read(Settings::default()));
    let again = state.apply(read(Settings::default()));

    assert!(first.contains(&Effect::CheckIcon), "{first:?}");
    assert!(!again.contains(&Effect::CheckIcon), "{again:?}");
}

#[test]
fn a_hidden_icon_opens_the_details_window_with_its_note() {
    let mut state = State::new();
    state.apply(read(Settings::default()));

    let effects = state.apply(Input::IconChecked(IconCheck::Hidden));

    assert_eq!(effects, [Effect::OpenDetails(Opening::HiddenIcon)]);
}

#[test]
fn a_shown_icon_opens_nothing() {
    let mut state = State::new();
    state.apply(read(Settings::default()));

    let effects = state.apply(Input::IconChecked(IconCheck::Shown));

    assert_eq!(effects, []);
}

#[test]
fn with_the_check_off_the_icon_is_never_checked() {
    let mut state = State::new();

    let effects = state.apply(read(Settings {
        hidden_icon_check: false,
        ..Settings::default()
    }));

    assert!(!effects.contains(&Effect::CheckIcon), "{effects:?}");
}

#[test]
fn a_check_turned_off_before_its_reading_opens_nothing() {
    let mut state = State::new();
    state.apply(read(Settings::default()));
    state.apply(read(Settings {
        hidden_icon_check: false,
        ..Settings::default()
    }));

    let effects = state.apply(Input::IconChecked(IconCheck::Hidden));

    assert_eq!(effects, []);
}

#[test]
fn a_menu_only_kennel_checks_nothing_since_it_has_no_window_to_open() {
    let mut state = State::new();

    let effects = state.apply(read(Settings {
        window: false,
        ..Settings::default()
    }));

    assert!(!effects.contains(&Effect::CheckIcon), "{effects:?}");
}

#[test]
fn an_unreadable_settings_file_gives_the_default_check() {
    let mut state = State::new();

    let effects = state.apply(Input::Settings(Err(SettingsError::Unreadable(
        "expected a boolean".to_owned(),
    ))));

    assert_eq!(effects, [Effect::CheckIcon]);
}

#[test]
fn a_hidden_icon_while_the_window_is_open_opens_no_second_one() {
    let mut state = State::new();
    state.apply(read(Settings::default()));
    state.choose(super::super::Choice::OpenDetails);

    let effects = state.apply(Input::IconChecked(IconCheck::Hidden));

    assert_eq!(effects, []);
}

#[test]
fn start_at_login_follows_its_setting_from_the_first_read() {
    let mut state = State::new();
    state.apply(read(Settings {
        hidden_icon_check: false,
        ..Settings::default()
    }));
    let on = |on| Settings {
        hidden_icon_check: false,
        start_at_login: on,
        ..Settings::default()
    };

    let turned_on = state.apply(read(on(true)));
    let unchanged = state.apply(read(on(true)));
    let turned_off = state.apply(read(on(false)));

    assert_eq!(turned_on, [Effect::StartAtLogin(true)]);
    assert_eq!(unchanged, []);
    assert_eq!(turned_off, [Effect::StartAtLogin(false)]);
}

#[test]
fn the_first_read_makes_the_login_agent_as_the_file_says_even_when_off() {
    let mut state = State::new();

    let effects = state.apply(read(Settings {
        hidden_icon_check: false,
        ..Settings::default()
    }));

    // The person may have turned it off in the file while the Kennel was
    // not running.
    assert_eq!(effects, [Effect::StartAtLogin(false)]);
}

#[test]
fn an_unreadable_settings_file_leaves_the_login_agent_as_it_is() {
    let mut state = State::new();
    state.apply(read(Settings {
        hidden_icon_check: false,
        start_at_login: true,
        ..Settings::default()
    }));

    let effects = state.apply(Input::Settings(Err(SettingsError::Unreadable(
        "expected a boolean".to_owned(),
    ))));

    assert_eq!(effects, []);
}
