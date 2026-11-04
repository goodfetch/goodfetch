//! The details window's process, against a stand-in executable: a script
//! that records how it was run, then exits.
#![cfg(unix)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests write the stand-in on the test thread, outside any runtime"
)]

use std::fs;
use std::path::Path;

use goodfetch_kennel::window::{self, Opening};
use rstest::rstest;

use crate::support::{executable_script, runtime};

/// Writes a stand-in executable at `path` that writes its arguments to
/// `arguments` beside it, then exits.
fn stand_in(path: &Path) {
    let record = path.with_file_name("arguments");
    executable_script(
        path,
        &format!("printf '%s\\n' \"$@\" > '{}'\n", record.display()),
    );
}

#[test]
fn the_window_runs_as_a_second_process_of_the_executable_given_only_its_argument() {
    let folder = tempfile::tempdir().unwrap();
    let executable = folder.path().join("goodfetch-kennel");
    stand_in(&executable);
    let runtime = runtime();

    let mut opened = None;
    let ran = runtime.block_on(window::run(&executable, Opening::Asked, |pid| {
        opened = Some(pid);
    }));

    assert!(ran.is_ok(), "{ran:?}");
    assert!(opened.is_some_and(|pid| pid > 0));
    let arguments = fs::read_to_string(folder.path().join("arguments")).unwrap();
    assert_eq!(arguments, format!("{}\n", window::ARGUMENT));
}

#[test]
fn a_window_whose_executable_is_gone_says_why() {
    let folder = tempfile::tempdir().unwrap();
    let runtime = runtime();

    let ran = runtime.block_on(window::run(
        &folder.path().join("gone"),
        Opening::Asked,
        |_| {},
    ));

    assert!(ran.is_err());
}

#[rstest]
#[case::asked(Opening::Asked)]
#[case::hidden_icon(Opening::HiddenIcon)]
fn the_window_knows_why_it_was_opened(#[case] opening: Opening) {
    let folder = tempfile::tempdir().unwrap();
    let executable = folder.path().join("goodfetch-kennel");
    stand_in(&executable);
    let runtime = runtime();

    runtime
        .block_on(window::run(&executable, opening, |_| {}))
        .unwrap();

    let arguments = fs::read_to_string(folder.path().join("arguments")).unwrap();
    let arguments = arguments.lines().map(std::ffi::OsString::from);
    assert_eq!(Opening::from_arguments(arguments), Some(opening));
}

#[test]
fn without_the_window_argument_the_executable_is_the_tray() {
    assert_eq!(Opening::from_arguments(Vec::new()), None);
    assert_eq!(
        Opening::from_arguments(["something".into()]),
        None::<Opening>
    );
}
