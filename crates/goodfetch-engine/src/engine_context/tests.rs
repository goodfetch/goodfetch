//! What the engine reads of its environment map: the OS user's name, the
//! person a single-user install acts for (ADR 0102), whom the Audit log
//! names, and the person's folders a Plugin's confinement denies (ADR 0033).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use rstest::rstest;

use super::{os_user, places};
use crate::home::Home;

/// The environment map of `variables`.
fn environment(variables: &[(&str, &str)]) -> BTreeMap<OsString, OsString> {
    variables
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

#[rstest]
#[case::unix(&[("USER", "ada"), ("LOGNAME", "lovelace")], "ada")]
#[case::login_name(&[("LOGNAME", "lovelace")], "lovelace")]
#[case::windows(&[("USERNAME", "ada")], "ada")]
#[case::none(&[], "os_user")]
fn the_os_users_name_comes_from_the_environment(
    #[case] variables: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(os_user(&environment(variables)), expected);
}

/// The person's folders come from the environment, the Goodfetch home from
/// the engine's; without a home folder there are none, and no Plugin
/// starts.
#[rstest]
#[case::home_only(&[("HOME", "/home/ada"), ("USERPROFILE", "C:\\Users\\ada")], true, None)]
#[case::app_data(&[("HOME", "/home/ada"), ("USERPROFILE", "C:\\Users\\ada"), ("APPDATA", "/roaming"), ("LOCALAPPDATA", "")], true, Some("/roaming"))]
#[case::no_home(&[("HOME", ""), ("USERPROFILE", ""), ("APPDATA", "/roaming")], false, None)]
fn the_persons_folders_come_from_the_environment(
    #[case] variables: &[(&str, &str)],
    #[case] placed: bool,
    #[case] app_data: Option<&str>,
) {
    let home = Home::new(PathBuf::from("/goodfetch"));

    let found = places(&home, &environment(variables));

    let user_home = if cfg!(windows) {
        "C:\\Users\\ada"
    } else {
        "/home/ada"
    };
    assert_eq!(
        found.map(|places| (
            places.home,
            places.goodfetch_home,
            places.app_data,
            places.local_app_data
        )),
        placed.then(|| {
            (
                PathBuf::from(user_home),
                PathBuf::from("/goodfetch"),
                app_data.map(PathBuf::from),
                None,
            )
        })
    );
}
