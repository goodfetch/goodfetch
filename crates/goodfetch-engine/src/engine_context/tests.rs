//! The OS user's name, read from the engine's environment map: the person
//! a single-user install acts for (ADR 0102), whom the Audit log names.

use std::ffi::OsString;

use rstest::rstest;

use super::os_user;

#[rstest]
#[case::unix(&[("USER", "ada"), ("LOGNAME", "lovelace")], "ada")]
#[case::login_name(&[("LOGNAME", "lovelace")], "lovelace")]
#[case::windows(&[("USERNAME", "ada")], "ada")]
#[case::none(&[], "os_user")]
fn the_os_users_name_comes_from_the_environment(
    #[case] variables: &[(&str, &str)],
    #[case] expected: &str,
) {
    let environment = variables
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect();

    assert_eq!(os_user(&environment), expected);
}
