//! The container seccomp profile generator and check (ADR 0049; issue 1260):
//! generates `packaging/oci/seccomp-profile.json` from Docker's default seccomp
//! profile at the pinned moby version in `packaging/oci/seccomp-pin.toml` plus
//! exactly the three user-namespace rules Chromium's sandbox needs (`clone`
//! with `CLONE_NEWUSER`, `unshare`, `setns`), and verifies the committed
//! profile matches with no silent widening or drift.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use toml_edit::DocumentMut;

use crate::check::{self, Finding};
use crate::output;

/// Relative path to the moby version pin file.
pub(crate) const PIN_PATH: &str = "packaging/oci/seccomp-pin.toml";
/// Relative path to the verbatim committed copy of Docker's default profile.
pub(crate) const UPSTREAM_PATH: &str = "packaging/oci/moby-default-seccomp.json";
/// Relative path to the generated Goodfetch seccomp profile.
pub(crate) const PROFILE_PATH: &str = "packaging/oci/seccomp-profile.json";

/// Linux `CLONE_NEWUSER` flag bit (`0x1000_0000`).
const CLONE_NEWUSER: u64 = 0x1000_0000;
/// Cap on upstream default profile download size (4 MiB).
const MAX_UPSTREAM_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Fails when `packaging/oci/seccomp-profile.json` differs from the pinned
    /// moby default profile plus Chromium's three user-namespace rules, or
    /// when `packaging/oci/seccomp-pin.toml` drifted without regenerating.
    Check,
    /// Regenerates `packaging/oci/seccomp-profile.json` from the pinned moby
    /// default profile plus Chromium's three user-namespace rules.
    Generate(GenerateArguments),
}

#[derive(Args)]
pub(crate) struct GenerateArguments {
    /// Bump the pinned moby version in `packaging/oci/seccomp-pin.toml` before
    /// regenerating (e.g. `v28.0.0`).
    #[arg(long)]
    version: Option<String>,
    /// Read the upstream moby default seccomp JSON from a local file instead
    /// of downloading it over HTTP.
    #[arg(long)]
    upstream: Option<PathBuf>,
    /// Re-download the upstream moby default seccomp JSON even when the kept
    /// copy already matches the pinned digest.
    #[arg(long)]
    fetch: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinFile {
    version: String,
    url: String,
    sha256: String,
}

pub(crate) fn run(command: Command, root: &Path) -> Result<ExitCode> {
    match command {
        Command::Check => Ok(check::report(&check(root))),
        Command::Generate(arguments) => {
            generate(root, &arguments)?;
            output::line(&format!(
                "wrote {PROFILE_PATH} from {PIN_PATH} and {UPSTREAM_PATH}"
            ));
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Canonical upstream URL for Docker's default seccomp profile at `version`.
pub(crate) fn upstream_url(version: &str) -> String {
    format!("https://raw.githubusercontent.com/moby/moby/{version}/profiles/seccomp/default.json")
}

/// The exact three rule additions Chromium's Linux user-namespace sandbox needs
/// on top of Docker's default seccomp profile (ADR 0049).
fn userns_rules(moby_version: &str) -> Vec<Value> {
    vec![
        json!({
            "names": ["clone"],
            "action": "SCMP_ACT_ALLOW",
            "args": [
                {
                    "index": 0,
                    "value": CLONE_NEWUSER,
                    "valueTwo": CLONE_NEWUSER,
                    "op": "SCMP_CMP_MASKED_EQ"
                }
            ],
            "comment": format!(
                "Goodfetch (ADR 0049, moby {moby_version}): allow clone with CLONE_NEWUSER for Chromium's namespace sandbox."
            ),
            "excludes": {
                "caps": ["CAP_SYS_ADMIN"],
                "arches": ["s390", "s390x"]
            }
        }),
        json!({
            "names": ["unshare"],
            "action": "SCMP_ACT_ALLOW",
            "comment": format!(
                "Goodfetch (ADR 0049, moby {moby_version}): allow unshare for Chromium's namespace sandbox."
            ),
            "excludes": {
                "caps": ["CAP_SYS_ADMIN"]
            }
        }),
        json!({
            "names": ["setns"],
            "action": "SCMP_ACT_ALLOW",
            "comment": format!(
                "Goodfetch (ADR 0049, moby {moby_version}): allow setns for Chromium's namespace sandbox."
            ),
            "excludes": {
                "caps": ["CAP_SYS_ADMIN"]
            }
        }),
    ]
}

/// Deterministically renders `packaging/oci/seccomp-profile.json` from the
/// upstream moby default profile bytes and the pinned moby version.
pub(crate) fn render_profile(upstream_bytes: &[u8], moby_version: &str) -> Result<String> {
    let mut profile: Value = serde_json::from_slice(upstream_bytes)
        .context("parsing upstream moby default seccomp JSON")?;
    let Some(syscalls) = profile.get_mut("syscalls").and_then(Value::as_array_mut) else {
        bail!("upstream moby default seccomp JSON has no `syscalls` array");
    };
    syscalls.extend(userns_rules(moby_version));
    let mut rendered =
        serde_json::to_string_pretty(&profile).context("serializing generated seccomp profile")?;
    rendered.push('\n');
    Ok(rendered)
}

pub(crate) fn generate(root: &Path, arguments: &GenerateArguments) -> Result<()> {
    let pin_path = root.join(PIN_PATH);
    let pin_text = std::fs::read_to_string(&pin_path)
        .with_context(|| format!("reading {}", pin_path.display()))?;
    let mut doc: DocumentMut = pin_text
        .parse()
        .with_context(|| format!("parsing {}", pin_path.display()))?;
    let current: PinFile =
        toml::from_str(&pin_text).with_context(|| format!("parsing {}", pin_path.display()))?;

    let target_version = arguments
        .version
        .as_deref()
        .unwrap_or(&current.version)
        .to_owned();
    validate_version(&target_version)?;
    let target_url = upstream_url(&target_version);

    let upstream_path = root.join(UPSTREAM_PATH);
    let (upstream_bytes, write_upstream) = if let Some(local) = &arguments.upstream {
        let local_path = if local.is_absolute() {
            local.clone()
        } else {
            root.join(local)
        };
        let bytes = std::fs::read(&local_path)
            .with_context(|| format!("reading {}", local_path.display()))?;
        (bytes, true)
    } else if !arguments.fetch
        && target_url == current.url
        && let Ok(cached) = std::fs::read(&upstream_path)
        && sha256_hex(&cached) == current.sha256
    {
        (cached, false)
    } else {
        let bytes = download(&target_url)?;
        (bytes, true)
    };

    let digest = sha256_hex(&upstream_bytes);
    let rendered = render_profile(&upstream_bytes, &target_version)?;

    if write_upstream {
        std::fs::write(&upstream_path, &upstream_bytes)
            .with_context(|| format!("writing {}", upstream_path.display()))?;
    }

    doc["version"] = toml_edit::value(target_version.as_str());
    doc["url"] = toml_edit::value(target_url.as_str());
    doc["sha256"] = toml_edit::value(digest.as_str());
    std::fs::write(&pin_path, doc.to_string())
        .with_context(|| format!("writing {}", pin_path.display()))?;

    let profile_path = root.join(PROFILE_PATH);
    std::fs::write(&profile_path, rendered)
        .with_context(|| format!("writing {}", profile_path.display()))?;
    Ok(())
}

fn load_and_validate_pin(root: &Path, findings: &mut Vec<Finding>) -> Option<PinFile> {
    let pin_path = root.join(PIN_PATH);
    let pin_text = match std::fs::read_to_string(&pin_path) {
        Ok(text) => text,
        Err(error) => {
            findings.push(Finding::new(
                "seccomp-pin-missing",
                PIN_PATH,
                format!("cannot read {PIN_PATH}: {error}"),
            ));
            return None;
        }
    };
    let pin: PinFile = match toml::from_str(&pin_text) {
        Ok(pin) => pin,
        Err(error) => {
            findings.push(Finding::new(
                "seccomp-pin-invalid",
                PIN_PATH,
                format!("invalid TOML in {PIN_PATH}: {error}"),
            ));
            return None;
        }
    };
    if let Err(error) = validate_version(&pin.version) {
        findings.push(Finding::new(
            "seccomp-pin-invalid",
            PIN_PATH,
            error.to_string(),
        ));
    }
    let expected_url = upstream_url(&pin.version);
    if pin.url != expected_url {
        findings.push(Finding::new(
            "seccomp-version-drift",
            PIN_PATH,
            format!(
                "pinned moby version `{}` expects url `{expected_url}`, found `{}`; run `scripts/seccomp-profile.sh` to regenerate",
                pin.version, pin.url
            ),
        ));
    }
    if !is_sha256_hex(&pin.sha256) {
        findings.push(Finding::new(
            "seccomp-pin-invalid",
            PIN_PATH,
            format!(
                "`sha256` must be 64 lowercase hexadecimal digits, got `{}`",
                pin.sha256
            ),
        ));
    }
    Some(pin)
}

pub(crate) fn check(root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    let Some(pin) = load_and_validate_pin(root, &mut findings) else {
        return findings;
    };

    let upstream_path = root.join(UPSTREAM_PATH);
    let upstream_bytes = match std::fs::read(&upstream_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            findings.push(Finding::new(
                "seccomp-upstream-missing",
                UPSTREAM_PATH,
                format!("cannot read {UPSTREAM_PATH}: {error}; run `scripts/seccomp-profile.sh`"),
            ));
            return findings;
        }
    };
    let actual_digest = sha256_hex(&upstream_bytes);
    if actual_digest != pin.sha256 {
        findings.push(Finding::new(
            "seccomp-upstream-digest",
            UPSTREAM_PATH,
            format!(
                "{UPSTREAM_PATH} has SHA-256 {actual_digest}, not {PIN_PATH}'s {}; run `scripts/seccomp-profile.sh`",
                pin.sha256
            ),
        ));
    }

    let expected_profile = match render_profile(&upstream_bytes, &pin.version) {
        Ok(rendered) => rendered,
        Err(error) => {
            findings.push(Finding::new(
                "seccomp-upstream-invalid",
                UPSTREAM_PATH,
                format!("{error}"),
            ));
            return findings;
        }
    };

    let profile_path = root.join(PROFILE_PATH);
    let actual_profile = match std::fs::read_to_string(&profile_path) {
        Ok(text) => text,
        Err(error) => {
            findings.push(Finding::new(
                "seccomp-profile-missing",
                PROFILE_PATH,
                format!("cannot read {PROFILE_PATH}: {error}; run `scripts/seccomp-profile.sh`"),
            ));
            return findings;
        }
    };

    if actual_profile != expected_profile {
        findings.push(describe_profile_drift(
            &actual_profile,
            &expected_profile,
            &pin.version,
        ));
    }

    findings
}

fn describe_profile_drift(actual: &str, expected: &str, moby_version: &str) -> Finding {
    let detail = match (
        serde_json::from_str::<Value>(actual),
        serde_json::from_str::<Value>(expected),
    ) {
        (Ok(actual_json), Ok(expected_json)) => {
            let actual_count = actual_json
                .get("syscalls")
                .and_then(Value::as_array)
                .map(Vec::len);
            let expected_count = expected_json
                .get("syscalls")
                .and_then(Value::as_array)
                .map(Vec::len);
            match (actual_count, expected_count) {
                (Some(got), Some(want)) if got != want => format!(
                    "expected {want} syscall rules (moby {moby_version} default plus 3 user-namespace rules), found {got}; run `scripts/seccomp-profile.sh`"
                ),
                _ => format!(
                    "committed profile differs from moby {moby_version} default plus the 3 user-namespace rules; run `scripts/seccomp-profile.sh`"
                ),
            }
        }
        (Err(error), _) => format!("invalid JSON in {PROFILE_PATH}: {error}"),
        (_, Err(error)) => format!("invalid expected profile JSON: {error}"),
    };
    Finding::new("seccomp-profile-drift", PROFILE_PATH, detail)
}

fn validate_version(version: &str) -> Result<()> {
    let valid = !version.is_empty()
        && version != "."
        && version != ".."
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
    if !valid {
        bail!(
            "pinned moby version `{version}` must be a non-empty Git tag of ASCII alphanumerics, `.`, `-`, or `_`"
        );
    }
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

fn download(url: &str) -> Result<Vec<u8>> {
    ureq::get(url)
        .call()
        .with_context(|| format!("downloading {url}"))?
        .body_mut()
        .with_config()
        .limit(MAX_UPSTREAM_BYTES)
        .read_to_vec()
        .with_context(|| format!("reading {url}"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::{
        GenerateArguments, PIN_PATH, PROFILE_PATH, UPSTREAM_PATH, check, generate, sha256_hex,
        upstream_url,
    };

    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask lives under repository root")
    }

    fn fixture_workspace() -> TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let oci = dir.path().join("packaging/oci");
        std::fs::create_dir_all(&oci).expect("mkdir");
        let upstream = serde_json::to_string_pretty(&json!({
            "defaultAction": "SCMP_ACT_ERRNO",
            "defaultErrnoRet": 1,
            "archMap": [
                {
                    "architecture": "SCMP_ARCH_X86_64",
                    "subArchitectures": ["SCMP_ARCH_X86", "SCMP_ARCH_X32"]
                }
            ],
            "syscalls": [
                {
                    "names": ["read", "write"],
                    "action": "SCMP_ACT_ALLOW"
                }
            ]
        }))
        .expect("json");
        let upstream_bytes = format!("{upstream}\n");
        let digest = sha256_hex(upstream_bytes.as_bytes());
        std::fs::write(dir.path().join(UPSTREAM_PATH), &upstream_bytes).expect("write upstream");
        let pin = format!(
            "version = \"v27.5.1\"\nurl = \"{}\"\nsha256 = \"{digest}\"\n",
            upstream_url("v27.5.1")
        );
        std::fs::write(dir.path().join(PIN_PATH), pin).expect("write pin");
        generate(
            dir.path(),
            &GenerateArguments {
                version: None,
                upstream: None,
                fetch: false,
            },
        )
        .expect("generate initial profile");
        dir
    }

    /// Covers: story:13-55
    #[test]
    fn committed_seccomp_profile_matches_pinned_moby_default_plus_userns_rules() {
        let findings = check(repo_root());
        let summary: Vec<String> = findings
            .iter()
            .map(|f| format!("[{}] {}: {}", f.kind, f.subject, f.message))
            .collect();
        assert!(
            summary.is_empty(),
            "committed seccomp profile drifted: {summary:?}"
        );
    }

    /// Covers: story:13-55
    #[test]
    fn generator_is_deterministic_across_repeated_runs() {
        let dir = fixture_workspace();
        let first = std::fs::read(dir.path().join(PROFILE_PATH)).expect("read first");
        generate(
            dir.path(),
            &GenerateArguments {
                version: None,
                upstream: None,
                fetch: false,
            },
        )
        .expect("regenerate");
        let second = std::fs::read(dir.path().join(PROFILE_PATH)).expect("read second");
        assert_eq!(
            first, second,
            "running generate twice must emit identical bytes"
        );
    }

    /// Covers: story:13-55
    #[test]
    fn check_fails_when_a_rule_is_added_or_removed_by_hand() {
        let dir = fixture_workspace();
        let profile_path = dir.path().join(PROFILE_PATH);
        let original = std::fs::read_to_string(&profile_path).expect("read original");

        // Adding a rule by hand fails the check.
        let mut added: Value = serde_json::from_str(&original).expect("parse");
        added["syscalls"]
            .as_array_mut()
            .expect("syscalls array")
            .push(json!({"names": ["bpf"], "action": "SCMP_ACT_ALLOW"}));
        std::fs::write(
            &profile_path,
            format!("{}\n", serde_json::to_string_pretty(&added).expect("ser")),
        )
        .expect("write added");
        let findings = check(dir.path());
        assert!(
            !findings.is_empty(),
            "adding a syscall rule by hand must fail check"
        );

        // Removing a rule by hand fails the check.
        let mut removed: Value = serde_json::from_str(&original).expect("parse");
        removed["syscalls"]
            .as_array_mut()
            .expect("syscalls array")
            .pop();
        std::fs::write(
            &profile_path,
            format!("{}\n", serde_json::to_string_pretty(&removed).expect("ser")),
        )
        .expect("write removed");
        let findings = check(dir.path());
        assert!(
            !findings.is_empty(),
            "removing a syscall rule by hand must fail check"
        );
    }

    /// Covers: story:13-55
    #[test]
    fn check_fails_when_pinned_moby_version_changes_without_regenerating_and_bump_regenerates() {
        let dir = fixture_workspace();
        let pin_path = dir.path().join(PIN_PATH);
        let bumped_pin = std::fs::read_to_string(&pin_path)
            .expect("read pin")
            .replace("v27.5.1", "v28.0.0");
        std::fs::write(&pin_path, bumped_pin).expect("write bumped pin");

        // Before regenerating, the check must fail on version drift.
        let findings = check(dir.path());
        assert!(
            !findings.is_empty(),
            "bumping pinned moby version without regenerating must fail check"
        );

        // Regenerating with the new upstream default in one step makes check pass.
        let next_upstream = dir.path().join("next-moby-default.json");
        let next_json = serde_json::to_string_pretty(&json!({
            "defaultAction": "SCMP_ACT_ERRNO",
            "defaultErrnoRet": 1,
            "syscalls": [
                {
                    "names": ["read", "write", "openat2"],
                    "action": "SCMP_ACT_ALLOW"
                }
            ]
        }))
        .expect("json");
        std::fs::write(&next_upstream, format!("{next_json}\n")).expect("write next upstream");

        generate(
            dir.path(),
            &GenerateArguments {
                version: Some("v28.0.0".to_owned()),
                upstream: Some(next_upstream),
                fetch: false,
            },
        )
        .expect("regenerate after bump");

        let findings = check(dir.path());
        let summary: Vec<String> = findings
            .iter()
            .map(|f| format!("[{}] {}: {}", f.kind, f.subject, f.message))
            .collect();
        assert!(
            summary.is_empty(),
            "check must pass after regenerating for bumped moby version: {summary:?}"
        );
    }

    #[test]
    fn validate_version_rejects_url_altering_and_traversal_tags() {
        for bad in [
            "", ".", "..", "../main", "v1?x=1", "v1#frag", "v1%2f2", "v 1",
        ] {
            assert!(
                super::validate_version(bad).is_err(),
                "expected `{bad}` to be rejected"
            );
        }
        assert!(super::validate_version("v27.5.1").is_ok());
        assert!(super::validate_version("docker-v28.0.0_rc1").is_ok());
    }

    #[test]
    fn generate_leaves_upstream_untouched_when_input_is_invalid_json() {
        let dir = fixture_workspace();
        let original_upstream = std::fs::read(dir.path().join(UPSTREAM_PATH)).expect("read");
        let original_pin = std::fs::read(dir.path().join(PIN_PATH)).expect("read");
        let original_profile = std::fs::read(dir.path().join(PROFILE_PATH)).expect("read");

        let bad_upstream = dir.path().join("bad-upstream.json");
        std::fs::write(&bad_upstream, "{not valid json").expect("write bad");
        let result = generate(
            dir.path(),
            &GenerateArguments {
                version: Some("v28.0.0".to_owned()),
                upstream: Some(bad_upstream),
                fetch: false,
            },
        );
        assert!(result.is_err(), "invalid upstream JSON must fail");
        assert_eq!(
            std::fs::read(dir.path().join(UPSTREAM_PATH)).expect("read"),
            original_upstream
        );
        assert_eq!(
            std::fs::read(dir.path().join(PIN_PATH)).expect("read"),
            original_pin
        );
        assert_eq!(
            std::fs::read(dir.path().join(PROFILE_PATH)).expect("read"),
            original_profile
        );
    }
}
