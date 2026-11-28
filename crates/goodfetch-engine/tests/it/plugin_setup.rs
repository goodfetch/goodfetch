//! Integration tests for `plugin setup` attestation verification and
//! installation (ADRs 0033, 0039 and 0099; issue 884).
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests create temporary files and inspect permissions outside any async worker"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap"
)]

use std::fs;
use std::path::Path;

use data_encoding::HEXLOWER;
use goodfetch_engine::plugin_provenance::{AttestationSigner, AttestationTrustRoot};
use goodfetch_engine::plugin_setup::{SetupTarget, install_verified, release_signer};
use goodfetch_protocol::plugin::index::{ProvenanceStatement, SignerIdentity};
use goodfetch_protocol::plugin::setup::{PluginSetupComponent, PluginSetupParams};
use sha2::{Digest as _, Sha256};

const TEST_KEY_ID: &str = "sigstore-sandbox-test-key-1";

fn sha256_of(bytes: &[u8]) -> String {
    format!("sha256:{}", HEXLOWER.encode(&Sha256::digest(bytes)))
}

fn sign_bytes(signer: &AttestationSigner, identity: SignerIdentity, bytes: &[u8], dest: &Path) {
    let att = signer
        .sign(ProvenanceStatement {
            subject_digest: sha256_of(bytes),
            signer: identity,
        })
        .unwrap();
    fs::write(dest, serde_json::to_string_pretty(&att).unwrap()).unwrap();
}

/// `plugin setup` verifies the helper's attestation under a test trust root
/// and installs a non-setuid copy of the helper plus its AppArmor profile
/// granting `userns` exclusively to the installed helper path.
///
/// Covers: story:7-23, operation:plugin.setup
#[test]
fn plugin_setup_installs_helper_and_profile_after_attestation_verifies() {
    let root = tempfile::tempdir().unwrap();
    let src_dir = root.path().join("release");
    let dst_dir = root.path().join("installed");
    fs::create_dir_all(&src_dir).unwrap();

    let helper_bytes = b"#!/bin/sh\n# verified goodfetch-sandbox test binary\n";
    let helper_src = src_dir.join("goodfetch-sandbox");
    let att_src = src_dir.join("goodfetch-sandbox.att.json");
    fs::write(&helper_src, helper_bytes).unwrap();

    let (signer, trust) = AttestationSigner::for_root(TEST_KEY_ID, [0x42; 32]);
    let expected_signer = release_signer();
    sign_bytes(&signer, expected_signer.clone(), helper_bytes, &att_src);

    let target = SetupTarget {
        helper_dest: dst_dir.join("libexec/goodfetch-sandbox"),
        profile_dest: dst_dir.join("apparmor.d/goodfetch-sandbox"),
        expected_signer: expected_signer.clone(),
        load_profile: false,
        register_service: false,
    };
    let params = PluginSetupParams {
        component: Some(PluginSetupComponent::SandboxHelper),
        helper: helper_src.display().to_string(),
        attestation: att_src.display().to_string(),
        profile: None,
        profile_attestation: None,
    };

    let installed = install_verified(&params, &trust, &target).unwrap();

    assert_eq!(installed.helper_digest, sha256_of(helper_bytes));
    assert_eq!(installed.signer, expected_signer);
    assert_eq!(fs::read(&target.helper_dest).unwrap(), helper_bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&target.helper_dest)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(mode, 0o755, "helper must be 0755 and never setuid/setgid");
        let pmode = fs::metadata(&target.profile_dest)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(pmode, 0o644);
    }
    let profile = fs::read_to_string(&target.profile_dest).unwrap();
    assert!(
        profile.contains(&target.helper_dest.display().to_string()),
        "profile must name installed helper path:\n{profile}"
    );
    assert!(
        profile.contains("userns,"),
        "profile must grant userns:\n{profile}"
    );
}

fn assert_refused(
    params: &PluginSetupParams,
    trust: &AttestationTrustRoot,
    target: &SetupTarget,
    expected_reason: &str,
) {
    let err = install_verified(params, trust, target).unwrap_err();
    assert_eq!(err.code(), "plugin_attestation_invalid");
    assert_eq!(err.details().unwrap()["reason"], expected_reason);
    assert!(!target.helper_dest.exists());
    assert!(!target.profile_dest.exists());
}

/// `plugin setup` refuses any helper whose attestation does not verify —
/// tampered helper bytes, wrong signing key, wrong signer identity, missing or
/// malformed attestation, or custom profile with missing/bad attestation —
/// with `plugin_attestation_invalid` and writes nothing to disk.
///
/// Covers: story:7-23, operation:plugin.setup
#[test]
fn plugin_setup_refuses_unverified_helper_or_profile_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let src_dir = root.path().join("release");
    let dst_dir = root.path().join("installed");
    fs::create_dir_all(&src_dir).unwrap();

    let helper_bytes = b"genuine-goodfetch-sandbox-bytes";
    let helper_src = src_dir.join("goodfetch-sandbox");
    fs::write(&helper_src, helper_bytes).unwrap();

    let (good_signer, trust) = AttestationSigner::for_root(TEST_KEY_ID, [0x11; 32]);
    let (rogue_signer, _) = AttestationSigner::for_root(TEST_KEY_ID, [0x99; 32]);
    let expected_signer = release_signer();
    let target = SetupTarget {
        helper_dest: dst_dir.join("libexec/goodfetch-sandbox"),
        profile_dest: dst_dir.join("apparmor.d/goodfetch-sandbox"),
        expected_signer: expected_signer.clone(),
        load_profile: false,
        register_service: false,
    };
    let params_for = |att: &Path, profile: Option<&Path>| PluginSetupParams {
        component: Some(PluginSetupComponent::SandboxHelper),
        helper: helper_src.display().to_string(),
        attestation: att.display().to_string(),
        profile: profile.map(|p| p.display().to_string()),
        profile_attestation: None,
    };

    // 1. Tampered helper / digest mismatch.
    let mismatch_att = src_dir.join("mismatch.att.json");
    sign_bytes(
        &good_signer,
        expected_signer.clone(),
        b"different-bytes",
        &mismatch_att,
    );
    assert_refused(
        &params_for(&mismatch_att, None),
        &trust,
        &target,
        "digest_mismatch",
    );

    // 2. Signed by untrusted key.
    let bad_sig_att = src_dir.join("bad-sig.att.json");
    sign_bytes(
        &rogue_signer,
        expected_signer.clone(),
        helper_bytes,
        &bad_sig_att,
    );
    assert_refused(
        &params_for(&bad_sig_att, None),
        &trust,
        &target,
        "bad_signature",
    );

    // 3. Signed under wrong signer identity.
    let wrong_signer_att = src_dir.join("wrong-signer.att.json");
    sign_bytes(
        &good_signer,
        SignerIdentity {
            repository: "attacker/fork".to_owned(),
            workflow: ".github/workflows/release.yml".to_owned(),
            issuer: expected_signer.issuer.clone(),
        },
        helper_bytes,
        &wrong_signer_att,
    );
    assert_refused(
        &params_for(&wrong_signer_att, None),
        &trust,
        &target,
        "signer_mismatch",
    );

    // 4. Empty bundled trust root refuses everything.
    let valid_att = src_dir.join("valid.att.json");
    sign_bytes(&good_signer, expected_signer, helper_bytes, &valid_att);
    assert_refused(
        &params_for(&valid_att, None),
        &AttestationTrustRoot::bundled(),
        &target,
        "bad_signature",
    );

    // 5. Custom profile without attestation refuses and installs neither file.
    let custom_profile = src_dir.join("custom.profile");
    fs::write(&custom_profile, "abi <abi/4.0>,\n").unwrap();
    assert_refused(
        &params_for(&valid_att, Some(&custom_profile)),
        &trust,
        &target,
        "missing",
    );
}

/// `plugin setup` installs the Windows `goodfetch-filter-broker` only after
/// verifying its Sigstore provenance attestation under a test trust root
/// (ADR 0059) and refuses any unattested, tampered, or rogue-signed binary
/// with `plugin_attestation_invalid`, writing nothing to disk (ADRs 0033,
/// 0039 and 0099; issue 879).
///
/// Covers: story:7-24, operation:plugin.setup
#[test]
fn plugin_setup_installs_windows_filter_broker_only_after_attestation_verifies() {
    let root = tempfile::tempdir().unwrap();
    let src_dir = root.path().join("release");
    let dst_dir = root.path().join("installed");
    fs::create_dir_all(&src_dir).unwrap();

    let broker_bytes = b"MZ-verified-goodfetch-filter-broker-test-binary";
    let broker_src = src_dir.join("goodfetch-filter-broker.exe");
    let good_att = src_dir.join("goodfetch-filter-broker.att.json");
    let bad_att = src_dir.join("rogue.att.json");
    let wrong_digest_att = src_dir.join("wrong-digest.att.json");
    fs::write(&broker_src, broker_bytes).unwrap();

    let (signer, trust) = AttestationSigner::for_root("broker-setup-key", [0x24; 32]);
    let (rogue, _) = AttestationSigner::for_root("broker-setup-key", [0x88; 32]);
    let expected_signer = release_signer();
    sign_bytes(&signer, expected_signer.clone(), broker_bytes, &good_att);
    sign_bytes(&rogue, expected_signer.clone(), broker_bytes, &bad_att);
    sign_bytes(
        &signer,
        expected_signer.clone(),
        b"other-bytes",
        &wrong_digest_att,
    );

    let target = SetupTarget {
        helper_dest: dst_dir.join("Goodfetch/goodfetch-filter-broker.exe"),
        profile_dest: dst_dir.join("unused"),
        expected_signer: expected_signer.clone(),
        load_profile: false,
        register_service: false,
    };

    // Unverified (rogue key) and tampered (digest mismatch) attestations are refused and write nothing.
    for (att_path, expected_reason) in [
        (&bad_att, "bad_signature"),
        (&wrong_digest_att, "digest_mismatch"),
    ] {
        let params = PluginSetupParams {
            component: Some(PluginSetupComponent::FilterBroker),
            helper: broker_src.display().to_string(),
            attestation: att_path.display().to_string(),
            profile: None,
            profile_attestation: None,
        };
        let err = install_verified(&params, &trust, &target).unwrap_err();
        assert_eq!(err.code(), "plugin_attestation_invalid");
        assert_eq!(err.details().unwrap()["reason"], expected_reason);
        assert!(!target.helper_dest.exists());
    }

    // Valid attestation installs the verified broker binary.
    let valid_params = PluginSetupParams {
        component: Some(PluginSetupComponent::FilterBroker),
        helper: broker_src.display().to_string(),
        attestation: good_att.display().to_string(),
        profile: None,
        profile_attestation: None,
    };
    let installed = install_verified(&valid_params, &trust, &target).unwrap();
    assert_eq!(installed.component, PluginSetupComponent::FilterBroker);
    assert_eq!(installed.helper_digest, sha256_of(broker_bytes));
    assert_eq!(
        installed.service_name.as_deref(),
        Some("GoodfetchFilterBroker")
    );
    assert_eq!(installed.signer, expected_signer);
    assert_eq!(fs::read(&target.helper_dest).unwrap(), broker_bytes);
}
