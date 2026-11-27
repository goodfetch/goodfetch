//! Local rehearsal signing and in-toto DSSE attestation for release artifacts
//! (`xtask release sign-rehearsal` and `xtask release verify-rehearsal`, ADRs
//! 0016 and 0032; issue 1662).
//!
//! Uses `openssl` with an ephemeral Ed25519 keypair so rehearsal tests verify
//! real cryptographic signatures (`<file>.sig`) and signed in-toto v1 SLSA
//! provenance DSSE envelopes (`<file>.sigstore.jsonl`) offline without network
//! access or production signing secrets.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result, bail, ensure};
use data_encoding::BASE64;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::assets;
use crate::output;

/// The DSSE payload media type for in-toto v1 statements.
const IN_TOTO_PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";

/// The in-toto v1 statement type URI.
const IN_TOTO_STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";

/// The SLSA provenance v1 predicate type URI.
const SLSA_PROVENANCE_V1: &str = "https://slsa.dev/provenance/v1";

#[derive(clap::Args)]
pub(crate) struct SignRehearsalArguments {
    /// Release file to sign and attest (for example `dist/assets/goodfetch-catalog.json`).
    #[arg(long)]
    file: PathBuf,
    /// Directory where the ephemeral Ed25519 keypair (`rehearsal.key`,
    /// `rehearsal.pub`) is created or reused.
    #[arg(long)]
    key_dir: PathBuf,
    /// Optional git commit recorded in the SLSA provenance predicate.
    #[arg(long, default_value = "rehearsal")]
    commit: String,
}

#[derive(clap::Args)]
pub(crate) struct VerifyRehearsalArguments {
    /// Release file to verify before reading.
    #[arg(long)]
    file: PathBuf,
    /// Detached Ed25519 signature (`<file>.sig`).
    #[arg(long)]
    signature: PathBuf,
    /// Signed in-toto DSSE attestation (`<file>.sigstore.jsonl`).
    #[arg(long)]
    attestation: PathBuf,
    /// Public key (`rehearsal.pub`) matching the rehearsal signer.
    #[arg(long)]
    public_key: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct DsseEnvelope {
    #[serde(rename = "payloadType")]
    payload_type: String,
    payload: String,
    signatures: Vec<DsseSignature>,
}

#[derive(Serialize, Deserialize)]
struct DsseSignature {
    sig: String,
}

#[derive(Deserialize)]
struct InTotoStatement {
    #[serde(rename = "_type")]
    statement_type: String,
    subject: Vec<InTotoSubject>,
    #[serde(rename = "predicateType")]
    predicate_type: String,
}

#[derive(Deserialize)]
struct InTotoSubject {
    name: String,
    digest: InTotoDigest,
}

#[derive(Deserialize)]
struct InTotoDigest {
    sha256: String,
}

/// Signs `arguments.file` with an ephemeral Ed25519 key and writes `<file>.sig`
/// and `<file>.sigstore.jsonl` beside it.
pub(super) fn sign_rehearsal(root: &Path, arguments: &SignRehearsalArguments) -> Result<ExitCode> {
    let file = resolve(root, &arguments.file);
    let key_dir = resolve(root, &arguments.key_dir);
    ensure!(
        file.is_file(),
        "release file does not exist: {}",
        file.display()
    );
    std::fs::create_dir_all(&key_dir).with_context(|| format!("creating {}", key_dir.display()))?;

    let private_key = key_dir.join("rehearsal.key");
    let public_key = key_dir.join("rehearsal.pub");
    if !private_key.is_file() || !public_key.is_file() {
        generate_ed25519_keypair(&private_key, &public_key)?;
    }

    let sig_path = append_extension(&file, "sig");
    openssl_sign_file(&private_key, &file, &sig_path)?;

    let file_name = file
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("non-UTF-8 file name {}", file.display()))?;
    let sha256 = assets::sha256_file(&file)?;
    let statement = json!({
        "_type": IN_TOTO_STATEMENT_TYPE,
        "subject": [{
            "name": file_name,
            "digest": { "sha256": sha256 },
        }],
        "predicateType": SLSA_PROVENANCE_V1,
        "predicate": {
            "buildDefinition": {
                "buildType": "https://goodfetch.dev/release-rehearsal/v1",
                "externalParameters": {
                    "commit": arguments.commit,
                }
            }
        }
    });
    let payload_bytes = serde_json::to_vec(&statement)?;
    let pae = dsse_pae(IN_TOTO_PAYLOAD_TYPE, &payload_bytes);
    let dsse_sig = openssl_sign_bytes(&private_key, &pae)?;
    let envelope = DsseEnvelope {
        payload_type: IN_TOTO_PAYLOAD_TYPE.to_owned(),
        payload: BASE64.encode(&payload_bytes),
        signatures: vec![DsseSignature {
            sig: BASE64.encode(&dsse_sig),
        }],
    };
    let attestation_path = append_extension(&file, "sigstore.jsonl");
    let mut line = serde_json::to_string(&envelope)?;
    line.push('\n');
    std::fs::write(&attestation_path, line)
        .with_context(|| format!("writing {}", attestation_path.display()))?;

    output::line(&sig_path.display().to_string());
    output::line(&attestation_path.display().to_string());
    output::line(&public_key.display().to_string());
    Ok(ExitCode::SUCCESS)
}

/// Verifies both the detached signature and the signed in-toto DSSE
/// attestation of `arguments.file`.
pub(super) fn verify_rehearsal(
    root: &Path,
    arguments: &VerifyRehearsalArguments,
) -> Result<ExitCode> {
    let file = resolve(root, &arguments.file);
    let signature = resolve(root, &arguments.signature);
    let attestation = resolve(root, &arguments.attestation);
    let public_key = resolve(root, &arguments.public_key);

    ensure!(file.is_file(), "missing artifact {}", file.display());
    ensure!(
        signature.is_file(),
        "missing signature {}",
        signature.display()
    );
    ensure!(
        attestation.is_file(),
        "missing attestation {}",
        attestation.display()
    );
    ensure!(
        public_key.is_file(),
        "missing public key {}",
        public_key.display()
    );

    openssl_verify_file(&public_key, &file, &signature)
        .with_context(|| format!("verifying detached signature {}", signature.display()))?;

    let attestation_text = std::fs::read_to_string(&attestation)
        .with_context(|| format!("reading {}", attestation.display()))?;
    let line = attestation_text
        .lines()
        .find(|line| !line.trim().is_empty())
        .with_context(|| format!("empty attestation {}", attestation.display()))?;
    let envelope: DsseEnvelope = serde_json::from_str(line)
        .with_context(|| format!("parsing DSSE envelope {}", attestation.display()))?;
    ensure!(
        envelope.payload_type == IN_TOTO_PAYLOAD_TYPE,
        "unexpected DSSE payloadType `{}`",
        envelope.payload_type
    );
    ensure!(
        !envelope.signatures.is_empty(),
        "DSSE envelope carries no signatures"
    );
    let payload_bytes = BASE64
        .decode(envelope.payload.as_bytes())
        .context("decoding base64 DSSE payload")?;
    let pae = dsse_pae(&envelope.payload_type, &payload_bytes);

    let mut verified_any = false;
    for entry in &envelope.signatures {
        let sig_bytes = BASE64
            .decode(entry.sig.as_bytes())
            .context("decoding base64 DSSE signature")?;
        if openssl_verify_bytes(&public_key, &pae, &sig_bytes).is_ok() {
            verified_any = true;
            break;
        }
    }
    ensure!(
        verified_any,
        "DSSE attestation signature failed verification against {}",
        public_key.display()
    );

    let statement: InTotoStatement =
        serde_json::from_slice(&payload_bytes).context("parsing in-toto Statement")?;
    ensure!(
        statement.statement_type == IN_TOTO_STATEMENT_TYPE,
        "unexpected in-toto statement type `{}`",
        statement.statement_type
    );
    ensure!(
        statement.predicate_type == SLSA_PROVENANCE_V1,
        "unexpected predicateType `{}`",
        statement.predicate_type
    );
    let expected_name = file
        .file_name()
        .and_then(|name| name.to_str())
        .with_context(|| format!("non-UTF-8 file name {}", file.display()))?;
    let actual_sha256 = assets::sha256_file(&file)?;
    let matches_subject = statement
        .subject
        .iter()
        .any(|subject| subject.name == expected_name && subject.digest.sha256 == actual_sha256);
    ensure!(
        matches_subject,
        "attestation subject does not match `{expected_name}` sha256:{actual_sha256}"
    );

    output::line(&format!("verified {expected_name} sha256:{actual_sha256}"));
    Ok(ExitCode::SUCCESS)
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn append_extension(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".");
    name.push(suffix);
    PathBuf::from(name)
}

/// DSSE Pre-Authentication Encoding (`DSSEv1 <type_len> <type> <body_len> <body>`).
fn dsse_pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let header = format!(
        "DSSEv1 {} {} {} ",
        payload_type.len(),
        payload_type,
        payload.len()
    );
    let mut pae = Vec::with_capacity(header.len() + payload.len());
    pae.extend_from_slice(header.as_bytes());
    pae.extend_from_slice(payload);
    pae
}

fn generate_ed25519_keypair(private_key: &Path, public_key: &Path) -> Result<()> {
    run_openssl(&[
        "genpkey",
        "-algorithm",
        "ED25519",
        "-out",
        &private_key.display().to_string(),
    ])
    .context("generating rehearsal Ed25519 private key")?;
    run_openssl(&[
        "pkey",
        "-in",
        &private_key.display().to_string(),
        "-pubout",
        "-out",
        &public_key.display().to_string(),
    ])
    .context("extracting rehearsal Ed25519 public key")?;
    Ok(())
}

fn openssl_sign_file(private_key: &Path, input: &Path, output: &Path) -> Result<()> {
    run_openssl(&[
        "pkeyutl",
        "-sign",
        "-rawin",
        "-inkey",
        &private_key.display().to_string(),
        "-in",
        &input.display().to_string(),
        "-out",
        &output.display().to_string(),
    ])
}

fn openssl_verify_file(public_key: &Path, input: &Path, signature: &Path) -> Result<()> {
    run_openssl(&[
        "pkeyutl",
        "-verify",
        "-pubin",
        "-rawin",
        "-inkey",
        &public_key.display().to_string(),
        "-in",
        &input.display().to_string(),
        "-sigfile",
        &signature.display().to_string(),
    ])
}

fn openssl_sign_bytes(private_key: &Path, bytes: &[u8]) -> Result<Vec<u8>> {
    let scratch = tempfile::tempdir().context("creating temporary signing directory")?;
    let input = scratch.path().join("payload.bin");
    let output = scratch.path().join("payload.sig");
    std::fs::write(&input, bytes)?;
    openssl_sign_file(private_key, &input, &output)?;
    std::fs::read(&output).context("reading signature bytes")
}

fn openssl_verify_bytes(public_key: &Path, bytes: &[u8], signature: &[u8]) -> Result<()> {
    let scratch = tempfile::tempdir().context("creating temporary verification directory")?;
    let input = scratch.path().join("payload.bin");
    let sig_file = scratch.path().join("payload.sig");
    std::fs::write(&input, bytes)?;
    std::fs::write(&sig_file, signature)?;
    openssl_verify_file(public_key, &input, &sig_file)
}

fn run_openssl(args: &[&str]) -> Result<()> {
    let output = Command::new("openssl")
        .args(args)
        .output()
        .context("running `openssl`")?;
    if !output.status.success() {
        bail!(
            "`openssl {}` failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
