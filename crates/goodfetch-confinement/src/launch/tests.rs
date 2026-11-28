use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;

use goodfetch_protocol::plugin::{COOKIE, COOKIE_VARIABLE, Limits, NetworkLevel};
use rstest::rstest;

use super::{Launch, LaunchError, shim};
use crate::policy::Policy;

fn launch(env: &[(&str, &str)]) -> Launch {
    Launch {
        policy: Policy {
            network_level: NetworkLevel::None,
            reads: Vec::new(),
            writes: Vec::new(),
            executables: Vec::new(),
            env: env
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            limits: Limits::default(),
        },
        program: PathBuf::from("/plugin/run"),
        args: Vec::new(),
        folder: PathBuf::from("/instance/tmp"),
        proxy_relay: None,
        reduced_confinement: false,
    }
}

#[test]
fn a_plugins_environment_is_the_policy_variables_and_the_cookie_alone() {
    let launch = launch(&[("API_URL", "https://api.example"), ("REGION", "eu")]);

    let env: BTreeMap<&str, &str> = launch.env().collect();

    assert_eq!(
        env,
        BTreeMap::from([
            ("API_URL", "https://api.example"),
            ("REGION", "eu"),
            (COOKIE_VARIABLE, COOKIE),
        ])
    );
}

#[rstest]
#[case::no_variable(None)]
#[case::no_launch(Some("{}"))]
#[case::not_json(Some("plugin"))]
fn a_launcher_without_a_readable_launch_starts_nothing(#[case] launch: Option<&str>) {
    let error = shim(launch.map(OsStr::new));

    assert!(matches!(error, LaunchError::Request), "{error}");
    assert_eq!(error.status(), 246);
}

#[cfg(target_os = "linux")]
#[test]
fn no_launcher_status_is_one_the_probe_reports() {
    use crate::linux::probe::Report;

    let errors = [
        LaunchError::Request,
        LaunchError::Unsupported,
        LaunchError::Unavailable(String::new()),
        LaunchError::Confine(String::new()),
        LaunchError::Folder(std::io::Error::other("")),
        LaunchError::Handles(std::io::Error::other("")),
        LaunchError::Start(std::io::Error::other("")),
        LaunchError::Limits(std::io::Error::other("")),
    ];

    let reported: Vec<_> = errors
        .iter()
        .filter_map(|error| Report::of_status(error.status()))
        .collect();

    assert_eq!(reported, Vec::new());
}
