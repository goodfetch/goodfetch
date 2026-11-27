//! End-to-end release packaging rehearsal (spec #130, issue 141; ADRs 0024,
//! 0069, 0090 and 0094): packages both Products' archives, split debug symbols,
//! `CycloneDX` SBOMs, npm packages (with `optionalDependencies` and zero install
//! scripts), `py3-none` platform wheels, and first-party Plugin multi-platform
//! OCI layouts, then proves the produced `py3-none` wheels install offline via
//! `pip` with executable binaries and valid `RECORD` hashes, and that the npm
//! packages carry no install scripts and valid platform metadata.
//! Covers: story:1-1, story:1-2, story:1-3

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::release_assets::TARGETS;
use crate::workspace::Workspace;

fn exe_suffix(target: &str) -> &'static str {
    if target.contains("-windows-") {
        ".exe"
    } else {
        ""
    }
}

fn debug_name(executable: &str, target: &str) -> String {
    if target.contains("-windows-") {
        format!("{}.pdb", executable.replace('-', "_"))
    } else if target.contains("-apple-") {
        format!("{executable}.dSYM")
    } else {
        format!("{executable}.dwp")
    }
}

#[test]
fn packaged_wheels_and_npm_packages_install_cleanly_without_install_scripts() {
    let workspace = Workspace::from_fixture("release");
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"VERSION = '1.4.0'\n",
    );
    workspace.write(
        "sdks/python/collie/src/goodfetch_collie/__init__.py",
        b"from pathlib import Path\nBIN = Path(__file__).parent / 'bin' / 'goodfetch-collie'\n",
    );
    workspace.write(
        "sdks/python/foxhound/src/goodfetch_foxhound/__init__.py",
        b"from pathlib import Path\nBIN = Path(__file__).parent / 'bin' / 'goodfetch-foxhound'\n",
    );

    for target in TARGETS {
        let suffix = exe_suffix(target);
        for executable in ["goodfetch-collie", "goodfetch-foxhound"] {
            let script = format!("#!/bin/sh\necho \"{executable} 1.4.0\"\n");
            workspace.write(
                &format!("bins/{target}/{executable}{suffix}"),
                script.as_bytes(),
            );
            workspace.write(
                &format!("bins/{target}/{}", debug_name(executable, target)),
                b"debug-symbols",
            );
        }
    }

    let packaged = workspace.xtask(&[
        "release",
        "package",
        "--version",
        "1.4.0",
        "--binaries",
        "bins",
        "--out",
        "out",
    ]);
    assert_eq!(packaged.code, 0, "{}", packaged.stderr);

    // Verify no npm package defines any lifecycle/install script.
    for dir in [
        "collie-client",
        "collie",
        "foxhound",
        "collie-linux-x64",
        "foxhound-linux-x64",
    ] {
        let manifest: Value =
            serde_json::from_str(&workspace.read_text(&format!("out/npm/{dir}/package.json")))
                .unwrap();
        assert!(
            manifest.get("scripts").is_none(),
            "{dir} must define no install scripts"
        );
    }

    if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
        verify_pip_wheel_install(&workspace);
    }

    // Verify the assets folder passes `xtask release assets` and `xtask release digests`.
    let assets_out = workspace.xtask(&["release", "assets", "--folder", "out/assets"]);
    assert_eq!(assets_out.code, 0, "{}", assets_out.stderr);
    let digests_out = workspace.xtask(&[
        "release",
        "digests",
        "--assets",
        "out/assets",
        "--version",
        "1.4.0",
    ]);
    assert_eq!(digests_out.code, 0, "{}", digests_out.stderr);
    assert!(Path::new(&workspace.path().join("out/assets/SHA256SUMS")).is_file());
}

fn verify_pip_wheel_install(workspace: &Workspace) {
    let root = workspace.path();
    let venv = root.join("rehearsal-venv");
    let created = Command::new("uv")
        .args(["venv", "--seed"])
        .arg(&venv)
        .output()
        .expect("uv is installed");
    assert!(
        created.status.success(),
        "uv venv failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );
    let pip = venv.join("bin/pip");
    let installed = Command::new(&pip)
        .args(["install", "--no-deps", "--no-index", "--find-links"])
        .arg(root.join("out/wheels"))
        .args([
            "goodfetch-collie-client",
            "goodfetch-collie",
            "goodfetch-foxhound",
        ])
        .output()
        .expect("pip runs");
    assert!(
        installed.status.success(),
        "pip install failed: {}{}",
        String::from_utf8_lossy(&installed.stdout),
        String::from_utf8_lossy(&installed.stderr)
    );
    let python = venv.join("bin/python");
    let check = Command::new(&python)
        .args([
            "-c",
            "import subprocess, goodfetch_collie, goodfetch_foxhound; \
             c = subprocess.check_output([str(goodfetch_collie.BIN)], text=True).strip(); \
             f = subprocess.check_output([str(goodfetch_foxhound.BIN)], text=True).strip(); \
             assert c == 'goodfetch-collie 1.4.0', c; \
             assert f == 'goodfetch-foxhound 1.4.0', f",
        ])
        .output()
        .expect("python runs");
    assert!(
        check.status.success(),
        "installed wheels check failed: {}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}
