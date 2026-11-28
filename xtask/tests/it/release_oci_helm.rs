//! Five OCI image tags (`<v>`, `-slim`, `-full`, `-node`, `-python`), shared
//! Helm chart OCI artifacts (`ghcr.io/goodfetch/charts/{collie,foxhound}`),
//! and the ADR 0049 seccomp profile (spec #130, issue 202; ADRs 0047, 0049,
//! 0088, 0090, 0094, 0096 and 0115).
//! Covers: story:1-13, story:1-14

use std::path::Path;

use serde_json::Value;

use crate::release_assets::TARGETS;
use crate::workspace::Workspace;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

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

fn workspace_with_oci_and_helm() -> Workspace {
    let workspace = Workspace::from_fixture("release");
    workspace.write(
        "crates/goodfetch-protocol/generated/goodfetch-catalog.json",
        include_bytes!("../../../crates/goodfetch-protocol/generated/goodfetch-catalog.json"),
    );
    workspace.write(
        "packaging/oci/seccomp-profile.json",
        include_bytes!("../../../packaging/oci/seccomp-profile.json"),
    );
    workspace.write(
        "packaging/helm/goodfetch/Chart.yaml",
        include_bytes!("../../../packaging/helm/goodfetch/Chart.yaml"),
    );
    workspace.write(
        "packaging/helm/goodfetch/values.yaml",
        include_bytes!("../../../packaging/helm/goodfetch/values.yaml"),
    );
    workspace.write(
        "packaging/helm/goodfetch/templates/deployment.yaml",
        include_bytes!("../../../packaging/helm/goodfetch/templates/deployment.yaml"),
    );
    workspace.write(
        "packaging/helm/goodfetch/templates/service.yaml",
        include_bytes!("../../../packaging/helm/goodfetch/templates/service.yaml"),
    );
    workspace.write(
        "sdks/python/collie-client/src/goodfetch_collie_client/__init__.py",
        b"__all__ = []\n",
    );
    workspace.write(
        "sdks/python/collie/src/goodfetch_collie/__init__.py",
        b"__all__ = []\n",
    );
    workspace.write(
        "sdks/python/foxhound/src/goodfetch_foxhound/__init__.py",
        b"__all__ = []\n",
    );
    for target in TARGETS {
        let suffix = exe_suffix(target);
        for executable in ["goodfetch-collie", "goodfetch-foxhound"] {
            workspace.write(
                &format!("bins/{target}/{executable}{suffix}"),
                format!("bin:{executable}:{target}").as_bytes(),
            );
            workspace.write(
                &format!("bins/{target}/{}", debug_name(executable, target)),
                b"debug-symbols",
            );
        }
    }
    workspace
}

#[test]
fn containerfile_builds_five_tags_on_debian_13_slim_uid_10001_with_rules_and_executable_last() {
    let text = std::fs::read_to_string(root().join("packaging/oci/Containerfile")).unwrap();
    assert!(
        text.contains("FROM docker.io/library/debian:trixie-slim@sha256:"),
        "Containerfile must pin Debian 13 slim by digest"
    );
    assert!(
        text.contains("GOODFETCH_BROWSERS_PATH=/usr/local/share/goodfetch/browsers"),
        "every image tag must set GOODFETCH_BROWSERS_PATH"
    );
    assert!(
        text.contains("COPY clean-up-rules/ /usr/local/share/goodfetch/browsers/clean-up-rules/"),
        "every image tag must ship the Clean-up rules read-only beside browsers"
    );

    // Every one of the five tags exists as a stage running as UID 10001 whose
    // final COPY layer is `PRODUCT_EXECUTABLE`.
    for stage in ["slim", "default", "full", "node", "python"] {
        let marker = format!("AS {stage}\n");
        let start = text
            .find(&marker)
            .unwrap_or_else(|| panic!("missing stage `{stage}`"));
        let rest = &text[start..];
        let next_from = rest[marker.len()..]
            .find("\nFROM ")
            .map_or(rest.len(), |idx| marker.len() + idx);
        let stage_body = &rest[..next_from];
        assert!(
            stage_body.contains("USER 10001:10001"),
            "stage `{stage}` must run as non-root UID 10001"
        );
        let last_non_empty = stage_body
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .unwrap();
        assert!(
            last_non_empty.contains("COPY --chmod=0755 bin/${TARGETARCH}/${PRODUCT_EXECUTABLE}"),
            "stage `{stage}` must keep PRODUCT_EXECUTABLE as its final layer, got: {last_non_empty}"
        );
    }

    // Seccomp profile allows Chromium's three user-namespace rules (`clone`
    // with `CLONE_NEWUSER`, `unshare`, `setns`) on top of Docker's default
    // profile (where `clone3` returns ENOSYS so glibc falls back to `clone`).
    let seccomp: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("packaging/oci/seccomp-profile.json")).unwrap(),
    )
    .unwrap();
    let rules = seccomp["syscalls"].as_array().unwrap();
    let allows_unconditional = |syscall: &str| {
        rules.iter().any(|rule| {
            rule["action"].as_str() == Some("SCMP_ACT_ALLOW")
                && rule.get("includes").is_none()
                && rule
                    .get("args")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty)
                && rule["names"]
                    .as_array()
                    .is_some_and(|names| names.iter().any(|n| n.as_str() == Some(syscall)))
        })
    };
    for syscall in ["unshare", "setns"] {
        assert!(
            allows_unconditional(syscall),
            "seccomp profile must allow `{syscall}`"
        );
    }
    let allows_clone_newuser = rules.iter().any(|rule| {
        rule["action"].as_str() == Some("SCMP_ACT_ALLOW")
            && rule["names"]
                .as_array()
                .is_some_and(|names| names.iter().any(|n| n.as_str() == Some("clone")))
            && rule["args"].as_array().is_some_and(|args| {
                args.iter().any(|arg| {
                    arg["op"].as_str() == Some("SCMP_CMP_MASKED_EQ")
                        && arg["value"].as_u64() == Some(0x1000_0000)
                        && arg["valueTwo"].as_u64() == Some(0x1000_0000)
                })
            })
    });
    assert!(
        allows_clone_newuser,
        "seccomp profile must allow `clone` with CLONE_NEWUSER"
    );
}

#[test]
fn prepare_versions_helm_chart_and_package_produces_attested_seccomp_and_both_charts() {
    let workspace = workspace_with_oci_and_helm();

    let prepared = workspace.xtask(&["release", "prepare", "--version", "1.4.0"]);
    assert_eq!(prepared.code, 0, "{}", prepared.stderr);
    let chart_yaml = workspace.read_text("packaging/helm/goodfetch/Chart.yaml");
    assert!(chart_yaml.contains("version: 1.4.0\n"), "{chart_yaml}");
    assert!(
        chart_yaml.contains("appVersion: \"1.4.0\"\n"),
        "{chart_yaml}"
    );

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
    assert!(
        workspace
            .read("out/assets/goodfetch-seccomp.json")
            .is_some(),
        "seccomp profile must be packaged as a GitHub Release asset"
    );

    // Sign and verify the seccomp profile asset in rehearsal.
    let signed = workspace.xtask(&[
        "release",
        "sign-rehearsal",
        "--file",
        "out/assets/goodfetch-seccomp.json",
        "--key-dir",
        "keys",
    ]);
    assert_eq!(signed.code, 0, "{}", signed.stderr);
    let verified = workspace.xtask(&[
        "release",
        "verify-rehearsal",
        "--file",
        "out/assets/goodfetch-seccomp.json",
        "--signature",
        "out/assets/goodfetch-seccomp.json.sig",
        "--attestation",
        "out/assets/goodfetch-seccomp.json.sigstore.jsonl",
        "--public-key",
        "keys/rehearsal.pub",
    ]);
    assert_eq!(verified.code, 0, "{}", verified.stderr);

    // Both Helm chart OCI layouts are emitted with CNCF Helm config media types.
    for product in ["collie", "foxhound"] {
        let index: Value = serde_json::from_str(
            &workspace.read_text(&format!("out/oci/charts/{product}/index.json")),
        )
        .unwrap();
        let digest = index["manifests"][0]["digest"]
            .as_str()
            .unwrap()
            .strip_prefix("sha256:")
            .unwrap();
        let manifest: Value = serde_json::from_str(
            &workspace.read_text(&format!("out/oci/charts/{product}/blobs/sha256/{digest}")),
        )
        .unwrap();
        assert_eq!(
            manifest["config"]["mediaType"].as_str(),
            Some("application/vnd.cncf.helm.config.v1+json")
        );
        assert_eq!(
            manifest["layers"][0]["mediaType"].as_str(),
            Some("application/vnd.cncf.helm.chart.content.v1.tar+gzip")
        );
    }

    // Both workflows build/publish all five image tags and both Helm charts.
    let build_yml =
        std::fs::read_to_string(root().join(".github/workflows/release-build.yml")).unwrap();
    assert!(build_yml.contains("for variant in default slim full node python"));
    assert!(build_yml.contains("--sbom=true"));

    let nightly_yml =
        std::fs::read_to_string(root().join(".github/workflows/release-nightly.yml")).unwrap();
    assert!(nightly_yml.contains("for variant in \"\" -slim -full -node -python"));
    assert!(nightly_yml.contains("goodfetch/charts/${product}"));
}

/// Pushes both Helm chart OCI artifacts to a real `registry:2` container and
/// pulls them back through OCI Distribution, verifying both charts install from
/// their own Product repositories and differ only by Product image.
#[cfg(target_os = "linux")]
#[test]
fn rehearsal_pushes_and_pulls_both_helm_charts_on_registry_2() {
    use crate::container::Container;

    const DISTRIBUTION_PIN: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ci/registry/image.toml"
    ));
    let workspace = workspace_with_oci_and_helm();
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

    let registry = Container::start(DISTRIBUTION_PIN, 5000, "/v2/", &[]);
    let url = registry.url();

    for product in ["collie", "foxhound"] {
        let pushed = workspace.xtask(&[
            "release",
            "oci-push",
            "--layout",
            &format!("out/oci/charts/{product}"),
            "--registry",
            url,
            "--repository",
            &format!("goodfetch/charts/{product}"),
            "--tag",
            "1.4.0",
        ]);
        assert_eq!(pushed.code, 0, "{}", pushed.stderr);

        let mut response = ureq::get(&format!(
            "{url}/v2/goodfetch/charts/{product}/manifests/1.4.0"
        ))
        .header("Accept", "application/vnd.oci.image.manifest.v1+json")
        .call()
        .unwrap();
        let manifest: Value =
            serde_json::from_slice(&response.body_mut().read_to_vec().unwrap()).unwrap();
        assert_eq!(
            manifest["config"]["mediaType"].as_str(),
            Some("application/vnd.cncf.helm.config.v1+json")
        );
        let layer_digest = manifest["layers"][0]["digest"].as_str().unwrap();
        let mut blob_resp = ureq::get(&format!(
            "{url}/v2/goodfetch/charts/{product}/blobs/{layer_digest}"
        ))
        .call()
        .unwrap();
        let tgz_bytes = blob_resp.body_mut().read_to_vec().unwrap();
        assert!(!tgz_bytes.is_empty(), "pulled chart tarball is empty");
    }
}
