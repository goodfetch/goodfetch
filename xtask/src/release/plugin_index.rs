//! `xtask release plugin-index`: freshness/expiry updates for signing
//! `plugin-index/index.json` and pull-request entry validation by digest,
//! provenance attestation, `plugins test`, and ADR 0052 policy checks
//! (ADRs 0033 and 0052; issue 1534).

use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result, bail, ensure};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::output;

/// Expiry window of every signed Plugin index: 14 days from signing (ADR 0033).
const EXPIRY_DAYS: i64 = 14;

/// Phrases indicating a Plugin presents itself as a way past Walls, crawler
/// verification, or paywalls (ADR 0052).
const REFUSED_PHRASES: [&str; 15] = [
    "bypass wall",
    "bypass walls",
    "way past wall",
    "way past walls",
    "past walls",
    "bypass paywall",
    "bypass paywalls",
    "circumvent paywall",
    "evade wall",
    "evade walls",
    "bypass captcha",
    "solve captcha",
    "bypass crawler verification",
    "evade crawler verification",
    "past paywalls",
];

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Updates `version`, `signed_at`, and 14-day `expires_at` on
    /// `plugin-index/index.json` while preserving `revocations` and `entries`.
    Prepare(PrepareArguments),
    /// Validates a Plugin index entry by pinned digest, provenance attestation,
    /// `plugins test`, and ADR 0052 description rules, recording results.
    Validate(ValidateArguments),
}

#[derive(clap::Args)]
pub(crate) struct PrepareArguments {
    /// Path to `plugin-index/index.json`.
    #[arg(long)]
    index: PathBuf,
    /// Optional path to the previously published `index.json`, so `version`
    /// always rises past the published copy and `min-age-days` measures its
    /// last signing time even when `main` itself was not rewritten.
    #[arg(long)]
    published: Option<PathBuf>,
    /// RFC 3339 timestamp used as `signed_at`; defaults to current UTC time.
    #[arg(long)]
    now: Option<String>,
    /// Minimum age in days before re-signing (`0` always re-signs; `10` for the
    /// scheduled job so a quiet index never expires).
    #[arg(long, default_value_t = 0)]
    min_age_days: i64,
}

#[derive(clap::Args)]
pub(crate) struct ValidateArguments {
    /// Path to the Plugin index entry JSON file.
    #[arg(long)]
    entry: PathBuf,
    /// Optional executable override invoked as `<exe> <repository> <digest>`
    /// to verify provenance attestation; defaults to `gh attestation verify`.
    #[arg(long)]
    attestation_verify: Option<PathBuf>,
    /// Optional executable override invoked as
    /// `<exe> --kind <kind> --digest <digest>` to run the confined kind suite;
    /// defaults to `goodfetch-collie plugins test`.
    #[arg(long)]
    plugins_test: Option<PathBuf>,
    /// RFC 3339 timestamp recorded alongside verification results.
    #[arg(long)]
    now: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PluginIndex {
    #[serde(default = "default_format_version")]
    format_version: String,
    version: u64,
    signed_at: String,
    expires_at: String,
    #[serde(default)]
    revocations: Vec<Value>,
    #[serde(default)]
    entries: Vec<Value>,
}

fn default_format_version() -> String {
    "0.1".to_owned()
}

#[derive(Serialize, Deserialize)]
struct PluginEntry {
    name: String,
    kind: String,
    repository: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signer_repository: Option<String>,
    digest: String,
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    verification: Option<Value>,
}

pub(super) fn run(root: &Path, command: &Command) -> Result<ExitCode> {
    match command {
        Command::Prepare(args) => prepare(root, args),
        Command::Validate(args) => validate(root, args),
    }
}

fn parse_now(raw: Option<&str>) -> Result<Timestamp> {
    match raw {
        Some(text) => text
            .parse::<Timestamp>()
            .with_context(|| format!("parsing --now timestamp {text:?}")),
        None => Ok(super::now()),
    }
}

fn prepare(root: &Path, args: &PrepareArguments) -> Result<ExitCode> {
    ensure!(
        args.min_age_days >= 0,
        "--min-age-days must be non-negative (got {})",
        args.min_age_days
    );
    let path = root.join(&args.index);
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut index: PluginIndex =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

    let published = match args.published.as_deref() {
        Some(rel) => {
            let published_path = root.join(rel);
            if published_path.is_file() {
                let text = std::fs::read_to_string(&published_path)
                    .with_context(|| format!("reading {}", published_path.display()))?;
                let parsed: PluginIndex = serde_json::from_str(&text)
                    .with_context(|| format!("parsing {}", published_path.display()))?;
                Some(parsed)
            } else {
                None
            }
        }
        None => None,
    };

    let now = parse_now(args.now.as_deref())?;
    if args.min_age_days > 0 {
        let last_signed_raw = published
            .as_ref()
            .map_or(index.signed_at.as_str(), |p| p.signed_at.as_str());
        let last_signed: Timestamp = last_signed_raw
            .parse()
            .with_context(|| format!("parsing signed_at {last_signed_raw:?}"))?;
        let age = now.duration_since(last_signed);
        let threshold = SignedDuration::from_hours(args.min_age_days * 24);
        if age < threshold {
            output::line("unchanged");
            return Ok(ExitCode::SUCCESS);
        }
    }

    let base_version = published
        .as_ref()
        .map_or(index.version, |p| index.version.max(p.version));
    index.version = base_version
        .checked_add(1)
        .context("plugin index version overflow")?;
    let expires = now
        .checked_add(SignedDuration::from_hours(EXPIRY_DAYS * 24))
        .context("computing 14-day expiry")?;
    index.signed_at = now.to_string();
    index.expires_at = expires.to_string();

    std::fs::write(&path, serde_json::to_string_pretty(&index)? + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    output::line(&format!(
        "updated version={} expires_at={}",
        index.version, index.expires_at
    ));
    Ok(ExitCode::SUCCESS)
}

fn validate(root: &Path, args: &ValidateArguments) -> Result<ExitCode> {
    let path = root.join(&args.entry);
    let raw =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut entry: PluginEntry =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

    // 1. Validate by pinned SHA-256 digest only (never mutable tags).
    let hex = entry
        .digest
        .strip_prefix("sha256:")
        .with_context(|| format!("entry {} must pin a sha256:<hex> digest", entry.name))?;
    ensure!(
        hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        "entry {} has invalid digest {:?}",
        entry.name,
        entry.digest
    );

    // 2. Refuse descriptions presenting the Plugin as a way past Walls,
    //    crawler verification, or paywalls (ADR 0052).
    let lowered = entry.description.to_ascii_lowercase();
    for phrase in REFUSED_PHRASES {
        if lowered.contains(phrase) {
            bail!(
                "refusing plugin {}: description presents itself as a way past Walls, crawler verification, or paywalls (matched {phrase:?}, ADR 0052)",
                entry.name
            );
        }
    }

    // 3. Verify provenance attestation by digest.
    let att_out = if let Some(verifier) = args.attestation_verify.as_deref() {
        let verifier = root.join(verifier);
        Process::new(&verifier)
            .args([&entry.repository, &entry.digest])
            .output()
            .with_context(|| format!("running provenance verifier {}", verifier.display()))?
    } else {
        let reference = format!("oci://{}@{}", entry.repository, entry.digest);
        let mut command = Process::new("gh");
        command.args(["attestation", "verify", &reference]);
        if let Some(repo) = entry.signer_repository.as_deref() {
            command.args(["--repo", repo]);
        } else {
            bail!(
                "entry {} must declare `signer_repository` for `gh attestation verify`",
                entry.name
            );
        }
        command
            .output()
            .context("running `gh attestation verify`")?
    };
    ensure!(
        att_out.status.success(),
        "provenance verification failed for {}@{}:\n{}",
        entry.repository,
        entry.digest,
        String::from_utf8_lossy(&att_out.stderr)
    );

    // 4. Run the entry's kind suite through `plugins test` inside confinement.
    let test_out = if let Some(tester) = args.plugins_test.as_deref() {
        let tester = root.join(tester);
        Process::new(&tester)
            .args(["--kind", &entry.kind, "--digest", &entry.digest])
            .output()
            .with_context(|| format!("running plugins test {}", tester.display()))?
    } else {
        let binary = std::env::var_os("GOODFETCH_COLLIE_BIN")
            .map_or_else(|| PathBuf::from("goodfetch-collie"), PathBuf::from);
        Process::new(&binary)
            .args([
                "plugins",
                "test",
                "--kind",
                &entry.kind,
                "--digest",
                &entry.digest,
            ])
            .output()
            .with_context(|| {
                format!(
                    "running `{} plugins test` (install the Product executable or pass --plugins-test)",
                    binary.display()
                )
            })?
    };
    ensure!(
        test_out.status.success(),
        "plugins test suite failed for {} ({}):\n{}",
        entry.name,
        entry.kind,
        String::from_utf8_lossy(&test_out.stderr)
    );

    // 5. Record verification results back into the entry.
    let now = parse_now(args.now.as_deref())?;
    entry.verification = Some(json!({
        "status": "passed",
        "verified_at": now.to_string(),
        "digest": entry.digest,
        "suite": entry.kind,
    }));
    std::fs::write(&path, serde_json::to_string_pretty(&entry)? + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    output::line(&format!("validated {}@{}", entry.name, entry.digest));
    Ok(ExitCode::SUCCESS)
}
