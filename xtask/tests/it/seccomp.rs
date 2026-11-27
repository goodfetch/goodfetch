//! Integration tests for the container seccomp profile script, drift check,
//! version bump, and real Linux x64 container runtime loading (ADR 0049;
//! issue 1260).
//! Covers: story:13-55

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use tempfile::TempDir;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn seccomp_workspace() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let write = |rel: &str, bytes: &[u8]| {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    write(
        "scripts/seccomp-profile.sh",
        include_bytes!("../../../scripts/seccomp-profile.sh"),
    );
    write(
        "packaging/oci/seccomp-pin.toml",
        include_bytes!("../../../packaging/oci/seccomp-pin.toml"),
    );
    write(
        "packaging/oci/moby-default-seccomp.json",
        include_bytes!("../../../packaging/oci/moby-default-seccomp.json"),
    );
    write(
        "packaging/oci/seccomp-profile.json",
        include_bytes!("../../../packaging/oci/seccomp-profile.json"),
    );
    dir
}

#[cfg(unix)]
fn run_xtask(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run xtask")
}

#[cfg(unix)]
#[test]
fn script_generates_identical_bytes_twice_and_check_passes() {
    let dir = seccomp_workspace();
    let run_script = || {
        Command::new("bash")
            .arg(dir.path().join("scripts/seccomp-profile.sh"))
            .env("GOODFETCH_XTASK_BIN", env!("CARGO_BIN_EXE_xtask"))
            .current_dir(dir.path())
            .output()
            .expect("run seccomp-profile.sh")
    };

    let first_run = run_script();
    assert!(
        first_run.status.success(),
        "first run failed: {}",
        String::from_utf8_lossy(&first_run.stderr)
    );
    let first_bytes = std::fs::read(dir.path().join("packaging/oci/seccomp-profile.json")).unwrap();

    let second_run = run_script();
    assert!(
        second_run.status.success(),
        "second run failed: {}",
        String::from_utf8_lossy(&second_run.stderr)
    );
    let second_bytes =
        std::fs::read(dir.path().join("packaging/oci/seccomp-profile.json")).unwrap();
    assert_eq!(first_bytes, second_bytes);

    let checked = run_xtask(dir.path(), &["seccomp", "check"]);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&checked.stdout)
    );
}

#[cfg(unix)]
#[test]
fn bumping_pinned_moby_version_fails_check_until_script_regenerates_in_one_change() {
    let dir = seccomp_workspace();
    let pin_path = dir.path().join("packaging/oci/seccomp-pin.toml");
    let bumped_pin = std::fs::read_to_string(&pin_path)
        .unwrap()
        .replace("v27.5.1", "v28.0.0");
    std::fs::write(&pin_path, bumped_pin).unwrap();

    let before = run_xtask(dir.path(), &["seccomp", "check"]);
    assert_eq!(
        before.status.code(),
        Some(1),
        "check must fail when pinned version changes without regenerating"
    );

    let mut next_default: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("packaging/oci/moby-default-seccomp.json"))
            .unwrap(),
    )
    .unwrap();
    next_default["syscalls"]
        .as_array_mut()
        .unwrap()
        .push(json!({"names": ["getpid"], "action": "SCMP_ACT_ALLOW"}));
    let next_file = dir.path().join("next-default.json");
    std::fs::write(
        &next_file,
        format!("{}\n", serde_json::to_string_pretty(&next_default).unwrap()),
    )
    .unwrap();

    let regenerated = Command::new("bash")
        .arg(dir.path().join("scripts/seccomp-profile.sh"))
        .args(["--version", "v28.0.0", "--upstream"])
        .arg(&next_file)
        .env("GOODFETCH_XTASK_BIN", env!("CARGO_BIN_EXE_xtask"))
        .current_dir(dir.path())
        .output()
        .expect("run seccomp-profile.sh --version");
    assert!(
        regenerated.status.success(),
        "regenerate failed: {}",
        String::from_utf8_lossy(&regenerated.stderr)
    );

    let after = run_xtask(dir.path(), &["seccomp", "check"]);
    assert_eq!(
        after.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&after.stdout)
    );
}

/// Docker (or Podman on hosts where Podman is the container engine, per
/// `ci/container.sh`) loads `packaging/oci/seccomp-profile.json` via
/// `--security-opt seccomp=<path>` on Linux x64, starts a container under it,
/// and allows `unshare -Ur` while Docker's unmodified default refuses it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn container_engine_loads_committed_seccomp_profile_and_allows_user_namespaces() {
    const VALKEY_PIN: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ci/valkey/image.toml"
    ));
    let pin: toml::Table = toml::from_str(VALKEY_PIN).unwrap();
    let image = format!(
        "{}@{}",
        pin["repository"].as_str().unwrap(),
        pin["digest"].as_str().unwrap()
    );
    let engine = ["docker", "podman"]
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("info")
                .output()
                .is_ok_and(|out| out.status.success())
        })
        .expect("Docker or Podman answers on Linux x64");

    let profile = root().join("packaging/oci/seccomp-profile.json");
    let under_profile = Command::new(engine)
        .args(["run", "--rm", "--security-opt"])
        .arg(format!("seccomp={}", profile.display()))
        .arg(&image)
        .args(["sh", "-c", "true && unshare -Ur true"])
        .output()
        .expect("start container under committed seccomp profile");
    assert!(
        under_profile.status.success(),
        "{engine} failed to run container under {}: {}",
        profile.display(),
        String::from_utf8_lossy(&under_profile.stderr)
    );

    let default_profile = root().join("packaging/oci/moby-default-seccomp.json");
    let under_default = Command::new(engine)
        .args(["run", "--rm", "--security-opt"])
        .arg(format!("seccomp={}", default_profile.display()))
        .arg(&image)
        .args(["unshare", "-Ur", "true"])
        .output()
        .expect("start container under default moby seccomp profile");
    assert!(
        !under_default.status.success(),
        "unmodified moby default profile should refuse `unshare -Ur`"
    );
}
