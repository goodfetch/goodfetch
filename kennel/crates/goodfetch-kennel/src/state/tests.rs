//! The State module through its interface: inputs in, the view, the
//! redraws, the effects and the reads wanted out, with no toolkit and no
//! socket.

mod controls;
mod launch;
mod needs_you;
mod notices;
mod runs;

use std::path::PathBuf;

use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::{DaemonHealth, Unready};
use rstest::rstest;

use super::{Choice, Effect, Input, Row, State, Verdict};
use crate::client::{ConnectionChange, DaemonIdentity, Refusal};
use crate::daemon_start::{InstalledProduct, StartFailure};
use crate::settings::{Settings, SettingsError};
use crate::window::Opening;

fn foxhound_daemon() -> ConnectionChange {
    ConnectionChange::Connected(DaemonIdentity {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("0.4.1").unwrap(),
    })
}

fn installed() -> InstalledProduct {
    InstalledProduct {
        product: Product::Foxhound,
        path: PathBuf::from("/opt/goodfetch/bin/goodfetch-foxhound"),
    }
}

/// A State with the installed Product found at launch and no Daemon.
fn no_daemon() -> State {
    let mut state = State::new();
    state.apply(Input::Installed(Some(installed())));
    state.apply(Input::Connection(ConnectionChange::NoDaemon));
    state
}

fn texts(state: &State) -> Vec<String> {
    state
        .view()
        .rows
        .into_iter()
        .filter_map(|row| match row {
            Row::Text(text) => Some(text),
            Row::Detail(_) | Row::Choice { .. } | Row::Separator | Row::Submenu { .. } => None,
        })
        .collect()
}

fn choices(state: &State) -> Vec<(String, Choice)> {
    state
        .view()
        .rows
        .into_iter()
        .filter_map(|row| match row {
            Row::Choice { label, choice } => Some((label, choice)),
            Row::Text(_) | Row::Detail(_) | Row::Separator | Row::Submenu { .. } => None,
        })
        .collect()
}

fn offers_start(state: &State) -> bool {
    choices(state)
        .iter()
        .any(|(_, choice)| *choice == Choice::StartDaemon)
}

#[test]
fn at_launch_the_icon_shows_the_daemon_unreachable() {
    let state = State::new();

    assert_eq!(state.view().verdict, Verdict::Unreachable);
    assert_eq!(texts(&state), ["Looking for the Daemon…"]);
}

#[test]
fn a_reached_daemon_shows_idle_with_its_product_and_version() {
    let mut state = no_daemon();

    state.apply(Input::Connection(foxhound_daemon()));

    assert_eq!(state.view().verdict, Verdict::Idle);
    assert_eq!(texts(&state), ["Foxhound Daemon 0.4.1"]);
    assert!(!offers_start(&state));
}

#[test]
fn with_no_daemon_the_icon_shows_it_unreachable_and_the_menu_offers_start_daemon() {
    let state = no_daemon();

    assert_eq!(state.view().verdict, Verdict::Unreachable);
    assert_eq!(texts(&state), ["No Daemon"]);
    assert_eq!(
        choices(&state),
        [
            ("Start Daemon".to_owned(), Choice::StartDaemon),
            ("Open details".to_owned(), Choice::OpenDetails),
            ("Quit".to_owned(), Choice::Quit),
        ]
    );
}

#[test]
fn a_lost_daemon_shows_unreachable_again() {
    let mut state = no_daemon();
    state.apply(Input::Connection(foxhound_daemon()));

    state.apply(Input::Connection(ConnectionChange::NoDaemon));

    assert_eq!(state.view().verdict, Verdict::Unreachable);
    assert!(offers_start(&state));
}

#[rstest]
#[case::no_daemon(vec![Input::Connection(ConnectionChange::NoDaemon)])]
#[case::found_installed(vec![Input::Installed(Some(installed())), Input::Connection(ConnectionChange::NoDaemon)])]
#[case::lost(vec![Input::Connection(foxhound_daemon()), Input::Connection(ConnectionChange::NoDaemon)])]
#[case::start_failed(vec![
    Input::Installed(Some(installed())),
    Input::Connection(ConnectionChange::NoDaemon),
    Input::StartEnded(Err(StartFailure::Exited { code: Some(1) })),
])]
#[case::settings_changed(vec![Input::Connection(ConnectionChange::NoDaemon), Input::Settings(Ok(Settings::default()))])]
fn the_kennel_never_starts_a_daemon_by_itself(#[case] inputs: Vec<Input>) {
    let mut state = State::new();

    let effects: Vec<Effect> = inputs
        .into_iter()
        .flat_map(|input| state.apply(input))
        .collect();

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::StartDaemon(_))),
        "{effects:?}"
    );
}

#[test]
fn choosing_start_daemon_runs_the_installed_products_daemon_start() {
    let mut state = no_daemon();

    let effects = state.choose(Choice::StartDaemon);

    assert_eq!(effects, [Effect::StartDaemon(installed())]);
    assert_eq!(texts(&state), ["Starting the Daemon…"]);
    assert!(!offers_start(&state));
}

#[test]
fn a_started_daemon_shows_once_the_client_reaches_it() {
    let mut state = no_daemon();
    state.choose(Choice::StartDaemon);

    state.apply(Input::StartEnded(Ok(())));
    let while_waiting = texts(&state);
    state.apply(Input::Connection(foxhound_daemon()));

    assert_eq!(while_waiting, ["Starting the Daemon…"]);
    assert_eq!(state.view().verdict, Verdict::Idle);
    assert_eq!(texts(&state), ["Foxhound Daemon 0.4.1"]);
}

#[test]
fn a_daemon_that_did_not_start_says_why_and_offers_start_again() {
    let mut state = no_daemon();
    state.choose(Choice::StartDaemon);

    state.apply(Input::StartEnded(Err(StartFailure::Exited {
        code: Some(3),
    })));

    assert_eq!(
        texts(&state),
        [
            "No Daemon",
            "The Daemon did not start: `daemon start` exited with status 3"
        ]
    );
    assert!(offers_start(&state));
}

#[test]
fn a_start_daemon_choice_from_a_menu_that_no_longer_offers_it_does_nothing() {
    let mut state = no_daemon();
    state.apply(Input::Connection(foxhound_daemon()));

    assert_eq!(state.choose(Choice::StartDaemon), []);
}

#[test]
fn with_no_installed_product_start_daemon_is_not_offered_and_the_menu_says_why() {
    let mut state = State::new();
    state.apply(Input::Installed(None));
    state.apply(Input::Connection(ConnectionChange::NoDaemon));

    assert_eq!(
        texts(&state),
        [
            "No Daemon",
            "Start Daemon: no installed Collie or Foxhound was found"
        ]
    );
    assert!(!offers_start(&state));
    assert_eq!(state.choose(Choice::StartDaemon), []);
}

#[test]
fn a_read_only_kennel_offers_no_start_daemon() {
    let mut state = no_daemon();

    state.apply(Input::Settings(Ok(Settings {
        controls: false,
        ..Settings::default()
    })));

    assert_eq!(texts(&state), ["No Daemon"]);
    assert!(!offers_start(&state));
    assert_eq!(state.choose(Choice::StartDaemon), []);
}

#[test]
fn a_daemon_of_another_protocol_version_names_the_version_the_kennel_needs() {
    let mut state = no_daemon();

    state.apply(Input::Connection(ConnectionChange::Refused(
        Refusal::ProtocolVersion {
            engine: ProtocolVersion { major: 0, minor: 2 },
            needs: ProtocolVersion { major: 0, minor: 1 },
        },
    )));

    assert_eq!(state.view().verdict, Verdict::Unreachable);
    assert_eq!(
        texts(&state),
        ["The Daemon speaks protocol 0.2, and this Kennel needs a Daemon of protocol 0.1"]
    );
    assert!(!offers_start(&state));
}

#[test]
fn an_unreadable_settings_file_says_so_and_the_defaults_apply() {
    let mut state = no_daemon();

    state.apply(Input::Settings(Err(SettingsError::Unreadable(
        "unknown field `contrls`".to_owned(),
    ))));

    assert_eq!(
        texts(&state),
        [
            "No Daemon",
            "kennel.toml cannot be read, so the defaults apply: unknown field `contrls`"
        ]
    );
    assert!(offers_start(&state));
}

#[test]
fn quit_is_always_offered_and_quits() {
    let mut state = no_daemon();
    state.apply(Input::Connection(foxhound_daemon()));

    let last = state.view().rows.pop();

    assert_eq!(
        last,
        Some(Row::Choice {
            label: "Quit".to_owned(),
            choice: Choice::Quit
        })
    );
    assert_eq!(state.choose(Choice::Quit), [Effect::Quit]);
}

fn health(live: bool, unready: Vec<Unready>) -> Input {
    Input::Connection(ConnectionChange::Health(DaemonHealth {
        live,
        ready: live && unready.is_empty(),
        unready,
    }))
}

#[rstest]
#[case::serving(health(true, vec![]), "Serving")]
#[case::draining(health(true, vec![Unready::Draining]), "Draining: it takes no new Run")]
#[case::draining_and_more(
    health(true, vec![Unready::Draining, Unready::StorageCritical]),
    "Draining: it takes no new Run; its storage is at the critical level"
)]
#[case::not_ready(
    health(true, vec![Unready::BrowsersMissing, Unready::GovernorLastResort]),
    "Takes no new Run: a browser it uses is not installed; its Resource governor is at its last resort"
)]
#[case::run_state(
    health(true, vec![Unready::RunStateNotWritable]),
    "Takes no new Run: its run state cannot be written"
)]
#[case::not_live(health(false, vec![]), "Not live: its Runs' registry does not respond")]
fn the_daemons_health_shows_under_it(#[case] reading: Input, #[case] shown: &str) {
    let mut state = no_daemon();
    state.apply(Input::Connection(foxhound_daemon()));

    state.apply(reading);

    assert_eq!(texts(&state), ["Foxhound Daemon 0.4.1", shown]);
    assert_eq!(state.view().verdict, Verdict::Idle);
}

#[test]
fn a_lost_connection_and_a_reconnection_change_the_verdict_and_resync_the_view() {
    let mut state = no_daemon();
    state.apply(Input::Connection(foxhound_daemon()));
    state.apply(health(true, vec![Unready::Draining]));

    state.apply(Input::Connection(ConnectionChange::NoDaemon));
    let lost = (state.view().verdict, texts(&state));
    state.apply(Input::Connection(ConnectionChange::Connected(
        DaemonIdentity {
            product: Product::Collie,
            product_version: ProductVersion::parse("0.4.2").unwrap(),
        },
    )));

    assert_eq!(lost, (Verdict::Unreachable, vec!["No Daemon".to_owned()]));
    assert_eq!(state.view().verdict, Verdict::Idle);
    assert_eq!(texts(&state), ["Collie Daemon 0.4.2"]);
}

#[test]
fn a_health_reading_with_no_daemon_reached_changes_nothing() {
    let mut state = no_daemon();

    state.apply(health(true, vec![Unready::Draining]));

    assert_eq!(state.view().verdict, Verdict::Unreachable);
    assert_eq!(texts(&state), ["No Daemon"]);
}

#[test]
fn open_details_opens_the_details_window() {
    let mut state = no_daemon();
    state.apply(Input::Connection(foxhound_daemon()));

    let effects = state.choose(Choice::OpenDetails);

    assert_eq!(
        choices(&state),
        [
            ("Open details".to_owned(), Choice::OpenDetails),
            ("Quit".to_owned(), Choice::Quit),
        ]
    );
    assert_eq!(effects, [Effect::OpenDetails(Opening::Asked)]);
}

#[test]
fn open_details_while_the_window_is_open_brings_it_forward_and_opens_none() {
    let mut state = no_daemon();
    state.choose(Choice::OpenDetails);

    let while_open = state.choose(Choice::OpenDetails);
    state.apply(Input::DetailsClosed);
    let once_closed = state.choose(Choice::OpenDetails);

    assert_eq!(while_open, [Effect::RaiseDetails]);
    assert_eq!(once_closed, [Effect::OpenDetails(Opening::Asked)]);
}

#[test]
fn with_the_window_setting_off_the_kennel_is_a_menu_only() {
    let mut state = no_daemon();

    state.apply(Input::Settings(Ok(Settings {
        window: false,
        ..Settings::default()
    })));

    assert!(
        !choices(&state)
            .iter()
            .any(|(_, choice)| *choice == Choice::OpenDetails)
    );
    assert_eq!(state.choose(Choice::OpenDetails), []);
}
