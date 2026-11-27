//! Platform signing gate and signed-artifact reuse (`xtask release
//! signature-gate` and `xtask release signing-reuse-key`; ADRs 0039, 0051,
//! 0074, 0076, 0088 and 0094; issue 142).
//!
//! Policy (ADR 0039 + 2026-11-11 grilling G-18 Q11):
//! - While the paid Apple Developer ID and Azure Artifact Signing accounts
//!   remain declined (`--accounts-open=false`), a `0.y` preview (`major == 0`)
//!   skips Platform signing and the gate reports `not run: accounts not open`
//!   without blocking.
//! - Any other version (`major >= 1`, including `1.0.0-rc.1`) blocks unless
//!   Platform signing accounts are open and every macOS and Windows executable
//!   and Goodfetch Chromium `.app` bundle passes verification.
//! - Hardened runtime on macOS must never request `com.apple.security.cs.allow-jit`
//!   (both Products embed V8 with `--jitless`, ADRs 0051 and 0088).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{ProductVersion, assets};
use crate::check::{self, Finding};
use crate::output;

/// Exact status message printed and posted to the release PR when signing
/// accounts are not yet open for a `0.y` preview (issue 142, grilling G-18).
pub(super) const ACCOUNTS_NOT_OPEN_MESSAGE: &str = "not run: accounts not open";

/// macOS entitlement forbidden for both Products (V8 runs `--jitless`).
const FORBIDDEN_ENTITLEMENT: &str = "com.apple.security.cs.allow-jit";

const MACOS_TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
const WINDOWS_TARGETS: [&str; 2] = ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"];

const MACOS_REQUIRED_BINARIES: [&str; 3] = [
    "goodfetch-collie",
    "goodfetch-foxhound",
    "goodfetch-proving-ground",
];

const WINDOWS_REQUIRED_BINARIES: [&str; 4] = [
    "goodfetch-collie.exe",
    "goodfetch-foxhound.exe",
    "goodfetch-proving-ground.exe",
    "goodfetch-filter-broker.exe",
];

#[derive(clap::Args)]
pub(crate) struct SignatureGateArguments {
    /// Product version being released.
    #[arg(long)]
    version: ProductVersion,
    /// Whether the Apple Developer ID and Azure Artifact Signing accounts are
    /// open and signing jobs ran.
    #[arg(long, default_value_t = false)]
    accounts_open: bool,
    /// JSON verification report emitted by `verify-platform-signatures.{sh,ps1}`
    /// when `accounts_open` is true.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Optional macOS entitlements file to verify contains no `allow-jit`.
    #[arg(long)]
    entitlements: Option<PathBuf>,
    /// Optional path where the gate writes its status note for the release PR.
    #[arg(long)]
    pr_comment: Option<PathBuf>,
}

#[derive(clap::Args)]
pub(crate) struct ReuseKeyArguments {
    /// Product version being signed.
    #[arg(long)]
    version: ProductVersion,
    /// Folder of unsigned binaries to fingerprint (`<target>/<file>`).
    #[arg(long)]
    binaries: PathBuf,
    /// Path where the computed reuse manifest JSON is written.
    #[arg(long)]
    out: PathBuf,
    /// Optional prior signed-artifact reuse manifest for the same version;
    /// when its version and unsigned digests match, signed bytes are reused.
    #[arg(long)]
    existing: Option<PathBuf>,
}

#[derive(Deserialize)]
struct SignatureReport {
    #[serde(default)]
    macos: Vec<MacosBinaryCheck>,
    #[serde(default)]
    macos_apps: Vec<MacosAppCheck>,
    #[serde(default)]
    windows: Vec<WindowsBinaryCheck>,
}

#[derive(Deserialize)]
struct MacosBinaryCheck {
    target: String,
    binary: String,
    codesign_strict: bool,
    notarized: bool,
    allow_jit: bool,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "deserializes the four independent macOS .app verification checks emitted by verify-platform-signatures.sh"
)]
#[derive(Deserialize)]
struct MacosAppCheck {
    target: String,
    bundle: String,
    codesign_deep: bool,
    spctl_ok: bool,
    stapler_valid: bool,
    /// ADR 0039 allows `allow-jit` only on Chromium renderer/GPU helpers (signed
    /// without library validation), never on the main `.app` binary or any
    /// other part.
    #[serde(alias = "allow_jit")]
    allow_jit_outside_helpers: bool,
}

#[derive(Deserialize)]
struct WindowsBinaryCheck {
    target: String,
    binary: String,
    signtool_pa: bool,
    rfc3161_timestamp: bool,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct ReuseManifest {
    version: String,
    reuse_key: String,
    files: BTreeMap<String, String>,
}

/// Evaluates the Platform signature gate for `arguments.version`.
pub(super) fn run_signature_gate(
    root: &Path,
    arguments: &SignatureGateArguments,
) -> Result<ExitCode> {
    let mut findings = Vec::new();

    if let Some(entitlements) = &arguments.entitlements {
        let path = resolve(root, entitlements);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading entitlements {}", path.display()))?;
        if text.contains(FORBIDDEN_ENTITLEMENT) {
            findings.push(Finding::new(
                "forbidden-entitlement",
                entitlements.display().to_string(),
                format!(
                    "hardened runtime must not request `{FORBIDDEN_ENTITLEMENT}` (both Products run V8 with --jitless)"
                ),
            ));
        }
    }

    if !arguments.accounts_open {
        if !findings.is_empty() {
            return Ok(check::report(&findings));
        }
        if arguments.version.major == 0 {
            if let Some(pr_comment) = &arguments.pr_comment {
                let path = resolve(root, pr_comment);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, format!("{ACCOUNTS_NOT_OPEN_MESSAGE}\n"))
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            output::line(ACCOUNTS_NOT_OPEN_MESSAGE);
            return Ok(ExitCode::SUCCESS);
        }
        findings.push(Finding::new(
            "platform-signature-required",
            arguments.version.semver(),
            "Platform signing accounts are not open; only 0.y previews may publish Sigstore-only (ADR 0039)",
        ));
        return Ok(check::report(&findings));
    }

    let Some(report_path) = &arguments.report else {
        findings.push(Finding::new(
            "missing-signature-report",
            arguments.version.semver(),
            "accounts are open, so `--report` must provide per-target Platform signature checks",
        ));
        return Ok(check::report(&findings));
    };
    let report_full = resolve(root, report_path);
    let report_text = std::fs::read_to_string(&report_full)
        .with_context(|| format!("reading signature report {}", report_full.display()))?;
    let report: SignatureReport = serde_json::from_str(&report_text)
        .with_context(|| format!("parsing signature report {}", report_full.display()))?;

    check_macos_binaries(&report.macos, &mut findings);
    check_macos_apps(&report.macos_apps, &mut findings);
    check_windows_binaries(&report.windows, &mut findings);

    if findings.is_empty() {
        let message = format!(
            "platform signatures verified for {}",
            arguments.version.semver()
        );
        if let Some(pr_comment) = &arguments.pr_comment {
            let path = resolve(root, pr_comment);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, format!("{message}\n"))
                .with_context(|| format!("writing {}", path.display()))?;
        }
        output::line(&message);
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(check::report(&findings))
    }
}

fn check_macos_binaries(entries: &[MacosBinaryCheck], findings: &mut Vec<Finding>) {
    let mut seen = BTreeMap::new();
    for entry in entries {
        let key = format!("{}/{}", entry.target, entry.binary);
        if !entry.codesign_strict {
            findings.push(Finding::new(
                "unsigned-artifact",
                &key,
                "`codesign --verify --strict` failed",
            ));
        }
        if !entry.notarized {
            findings.push(Finding::new(
                "notarization-failed",
                &key,
                "`codesign --check-notarization` failed",
            ));
        }
        if entry.allow_jit {
            findings.push(Finding::new(
                "forbidden-entitlement",
                &key,
                format!("binary carries forbidden `{FORBIDDEN_ENTITLEMENT}` entitlement"),
            ));
        }
        seen.insert((entry.target.as_str(), entry.binary.as_str()), true);
    }
    for target in MACOS_TARGETS {
        for binary in MACOS_REQUIRED_BINARIES {
            if !seen.contains_key(&(target, binary)) {
                findings.push(Finding::new(
                    "missing-target-verification",
                    format!("{target}/{binary}"),
                    "macOS executable was not verified by the Platform signature gate",
                ));
            }
        }
    }
}

fn check_macos_apps(entries: &[MacosAppCheck], findings: &mut Vec<Finding>) {
    let mut seen_targets = BTreeMap::new();
    for app in entries {
        let key = format!("{}/{}", app.target, app.bundle);
        if !app.codesign_deep {
            findings.push(Finding::new(
                "app-bundle-invalid",
                &key,
                "`codesign --verify --deep --strict` failed on `.app` bundle",
            ));
        }
        if !app.spctl_ok {
            findings.push(Finding::new(
                "app-bundle-invalid",
                &key,
                "`spctl --assess --type execute` rejected `.app` bundle",
            ));
        }
        if !app.stapler_valid {
            findings.push(Finding::new(
                "notarization-failed",
                &key,
                "`xcrun stapler validate` failed on `.app` bundle",
            ));
        }
        if app.allow_jit_outside_helpers {
            findings.push(Finding::new(
                "forbidden-entitlement",
                &key,
                format!(
                    "`.app` bundle carries `{FORBIDDEN_ENTITLEMENT}` outside renderer/GPU helpers (ADR 0039)"
                ),
            ));
        }
        seen_targets.insert(app.target.as_str(), true);
    }
    for target in MACOS_TARGETS {
        if !seen_targets.contains_key(target) {
            findings.push(Finding::new(
                "missing-target-verification",
                format!("{target}/Goodfetch Chromium.app"),
                "signed macOS Goodfetch Chromium `.app` bundle was not verified for this target",
            ));
        }
    }
}

fn check_windows_binaries(entries: &[WindowsBinaryCheck], findings: &mut Vec<Finding>) {
    let mut seen = BTreeMap::new();
    for entry in entries {
        let key = format!("{}/{}", entry.target, entry.binary);
        if !entry.signtool_pa {
            findings.push(Finding::new(
                "unsigned-artifact",
                &key,
                "`signtool verify /pa` failed",
            ));
        }
        if !entry.rfc3161_timestamp {
            findings.push(Finding::new(
                "missing-timestamp",
                &key,
                "Windows Authenticode signature lacks a valid RFC 3161 timestamp",
            ));
        }
        seen.insert((entry.target.as_str(), entry.binary.as_str()), true);
    }
    for target in WINDOWS_TARGETS {
        for binary in WINDOWS_REQUIRED_BINARIES {
            if !seen.contains_key(&(target, binary)) {
                findings.push(Finding::new(
                    "missing-target-verification",
                    format!("{target}/{binary}"),
                    "Windows executable was not verified by the Platform signature gate",
                ));
            }
        }
    }
}

/// Computes a deterministic reuse key over `(version, unsigned_file_digests)`
/// so re-runs of the same version reuse already-signed artifacts (issue 142).
pub(super) fn run_reuse_key(root: &Path, arguments: &ReuseKeyArguments) -> Result<ExitCode> {
    let binaries = resolve(root, &arguments.binaries);
    ensure!(
        binaries.is_dir(),
        "unsigned binaries directory does not exist: {}",
        binaries.display()
    );
    let mut files = BTreeMap::new();
    collect_files(&binaries, &binaries, &mut files)?;
    ensure!(
        !files.is_empty(),
        "no files found under {}",
        binaries.display()
    );

    let semver = arguments.version.semver();
    let mut hasher = Sha256::new();
    hasher.update(semver.as_bytes());
    hasher.update(b"\0");
    for (rel, digest) in &files {
        hasher.update(rel.as_bytes());
        hasher.update(b"=");
        hasher.update(digest.as_bytes());
        hasher.update(b"\n");
    }
    let reuse_key = format!("{:x}", hasher.finalize());
    let manifest = ReuseManifest {
        version: semver,
        reuse_key: reuse_key.clone(),
        files,
    };

    let out_path = resolve(root, &arguments.out);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(&manifest)?;
    text.push('\n');
    std::fs::write(&out_path, text).with_context(|| format!("writing {}", out_path.display()))?;

    if let Some(existing_path) = &arguments.existing {
        let existing_full = resolve(root, existing_path);
        if existing_full.is_file() {
            let existing_text = std::fs::read_to_string(&existing_full)
                .with_context(|| format!("reading {}", existing_full.display()))?;
            if let Ok(existing) = serde_json::from_str::<ReuseManifest>(&existing_text)
                && existing == manifest
            {
                output::line(&format!("reuse {reuse_key}"));
                return Ok(ExitCode::SUCCESS);
            }
        }
    }

    output::line(&format!("sign {reuse_key}"));
    Ok(ExitCode::SUCCESS)
}

fn collect_files(base: &Path, current: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(current)
        .with_context(|| format!("reading {}", current.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(base)
                .context("computing relative path")?
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, assets::sha256_file(&path)?);
        }
    }
    Ok(())
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}
