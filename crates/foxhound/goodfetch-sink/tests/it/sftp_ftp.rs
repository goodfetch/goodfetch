//! Conformance tests for SFTP and FTP Files-mode Sinks (ADRs 0008, 0026,
//! 0067 and 0127; issue 1159) against pinned containers (`ci/sftp/image.toml`
//! and `ci/ftp/image.toml`): atomic temp-file + rename so readers never observe
//! half-written parts, `GuaranteeClass::Keyed`, resume probe, Unicode
//! byte-for-byte round-trip, Secret canary scan, and ADR 0127 `map_failure()`.
//!
//! SFTP and FTP are self-hosted protocols with no single free-tier cloud
//! service; their conformance suites run against containers and their Live
//! checks wait (ADR 0064).
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests launch pinned containers and run on Tokio's runtime"
)]

use std::io::Read as _;
use std::process::Command;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use goodfetch_sink::{
    Driver, FailureKind, FileDestination as _, FilesConfig, FilesSink, FtpDestination,
    FtpSinkError, GuaranteeClass, SftpDestination, SftpError, Status, map_ftp_error,
    map_sftp_error,
};
use goodfetch_test_support::{Canaries, Output, SecretKind, unicode_record};
use secrecy::SecretString;
use serde_json::{Value, json};
use suppaftp::{FtpError, FtpStream, Mode};

use crate::run_files::{copy, run_file};

fn unicode_value() -> Value {
    unicode_record()
        .fields
        .iter()
        .map(|field| (field.name.clone(), Value::String(field.text.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

fn decode_zstd_jsonl(bytes: &[u8]) -> Vec<Value> {
    let mut decoder = zstd::stream::read::Decoder::new(bytes).unwrap();
    let mut text = String::new();
    decoder.read_to_string(&mut text).unwrap();
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[cfg(target_os = "linux")]
struct ContainerGuard {
    engine: &'static str,
    id: String,
}

#[cfg(target_os = "linux")]
impl ContainerGuard {
    fn start(image_toml: &str, args: &[&str], port_spec: &str) -> (Self, u16) {
        let engine = ["docker", "podman"]
            .into_iter()
            .find(|c| {
                Command::new(c)
                    .arg("info")
                    .output()
                    .is_ok_and(|o| o.status.success())
            })
            .expect("no container engine (docker/podman) answers");
        let pin: toml::Table = toml::from_str(image_toml).expect("valid image.toml");
        let image = format!(
            "{}@{}",
            pin["repository"].as_str().unwrap(),
            pin["digest"].as_str().unwrap()
        );
        let mut cmd = Command::new(engine);
        cmd.args(["run", "--detach", "--rm"]);
        cmd.args(args);
        cmd.arg(&image);
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{engine} failed to start {image}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let port_out = Command::new(engine)
            .args(["port", &id, port_spec])
            .output()
            .unwrap();
        assert!(port_out.status.success(), "cannot read port {port_spec}");
        let host_port: u16 = String::from_utf8_lossy(&port_out.stdout)
            .lines()
            .next()
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        (Self { engine, id }, host_port)
    }
}

#[cfg(target_os = "linux")]
impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = Command::new(self.engine)
            .args(["rm", "-f", &self.id])
            .output();
    }
}

#[test]
fn sftp_and_ftp_failure_translation_marks_unrecognized_as_transient() {
    let sftp_unrecognized = map_sftp_error(SftpError::Sftp(
        russh_sftp::client::error::Error::UnexpectedPacket,
    ));
    assert_eq!(sftp_unrecognized.kind(), FailureKind::Unrecognized);

    let sftp_auth = map_sftp_error(SftpError::Auth("bad pass".into()));
    assert_eq!(sftp_auth.kind(), FailureKind::Auth);

    let ftp_unrecognized = map_ftp_error(FtpSinkError::Ftp(FtpError::BadResponse));
    assert_eq!(ftp_unrecognized.kind(), FailureKind::Unrecognized);

    let ftp_config = map_ftp_error(FtpSinkError::Config("bad url".into()));
    assert_eq!(ftp_config.kind(), FailureKind::Permanent);
}

/// SFTP Sink conformance against the pinned `atmoz/sftp:alpine` container:
/// atomic temp-file + rename (no `.tmp-*` leftovers), `GuaranteeClass::Keyed`,
/// resume probe, byte-for-byte Unicode round-trip, and Secret canary scan.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sftp_sink_conformance_against_sftp_container() {
    const IMAGE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/sftp/image.toml"
    ));
    let (guard, port) = start_sftp_container(IMAGE);
    let url =
        format!("sftp://goodfetch:goodfetchpass@127.0.0.1:{port}/upload?accept_unknown_host=true");
    wait_for_sftp(&url).await;

    let mut canaries = Canaries::new(1159);
    let sftp_secret = canaries.plant(SecretKind::ProxyPassword).value().to_owned();
    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join("sftp_pass", &SecretString::from(sftp_secret), Reach::Value);

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
        let records = run_file(&[(
            "items",
            vec![
                json!({ "id": "a", "n": 1 }),
                unicode,
                json!({ "id": "b", "n": 2 }),
            ],
        )]);
        let mut driver = Driver::new("sftp_sink", sink, copy(1), vec!["items".to_owned()])
            .with_clock(Clock::system())
            .with_masks(masks);
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.finish(), Status::Complete);
        let class = driver.class();
        let keys = driver.into_sink().written_keys().to_vec();
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

    // Verify resume probe finds all 3 delivered records and no `.tmp-*` files exist.
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

    // Download the uploaded parts from the container to verify atomic rename,
    // Unicode byte-for-byte round-trip, and zero Secret canary leaks.
    let mut all_values = Vec::new();
    let mut all_bytes = Vec::new();
    for key in &written_keys {
        let remote = format!("/home/goodfetch/upload/{key}");
        let cat = Command::new(guard.engine)
            .args(["exec", &guard.id, "cat", &remote])
            .output()
            .unwrap();
        assert!(cat.status.success(), "missing uploaded SFTP part {remote}");
        all_bytes.extend_from_slice(&cat.stdout);
        all_values.extend(decode_zstd_jsonl(&cat.stdout));
    }
    assert_eq!(all_values.len(), 3);
    assert_eq!(all_values[1], expected_unicode);

    // Ensure no `.tmp-*` files remain in the target directory.
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
                name: "sftp_parts",
                bytes: &all_bytes,
            }])
            .unwrap()
            .is_empty(),
        "secret canary leaked into SFTP parts"
    );
}

#[cfg(target_os = "linux")]
fn start_sftp_container(image_toml: &str) -> (ContainerGuard, u16) {
    let engine = ["docker", "podman"]
        .into_iter()
        .find(|c| {
            Command::new(c)
                .arg("info")
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .expect("no container engine (docker/podman) answers");
    let pin: toml::Table = toml::from_str(image_toml).expect("valid image.toml");
    let image = format!(
        "{}@{}",
        pin["repository"].as_str().unwrap(),
        pin["digest"].as_str().unwrap()
    );
    let out = Command::new(engine)
        .args([
            "run",
            "--detach",
            "--rm",
            "-p",
            "127.0.0.1::22",
            &image,
            "goodfetch:goodfetchpass:::upload",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{engine} failed to start {image}: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let port_out = Command::new(engine)
        .args(["port", &id, "22/tcp"])
        .output()
        .unwrap();
    let host_port: u16 = String::from_utf8_lossy(&port_out.stdout)
        .lines()
        .next()
        .unwrap()
        .rsplit(':')
        .next()
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    (ContainerGuard { engine, id }, host_port)
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
/// resume probe, byte-for-byte Unicode round-trip, and Secret canary scan.
#[cfg(target_os = "linux")]
#[test]
fn ftp_sink_conformance_against_ftp_container() {
    const IMAGE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/ftp/image.toml"
    ));
    let (guard, port) = ContainerGuard::start(
        IMAGE,
        &[
            "-p",
            "127.0.0.1::21",
            "-p",
            "127.0.0.1:21120-21130:21120-21130",
            "-e",
            "USERS=goodfetch|goodfetchpass|/ftp/goodfetch",
            "-e",
            "ADDRESS=127.0.0.1",
            "-e",
            "MIN_PORT=21120",
            "-e",
            "MAX_PORT=21130",
        ],
        "21/tcp",
    );

    let url = format!("ftp://goodfetch:goodfetchpass@127.0.0.1:{port}");
    wait_for_ftp(&url);

    let mut canaries = Canaries::new(11590);
    let ftp_secret = canaries.plant(SecretKind::ProxyPassword).value().to_owned();
    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join("ftp_pass", &SecretString::from(ftp_secret), Reach::Value);

    let unicode = unicode_value();
    let dest = FtpDestination::from_url("ftp_sink", &url, Some("exports")).unwrap();
    let config = FilesConfig {
        format: ExportFormat::Jsonl,
        compression: ExportCompression::Zstd,
        max_records: 2,
        ..FilesConfig::default()
    };
    let sink = FilesSink::new("ftp_sink", "exports", config, Clock::system(), dest);
    let records = run_file(&[(
        "items",
        vec![
            json!({ "id": "a", "n": 1 }),
            unicode.clone(),
            json!({ "id": "b", "n": 2 }),
        ],
    )]);
    let mut driver = Driver::new("ftp_sink", sink, copy(1), vec!["items".to_owned()])
        .with_clock(Clock::system())
        .with_masks(masks);
    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    assert_eq!(driver.finish(), Status::Complete);

    assert_eq!(driver.class(), Some(GuaranteeClass::Keyed));
    let written_keys = driver.into_sink().written_keys().to_vec();
    assert_eq!(written_keys.len(), 2);

    // Resume probe finds all 3 delivered records.
    let mut probe_dest = FtpDestination::from_url("ftp_sink", &url, Some("exports")).unwrap();
    let probed = probe_dest.probe(&copy(1)).unwrap();
    assert_eq!(probed.class, GuaranteeClass::Keyed);
    assert_eq!(probed.delivered.get("items").copied(), Some(3));
    assert_eq!(probed.next_parts.get("items").copied(), Some(3));

    // Fetch each uploaded part back over FTP and verify Unicode + canary scan.
    let mut ftp = FtpStream::connect(("127.0.0.1", port)).unwrap();
    ftp.set_mode(Mode::Passive);
    ftp.login("goodfetch", "goodfetchpass").unwrap();
    ftp.transfer_type(suppaftp::types::FileType::Binary)
        .unwrap();

    let mut all_values = Vec::new();
    let mut all_bytes = Vec::new();
    for key in &written_keys {
        let buf = ftp.retr_as_buffer(key).unwrap().into_inner();
        all_bytes.extend_from_slice(&buf);
        all_values.extend(decode_zstd_jsonl(&buf));
    }
    let _ = ftp.quit();

    assert_eq!(all_values.len(), 3);
    assert_eq!(all_values[1], unicode);

    // Ensure no `.tmp-*` files remain in the FTP root.
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
                name: "ftp_parts",
                bytes: &all_bytes,
            }])
            .unwrap()
            .is_empty(),
        "secret canary leaked into FTP parts"
    );
}
