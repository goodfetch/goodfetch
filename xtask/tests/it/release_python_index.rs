//! Tests for the nightly Python PEP 503 / PEP 691 simple index (ADRs 0057,
//! 0090 and 0094; issue 205).

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::workspace::Workspace;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Builds a minimal valid PEP 427 pure-Python wheel on disk with no external
/// dependencies using Python's stdlib `zipfile`, returning its SHA-256 hex.
fn write_valid_wheel(wheel_path: &Path, dist: &str, module: &str, version: &str) -> String {
    let script = r#"
import base64, hashlib, sys, zipfile
wheel_path, dist, module, version = sys.argv[1:5]
dist_info = f"{dist}-{version}.dist-info"
files = {
    f"{module}/__init__.py": f"__version__ = {version!r}\n".encode(),
    f"{dist_info}/METADATA": (
        f"Metadata-Version: 2.1\nName: {dist.replace('_', '-')}\nVersion: {version}\n"
    ).encode(),
    f"{dist_info}/WHEEL": (
        "Wheel-Version: 1.0\nGenerator: goodfetch-test\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
    ).encode(),
}
record_lines = []
with zipfile.ZipFile(wheel_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
    for name, data in files.items():
        zf.writestr(name, data)
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        record_lines.append(f"{name},sha256={digest},{len(data)}")
    record_lines.append(f"{dist_info}/RECORD,,")
    zf.writestr(f"{dist_info}/RECORD", ("\n".join(record_lines) + "\n").encode())
"#;
    let status = Command::new("uv")
        .args([
            "run",
            "--no-project",
            "python",
            "-c",
            script,
            &wheel_path.to_string_lossy(),
            dist,
            module,
            version,
        ])
        .status()
        .expect("build fixture wheel with python zipfile");
    assert!(status.success(), "fixture wheel build failed");
    let bytes = std::fs::read(wheel_path).expect("read fixture wheel");
    sha256_hex(&bytes)
}

fn seed_35_nightlies(workspace: &Workspace) -> (String, String) {
    let wheels_dir = workspace.path().join("wheels");
    std::fs::create_dir_all(&wheels_dir).expect("create wheels dir");

    let collie_35_path = wheels_dir.join("goodfetch_collie-0.1.0.dev35-py3-none-any.whl");
    let foxhound_35_path = wheels_dir.join("goodfetch_foxhound-0.1.0.dev35-py3-none-any.whl");
    let collie_35_sha = write_valid_wheel(
        &collie_35_path,
        "goodfetch_collie",
        "goodfetch_collie",
        "0.1.0.dev35",
    );
    let foxhound_35_sha = write_valid_wheel(
        &foxhound_35_path,
        "goodfetch_foxhound",
        "goodfetch_foxhound",
        "0.1.0.dev35",
    );

    let mut releases = Vec::new();
    for n in 1..=35u32 {
        let day = ((n - 1) % 28) + 1;
        let month = if n <= 28 { 10 } else { 11 };
        let published_at = format!("2026-{month:02}-{day:02}T01:00:00Z");
        let c_name = format!("goodfetch_collie-0.1.0.dev{n}-py3-none-any.whl");
        let f_name = format!("goodfetch_foxhound-0.1.0.dev{n}-py3-none-any.whl");
        let (c_url, c_sha, f_url, f_sha) = if n == 35 {
            (
                format!("file://{}", collie_35_path.display()),
                collie_35_sha.clone(),
                format!("file://{}", foxhound_35_path.display()),
                foxhound_35_sha.clone(),
            )
        } else {
            (
                format!(
                    "https://github.com/goodfetch/goodfetch/releases/download/v0.1.0-dev.{n}/{c_name}"
                ),
                format!("{n:064x}"),
                format!(
                    "https://github.com/goodfetch/goodfetch/releases/download/v0.1.0-dev.{n}/{f_name}"
                ),
                format!("{:064x}", n + 1000),
            )
        };
        releases.push(json!({
            "tag_name": format!("v0.1.0-dev.{n}"),
            "prerelease": true,
            "draft": false,
            "published_at": published_at,
            "assets": [
                { "name": c_name, "browser_download_url": c_url, "digest": format!("sha256:{c_sha}") },
                { "name": format!("{c_name}.sigstore.json"), "browser_download_url": format!("{c_url}.sigstore.json") },
                { "name": f_name, "browser_download_url": f_url, "digest": format!("sha256:{f_sha}") },
                { "name": format!("{f_name}.sigstore.json"), "browser_download_url": format!("{f_url}.sigstore.json") },
            ]
        }));
    }
    // Stable release must be ignored by the nightly index.
    releases.push(json!({
        "tag_name": "v0.1.0",
        "prerelease": false,
        "draft": false,
        "published_at": "2026-11-28T00:00:00Z",
        "assets": []
    }));
    workspace.write(
        "releases.json",
        serde_json::to_string_pretty(&releases)
            .expect("serialize releases")
            .as_bytes(),
    );
    (collie_35_sha, foxhound_35_sha)
}

fn verify_pip_and_uv_install(workspace: &Workspace, simple_url: &str, product: &str, module: &str) {
    let pip_venv = workspace.path().join(format!("venv-pip-{product}"));
    assert!(
        Command::new("uv")
            .args(["venv", "--seed", &pip_venv.to_string_lossy()])
            .status()
            .expect("create pip venv")
            .success()
    );
    let pip_install = Command::new(pip_venv.join("bin/pip"))
        .args([
            "install",
            "--quiet",
            "--pre",
            "--no-deps",
            "--index-url",
            simple_url,
            product,
        ])
        .output()
        .expect("run pip install");
    assert!(
        pip_install.status.success(),
        "pip install {product} failed:\n{}",
        String::from_utf8_lossy(&pip_install.stderr)
    );
    let pip_check = Command::new(pip_venv.join("bin/python"))
        .args([
            "-c",
            &format!("import {module}; assert {module}.__version__ == '0.1.0.dev35'"),
        ])
        .output()
        .expect("import wheel from pip");
    assert!(
        pip_check.status.success(),
        "{}",
        String::from_utf8_lossy(&pip_check.stderr)
    );

    let uv_venv = workspace.path().join(format!("venv-uv-{product}"));
    assert!(
        Command::new("uv")
            .args(["venv", &uv_venv.to_string_lossy()])
            .status()
            .expect("create uv venv")
            .success()
    );
    let uv_install = Command::new("uv")
        .args([
            "pip",
            "install",
            "--quiet",
            "--python",
            &uv_venv.to_string_lossy(),
            "--prerelease=allow",
            "--no-deps",
            "--index",
            &format!("goodfetch-nightly={simple_url}"),
            product,
        ])
        .output()
        .expect("run uv pip install");
    assert!(
        uv_install.status.success(),
        "uv pip install {product} failed:\n{}",
        String::from_utf8_lossy(&uv_install.stderr)
    );
    let uv_check = Command::new(uv_venv.join("bin/python"))
        .args([
            "-c",
            &format!("import {module}; assert {module}.__version__ == '0.1.0.dev35'"),
        ])
        .output()
        .expect("import wheel from uv");
    assert!(
        uv_check.status.success(),
        "{}",
        String::from_utf8_lossy(&uv_check.stderr)
    );
}

/// Covers: story:1-20
/// On a fixture of 35 nightlies of both Products, `xtask release python-index`
/// lists only the newest 30 with SHA-256 digests and provenance URLs on each
/// Product's own project page in both PEP 503 HTML and PEP 691 JSON, and both
/// `pip install --pre --index-url` and a `uv` index pinned to
/// `goodfetch-collie` or `goodfetch-foxhound` install from it served from a
/// temporary folder.
#[test]
fn nightly_python_index_keeps_last_30_and_installs_with_pip_and_uv() {
    let workspace = Workspace::from_fixture("empty");
    let (collie_35_sha, foxhound_35_sha) = seed_35_nightlies(&workspace);

    let outcome = workspace.xtask(&[
        "release",
        "python-index",
        "--releases",
        "releases.json",
        "--out",
        "site",
        "--keep",
        "30",
    ]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);

    for (project, dist, expected_35_sha) in [
        ("goodfetch-collie", "goodfetch_collie", &collie_35_sha),
        ("goodfetch-foxhound", "goodfetch_foxhound", &foxhound_35_sha),
    ] {
        let html = workspace.read_text(&format!("site/simple/{project}/index.html"));
        assert!(
            html.contains(&format!("{dist}-0.1.0.dev35-py3-none-any.whl"))
                && html.contains(&format!("{dist}-0.1.0.dev6-py3-none-any.whl"))
                && html.contains(&format!("#sha256={expected_35_sha}"))
                && html.contains("data-provenance="),
            "PEP 503 HTML for {project} missing expected entries:\n{html}"
        );
        for old in 1..=5u32 {
            assert!(
                !html.contains(&format!("{dist}-0.1.0.dev{old}-")),
                "PEP 503 HTML for {project} must omit older nightly dev{old}"
            );
        }

        let doc: Value = serde_json::from_str(
            &workspace.read_text(&format!("site/simple/{project}/index.json")),
        )
        .expect("parse PEP 691 JSON");
        assert_eq!(doc["meta"]["api-version"], "1.1");
        assert_eq!(doc["name"], project);
        let files = doc["files"].as_array().expect("files array");
        assert_eq!(files.len(), 30, "expected 30 nightlies for {project}");
        let latest = files
            .iter()
            .find(|f| f["filename"] == format!("{dist}-0.1.0.dev35-py3-none-any.whl"))
            .expect("dev35 entry");
        assert_eq!(latest["hashes"]["sha256"], expected_35_sha.as_str());
        assert!(latest["provenance"].as_str().is_some());
    }

    let simple_url = format!("file://{}", workspace.path().join("site/simple").display());
    verify_pip_and_uv_install(
        &workspace,
        &simple_url,
        "goodfetch-collie",
        "goodfetch_collie",
    );
    verify_pip_and_uv_install(
        &workspace,
        &simple_url,
        "goodfetch-foxhound",
        "goodfetch_foxhound",
    );

    let nightly =
        std::fs::read_to_string(repo_root().join(".github/workflows/release-nightly.yml"))
            .expect("read release-nightly.yml");
    let pub_pos = nightly
        .find("publish-github-prerelease:")
        .expect("publish-github-prerelease");
    let deploy_pos = nightly
        .find("deploy-python-index:")
        .expect("deploy-python-index");
    let smoke_pos = nightly
        .find("smoke-channels-linux:")
        .expect("smoke-channels-linux");
    assert!(pub_pos < deploy_pos && deploy_pos < smoke_pos);
    assert!(nightly.contains("release python-index") && nightly.contains("--keep 30"));
}
