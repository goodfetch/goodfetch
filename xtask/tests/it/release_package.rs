//! `xtask release package` and `xtask release oci-push`: produces every release
//! artifact of both Products from one set of built binaries — release archives,
//! split debug symbols, `CycloneDX` SBOMs, shared helpers built once, npm root
//! and platform packages (with `optionalDependencies` and no install scripts),
//! `py3-none` platform wheels and pure client wheel, and multi-platform OCI
//! image indexes for first-party Plugins (spec #130, issue 141; ADRs 0024,
//! 0032, 0033, 0090, 0094 and 0095).
//! Covers: story:1-80, story:1-91

use std::path::Path;

use serde_json::Value;

use crate::release_assets::TARGETS;
use crate::workspace::Workspace;

const NPM_PLATFORMS: [&str; 6] = [
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64",
    "linux-x64",
    "win32-arm64",
    "win32-x64",
];

const WHEEL_TAGS: [&str; 6] = [
    "manylinux_2_17_x86_64.musllinux_1_2_x86_64",
    "manylinux_2_17_aarch64.musllinux_1_2_aarch64",
    "macosx_11_0_arm64",
    "macosx_10_15_x86_64",
    "win_amd64",
    "win_arm64",
];

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

fn populated_workspace() -> Workspace {
    let workspace = Workspace::from_fixture("release");
    // Minimal Python package sources so wheel assembly packages real modules.
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"__all__ = []\n",
    );
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/py.typed",
        b"",
    );
    workspace.write(
        "sdks/python/collie/src/goodfetch_collie/__init__.py",
        b"__all__ = []\n",
    );
    workspace.write("sdks/python/collie/src/goodfetch_collie/py.typed", b"");
    workspace.write(
        "sdks/python/foxhound/src/goodfetch_foxhound/__init__.py",
        b"__all__ = []\n",
    );
    workspace.write("sdks/python/foxhound/src/goodfetch_foxhound/py.typed", b"");

    for target in TARGETS {
        let suffix = exe_suffix(target);
        for executable in ["goodfetch-collie", "goodfetch-foxhound"] {
            workspace.write(
                &format!("dist-bins/{target}/{executable}{suffix}"),
                format!("bin:{executable}:{target}").as_bytes(),
            );
            workspace.write(
                &format!("dist-bins/{target}/{}", debug_name(executable, target)),
                format!("dbg:{executable}:{target}").as_bytes(),
            );
        }
        workspace.write(
            &format!("dist-bins/{target}/goodfetch-proving-ground{suffix}"),
            format!("bin:proving-ground:{target}").as_bytes(),
        );
        if target.contains("-linux-") {
            workspace.write(
                &format!("dist-bins/{target}/goodfetch-sandbox"),
                format!("bin:sandbox:{target}").as_bytes(),
            );
        }
        if target.contains("-windows-") {
            workspace.write(
                &format!("dist-bins/{target}/goodfetch-filter-broker.exe"),
                format!("bin:filter-broker:{target}").as_bytes(),
            );
        }
        workspace.write(
            &format!("dist-bins/{target}/goodfetch-plugin-archive{suffix}"),
            format!("bin:plugin-archive:{target}").as_bytes(),
        );
    }
    workspace
}

#[test]
fn package_produces_archives_debug_symbols_sboms_npm_wheels_and_plugin_oci_index() {
    let workspace = populated_workspace();

    let outcome = workspace.xtask(&[
        "release",
        "package",
        "--version",
        "1.4.0",
        "--binaries",
        "dist-bins",
        "--out",
        "packaged",
    ]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);

    // 1. Assets + digest table + crate digest pinning work end-to-end.
    let assets_out = workspace.xtask(&["release", "assets", "--folder", "packaged/assets"]);
    assert_eq!(assets_out.code, 0, "{}", assets_out.stderr);
    let digests_out = workspace.xtask(&[
        "release",
        "digests",
        "--assets",
        "packaged/assets",
        "--version",
        "1.4.0",
    ]);
    assert_eq!(digests_out.code, 0, "{}", digests_out.stderr);

    // Shared components built once, not once per Product.
    assert!(
        workspace
            .read("packaged/assets/goodfetch-proving-ground-x86_64-unknown-linux-musl.tar.xz")
            .is_some()
    );
    assert!(
        workspace
            .read("packaged/assets/goodfetch-sandbox-x86_64-unknown-linux-musl.tar.xz")
            .is_some()
    );
    assert!(
        workspace
            .read("packaged/assets/goodfetch-filter-broker-x86_64-pc-windows-msvc.tar.xz")
            .is_some()
    );
    assert!(
        workspace
            .read("packaged/assets/goodfetch-collie-sandbox-x86_64-unknown-linux-musl.tar.xz")
            .is_none()
    );

    // CycloneDX SBOM per Product.
    for executable in ["goodfetch-collie", "goodfetch-foxhound"] {
        let sbom: Value = serde_json::from_str(
            &workspace.read_text(&format!("packaged/assets/{executable}.cdx.json")),
        )
        .unwrap();
        assert_eq!(sbom["bomFormat"].as_str(), Some("CycloneDX"));
        assert_eq!(
            sbom["metadata"]["component"]["name"].as_str(),
            Some(executable)
        );
        assert_eq!(
            sbom["metadata"]["component"]["version"].as_str(),
            Some("1.4.0")
        );
    }

    assert_npm_packages(&workspace);
    assert_wheels_and_plugins(&workspace);
}

fn assert_npm_packages(workspace: &Workspace) {
    let client_pkg: Value =
        serde_json::from_str(&workspace.read_text("packaged/npm/collie-client/package.json"))
            .unwrap();
    assert_eq!(
        client_pkg["name"].as_str(),
        Some("@goodfetch/collie-client")
    );
    assert_eq!(client_pkg["version"].as_str(), Some("1.4.0"));
    assert!(client_pkg.get("private").is_none());
    assert!(client_pkg.get("scripts").is_none());

    for product in ["collie", "foxhound"] {
        let root_pkg: Value = serde_json::from_str(
            &workspace.read_text(&format!("packaged/npm/{product}/package.json")),
        )
        .unwrap();
        assert_eq!(root_pkg["version"].as_str(), Some("1.4.0"));
        assert!(root_pkg.get("private").is_none());
        assert!(root_pkg.get("scripts").is_none());
        assert_eq!(
            root_pkg["dependencies"]["@goodfetch/collie-client"].as_str(),
            Some("1.4.0")
        );
        let optional = root_pkg["optionalDependencies"].as_object().unwrap();
        assert_eq!(optional.len(), NPM_PLATFORMS.len());
        for platform in NPM_PLATFORMS {
            let pkg_name = format!("@goodfetch/{product}-{platform}");
            assert_eq!(optional[&pkg_name].as_str(), Some("1.4.0"));
            let plat_pkg: Value = serde_json::from_str(
                &workspace.read_text(&format!("packaged/npm/{product}-{platform}/package.json")),
            )
            .unwrap();
            assert_eq!(plat_pkg["name"].as_str(), Some(pkg_name.as_str()));
            assert!(plat_pkg.get("scripts").is_none());
        }
        // Linux npm platform package carries the static musl build (ADR 0090).
        let linux_bin = workspace
            .read(&format!(
                "packaged/npm/{product}-linux-x64/bin/goodfetch-{product}"
            ))
            .unwrap();
        assert_eq!(
            linux_bin,
            format!("bin:goodfetch-{product}:x86_64-unknown-linux-musl").as_bytes()
        );
    }
}

fn assert_wheels_and_plugins(workspace: &Workspace) {
    assert!(
        workspace
            .read("packaged/wheels/goodfetch_collie_client-1.4.0-py3-none-any.whl")
            .is_some()
    );
    for product in ["collie", "foxhound"] {
        for tag in WHEEL_TAGS {
            let wheel_path =
                format!("packaged/wheels/goodfetch_{product}-1.4.0-py3-none-{tag}.whl");
            let bytes = workspace
                .read(&wheel_path)
                .unwrap_or_else(|| panic!("missing wheel {wheel_path}"));
            assert_eq!(&bytes[..4], b"PK\x03\x04", "{wheel_path} is not a ZIP");
        }
    }

    let layout: Value = serde_json::from_str(
        &workspace.read_text("packaged/oci/plugins/goodfetch-plugin-archive/oci-layout"),
    )
    .unwrap();
    assert_eq!(layout["imageLayoutVersion"].as_str(), Some("1.0.0"));
    let index: Value = serde_json::from_str(
        &workspace.read_text("packaged/oci/plugins/goodfetch-plugin-archive/index.json"),
    )
    .unwrap();
    assert_eq!(
        index["mediaType"].as_str(),
        Some("application/vnd.oci.image.index.v1+json")
    );
    assert_eq!(index["manifests"].as_array().unwrap().len(), 6);
}

#[cfg(target_os = "linux")]
#[test]
fn oci_push_publishes_plugin_multi_platform_index_to_distribution() {
    use crate::container::Container;

    const PIN: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ci/registry/image.toml"
    ));

    let workspace = populated_workspace();
    let packaged = workspace.xtask(&[
        "release",
        "package",
        "--version",
        "1.4.0",
        "--binaries",
        "dist-bins",
        "--out",
        "packaged",
    ]);
    assert_eq!(packaged.code, 0, "{}", packaged.stderr);

    let registry = Container::start(PIN, 5000, "/v2/", &[]);
    let pushed = workspace.xtask(&[
        "release",
        "oci-push",
        "--layout",
        "packaged/oci/plugins/goodfetch-plugin-archive",
        "--registry",
        registry.url(),
        "--repository",
        "goodfetch/goodfetch-plugin-archive",
        "--tag",
        "1.4.0",
    ]);
    assert_eq!(pushed.code, 0, "{}", pushed.stderr);

    let response = ureq::get(&format!(
        "{}/v2/goodfetch/goodfetch-plugin-archive/manifests/1.4.0",
        registry.url()
    ))
    .header("Accept", "application/vnd.oci.image.index.v1+json")
    .call()
    .unwrap();
    assert_eq!(response.status(), 200);
    let index: Value = response.into_body().read_json().unwrap();
    assert_eq!(index["manifests"].as_array().unwrap().len(), 6);
}

#[test]
fn shared_containerfile_pins_debian_13_slim_runs_non_root_and_copies_executable_last() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let text = std::fs::read_to_string(root.join("packaging/oci/Containerfile")).unwrap();
    assert!(
        text.contains("FROM docker.io/library/debian:trixie-slim@sha256:"),
        "Containerfile must pin Debian 13 slim by digest"
    );
    assert!(
        text.contains("USER 10001:10001"),
        "Containerfile must run as non-root UID 10001"
    );
    let last_instruction = text
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap();
    assert!(
        last_instruction.starts_with("COPY ") && last_instruction.contains("PRODUCT_EXECUTABLE"),
        "final layer must be the Product executable COPY, got: {last_instruction}"
    );
}
