//! The Kennel's settings file, read at launch and on change.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the settings file on the test thread, outside any runtime"
)]

use std::fs;

use goodfetch_kennel::home::Home;
use goodfetch_kennel::settings::{
    self, DesktopNotices, Edit, Settings, SettingsError, Switch, Views,
};
use rstest::rstest;

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
            desktop_notices: DesktopNotices {
                enabled: true,
                needs_you: true,
                run_failed: true,
                run_stopped_by_limit: true,
                daemon_lost: true,
                run_finished: false,
                period_minutes: 20,
            },
            window: true,
            start_at_login: false,
            hidden_icon_check: true,
            views: Views {
                runs: true,
                run_progress: true,
                run_usage: true,
                run_failures: true,
                run_report: true,
                records: true,
                daemon_settings: true,
                governor: true,
                kennel_settings: true,
            },
        })
    );
}

#[test]
fn starting_at_login_and_the_hidden_icon_check_are_settings_of_their_own() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(
        home.settings_file(),
        "start_at_login = true\nhidden_icon_check = false\n",
    )
    .unwrap();

    let read = settings::read(&home.settings_file());

    assert_eq!(
        read,
        Ok(Settings {
            start_at_login: true,
            hidden_icon_check: false,
            ..Settings::default()
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
            desktop_notices: DesktopNotices {
                run_failed: false,
                run_finished: true,
                period_minutes: 5,
                ..DesktopNotices::default()
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
        read.map(|settings| settings.desktop_notices.enabled),
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

#[test]
fn turning_the_window_off_is_a_setting_of_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(home.settings_file(), "window = false\n").unwrap();

    let read = settings::read(&home.settings_file());

    assert_eq!(
        read,
        Ok(Settings {
            window: false,
            ..Settings::default()
        })
    );
}

#[rstest]
#[case::controls(Edit::Switch(Switch::Controls, false), "controls = false")]
#[case::confirm_stop(Edit::Switch(Switch::ConfirmStop, false), "confirm_stop = false")]
#[case::window(Edit::Switch(Switch::Window, false), "window = false")]
#[case::keep_daemon_alive(
    Edit::Switch(Switch::KeepDaemonAlive, true),
    "keep_daemon_alive = true"
)]
#[case::start_at_login(Edit::Switch(Switch::StartAtLogin, true), "start_at_login = true")]
#[case::hidden_icon_check(
    Edit::Switch(Switch::HiddenIconCheck, false),
    "hidden_icon_check = false"
)]
#[case::notices(
    Edit::Switch(Switch::Notices, false),
    "[notifications]\nenabled = false"
)]
#[case::needs_you(
    Edit::Switch(Switch::NeedsYouNotice, false),
    "[notifications]\nneeds_you = false"
)]
#[case::run_failed(
    Edit::Switch(Switch::RunFailedNotice, false),
    "[notifications]\nrun_failed = false"
)]
#[case::run_stopped_by_limit(
    Edit::Switch(Switch::RunStoppedByLimitNotice, false),
    "[notifications]\nrun_stopped_by_limit = false"
)]
#[case::daemon_lost(
    Edit::Switch(Switch::DaemonLostNotice, false),
    "[notifications]\ndaemon_lost = false"
)]
#[case::run_finished(
    Edit::Switch(Switch::RunFinishedNotice, true),
    "[notifications]\nrun_finished = true"
)]
#[case::runs_view(Edit::Switch(Switch::RunsView, false), "[views]\nruns = false")]
#[case::run_progress_view(
    Edit::Switch(Switch::RunProgressView, false),
    "[views]\nrun_progress = false"
)]
#[case::run_usage_view(
    Edit::Switch(Switch::RunUsageView, false),
    "[views]\nrun_usage = false"
)]
#[case::run_failures_view(
    Edit::Switch(Switch::RunFailuresView, false),
    "[views]\nrun_failures = false"
)]
#[case::run_report_view(
    Edit::Switch(Switch::RunReportView, false),
    "[views]\nrun_report = false"
)]
#[case::records_view(Edit::Switch(Switch::RecordsView, false), "[views]\nrecords = false")]
#[case::daemon_settings_view(
    Edit::Switch(Switch::DaemonSettingsView, false),
    "[views]\ndaemon_settings = false"
)]
#[case::governor_view(Edit::Switch(Switch::GovernorView, false), "[views]\ngovernor = false")]
#[case::kennel_settings_view(
    Edit::Switch(Switch::KennelSettingsView, false),
    "[views]\nkennel_settings = false"
)]
#[case::period(Edit::Period(5), "[notifications]\nperiod_minutes = 5")]
fn an_edit_writes_its_key_into_a_missing_settings_file(#[case] edit: Edit, #[case] written: &str) {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let expected = folder.path().join("expected.toml");
    fs::write(&expected, written).unwrap();

    settings::edit(&home.settings_file(), edit).unwrap();

    assert_eq!(
        settings::read(&home.settings_file()),
        settings::read(&expected)
    );
    assert_ne!(settings::read(&expected), Ok(Settings::default()));
}

#[test]
fn an_edit_keeps_the_files_other_keys_and_comments_as_written() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let before = "# my Kennel\ncontrols = false # read-only\n\n[notifications]\n# quiet\nperiod_minutes = 5\nlater_key = \"kept\"\n";
    fs::write(home.settings_file(), before).unwrap();

    settings::edit(
        &home.settings_file(),
        Edit::Switch(Switch::RunFinishedNotice, true),
    )
    .unwrap();
    settings::edit(
        &home.settings_file(),
        Edit::Switch(Switch::StartAtLogin, true),
    )
    .unwrap();

    let after = fs::read_to_string(home.settings_file()).unwrap();
    assert_eq!(
        after,
        "# my Kennel\ncontrols = false # read-only\nstart_at_login = true\n\n[notifications]\n# quiet\nperiod_minutes = 5\nlater_key = \"kept\"\nrun_finished = true\n"
    );
}

#[test]
fn an_edit_of_a_file_that_is_not_toml_changes_nothing_and_says_why() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(home.settings_file(), "controls = \n").unwrap();

    let edited = settings::edit(&home.settings_file(), Edit::Switch(Switch::Controls, false));

    assert!(
        matches!(edited, Err(SettingsError::Unreadable(_))),
        "{edited:?}"
    );
    assert_eq!(
        fs::read_to_string(home.settings_file()).unwrap(),
        "controls = \n"
    );
}

#[test]
fn an_edit_into_a_table_written_as_another_value_changes_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(home.settings_file(), "notifications = false\n").unwrap();

    let edited = settings::edit(&home.settings_file(), Edit::Period(5));

    assert!(
        matches!(edited, Err(SettingsError::Unreadable(_))),
        "{edited:?}"
    );
    assert_eq!(
        fs::read_to_string(home.settings_file()).unwrap(),
        "notifications = false\n"
    );
}

#[test]
fn an_edit_shows_through_the_watch_of_the_settings_file() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    let runtime = runtime();
    let (tasks, report, mut reports) = reports(&runtime);
    tasks.spawn(settings::follow(tasks.clone(), home.clone(), report));
    let at_launch = runtime.block_on(next(&mut reports));

    settings::edit(
        &home.settings_file(),
        Edit::Switch(Switch::StartAtLogin, true),
    )
    .unwrap();
    let after_the_edit = runtime.block_on(async {
        loop {
            let read = next(&mut reports).await;
            if read != at_launch {
                return read;
            }
        }
    });

    assert_eq!(at_launch, Ok(Settings::default()));
    assert_eq!(
        after_the_edit,
        Ok(Settings {
            start_at_login: true,
            ..Settings::default()
        })
    );
}

#[test]
fn each_view_of_the_window_is_a_setting_of_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(
        home.settings_file(),
        "[views]\nrecords = false\ndaemon_settings = false\ngovernor = false\n",
    )
    .unwrap();

    let read = settings::read(&home.settings_file());

    assert_eq!(
        read,
        Ok(Settings {
            views: Views {
                records: false,
                daemon_settings: false,
                governor: false,
                ..Views::default()
            },
            ..Settings::default()
        })
    );
}

#[test]
fn an_unknown_view_is_refused_and_named() {
    let folder = tempfile::tempdir().unwrap();
    let home = Home::new(folder.path());
    fs::write(home.settings_file(), "[views]\nrecord = false\n").unwrap();

    let read = settings::read(&home.settings_file());

    let Err(SettingsError::Unreadable(why)) = read else {
        panic!("an unknown view is refused: {read:?}");
    };
    assert!(why.contains("record"), "{why}");
}
