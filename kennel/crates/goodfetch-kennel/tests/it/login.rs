//! Start at login: the launchd user agent that opens the installed `.app`
//! through `LaunchServices`, written and removed in a test's folder, with
//! `launchctl` replaced by a recorder.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the agent's file on the test thread, outside any runtime"
)]

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_kennel::login::{self, Change, Launchctl, LoginAgent};
use rstest::rstest;

/// Records the jobs it was asked to unload.
#[derive(Default)]
struct Recorder(RefCell<Vec<String>>);

impl Launchctl for Recorder {
    fn unload(&self, label: &str) -> io::Result<()> {
        self.0.borrow_mut().push(label.to_owned());
        Ok(())
    }
}

const BUNDLE: &str = "/Users/someone/Applications/Kennel.app";

fn expected_agent(bundle: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>dev.goodfetch.kennel.login</string>
	<key>ProgramArguments</key>
	<array>
		<string>/usr/bin/open</string>
		<string>{bundle}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
</dict>
</plist>
"#
    )
}

fn agent_file(folder: &Path) -> PathBuf {
    folder.join("dev.goodfetch.kennel.login.plist")
}

#[test]
fn turning_start_at_login_on_writes_an_agent_that_opens_the_installed_app() {
    let folder = tempfile::tempdir().unwrap();
    let agents = folder.path().join("LaunchAgents");
    let launchctl = Recorder::default();

    let change = LoginAgent::in_folder(&agents)
        .apply(Some(Path::new(BUNDLE)), &launchctl)
        .unwrap();

    assert_eq!(change, Change::Installed);
    assert_eq!(
        fs::read_to_string(agent_file(&agents)).unwrap(),
        expected_agent(BUNDLE)
    );
    // It loads at the next login: loading it now would start a second
    // Kennel.
    assert!(launchctl.0.borrow().is_empty());
}

#[test]
fn turning_it_on_again_rewrites_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let agent = LoginAgent::in_folder(folder.path());
    let launchctl = Recorder::default();
    agent.apply(Some(Path::new(BUNDLE)), &launchctl).unwrap();

    let change = agent.apply(Some(Path::new(BUNDLE)), &launchctl).unwrap();

    assert_eq!(change, Change::Unchanged);
}

#[test]
fn an_agent_for_another_place_of_the_app_is_rewritten() {
    let folder = tempfile::tempdir().unwrap();
    let agent = LoginAgent::in_folder(folder.path());
    let launchctl = Recorder::default();
    agent
        .apply(Some(Path::new("/Applications/Kennel.app")), &launchctl)
        .unwrap();

    let change = agent.apply(Some(Path::new(BUNDLE)), &launchctl).unwrap();

    assert_eq!(change, Change::Installed);
    assert_eq!(
        fs::read_to_string(agent_file(folder.path())).unwrap(),
        expected_agent(BUNDLE)
    );
}

#[test]
fn turning_start_at_login_off_removes_the_agent_and_unloads_it() {
    let folder = tempfile::tempdir().unwrap();
    let agent = LoginAgent::in_folder(folder.path());
    let launchctl = Recorder::default();
    agent.apply(Some(Path::new(BUNDLE)), &launchctl).unwrap();

    let change = agent.apply(None, &launchctl).unwrap();

    assert_eq!(change, Change::Removed);
    assert!(!agent_file(folder.path()).exists());
    assert_eq!(*launchctl.0.borrow(), ["dev.goodfetch.kennel.login"]);
}

#[test]
fn with_start_at_login_off_and_no_agent_nothing_happens() {
    let folder = tempfile::tempdir().unwrap();
    let launchctl = Recorder::default();

    let change = LoginAgent::in_folder(folder.path())
        .apply(None, &launchctl)
        .unwrap();

    assert_eq!(change, Change::Unchanged);
    assert!(launchctl.0.borrow().is_empty());
}

#[test]
fn the_apps_path_is_escaped_in_the_agent() {
    let folder = tempfile::tempdir().unwrap();
    let launchctl = Recorder::default();

    LoginAgent::in_folder(folder.path())
        .apply(Some(Path::new("/Users/a&b/<Apps>/Kennel.app")), &launchctl)
        .unwrap();

    assert_eq!(
        fs::read_to_string(agent_file(folder.path())).unwrap(),
        expected_agent("/Users/a&amp;b/&lt;Apps&gt;/Kennel.app")
    );
}

#[rstest]
#[case::installed(
    "/Users/someone/Applications/Kennel.app/Contents/MacOS/goodfetch-kennel",
    Some("/Users/someone/Applications/Kennel.app")
)]
#[case::bare("/home/someone/kennel/target/release/goodfetch-kennel", None)]
#[case::not_an_app("/Users/someone/Kennel/Contents/MacOS/goodfetch-kennel", None)]
fn the_installed_app_is_the_bundle_around_the_executable(
    #[case] executable: &str,
    #[case] bundle: Option<&str>,
) {
    assert_eq!(
        login::bundle_of(Path::new(executable)),
        bundle.map(PathBuf::from)
    );
}
