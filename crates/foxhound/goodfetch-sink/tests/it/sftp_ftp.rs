//! Conformance tests for SFTP and FTP Files-mode Sinks (ADRs 0008, 0026,
//! 0067 and 0127; issue 1159) against pinned containers (`ci/sftp/image.toml`
//! and `ci/ftp/image.toml`): atomic temp-file + rename so readers never observe
//! half-written parts, `GuaranteeClass::Keyed`, resume probe, `Fault::AnswerLost`
//! retry, Unicode byte-for-byte round-trip, and non-vacuous Secret canary scan.
//!
//! SFTP and FTP are self-hosted protocols with no single free-tier cloud
//! service; their conformance suites run against containers and their Live
//! checks wait (ADR 0064).
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests launch pinned containers and run on Tokio's runtime"
)]

use std::process::Command;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use goodfetch_sink::{
    Driver, FileDestination as _, FilesConfig, FilesSink, FtpDestination, GuaranteeClass,
    SftpDestination, Status,
};
use goodfetch_test_support::{Canaries, Output, SecretKind};
use secrecy::SecretString;
use serde_json::json;
use suppaftp::{FtpStream, Mode};

use crate::containers::{
    ContainerGuard, allocate_ephemeral_port, decode_zstd_jsonl, unicode_value, url_with_credentials,
};
use crate::faults::{Fault, Faulty};
use crate::run_files::{copy, run_file};

/// SFTP Sink conformance against the pinned `atmoz/sftp:alpine` container:
/// atomic temp-file + rename (no `.tmp-*` leftovers), `GuaranteeClass::Keyed`,
/// `Fault::AnswerLost` retry, resume probe, byte-for-byte Unicode round-trip,
/// and non-vacuous Secret canary scan.
///
/// Covers: sink:sftp
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end SFTP container conformance exercises delivery, fault retry, resume probe, Unicode, and canary checks"
)]
async fn sftp_sink_conformance_against_sftp_container() {
    const IMAGE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/sftp/image.toml"
    ));
    let mut canaries = Canaries::new(1159);
    let sftp_pass = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();
    let user_spec = format!("goodfetch:{sftp_pass}:::upload");
    let guard = ContainerGuard::spawn(IMAGE, &["-p", "127.0.0.1::22"], &[&user_spec]);
    let port = guard.published_port("22/tcp");
    let url = url_with_credentials(
        &format!("sftp://127.0.0.1:{port}/upload?accept_unknown_host=true"),
        "goodfetch",
        &sftp_pass,
    );
    wait_for_sftp(&url).await;

    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join("sftp_pass", &SecretString::from(sftp_pass), Reach::Value);

    let unicode = unicode_value();
    let expected_unicode = unicode.clone();
    let url_clone = url.clone();
    let (class, written_keys) = tokio::task::spawn_blocking(move || {
        let dest = SftpDestination::from_url("sftp_sink", &url_clone, Some("exports")).unwrap();
        let config = FilesConfig {
            format: ExportFormat::Jsonl,
            compression: ExportCompression::Zstd,
            max_records: 2,
            ..FilesConfig::default()
        };
        let sink = FilesSink::new("sftp_sink", "exports", config, Clock::system(), dest);
        let faulty = Faulty::new(sink, Fault::AnswerLost(1));
        let records = run_file(&[(
            "items",
            vec![
                json!({ "id": "a", "n": 1 }),
                unicode,
                json!({ "id": "b", "n": 2 }),
            ],
        )]);
        let mut driver = Driver::new("sftp_sink", faulty, copy(1), vec!["items".to_owned()])
            .with_clock(Clock::system())
            .with_masks(masks);
        // First pass hits simulated AnswerLost after uploading part 1; second pass resumes cleanly.
        assert!(matches!(
            driver.deliver(&records).unwrap(),
            Status::Behind { .. }
        ));
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.finish(), Status::Complete);
        let class = driver.class();
        let keys = driver.into_sink().into_inner().written_keys().to_vec();
        (class, keys)
    })
    .await
    .unwrap();

    assert_eq!(class, Some(GuaranteeClass::Keyed));
    assert_eq!(
        written_keys.len(),
        2,
        "3 records at max_records=2 -> 2 parts"
    );

    let probe_url = url.clone();
    let probed = tokio::task::spawn_blocking(move || {
        let mut dest = SftpDestination::from_url("sftp_sink", &probe_url, Some("exports")).unwrap();
        dest.probe(&copy(1)).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(probed.class, GuaranteeClass::Keyed);
    assert_eq!(probed.delivered.get("items").copied(), Some(3));
    assert_eq!(probed.next_parts.get("items").copied(), Some(3));

    let mut all_values = Vec::new();
    let mut decompressed_text = String::new();
    for key in &written_keys {
        let remote = format!("/home/goodfetch/upload/{key}");
        let cat = Command::new(guard.engine)
            .args(["exec", &guard.id, "cat", &remote])
            .output()
            .unwrap();
        assert!(cat.status.success(), "missing uploaded SFTP part {remote}");
        let vals = decode_zstd_jsonl(&cat.stdout);
        for v in &vals {
            decompressed_text.push_str(&v.to_string());
            decompressed_text.push('\n');
        }
        all_values.extend(vals);
    }
    assert_eq!(all_values.len(), 3);
    assert_eq!(all_values[1], expected_unicode);

    let find_tmp = Command::new(guard.engine)
        .args([
            "exec",
            &guard.id,
            "find",
            "/home/goodfetch/upload",
            "-name",
            ".tmp-*",
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&find_tmp.stdout).trim().is_empty(),
        "leftover temp files found: {}",
        String::from_utf8_lossy(&find_tmp.stdout)
    );
    assert!(
        canaries
            .leaks(&[Output::Bytes {
                name: "sftp_decompressed",
                bytes: decompressed_text.as_bytes(),
            }])
            .unwrap()
            .is_empty(),
        "secret canary leaked into SFTP parts"
    );
}

#[cfg(target_os = "linux")]
async fn wait_for_sftp(url: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let u = url.to_owned();
        let ok = tokio::task::spawn_blocking(move || {
            let mut dest = SftpDestination::from_url("probe", &u, None).unwrap();
            dest.probe(&copy(1)).is_ok()
        })
        .await
        .unwrap();
        if ok {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "SFTP container did not become ready in time"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(target_os = "linux")]
fn wait_for_ftp(url: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let mut probe_dest = FtpDestination::from_url("ftp_sink", url, Some("exports")).unwrap();
        if probe_dest.probe(&copy(1)).is_ok() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "FTP container did not become ready in time"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// FTP Sink conformance against the pinned `delfer/alpine-ftp-server`
/// container: atomic temp-file + `RNFR`/`RNTO` rename, `GuaranteeClass::Keyed`,
/// `Fault::AnswerLost` retry, resume probe, byte-for-byte Unicode round-trip,
/// and non-vacuous Secret canary scan.
///
/// Covers: sink:ftp
#[cfg(target_os = "linux")]
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end FTP container conformance exercises delivery, fault retry, resume probe, Unicode, and canary checks"
)]
fn ftp_sink_conformance_against_ftp_container() {
    const IMAGE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/ftp/image.toml"
    ));
    let mut canaries = Canaries::new(11590);
    let ftp_pass = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();
    let pasv_port = allocate_ephemeral_port();
    let pasv_map = format!("127.0.0.1:{pasv_port}:{pasv_port}");
    let users_env = format!("USERS=goodfetch|{ftp_pass}|/ftp/goodfetch");
    let min_env = format!("MIN_PORT={pasv_port}");
    let max_env = format!("MAX_PORT={pasv_port}");
    let guard = ContainerGuard::spawn(
        IMAGE,
        &[
            "-p",
            "127.0.0.1::21",
            "-p",
            &pasv_map,
            "-e",
            &users_env,
            "-e",
            "ADDRESS=127.0.0.1",
            "-e",
            &min_env,
            "-e",
            &max_env,
        ],
        &[],
    );
    let port = guard.published_port("21/tcp");

    let url = url_with_credentials(&format!("ftp://127.0.0.1:{port}"), "goodfetch", &ftp_pass);
    wait_for_ftp(&url);

    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join(
        "ftp_pass",
        &SecretString::from(ftp_pass.clone()),
        Reach::Value,
    );

    let unicode = unicode_value();
    let dest = FtpDestination::from_url("ftp_sink", &url, Some("exports")).unwrap();
    let config = FilesConfig {
        format: ExportFormat::Jsonl,
        compression: ExportCompression::Zstd,
        max_records: 2,
        ..FilesConfig::default()
    };
    let sink = FilesSink::new("ftp_sink", "exports", config, Clock::system(), dest);
    let faulty = Faulty::new(sink, Fault::AnswerLost(1));
    let records = run_file(&[(
        "items",
        vec![
            json!({ "id": "a", "n": 1 }),
            unicode.clone(),
            json!({ "id": "b", "n": 2 }),
        ],
    )]);
    let mut driver = Driver::new("ftp_sink", faulty, copy(1), vec!["items".to_owned()])
        .with_clock(Clock::system())
        .with_masks(masks);
    assert!(matches!(
        driver.deliver(&records).unwrap(),
        Status::Behind { .. }
    ));
    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    assert_eq!(driver.finish(), Status::Complete);

    assert_eq!(driver.class(), Some(GuaranteeClass::Keyed));
    let written_keys = driver.into_sink().into_inner().written_keys().to_vec();
    assert_eq!(written_keys.len(), 2);

    let mut probe_dest = FtpDestination::from_url("ftp_sink", &url, Some("exports")).unwrap();
    let probed = probe_dest.probe(&copy(1)).unwrap();
    assert_eq!(probed.class, GuaranteeClass::Keyed);
    assert_eq!(probed.delivered.get("items").copied(), Some(3));
    assert_eq!(probed.next_parts.get("items").copied(), Some(3));

    let mut ftp = FtpStream::connect(("127.0.0.1", port)).unwrap();
    ftp.set_mode(Mode::Passive);
    ftp.login("goodfetch", &ftp_pass).unwrap();
    ftp.transfer_type(suppaftp::types::FileType::Binary)
        .unwrap();

    let mut all_values = Vec::new();
    let mut decompressed_text = String::new();
    for key in &written_keys {
        let buf = ftp.retr_as_buffer(key).unwrap().into_inner();
        let vals = decode_zstd_jsonl(&buf);
        for v in &vals {
            decompressed_text.push_str(&v.to_string());
            decompressed_text.push('\n');
        }
        all_values.extend(vals);
    }
    let _ = ftp.quit();

    assert_eq!(all_values.len(), 3);
    assert_eq!(all_values[1], unicode);

    let find_tmp = Command::new(guard.engine)
        .args([
            "exec",
            &guard.id,
            "find",
            "/ftp/goodfetch",
            "-name",
            ".tmp-*",
        ])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&find_tmp.stdout).trim().is_empty(),
        "leftover temp files found: {}",
        String::from_utf8_lossy(&find_tmp.stdout)
    );
    assert!(
        canaries
            .leaks(&[Output::Bytes {
                name: "ftp_decompressed",
                bytes: decompressed_text.as_bytes(),
            }])
            .unwrap()
            .is_empty(),
        "secret canary leaked into FTP parts"
    );
}
