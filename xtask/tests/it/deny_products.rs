//! What each Product ships (ADR 0087): `ci/deny/products.sh` runs
//! cargo-deny's bans once per Product, rooted at its executable crate. A
//! Product whose executable crate the workspace lacks fails the check, so a
//! moved or renamed crate cannot leave a Product unchecked. The gate's own
//! tooling, so the module declares no Feature.
#![cfg(unix)]

use std::path::Path;

use crate::workspace::Workspace;

/// A workspace whose only member is Collie's executable crate, and a
/// stand-in cargo-deny that passes every check: `cargo deny` answers 0, and
/// every other cargo command is cargo's.
fn collie_alone() -> (Workspace, String) {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = Workspace::from_fixture("empty");
    workspace.write(
        "Cargo.toml",
        b"[workspace]\nresolver = \"3\"\nmembers = [\"goodfetch-collie\"]\n",
    );
    workspace.write(
        "goodfetch-collie/Cargo.toml",
        b"[package]\nname = \"goodfetch-collie\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    );
    workspace.write("goodfetch-collie/src/main.rs", b"fn main() {}\n");
    let stand_ins = workspace.path().join("stand-ins");
    std::fs::create_dir_all(&stand_ins).unwrap();
    let outer = std::env::var("PATH").unwrap();
    let cargo = stand_ins.join("cargo");
    std::fs::write(
        &cargo,
        format!(
            "#!/bin/sh\nif [ \"$1\" = deny ]; then exit 0; fi\nPATH=\"{outer}\" exec cargo \"$@\"\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{outer}", stand_ins.display());
    (workspace, path)
}

#[test]
fn a_product_whose_executable_crate_is_missing_fails() {
    let (workspace, path) = collie_alone();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ci/deny/products.sh");

    let output = workspace
        .command(script)
        .arg(workspace.path().join("Cargo.toml"))
        .env("PATH", path)
        .output()
        .unwrap();

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("goodfetch-foxhound"), "{stderr}");
}
