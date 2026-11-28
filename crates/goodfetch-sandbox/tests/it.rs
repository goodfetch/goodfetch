//! Integration tests for the `goodfetch-sandbox` helper executable (ADRs 0033
//! and 0039, issue 884): verifies that the helper starts only entries of the
//! Plugin store or runtime cache (refusing paths outside, traversal, and
//! escaping symlinks), drops every capability, sets `no_new_privs`, and
//! installs its baseline + caller seccomp filters.
#![cfg(target_os = "linux")]
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests run processes and create temp files outside any async runtime"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap"
)]

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use goodfetch_confinement::launch::{LAUNCH_VARIABLE, Launch, LaunchError, SHIM_COMMAND};
use goodfetch_confinement::policy::{Instance, Policy};
use goodfetch_confinement::probes::Layout;
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::settings::{BROWSERS_PATH_VARIABLE, HOME_VARIABLE};
use serde_json::json;

const SANDBOX: &str = env!("CARGO_BIN_EXE_goodfetch-sandbox");

fn system_cat() -> PathBuf {
    for candidate in ["/usr/bin/cat", "/bin/cat"] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return path;
        }
    }
    panic!("expected `/usr/bin/cat` or `/bin/cat` on Linux");
}

fn build_launch(layout: &Layout, program: PathBuf, args: Vec<String>) -> Launch {
    let manifest = json!({
        "manifest_version": "0.1",
        "name": "probe",
        "version": "1.0.0",
        "kinds": ["proxy"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "cat",
        "network_level": "none",
    });
    let manifest = PluginManifest::read(manifest.to_string().as_bytes()).unwrap();
    let instance = Instance {
        bundle: layout.bundle.clone(),
        runtime: Some(layout.runtime.clone()),
        folders: vec![layout.instance.clone()],
        ..Instance::default()
    };
    let policy = Policy::of(&manifest, &instance, &layout.places).unwrap();
    Launch {
        policy,
        program,
        args,
        folder: layout.instance.clone(),
        proxy_relay: None,
        proxy_loopback: None,
        windows_broker: None,
        reduced_confinement: false,
    }
}

fn sandbox_command(home: &Path, browsers_path: Option<&Path>, launch: &Launch) -> Command {
    let mut command = Command::new(SANDBOX);
    command
        .arg(SHIM_COMMAND)
        .env_clear()
        .env(LAUNCH_VARIABLE, serde_json::to_string(launch).unwrap())
        .env(HOME_VARIABLE, home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(browsers) = browsers_path {
        command.env(BROWSERS_PATH_VARIABLE, browsers);
    }
    command
}

/// `goodfetch-sandbox` starts a program inside the Plugin store
/// (`<home>/plugins/...`) or runtime cache (`<home>/runtimes/...`), and the
/// started process runs with all capabilities dropped, `NoNewPrivs: 1`, and
/// seccomp filter mode `2`.
///
/// Covers: story:7-23
#[test]
fn sandbox_helper_starts_store_and_runtime_entries_with_zero_caps_no_new_privs_and_seccomp() {
    let root = tempfile::tempdir().unwrap();
    let canonical_root = fs::canonicalize(root.path()).unwrap();
    let layout = Layout::under(&canonical_root);
    for folder in [&layout.bundle, &layout.runtime, &layout.instance] {
        fs::create_dir_all(folder).unwrap();
    }
    let cat = system_cat();
    let store_entry = layout.bundle.join("cat");
    let runtime_entry = layout.runtime.join("cat");
    fs::copy(&cat, &store_entry).unwrap();
    fs::copy(&cat, &runtime_entry).unwrap();

    for entry in [&store_entry, &runtime_entry] {
        let launch = build_launch(&layout, entry.clone(), Vec::new());
        let mut child = sandbox_command(&layout.places.goodfetch_home, None, &launch)
            .spawn()
            .unwrap();
        let pid = child.id();
        let status_path = format!("/proc/{pid}/status");
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            let text = fs::read_to_string(&status_path).unwrap_or_default();
            if text.contains("Name:\tcat\n") && text.contains("Seccomp:\t2\n") {
                break text;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for helper to exec {} (status:\n{text})",
                entry.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        for expected in [
            "NoNewPrivs:\t1",
            "Seccomp:\t2",
            "CapPrm:\t0000000000000000",
            "CapEff:\t0000000000000000",
            "CapAmb:\t0000000000000000",
        ] {
            assert!(
                status.contains(expected),
                "missing `{expected}` in `/proc/{pid}/status` for {}:\n{status}",
                entry.display()
            );
        }
        // Every non-bounding capability mask (including inheritable) must be zero.
        let cap_lines: Vec<&str> = status
            .lines()
            .filter(|line| line.starts_with("Cap") && !line.starts_with("CapBnd:"))
            .collect();
        assert!(
            cap_lines.len() >= 4,
            "expected >=4 Cap* lines in:\n{status}"
        );
        for line in cap_lines {
            assert!(
                line.ends_with("\t0000000000000000"),
                "expected zero capability set in `{line}` for {}",
                entry.display()
            );
        }
        {
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(b"confined-ok\n").unwrap();
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "expected helper child {} to exit 0, got {:?}",
            entry.display(),
            output.status
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "confined-ok\n");
    }
}

/// `goodfetch-sandbox` refuses any program outside the Plugin store and
/// runtime cache — including system binaries, parent-directory traversals, and
/// symlinks inside the store pointing outside — with `DisallowedProgram` (254).
///
/// Covers: story:7-23
#[test]
fn sandbox_helper_refuses_paths_outside_plugin_store_and_runtime_cache() {
    let root = tempfile::tempdir().unwrap();
    let canonical_root = fs::canonicalize(root.path()).unwrap();
    let layout = Layout::under(&canonical_root);
    for folder in [&layout.bundle, &layout.runtime, &layout.instance] {
        fs::create_dir_all(folder).unwrap();
    }
    let outside_dir = canonical_root.join("outside");
    fs::create_dir_all(&outside_dir).unwrap();
    let outside_bin = outside_dir.join("cat");
    fs::copy(system_cat(), &outside_bin).unwrap();

    let escape_link = layout.bundle.join("escape");
    symlink(&outside_bin, &escape_link).unwrap();

    let traversal = layout.bundle.join("../../../../outside/cat");
    let disallowed = LaunchError::DisallowedProgram(PathBuf::new()).status();

    for candidate in [system_cat(), outside_bin, traversal, escape_link] {
        let launch = build_launch(&layout, candidate.clone(), Vec::new());
        let output = sandbox_command(&layout.places.goodfetch_home, None, &launch)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(i32::from(disallowed)),
            "expected DisallowedProgram ({disallowed}) for {}, got {:?}",
            candidate.display(),
            output.status
        );
    }
}
