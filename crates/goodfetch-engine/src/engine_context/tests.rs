//! The OS user's name, read from the engine's environment map: the person
//! a single-user install acts for (ADR 0102), whom the Audit log names; and
//! the host's time zone, read from the same map, which a Schedule stores
//! when it names none.

use std::ffi::OsString;

use rstest::rstest;

use super::{host_zone, os_user};

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

/// Covers: story:10-8
#[rstest]
#[case::an_iana_name("Asia/Kathmandu", "Asia/Kathmandu")]
#[case::with_the_c_library_colon(":Europe/Paris", "Europe/Paris")]
#[case::empty_as_utc("", "UTC")]
#[case::a_posix_rule_as_utc("CET-1CEST", "UTC")]
#[case::an_unknown_name_as_utc("Europe/Atlantis", "UTC")]
fn the_hosts_zone_comes_from_tz_as_an_iana_name(#[case] tz: &str, #[case] expected: &str) {
    let environment = [(OsString::from("TZ"), OsString::from(tz))].into();

    assert_eq!(host_zone(&environment), expected);
}
