//! Files-mode Sink conformance tests (ADR 0026; issue 1152): verifies part
//! rolling by count (`max_records`), size (`max_bytes`) and age (`max_age`),
//! final part on `finish`, part keys carrying the delivery ID and run epoch,
//! atomic visibility (interrupted writes never appear as parts), valid Export
//! encoding/compression with Records in number order, and end-to-end S3
//! delivery through `object_store` against the pinned RustFS container.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "container test helper spawns podman/docker and polls loopback TCP on the real clock"
)]
#![expect(
    clippy::disallowed_types,
    reason = "in-memory test destination and loopback TCP readiness probe"
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::net::TcpStream;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use futures_util::TryStreamExt as _;
use goodfetch_engine::clock::Clock;
use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use goodfetch_sink::{
    Driver, Failure, FailureKind, FileDestination, FileProbe, FilesConfig, FilesSink,
    GuaranteeClass, ObjectStoreBackend, ObjectStoreDestination, PutOutcome, RunCopy, Status,
};
use goodfetch_test_support::unicode_record;
use object_store::ObjectStoreExt as _;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::run_files::{commit, copy, items, run_file};

const SINK: &str = "lake";

/// In-memory [`FileDestination`] for deterministic unit/conformance tests of
/// part rolling, age expiration, schema union and interrupted uploads.
#[derive(Clone, Default)]
struct MemoryFiles {
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    fail_on_put: Arc<Mutex<Option<usize>>>,
    puts_seen: Arc<Mutex<usize>>,
}

impl MemoryFiles {
    fn new() -> Self {
        Self::default()
    }

    fn fail_put(&self, nth: usize) {
        *self.fail_on_put.lock().unwrap() = Some(nth);
    }

    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        self.objects.lock().unwrap().clone()
    }
}

impl FileDestination for MemoryFiles {
    type Error = String;

    fn probe(&mut self, _run: &RunCopy) -> Result<FileProbe, Self::Error> {
        Ok(FileProbe {
            class: GuaranteeClass::Fenced,
            delivered: BTreeMap::new(),
            next_parts: BTreeMap::new(),
            superseded_by: None,
        })
    }

    fn put_part(
        &mut self,
        key: &str,
        bytes: Vec<u8>,
        _run: &RunCopy,
    ) -> Result<PutOutcome, Self::Error> {
        let mut puts = self.puts_seen.lock().unwrap();
        *puts += 1;
        if *self.fail_on_put.lock().unwrap() == Some(*puts) {
            // Simulate an interrupted upload: nothing is published at `key`.
            return Err("simulated interrupted upload".to_owned());
        }
        self.objects.lock().unwrap().insert(key.to_owned(), bytes);
        Ok(PutOutcome::Written)
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        Failure::new(FailureKind::Transient, error)
    }
}

fn decode_jsonl(bytes: &[u8], compression: ExportCompression) -> Vec<Value> {
    let raw = match compression {
        ExportCompression::None => bytes.to_vec(),
        ExportCompression::Gzip => {
            let mut dec = GzDecoder::new(bytes);
            let mut out = Vec::new();
            dec.read_to_end(&mut out).unwrap();
            out
        }
        ExportCompression::Zstd => zstd::stream::decode_all(bytes).unwrap(),
    };
    let text = String::from_utf8(raw).unwrap();
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Parts roll by count (`files.part.max_records`), a final part is written at
/// `finish`, every part key carries the delivery ID and run epoch, and every
/// part is a valid compressed Export with Records in number order.
///
/// Covers: story:6-287, sink:s3
#[tokio::test(start_paused = true)]
async fn files_mode_rolls_by_count_and_writes_final_part_with_valid_exports() {
    let dest = MemoryFiles::new();
    let clock = Clock::paused(std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    let config = FilesConfig {
        format: ExportFormat::Jsonl,
        compression: ExportCompression::Zstd,
        max_records: 3,
        ..FilesConfig::default()
    };
    let sink = FilesSink::new(SINK, "exports", config, clock, dest.clone());
    let records = run_file(&[("products", items("p", 8))]);
    let mut driver = Driver::new(SINK, sink, copy(2), vec!["products".to_owned()])
        .with_clock(clock)
        .with_batch_records(10);

    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    // 8 records with max_records=3 -> 2 full parts (3 + 3) during deliver, 2 left buffered.
    assert_eq!(dest.snapshot().len(), 2);

    // Finish writes the final part (2 records).
    assert_eq!(driver.finish(), Status::Complete);
    let objects = dest.snapshot();
    assert_eq!(objects.len(), 3);

    let mut all_items = Vec::new();
    for (idx, (key, bytes)) in objects.iter().enumerate() {
        assert!(
            key.starts_with("exports/run-1/lake/products/epoch-2/part-"),
            "unexpected key: {key}"
        );
        assert!(
            key.contains("run-1_lake_products_"),
            "key must carry delivery ID: {key}"
        );
        assert!(key.ends_with(".jsonl.zst"), "key extension: {key}");
        let decoded = decode_jsonl(bytes, ExportCompression::Zstd);
        let expected_len = if idx < 2 { 3 } else { 2 };
        assert_eq!(decoded.len(), expected_len, "part {key}");
        all_items.extend(decoded);
    }
    assert_eq!(all_items, items("p", 8));
}

/// Parts roll by byte size (`files.part.max_bytes`) and by age
/// (`files.part.max_age`) on a paused `Clock`, and an interrupted part upload
/// is never visible as a part.
///
/// Covers: story:6-287, sink:s3
#[tokio::test(start_paused = true)]
async fn files_mode_rolls_by_size_and_age_and_hides_interrupted_parts() {
    // 1. Roll by size (gzip JSONL).
    let dest = MemoryFiles::new();
    let clock = Clock::paused(std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    let config = FilesConfig {
        format: ExportFormat::Jsonl,
        compression: ExportCompression::Gzip,
        max_records: 100,
        max_bytes: 30,
        ..FilesConfig::default()
    };
    let sink = FilesSink::new(SINK, "", config, clock, dest.clone());
    let records = run_file(&[("products", items("item-long-value-", 5))]);
    let mut driver = Driver::new(SINK, sink, copy(1), vec!["products".to_owned()])
        .with_clock(clock)
        .with_batch_records(10);
    assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
    assert_eq!(driver.finish(), Status::Complete);
    let objects = dest.snapshot();
    assert!(
        objects.len() >= 3,
        "expected size-rolled parts, got {}",
        objects.len()
    );
    let mut decoded_all = Vec::new();
    for bytes in objects.values() {
        decoded_all.extend(decode_jsonl(bytes, ExportCompression::Gzip));
    }
    assert_eq!(decoded_all, items("item-long-value-", 5));

    // 2. Roll by age on paused clock.
    let dest_age = MemoryFiles::new();
    let clock_age = Clock::paused(std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    let config_age = FilesConfig {
        format: ExportFormat::Jsonl,
        compression: ExportCompression::None,
        max_records: 100,
        max_bytes: 1024 * 1024,
        max_age: Duration::from_secs(60),
        ..FilesConfig::default()
    };
    let sink_age = FilesSink::new(SINK, "", config_age, clock_age, dest_age.clone());
    let records_age = run_file(&[("products", items("a", 2))]);
    let mut driver_age = Driver::new(SINK, sink_age, copy(1), vec!["products".to_owned()])
        .with_clock(clock_age)
        .with_batch_records(10);
    assert_eq!(driver_age.deliver(&records_age).unwrap(), Status::Complete);
    assert_eq!(dest_age.snapshot().len(), 0, "not yet expired");

    // Advance clock past max_age WITHOUT committing another record -> expired part rolls.
    assert!(driver_age.next_flush_after().is_some());
    tokio::time::advance(Duration::from_secs(61)).await;
    assert_eq!(driver_age.deliver(&records_age).unwrap(), Status::Complete);
    assert_eq!(
        dest_age.snapshot().len(),
        1,
        "expired part rolled on age without another record arriving"
    );
    commit(&records_age, "products", 3, &[json!({ "item": "a3" })]);
    assert_eq!(driver_age.deliver(&records_age).unwrap(), Status::Complete);
    assert_eq!(driver_age.finish(), Status::Complete);
    assert_eq!(dest_age.snapshot().len(), 2, "final part written on finish");

    // 3. Interrupted upload leaves no partial part visible.
    let dest_fail = MemoryFiles::new();
    dest_fail.fail_put(1);
    let clock_fail = Clock::paused(std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000));
    let config_fail = FilesConfig {
        format: ExportFormat::Jsonl,
        compression: ExportCompression::None,
        max_records: 2,
        ..FilesConfig::default()
    };
    let sink_fail = FilesSink::new(SINK, "", config_fail, clock_fail, dest_fail.clone());
    let records_fail = run_file(&[("products", items("x", 2))]);
    let mut driver_fail = Driver::new(SINK, sink_fail, copy(1), vec!["products".to_owned()])
        .with_clock(clock_fail)
        .with_batch_records(10);
    assert!(matches!(
        driver_fail.deliver(&records_fail).unwrap(),
        Status::Behind { .. }
    ));
    assert!(
        dest_fail.snapshot().is_empty(),
        "interrupted part must never be visible"
    );
}

/// Pinned RustFS container for S3 Files-mode conformance tests (ADR 0043).
#[cfg(target_os = "linux")]
struct RustFs {
    engine: &'static str,
    container: String,
    endpoint: String,
    _data_dir: TempDir,
}

#[cfg(target_os = "linux")]
impl RustFs {
    const IMAGE_FILE: &'static str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/rustfs/image.toml"
    ));

    fn start(bucket: &str) -> Self {
        let engine = ["docker", "podman"]
            .into_iter()
            .find(|c| {
                Command::new(c)
                    .arg("info")
                    .output()
                    .is_ok_and(|o| o.status.success())
            })
            .expect("no container engine (docker/podman) answers; install podman or docker");
        let pin: toml::Table =
            toml::from_str(Self::IMAGE_FILE).expect("ci/rustfs/image.toml is valid TOML");
        let repo = pin["repository"].as_str().unwrap();
        let digest = pin["digest"].as_str().unwrap();
        let image = format!("{repo}@{digest}");

        let data_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data_dir.path().join(bucket)).unwrap();
        let mut perms = std::fs::metadata(data_dir.path()).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o777);
        std::fs::set_permissions(data_dir.path(), perms).unwrap();
        let mut bperms = std::fs::metadata(data_dir.path().join(bucket))
            .unwrap()
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut bperms, 0o777);
        std::fs::set_permissions(data_dir.path().join(bucket), bperms).unwrap();

        let out = Command::new(engine)
            .args([
                "run",
                "--detach",
                "--rm",
                "--user",
                "0:0",
                "-p",
                "127.0.0.1::9000",
                "-e",
                "RUSTFS_ACCESS_KEY=goodfetch",
                "-e",
                "RUSTFS_SECRET_KEY=goodfetchsecret",
                "-v",
            ])
            .arg(format!("{}:/data", data_dir.path().display()))
            .arg(&image)
            .arg("/data")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{engine} cannot start {image}: {}; `ci/rustfs/fetch.sh` pulls it",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        let container = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let mut guard = Self {
            engine,
            container,
            endpoint: String::new(),
            _data_dir: data_dir,
        };
        let port_out = Command::new(engine)
            .args(["port", &guard.container, "9000/tcp"])
            .output()
            .unwrap();
        let host_port = String::from_utf8_lossy(&port_out.stdout)
            .lines()
            .next()
            .unwrap()
            .trim()
            .to_owned();

        let deadline = Instant::now() + Duration::from_secs(20);
        let mut ready = false;
        while Instant::now() < deadline {
            if TcpStream::connect(&host_port).is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            ready,
            "RustFS container did not listen on {host_port} within 20s"
        );
        // Allow RustFS to finish bucket initialization after opening its listener.
        std::thread::sleep(Duration::from_millis(500));
        guard.endpoint = format!("http://{host_port}");
        guard
    }

    fn s3_url(&self, bucket: &str) -> String {
        format!(
            "s3://goodfetch:goodfetchsecret@{bucket}/crawl?endpoint={}&region=us-east-1&allow_http=true",
            self.endpoint
        )
    }
}

#[cfg(target_os = "linux")]
impl Drop for RustFs {
    fn drop(&mut self) {
        let _ = Command::new(self.engine)
            .args(["rm", "-f", &self.container])
            .output();
    }
}

#[cfg(target_os = "linux")]
async fn deliver_s3_run(url: String, recs: Vec<Value>, epoch: u64) -> Option<GuaranteeClass> {
    tokio::task::spawn_blocking(move || {
        let dest =
            ObjectStoreDestination::from_url(ObjectStoreBackend::S3, SINK, &url, None).unwrap();
        let config = FilesConfig {
            format: ExportFormat::Jsonl,
            compression: ExportCompression::Zstd,
            max_records: 2,
            ..FilesConfig::default()
        };
        let clock = Clock::system();
        let sink = FilesSink::new(SINK, "crawl", config, clock, dest);
        let records = run_file(&[("products", recs)]);
        let mut driver = Driver::new(SINK, sink, copy(epoch), vec!["products".to_owned()])
            .with_clock(clock)
            .with_batch_records(10);
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.finish(), Status::Complete);
        driver.class()
    })
    .await
    .unwrap()
}

/// End-to-end Files-mode Sink conformance over `object_store`'s S3 driver
/// against the pinned RustFS container: parts roll by count, final part is
/// written at finish, keys carry delivery ID and epoch, Unicode round-trips
/// byte for byte, and a zombie copy with an older epoch is fenced out when
/// conditional writes are supported.
///
/// Covers: story:6-287, sink:s3
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_files_mode_conformance_against_rustfs() {
    let rustfs = RustFs::start("goodfetch-sink-test");
    let url = rustfs.s3_url("goodfetch-sink-test");
    let store = ObjectStoreDestination::from_url(ObjectStoreBackend::S3, SINK, &url, None)
        .unwrap()
        .store();

    let mut recs = items("s3-", 4);
    let unicode: Value = unicode_record()
        .fields
        .iter()
        .map(|field| (field.name.clone(), Value::String(field.text.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into();
    recs.push(unicode);

    let class = deliver_s3_run(url.clone(), recs.clone(), 1).await;
    assert!(matches!(
        class,
        Some(GuaranteeClass::Fenced | GuaranteeClass::Keyed)
    ));

    // Read all parts back directly from RustFS and verify valid Exports in order.
    let mut part_keys = Vec::new();
    let mut stream = store.list(None);
    while let Some(meta) = stream.try_next().await.unwrap() {
        let key = meta.location.as_ref().to_owned();
        if key.contains("/part-") {
            part_keys.push(key);
        }
    }
    part_keys.sort();
    assert_eq!(
        part_keys.len(),
        3,
        "5 records / 2 per part = 3 parts: {part_keys:?}"
    );

    let mut round_tripped = Vec::new();
    for key in &part_keys {
        assert!(key.contains("/epoch-1/part-"), "missing epoch in {key}");
        assert!(
            key.contains("run-1_lake_products_"),
            "missing delivery ID in {key}"
        );
        let data = store
            .get(&object_store::path::Path::from(key.as_str()))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        round_tripped.extend(decode_jsonl(&data, ExportCompression::Zstd));
    }
    assert_eq!(round_tripped, recs);

    // If RustFS proved Fenced (conditional writes), a copy at epoch 2 fences out epoch 1.
    if class == Some(GuaranteeClass::Fenced) {
        let class2 = deliver_s3_run(url.clone(), recs.clone(), 2).await;
        assert_eq!(class2, Some(GuaranteeClass::Fenced));

        let url_clone = url;
        tokio::task::spawn_blocking(move || {
            let dest_zombie =
                ObjectStoreDestination::from_url(ObjectStoreBackend::S3, SINK, &url_clone, None)
                    .unwrap();
            let prefix = dest_zombie.prefix().to_owned();
            let sink_zombie = FilesSink::new(
                SINK,
                prefix,
                FilesConfig {
                    max_records: 1,
                    ..FilesConfig::default()
                },
                Clock::system(),
                dest_zombie,
            );
            let records = run_file(&[("items", recs)]);
            let mut driver_zombie =
                Driver::new(SINK, sink_zombie, copy(1), vec!["items".to_owned()])
                    .with_clock(Clock::system());
            assert_eq!(
                driver_zombie.deliver(&records).unwrap(),
                Status::Superseded { run_epoch: 2 }
            );
        })
        .await
        .unwrap();
    }
}
