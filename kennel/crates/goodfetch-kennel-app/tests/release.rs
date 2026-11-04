//! The Kennel's `.app` and its release (ADR 0141), through the scripts
//! that make them: `bundle.sh` assembles the app, `release.sh` signs,
//! packages and verifies it, and the install script it publishes installs
//! it as a person does. The app's executable is a stand-in, a copy of
//! `/usr/bin/true`, so no test builds the Kennel. macOS only: the `.app`,
//! `codesign` and `ditto` exist only there.
#![cfg(target_os = "macos")]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests run the scripts and read their files on the test thread, outside any runtime"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The Kennel's scripts folder.
fn scripts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts")
}

/// Runs `script` with `arguments`, its output collected.
fn run(script: &str, arguments: &[&str]) -> Output {
    Command::new(scripts().join(script))
        .args(arguments)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A stand-in for the Kennel's executable: a Mach-O that `codesign` signs.
fn stand_in_executable(folder: &Path) -> PathBuf {
    let executable = folder.join("goodfetch-kennel");
    fs::copy("/usr/bin/true", &executable).unwrap();
    executable
}

/// The value of `key` in the app's `Info.plist`.
fn plist_value(app: &Path, key: &str) -> String {
    let output = Command::new("plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// What `codesign --display` says of the app's signature.
fn signature(app: &Path) -> String {
    let output = Command::new("codesign")
        .args(["--display", "--verbose=2"])
        .arg(app)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    stderr(&output)
}

#[test]
fn the_bundle_carries_the_kennels_own_version_and_an_ad_hoc_signature() {
    let folder = tempfile::tempdir().unwrap();
    let executable = stand_in_executable(folder.path());
    let out = folder.path().join("out");

    let bundled = run(
        "bundle.sh",
        &[executable.to_str().unwrap(), out.to_str().unwrap()],
    );

    assert!(bundled.status.success(), "{}", stderr(&bundled));
    let app = out.join("Kennel.app");
    assert_eq!(
        plist_value(&app, "CFBundleShortVersionString"),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        plist_value(&app, "CFBundleVersion"),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        plist_value(&app, "CFBundleIdentifier"),
        "dev.goodfetch.kennel"
    );
    assert!(signature(&app).contains("Signature=adhoc"));
}

/// The release environment's signing credentials (ADR 0039), which only
/// the signing job holds.
const CREDENTIALS: [&str; 5] = [
    "GOODFETCH_DEVELOPER_ID_P12",
    "GOODFETCH_DEVELOPER_ID_P12_PASSWORD",
    "GOODFETCH_NOTARY_KEY",
    "GOODFETCH_NOTARY_KEY_ID",
    "GOODFETCH_NOTARY_ISSUER_ID",
];

/// The Rust target the install script picks on this Mac.
fn host_target() -> String {
    format!("{}-apple-darwin", std::env::consts::ARCH)
}

/// A release folder as `release.sh build` leaves it: this Mac's target's
/// `Kennel.app`, signed ad hoc, around a stand-in executable.
fn built_release(folder: &Path) -> PathBuf {
    let release = folder.join("release");
    let executable = stand_in_executable(folder);
    let bundled = run(
        "bundle.sh",
        &[
            executable.to_str().unwrap(),
            release.join(host_target()).to_str().unwrap(),
        ],
    );
    assert!(bundled.status.success(), "{}", stderr(&bundled));
    release
}

/// Runs `release.sh` with only the `credentials` given set.
fn release(arguments: &[&str], credentials: &[(&str, &str)]) -> Output {
    let mut command = Command::new(scripts().join("release.sh"));
    command.args(arguments);
    for name in CREDENTIALS {
        command.env_remove(name);
    }
    command.envs(credentials.iter().copied());
    command.output().unwrap()
}

#[test]
fn signing_without_the_release_credentials_leaves_the_ad_hoc_app_and_says_it_is_inert() {
    let folder = tempfile::tempdir().unwrap();
    let release_folder = built_release(folder.path());

    let signed = release(&["sign", release_folder.to_str().unwrap()], &[]);

    assert!(signed.status.success(), "{}", stderr(&signed));
    assert!(stderr(&signed).contains("inert"), "{}", stderr(&signed));
    let app = release_folder.join(host_target()).join("Kennel.app");
    assert!(signature(&app).contains("Signature=adhoc"));
}

#[test]
fn signing_with_only_some_of_the_release_credentials_fails_and_names_the_missing_ones() {
    let folder = tempfile::tempdir().unwrap();
    let release_folder = built_release(folder.path());

    let signed = release(
        &["sign", release_folder.to_str().unwrap()],
        &[("GOODFETCH_DEVELOPER_ID_P12", "cDEy")],
    );

    assert!(!signed.status.success());
    let message = stderr(&signed);
    for missing in &CREDENTIALS[1..] {
        assert!(message.contains(missing), "{message}");
    }
    let app = release_folder.join(host_target()).join("Kennel.app");
    assert!(signature(&app).contains("Signature=adhoc"));
}

#[test]
fn the_publish_gate_refuses_an_app_signed_ad_hoc() {
    let folder = tempfile::tempdir().unwrap();
    let release_folder = built_release(folder.path());

    let verified = release(&["verify", release_folder.to_str().unwrap()], &[]);

    assert!(!verified.status.success());
    assert!(
        stderr(&verified).contains("Developer ID"),
        "{}",
        stderr(&verified)
    );
}

/// A Mac's user for the install script: a home of its own, and stand-ins
/// for `open`, `pgrep` and `pkill` that record their calls, so a test
/// neither opens an app nor quits the person's own Kennel. A Kennel counts
/// as running while `running` exists in the stand-ins' folder.
struct Person {
    home: PathBuf,
    stand_ins: PathBuf,
}

impl Person {
    fn new(folder: &Path) -> Self {
        let home = folder.join("home");
        let stand_ins = folder.join("stand-ins");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&stand_ins).unwrap();
        let calls = stand_ins.join("calls");
        let running = stand_ins.join("running");
        let scripts = [
            (
                "open",
                format!("echo \"open $*\" >>'{}'\n", calls.display()),
            ),
            ("pgrep", format!("[ -e '{}' ]\n", running.display())),
            (
                "pkill",
                format!(
                    "echo \"pkill $*\" >>'{}'\nrm -f '{}'\n",
                    calls.display(),
                    running.display()
                ),
            ),
        ];
        for (name, body) in scripts {
            let path = stand_ins.join(name);
            fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
            fs::set_permissions(&path, permissions).unwrap();
        }
        Self { home, stand_ins }
    }

    /// Runs the install script `script` as this person, with `bash`, as
    /// a person runs a downloaded one.
    fn install(&self, script: &Path) -> Output {
        let path = format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", self.stand_ins.display());
        Command::new("/bin/bash")
            .arg(script)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", path)
            .output()
            .unwrap()
    }

    fn installed_app(&self) -> PathBuf {
        self.home.join("Applications/Kennel.app")
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.stand_ins.join("calls"))
            .map(|calls| calls.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    fn start_a_kennel(&self) {
        fs::write(self.stand_ins.join("running"), "").unwrap();
    }
}

/// A release folder after `build`, `sign` (inert) and `package`, served
/// from its own `file://` URL; returns it with its install script.
fn packaged_release(folder: &Path) -> (PathBuf, PathBuf) {
    let release_folder = built_release(folder);
    let url = format!("file://{}", release_folder.display());
    let packaged = release(&["package", release_folder.to_str().unwrap(), &url], &[]);
    assert!(packaged.status.success(), "{}", stderr(&packaged));
    let script = release_folder.join("install-kennel.sh");
    (release_folder, script)
}

#[test]
fn the_install_script_installs_the_release_into_applications_and_opens_it_with_open() {
    let folder = tempfile::tempdir().unwrap();
    let (_, script) = packaged_release(folder.path());
    let person = Person::new(folder.path());

    let installed = person.install(&script);

    assert!(installed.status.success(), "{}", stderr(&installed));
    let app = person.installed_app();
    assert_eq!(
        plist_value(&app, "CFBundleIdentifier"),
        "dev.goodfetch.kennel"
    );
    assert_eq!(person.calls(), [format!("open {}", app.display())]);
    assert!(
        !person.home.join("Library/LaunchAgents").exists(),
        "the install script installs no login agent"
    );
}

#[test]
fn the_install_script_replaces_an_installed_kennel_by_rename_never_writing_its_files() {
    let folder = tempfile::tempdir().unwrap();
    let (release_folder, script) = packaged_release(folder.path());
    let released_executable = release_folder
        .join(host_target())
        .join("Kennel.app/Contents/MacOS/goodfetch-kennel");
    let person = Person::new(folder.path());
    let old_executable = person
        .installed_app()
        .join("Contents/MacOS/goodfetch-kennel");
    fs::create_dir_all(old_executable.parent().unwrap()).unwrap();
    fs::write(&old_executable, "the installed Kennel").unwrap();
    // A second name for the installed file: an install that wrote into it,
    // rather than renaming a new bundle into place, changes what it holds.
    let kept = folder.path().join("kept");
    fs::hard_link(&old_executable, &kept).unwrap();

    let installed = person.install(&script);

    assert!(installed.status.success(), "{}", stderr(&installed));
    assert_eq!(fs::read_to_string(&kept).unwrap(), "the installed Kennel");
    assert!(
        fs::read(&old_executable).unwrap() == fs::read(&released_executable).unwrap(),
        "the release's executable is installed"
    );
}

#[test]
fn the_install_script_quits_a_running_kennel_before_it_installs() {
    let folder = tempfile::tempdir().unwrap();
    let (_, script) = packaged_release(folder.path());
    let person = Person::new(folder.path());
    person.start_a_kennel();

    let installed = person.install(&script);

    assert!(installed.status.success(), "{}", stderr(&installed));
    assert_eq!(
        person.calls(),
        [
            "pkill -x goodfetch-kennel".to_owned(),
            format!("open {}", person.installed_app().display())
        ]
    );
}

#[test]
fn the_install_script_refuses_an_archive_whose_digest_is_not_the_releases() {
    let folder = tempfile::tempdir().unwrap();
    let (release_folder, script) = packaged_release(folder.path());
    let archive = release_folder.join(format!(
        "goodfetch-kennel-{}-{}.zip",
        env!("CARGO_PKG_VERSION"),
        host_target()
    ));
    let mut bytes = fs::read(&archive).unwrap();
    bytes.extend_from_slice(b"changed");
    fs::write(&archive, bytes).unwrap();
    let person = Person::new(folder.path());

    let installed = person.install(&script);

    assert!(!installed.status.success());
    assert!(
        stderr(&installed).contains("SHA-256"),
        "{}",
        stderr(&installed)
    );
    assert!(!person.installed_app().exists());
    assert_eq!(person.calls(), Vec::<String>::new());
}
