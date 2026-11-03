//! The Kennel's settings file, read at launch and on change.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the settings file on the test thread, outside any runtime"
)]

use std::fs;

use goodfetch_kennel::home::Home;
use goodfetch_kennel::settings::{self, Notifications, Settings, SettingsError};

use crate::support::{next, reports, runtime};

#[test]
fn without_a_settings_file_the_defaults_apply() {
    let folder = tempfile::tempdir().unwrap();

    let read = settings::read(&Home::new(folder.path()).settings_file());

    assert_eq!(
        read,
        Ok(Settings {
            controls: true,
            confirm_stop: true,
            keep_daemon_alive: false,
            notifications: Notifications {
                enabled: true,
                needs_you: true,
                run_failed: true,
                run_stopped_by_limit: true,
                daemon_lost: true,
                run_finished: false,
                period_minutes: 20,
            },
        })
    );
}

#[test]
fn keeping_the_daemon_alive_is_a_setting_of_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(home.settings_file(), "keep_daemon_alive = true\n").unwrap();

    let read = settings::read(&home.settings_file());

    assert_eq!(
        read,
        Ok(Settings {
            keep_daemon_alive: true,
            ..Settings::default()
        })
    );
}

#[test]
fn the_stop_confirmation_is_a_setting_of_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(home.settings_file(), "confirm_stop = false\n").unwrap();

    let read = settings::read(&home.settings_file());

    assert_eq!(
        read,
        Ok(Settings {
            confirm_stop: false,
            ..Settings::default()
        })
    );
}

#[test]
fn each_desktop_notice_type_and_the_period_are_settings_of_their_own() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(
        home.settings_file(),
        "[notifications]\nrun_failed = false\nrun_finished = true\nperiod_minutes = 5\n",
    )
    .unwrap();

    let read = settings::read(&home.settings_file());

    assert_eq!(
        read,
        Ok(Settings {
            notifications: Notifications {
                run_failed: false,
                run_finished: true,
                period_minutes: 5,
                ..Notifications::default()
            },
            ..Settings::default()
        })
    );
}

#[test]
fn desktop_notices_turn_off_as_a_whole() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(home.settings_file(), "[notifications]\nenabled = false\n").unwrap();

    let read = settings::read(&home.settings_file());

    assert_eq!(
        read.map(|settings| settings.notifications.enabled),
        Ok(false)
    );
}

#[test]
fn the_settings_are_read_at_launch_and_again_when_their_file_changes() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(home.settings_file(), "controls = true\n").unwrap();
    let runtime = runtime();
    let (tasks, report, mut reports) = reports(&runtime);
    tasks.spawn(settings::follow(tasks.clone(), home.clone(), report));

    let at_launch = runtime.block_on(next(&mut reports));
    fs::write(home.settings_file(), "controls = false\n").unwrap();
    let after_the_change = runtime.block_on(async {
        // A platform may report the launch's own write late: wait for the
        // file's new contents.
        loop {
            let read = next(&mut reports).await;
            if read != at_launch {
                return read;
            }
        }
    });

    let with_controls = |controls| Settings {
        controls,
        ..Settings::default()
    };
    assert_eq!(at_launch, Ok(with_controls(true)));
    assert_eq!(after_the_change, Ok(with_controls(false)));
}

#[test]
fn a_settings_file_with_an_unknown_key_says_which() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(home.settings_file(), "contrls = false\n").unwrap();

    let read = settings::read(&home.settings_file());

    let Err(SettingsError::Unreadable(why)) = read else {
        panic!("an unknown key is refused: {read:?}");
    };
    assert!(why.contains("contrls"), "{why}");
}

#[test]
fn an_unknown_key_among_the_desktop_notices_is_refused() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(
        home.settings_file(),
        "[notifications]\nrun_crashed = false\n",
    )
    .unwrap();

    let read = settings::read(&home.settings_file());

    let Err(SettingsError::Unreadable(why)) = read else {
        panic!("an unknown key is refused: {read:?}");
    };
    assert!(why.contains("run_crashed"), "{why}");
}
