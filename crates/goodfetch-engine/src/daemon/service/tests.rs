//! The per-user service's definitions and the commands that register and
//! remove it, as pure functions of a fixture home (spec #213's Seam 4; ADR
//! 0059: generating the unit file is a pure function tested with
//! snapshots).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::ServiceManager;
use rstest::rstest;

use super::{ServiceError, ServiceRequest, plan, service_name, unregister};
use crate::home::Home;

/// A Goodfetch home whose path holds a space and a non-ASCII letter, as
/// every test's does (ADR 0059).
const HOME: &str = "/Users/ada/Goodfetch é home";

fn version() -> ProductVersion {
    ProductVersion::parse("1.4.0").unwrap()
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<OsString, OsString> {
    pairs
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

fn fixture_environment() -> BTreeMap<OsString, OsString> {
    environment(&[
        ("HOME", "/Users/ada"),
        ("GOODFETCH_HOME", HOME),
        ("GOODFETCH_TOKEN", "a secret no service may hold"),
        ("PATH", "/usr/bin"),
        ("USERDOMAIN", "WORKSTATION"),
        ("USERNAME", "ada"),
    ])
}

const COPY: &str = "/Users/ada/Goodfetch é home/browsers/executable_copy/goodfetch-collie/1.4.0-0123456789abcdef/files/goodfetch-collie";
const SOCKET: &str = "/Users/ada/Goodfetch é home/1/daemon.sock";

fn request<'a>(
    manager: ServiceManager,
    home: &'a Home,
    version: &'a ProductVersion,
    environment: &'a BTreeMap<OsString, OsString>,
) -> ServiceRequest<'a> {
    ServiceRequest {
        manager,
        home,
        version,
        executable: Path::new(COPY),
        build: "1.4.0-0123456789abcdef",
        socket: Some(Path::new(SOCKET)),
        environment,
        launchd_domain: Some("gui/501"),
    }
}

fn text(contents: &[u8]) -> String {
    if contents.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = contents[2..]
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&units).unwrap()
    } else {
        String::from_utf8(contents.to_vec()).unwrap()
    }
}

/// Covers: story:3-21
#[rstest]
#[case::launchd(ServiceManager::Launchd)]
#[case::systemd(ServiceManager::Systemd)]
#[case::task_scheduler(ServiceManager::TaskScheduler)]
fn each_service_manager_gets_its_definition_of_the_service(#[case] manager: ServiceManager) {
    let home = Home::new(PathBuf::from(HOME));
    let version = version();
    let environment = fixture_environment();

    let planned = plan(&request(manager, &home, &version, &environment)).unwrap();

    let files: Vec<String> = planned
        .files
        .iter()
        .map(|file| format!("--- {}\n{}", file.path.display(), text(&file.contents)))
        .collect();
    let commands: Vec<String> = planned
        .register
        .iter()
        .map(|command| {
            let arguments: Vec<String> = command
                .arguments
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            format!("$ {} {}", command.program, arguments.join(" | "))
        })
        .collect();
    insta::assert_snapshot!(
        format!("{manager:?}").to_lowercase(),
        format!("{}\n{}", files.join("\n"), commands.join("\n"))
    );
}

/// The token a Client's `hello` carries never reaches a service's
/// definition, which other programs of the user can read.
///
/// Covers: story:3-21
#[rstest]
#[case::launchd(ServiceManager::Launchd)]
#[case::systemd(ServiceManager::Systemd)]
#[case::task_scheduler(ServiceManager::TaskScheduler)]
fn no_definition_holds_a_secret_of_the_environment(#[case] manager: ServiceManager) {
    let home = Home::new(PathBuf::from(HOME));
    let version = version();
    let environment = fixture_environment();

    let planned = plan(&request(manager, &home, &version, &environment)).unwrap();

    let leaked: Vec<String> = planned
        .files
        .iter()
        .map(|file| text(&file.contents))
        .filter(|contents| contents.contains("a secret") || contents.contains("/usr/bin"))
        .collect();
    assert_eq!(leaked, Vec::<String>::new());
}

/// Each major of each Goodfetch home has a service of its own, so tests
/// with their own homes never meet, and a 1.x and a 2.x never replace each
/// other's.
///
/// Covers: story:3-21, story:3-22
#[test]
fn each_major_of_each_goodfetch_home_has_its_own_service_name() {
    let names = [
        ("/home/a", "1.4.0"),
        ("/home/a", "1.9.2"),
        ("/home/a", "2.0.0"),
        ("/home/b", "1.4.0"),
    ]
    .map(|(home, version)| {
        service_name(
            ServiceManager::Systemd,
            &Home::new(PathBuf::from(home)),
            &ProductVersion::parse(version).unwrap(),
        )
    });

    assert_eq!(
        (
            names[0] == names[1],
            names[0] == names[2],
            names[0] == names[3]
        ),
        (true, false, false),
        "{names:?}"
    );
}

/// systemd reads `$XDG_CONFIG_HOME/systemd/user` when it is set.
///
/// Covers: story:3-21
#[test]
fn a_systemd_unit_goes_where_xdg_config_home_says() {
    let home = Home::new(PathBuf::from(HOME));
    let version = version();
    let environment = environment(&[("HOME", "/home/ada"), ("XDG_CONFIG_HOME", "/srv/config")]);

    let planned = plan(&request(
        ServiceManager::Systemd,
        &home,
        &version,
        &environment,
    ))
    .unwrap();

    let folders: Vec<&Path> = planned
        .files
        .iter()
        .map(|file| file.path.parent().unwrap())
        .collect();
    assert_eq!(folders, vec![Path::new("/srv/config/systemd/user"); 2]);
}

/// Covers: story:3-21
#[rstest]
#[case::launchd_without_home(ServiceManager::Launchd, &[], ServiceError::NoHomeFolder("HOME"))]
#[case::systemd_without_home(ServiceManager::Systemd, &[], ServiceError::NoHomeFolder("HOME"))]
fn a_service_manager_with_no_home_folder_gets_no_service(
    #[case] manager: ServiceManager,
    #[case] pairs: &[(&str, &str)],
    #[case] expected: ServiceError,
) {
    let home = Home::new(PathBuf::from(HOME));
    let version = version();
    let environment = environment(pairs);

    let planned = plan(&request(manager, &home, &version, &environment));

    assert_eq!(planned, Err(expected));
}

/// Removing a service stops it and forgets it, each step tolerating a
/// service already gone.
///
/// Covers: story:3-21
#[rstest]
#[case::launchd(ServiceManager::Launchd)]
#[case::systemd(ServiceManager::Systemd)]
#[case::task_scheduler(ServiceManager::TaskScheduler)]
fn removing_a_service_stops_it_then_forgets_it(#[case] manager: ServiceManager) {
    let home = Home::new(PathBuf::from(HOME));
    let version = version();
    let environment = fixture_environment();
    let planned = plan(&request(manager, &home, &version, &environment)).unwrap();

    let removing = unregister(&planned.service, Some("gui/501"));

    let lines: Vec<String> = removing
        .before_removal
        .iter()
        .chain(&removing.after_removal)
        .map(|command| {
            let arguments: Vec<String> = command
                .arguments
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect();
            format!(
                "{} {} ({})",
                command.program,
                arguments.join(" "),
                command.may_fail
            )
        })
        .collect();
    insta::assert_snapshot!(
        format!("removing_{manager:?}").to_lowercase(),
        lines.join("\n")
    );
}
