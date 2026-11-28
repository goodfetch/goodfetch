//! `plugin.setup` RPC handler and attested installation of the Linux
//! `goodfetch-sandbox` helper and its AppArmor profile (ADRs 0033, 0039 and
//! 0099; issue 884).
//!
//! Verification is TOCTOU-safe: the helper binary (and optional AppArmor
//! profile) is read into memory once, its SHA-256 digest is verified against
//! its Sigstore provenance attestation under the bundled [`AttestationTrustRoot`]
//! and expected Goodfetch release [`SignerIdentity`], and only those exact
//! verified in-memory bytes are written atomically to the destination (mode
//! `0o755` for the helper, never setuid/setgid; mode `0o644` for the AppArmor
//! profile). If verification fails, nothing is written.
#![expect(
    clippy::disallowed_methods,
    reason = "file reads, atomic writes, and apparmor_parser run on the tasks module's blocking policy, never on an async worker"
)]

use std::io;
use std::path::{Path, PathBuf};

use data_encoding::HEXLOWER;
use goodfetch_protocol::checkpoint::{FILE_WRITE_FAILED, FileWriteFailedDetails, WriteSide};
use goodfetch_protocol::plugin::add::{
    AttestationFailureReason, PLUGIN_ATTESTATION_INVALID, PluginAttestationInvalidDetails,
};
use goodfetch_protocol::plugin::index::{ProvenanceAttestation, SignerIdentity};
use goodfetch_protocol::plugin::setup::{PLUGIN_SETUP, PluginSetupParams, PluginSetupResult};
use sha2::{Digest as _, Sha256};

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::plugin_provenance::{self, AttestationTrustRoot, AttestationVerdict};
use crate::run_files::{os_code, reason_of};

/// Default installation path of the root-owned, non-setuid `goodfetch-sandbox`
/// helper on Linux.
pub const DEFAULT_HELPER_DEST: &str = "/usr/local/libexec/goodfetch/goodfetch-sandbox";

/// Default installation path of `goodfetch-sandbox`'s AppArmor profile on
/// Ubuntu 23.10 and later.
pub const DEFAULT_PROFILE_DEST: &str = "/etc/apparmor.d/goodfetch-sandbox";

/// Expected Sigstore signer identity for official Goodfetch release artifacts
/// (ADR 0039).
#[must_use]
pub fn release_signer() -> SignerIdentity {
    SignerIdentity {
        repository: "goodfetch/goodfetch".to_owned(),
        workflow: ".github/workflows/release.yml".to_owned(),
        issuer: "https://token.actions.githubusercontent.com".to_owned(),
    }
}

/// Generates the standard AppArmor profile granting `userns` exclusively to
/// the installed `goodfetch-sandbox` helper at `helper_path` (ADRs 0033 and
/// 0049, issue 884).
#[must_use]
pub fn apparmor_profile_for(helper_path: &Path) -> String {
    format!(
        "# Installed by `plugin setup` on Ubuntu 23.10 and later (ADRs 0033 and 0039):\n\
         # grants `userns` exclusively to the root-owned, non-setuid `goodfetch-sandbox`\n\
         # helper. The restriction stays active for every other program.\n\
         abi <abi/4.0>,\n\
         include <tunables/global>\n\
         \n\
         profile goodfetch-sandbox {} flags=(unconfined) {{\n\
         \x20 userns,\n\
         \x20 include if exists <local/goodfetch-sandbox>\n\
         }}\n",
        helper_path.display()
    )
}

/// Destination paths and verification policy for [`install_verified`].
/// Production calls use [`SetupTarget::system`]; tests inject isolated
/// temporary paths and a test signer (ADR 0059).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupTarget {
    /// Destination path for the installed `goodfetch-sandbox` helper.
    pub helper_dest: PathBuf,
    /// Destination path for the AppArmor profile.
    pub profile_dest: PathBuf,
    /// Expected release signer identity carried by the attestation.
    pub expected_signer: SignerIdentity,
    /// Whether to invoke `apparmor_parser --replace` after writing the profile.
    pub load_profile: bool,
}

impl SetupTarget {
    /// The production system installation target on Linux.
    #[must_use]
    pub fn system() -> Self {
        Self {
            helper_dest: PathBuf::from(DEFAULT_HELPER_DEST),
            profile_dest: PathBuf::from(DEFAULT_PROFILE_DEST),
            expected_signer: release_signer(),
            load_profile: cfg!(target_os = "linux"),
        }
    }
}

/// The `plugin.setup` handler.
pub(crate) fn handlers() -> [Handler; 1] {
    [Handler::new(&PLUGIN_SETUP, setup_operation)]
}

async fn setup_operation(
    params: PluginSetupParams,
    context: CallContext,
) -> Result<PluginSetupResult, CallError> {
    let trust = context.engine().plugin_attestation_trust_root().clone();
    let target = SetupTarget::system();
    context
        .engine()
        .owner()
        .blocking_threads()
        .run(move || install_verified(&params, &trust, &target))
        .await
}

/// Verifies the Sigstore provenance attestation of the helper (and optional
/// AppArmor profile) in memory under `trust` and `target.expected_signer`,
/// then atomically installs the non-setuid helper and profile to `target`.
///
/// # Errors
///
/// - `plugin_attestation_invalid` when the helper or profile attestation is
///   missing, malformed, has an invalid signature, mismatches the digest, or
///   mismatches `target.expected_signer`. Nothing is written to disk.
/// - `file_write_failed` when writing the helper or profile or running
///   `apparmor_parser` fails.
pub fn install_verified(
    params: &PluginSetupParams,
    trust: &AttestationTrustRoot,
    target: &SetupTarget,
) -> Result<PluginSetupResult, CallError> {
    let helper_source = Path::new(&params.helper);
    let helper_bytes = std::fs::read(helper_source).map_err(|error| {
        write_error(
            helper_source,
            &error,
            "reading the `goodfetch-sandbox` helper",
        )
    })?;
    let helper_digest = sha256_digest(&helper_bytes);
    let helper_signer = verify_file_attestation(
        "goodfetch-sandbox",
        &helper_digest,
        Some(Path::new(&params.attestation)),
        &target.expected_signer,
        trust,
    )?;

    let profile_bytes = if let Some(profile_path) = &params.profile {
        let source = Path::new(profile_path);
        let bytes = std::fs::read(source)
            .map_err(|error| write_error(source, &error, "reading the AppArmor profile"))?;
        let digest = sha256_digest(&bytes);
        verify_file_attestation(
            "goodfetch-sandbox-profile",
            &digest,
            params.profile_attestation.as_deref().map(Path::new),
            &target.expected_signer,
            trust,
        )?;
        bytes
    } else {
        apparmor_profile_for(&target.helper_dest).into_bytes()
    };

    write_atomic(&target.helper_dest, &helper_bytes, 0o755)?;
    write_atomic(&target.profile_dest, &profile_bytes, 0o644)?;

    let profile_loaded = if target.load_profile {
        load_apparmor_profile(&target.profile_dest)?
    } else {
        false
    };

    Ok(PluginSetupResult {
        helper_path: target.helper_dest.display().to_string(),
        helper_digest,
        profile_path: target.profile_dest.display().to_string(),
        profile_loaded,
        signer: helper_signer,
    })
}

fn sha256_digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    format!("sha256:{}", HEXLOWER.encode(&hash))
}

fn verify_file_attestation(
    artifact_name: &str,
    digest: &str,
    attestation_path: Option<&Path>,
    expected_signer: &SignerIdentity,
    trust: &AttestationTrustRoot,
) -> Result<SignerIdentity, CallError> {
    let Some(path) = attestation_path else {
        return Err(attestation_refusal(
            artifact_name,
            digest,
            AttestationFailureReason::Missing,
            Some(expected_signer.clone()),
            None,
        ));
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(attestation_refusal(
                artifact_name,
                digest,
                AttestationFailureReason::Missing,
                Some(expected_signer.clone()),
                None,
            ));
        }
        Err(_) => {
            return Err(attestation_refusal(
                artifact_name,
                digest,
                AttestationFailureReason::Malformed,
                Some(expected_signer.clone()),
                None,
            ));
        }
    };
    let Ok(attestation) = serde_json::from_str::<ProvenanceAttestation>(&text) else {
        return Err(attestation_refusal(
            artifact_name,
            digest,
            AttestationFailureReason::Malformed,
            Some(expected_signer.clone()),
            None,
        ));
    };
    match plugin_provenance::verify_attestations(
        &[attestation],
        digest,
        Some(expected_signer),
        trust,
    ) {
        AttestationVerdict::Verified(signer) => Ok(signer),
        AttestationVerdict::Refused {
            reason,
            expected_signer,
            found_signer,
        } => Err(attestation_refusal(
            artifact_name,
            digest,
            reason,
            expected_signer,
            found_signer,
        )),
    }
}

fn attestation_refusal(
    name: &str,
    digest: &str,
    reason: AttestationFailureReason,
    expected_signer: Option<SignerIdentity>,
    found_signer: Option<SignerIdentity>,
) -> CallError {
    CallError::new(
        &PLUGIN_ATTESTATION_INVALID,
        format!(
            "`plugin setup` refused `{name}` ({digest}): its Sigstore attestation failed verification ({reason:?}), so nothing was installed"
        ),
        &PluginAttestationInvalidDetails {
            name: name.to_owned(),
            digest: digest.to_owned(),
            reason,
            expected_signer,
            found_signer,
            fix: "Obtain the official release artifact and its matching Sigstore attestation from the Goodfetch release channel, then re-run `plugin setup`.".to_owned(),
        },
    )
}

fn write_atomic(dest: &Path, bytes: &[u8], mode: u32) -> Result<(), CallError> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| write_error(parent, &error, "creating installation folder"))?;
    let file_name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    if let Err(error) = write_temp_file(&temp, bytes, mode) {
        let _ = std::fs::remove_file(&temp);
        return Err(write_error(dest, &error, "writing verified file"));
    }
    if let Err(error) = std::fs::rename(&temp, dest) {
        let _ = std::fs::remove_file(&temp);
        return Err(write_error(dest, &error, "installing verified file"));
    }
    Ok(())
}

fn write_temp_file(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    use std::io::Write as _;

    // Strip any setuid/setgid bits defensively (ADR 0033).
    let safe_mode = mode & 0o0777;
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(safe_mode))?;
    }
    #[cfg(not(unix))]
    let _ = safe_mode;
    Ok(())
}

fn load_apparmor_profile(profile_dest: &Path) -> Result<bool, CallError> {
    #[cfg(target_os = "linux")]
    {
        let restricted =
            std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
                .is_ok_and(|value| value.trim() == "1");
        let parser = find_apparmor_parser();
        let Some(parser) = parser else {
            if restricted {
                return Err(write_error(
                    profile_dest,
                    &io::Error::new(
                        io::ErrorKind::NotFound,
                        "`apparmor_parser` was not found to load the AppArmor profile",
                    ),
                    "loading the AppArmor profile",
                ));
            }
            return Ok(false);
        };
        let output = std::process::Command::new(&parser)
            .arg("--replace")
            .arg(profile_dest)
            .output()
            .map_err(|error| write_error(profile_dest, &error, "running `apparmor_parser`"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(write_error(
                profile_dest,
                &io::Error::other(format!(
                    "`apparmor_parser --replace` exited with {}: {}",
                    output.status,
                    stderr.trim()
                )),
                "loading the AppArmor profile",
            ));
        }
        Ok(true)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = profile_dest;
        Ok(false)
    }
}

#[cfg(target_os = "linux")]
fn find_apparmor_parser() -> Option<PathBuf> {
    for candidate in ["/usr/sbin/apparmor_parser", "/sbin/apparmor_parser"] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn write_error(path: &Path, error: &io::Error, context: &str) -> CallError {
    CallError::new(
        &FILE_WRITE_FAILED,
        format!(
            "{context} at `{}` failed ({error}): run `plugin setup` with elevated privileges (such as `sudo`).",
            path.display()
        ),
        &FileWriteFailedDetails {
            side: WriteSide::Program,
            path: Some(path.display().to_string()),
            reason: reason_of(error.kind()).to_owned(),
            os_code: error.raw_os_error().and_then(os_code).map(str::to_owned),
        },
    )
}
