//! Release channel gate (`xtask release channel-gate`; ADRs 0030, 0032, 0039,
//! 0057 and 0094; issue 143):
//! - Verifies every channel (GitHub Release `.tar.xz` archive, npm platform
//!   package, Python `py3-none` platform wheel) carries the exact same
//!   executable SHA-256 bytes for both Products and every target.
//! - Refuses forbidden nightly publish channels (`pypi`, `crates.io`,
//!   `homebrew`, ADRs 0030 and 0057).
//! - Verifies artifact attestations and signatures (offline via rehearsal
//!   public key, or live via `gh attestation verify --repo`).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};

use super::{ProductVersion, Stage, assets, attest};
use crate::check::{self, Finding};
use crate::output;
use crate::products::Product;

/// Channels a nightly release must never publish to (ADRs 0030 and 0057).
const FORBIDDEN_NIGHTLY_CHANNELS: [&str; 3] = ["pypi", "crates.io", "homebrew"];

/// Platform triples mapped across GitHub archive, npm platform package, and
/// Python `py3-none` wheel tags.
const CHANNEL_PLATFORMS: [(&str, &str, &str); 6] = [
    ("aarch64-apple-darwin", "darwin-arm64", "macosx_11_0_arm64"),
    ("x86_64-apple-darwin", "darwin-x64", "macosx_10_15_x86_64"),
    (
        "aarch64-unknown-linux-musl",
        "linux-arm64",
        "manylinux_2_17_aarch64.musllinux_1_2_aarch64",
    ),
    (
        "x86_64-unknown-linux-musl",
        "linux-x64",
        "manylinux_2_17_x86_64.musllinux_1_2_x86_64",
    ),
    ("aarch64-pc-windows-msvc", "win32-arm64", "win_arm64"),
    ("x86_64-pc-windows-msvc", "win32-x64", "win_amd64"),
];

#[derive(clap::Args)]
pub(crate) struct ChannelGateArguments {
    /// Product version being gated.
    #[arg(long)]
    version: ProductVersion,
    /// Folder of built/signed reference binaries (`<target>/<exe>`).
    #[arg(long)]
    binaries: PathBuf,
    /// Folder produced by `xtask release package` (`assets/`, `npm/`, `wheels/`).
    #[arg(long)]
    packaged: PathBuf,
    /// Comma-separated publish channels targeted by this workflow.
    #[arg(long, value_delimiter = ',')]
    channels: Vec<String>,
    /// Verify attestations on release assets listed in `assets/SHA256SUMS`.
    #[arg(long, default_value_t = false)]
    verify_attestations: bool,
    /// Optional Ed25519 rehearsal public key for offline attestation checks;
    /// when omitted with `--verify-attestations`, calls `gh attestation verify`.
    #[arg(long)]
    public_key: Option<PathBuf>,
    /// Repository `owner/name` passed to `gh attestation verify`.
    #[arg(long, default_value = "goodfetch/goodfetch")]
    repository: String,
    /// Optional targets subset; defaults to all eight release targets.
    #[arg(long, value_delimiter = ',')]
    targets: Vec<String>,
}

/// Runs the cross-channel digest, nightly-channel policy, and attestation gate.
pub(super) fn run(root: &Path, arguments: &ChannelGateArguments) -> Result<ExitCode> {
    let binaries = resolve(root, &arguments.binaries);
    let packaged = resolve(root, &arguments.packaged);
    let mut findings = Vec::new();

    // 1. Nightly channel policy (ADRs 0030 and 0057): never PyPI, crates.io, or Homebrew.
    if matches!(arguments.version.stage, Stage::Nightly(_)) {
        for channel in &arguments.channels {
            let normalized = channel.to_ascii_lowercase();
            if FORBIDDEN_NIGHTLY_CHANNELS.iter().any(|forbidden| {
                normalized == *forbidden || normalized.starts_with(&format!("{forbidden}:"))
            }) {
                findings.push(Finding::new(
                    "forbidden-nightly-channel",
                    channel,
                    "nightlies never publish to PyPI, crates.io or Homebrew (ADRs 0030 and 0057)",
                ));
            }
        }
    }

    // 2. Verify identical executable bytes across GitHub Release archives, npm
    //    platform packages, and Python `py3-none` platform wheels.
    let targets: Vec<&str> = if arguments.targets.is_empty() {
        assets::TARGETS.to_vec()
    } else {
        arguments.targets.iter().map(String::as_str).collect()
    };
    let pep440 = arguments.version.pep440();

    for target in &targets {
        for product in Product::ALL {
            verify_target_channels(
                &binaries,
                &packaged,
                product,
                target,
                &pep440,
                &mut findings,
            )?;
        }
    }

    // 3. Attestation verification over release assets when requested.
    if arguments.verify_attestations {
        verify_asset_attestations(
            root,
            &packaged.join("assets"),
            arguments.public_key.as_deref(),
            &arguments.repository,
            &mut findings,
        )?;
    }

    if findings.is_empty() {
        output::line(&format!(
            "channel-gate passed for {} ({} targets)",
            arguments.version.semver(),
            targets.len()
        ));
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(check::report(&findings))
    }
}

fn verify_target_channels(
    binaries: &Path,
    packaged: &Path,
    product: Product,
    target: &str,
    pep440: &str,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let exe = exe_name(product.names().executable, target);
    let ref_path = binaries.join(target).join(&exe);
    if !ref_path.is_file() {
        findings.push(Finding::new(
            "missing-channel",
            format!("binaries/{target}/{exe}"),
            "reference binary missing",
        ));
        return Ok(());
    }
    let expected = assets::sha256_file(&ref_path)?;

    let archive_path = packaged
        .join("assets")
        .join(assets::archive(product, target));
    match sha256_in_tar_xz(&archive_path, &exe) {
        Ok(actual) if actual == expected => {}
        Ok(actual) => findings.push(Finding::new(
            "channel-digest-mismatch",
            format!("github-archive/{target}/{exe}"),
            format!("archive digest {actual} differs from signed binary {expected}"),
        )),
        Err(error) => findings.push(Finding::new(
            "missing-channel",
            format!("github-archive/{target}/{exe}"),
            error.to_string(),
        )),
    }

    if let Some((_, npm_suffix, wheel_tag)) =
        CHANNEL_PLATFORMS.iter().find(|(t, _, _)| *t == target)
    {
        let product_key = product.key();
        let npm_bin = packaged
            .join("npm")
            .join(format!("{product_key}-{npm_suffix}"))
            .join("bin")
            .join(&exe);
        if npm_bin.is_file() {
            let actual = assets::sha256_file(&npm_bin)?;
            if actual != expected {
                findings.push(Finding::new(
                    "channel-digest-mismatch",
                    format!("npm/{product_key}-{npm_suffix}/bin/{exe}"),
                    format!("npm digest {actual} differs from signed binary {expected}"),
                ));
            }
        } else {
            findings.push(Finding::new(
                "missing-channel",
                format!("npm/{product_key}-{npm_suffix}/bin/{exe}"),
                "npm platform package binary missing",
            ));
        }

        let dist = format!("goodfetch_{product_key}");
        let wheel_name = format!("{dist}-{pep440}-py3-none-{wheel_tag}.whl");
        let wheel_path = packaged.join("wheels").join(&wheel_name);
        let member = format!("{dist}/bin/{exe}");
        match sha256_in_zip(&wheel_path, &member) {
            Ok(actual) if actual == expected => {}
            Ok(actual) => findings.push(Finding::new(
                "channel-digest-mismatch",
                format!("wheel/{wheel_name}:{member}"),
                format!("wheel digest {actual} differs from signed binary {expected}"),
            )),
            Err(error) => findings.push(Finding::new(
                "missing-channel",
                format!("wheel/{wheel_name}:{member}"),
                error.to_string(),
            )),
        }
    }
    Ok(())
}

fn verify_asset_attestations(
    root: &Path,
    assets_dir: &Path,
    public_key: Option<&Path>,
    repository: &str,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let Some(listed) = assets::listed(assets_dir)? else {
        findings.push(Finding::new(
            "missing-attestation",
            assets::TABLE,
            "SHA256SUMS missing from assets directory",
        ));
        return Ok(());
    };
    let pub_key_full = public_key.map(|p| resolve(root, p));
    for file_name in listed.keys() {
        let is_sig = Path::new(file_name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sig"));
        if is_sig || file_name.ends_with(".sigstore.jsonl") {
            continue;
        }
        let file_path = assets_dir.join(file_name);
        if !file_path.is_file() {
            continue;
        }
        if let Some(pub_key) = pub_key_full.as_deref() {
            let sig = assets_dir.join(format!("{file_name}.sig"));
            let att = assets_dir.join(format!("{file_name}.sigstore.jsonl"));
            if let Err(error) = attest::verify_rehearsal_paths(&file_path, &sig, &att, pub_key) {
                findings.push(Finding::new(
                    "attestation-invalid",
                    file_name,
                    error.to_string(),
                ));
            }
        } else {
            let status = Command::new("gh")
                .args(["attestation", "verify"])
                .arg(&file_path)
                .args(["--repo", repository])
                .output()
                .context("running `gh attestation verify`")?;
            if !status.status.success() {
                findings.push(Finding::new(
                    "attestation-invalid",
                    file_name,
                    format!(
                        "`gh attestation verify` failed: {}",
                        String::from_utf8_lossy(&status.stderr).trim()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn sha256_in_tar_xz(archive_path: &Path, member: &str) -> Result<String> {
    if !archive_path.is_file() {
        bail!("missing archive {}", archive_path.display());
    }
    let output = Command::new("tar")
        .args(["-xJOf"])
        .arg(archive_path)
        .arg(member)
        .output()
        .with_context(|| format!("extracting {member} from {}", archive_path.display()))?;
    if !output.status.success() {
        bail!(
            "extracting {member} from {} failed: {}",
            archive_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(format!("{:x}", Sha256::digest(&output.stdout)))
}

fn sha256_in_zip(wheel_path: &Path, member: &str) -> Result<String> {
    if !wheel_path.is_file() {
        bail!("missing wheel {}", wheel_path.display());
    }
    // Extract member bytes via python3's standard library zipfile to compute an
    // independent SHA-256 over the actual archived bytes (not trusting RECORD).
    let output = Command::new("python3")
        .args([
            "-c",
            "import hashlib, sys, zipfile\nwith zipfile.ZipFile(sys.argv[1]) as z:\n    data = z.read(sys.argv[2])\nprint(hashlib.sha256(data).hexdigest())",
        ])
        .arg(wheel_path)
        .arg(member)
        .output()
        .with_context(|| format!("reading {member} from {}", wheel_path.display()))?;
    if !output.status.success() {
        bail!(
            "reading {member} from {} failed: {}",
            wheel_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn exe_name(base: &str, target: &str) -> String {
    if target.contains("-windows-") {
        format!("{base}.exe")
    } else {
        base.to_owned()
    }
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}
