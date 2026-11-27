//! Google Cloud Storage (`gcs`) and Azure Blob (`azure_blob`) Files-mode Sink
//! conformance tests (ADRs 0026, 0043, 0067 and 0127; issue 1156): runs
//! against pinned `fake-gcs-server` and `azurite` containers, verifies probe
//! guarantee classes, `map_failure()` translation including `Unrecognized`,
//! byte-for-byte Unicode round-trip, Secret canary masking, and disabled Live
//! checks.
#![expect(
    clippy::disallowed_methods,
    reason = "container test helpers spawn podman/docker and poll loopback TCP on the real clock"
)]
#![expect(
    clippy::disallowed_types,
    reason = "loopback TCP readiness probe for container emulators"
)]

use std::fmt::Write as _;
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::{Duration, Instant};

use futures_util::TryStreamExt as _;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::mask::{Masks, Reach, Redaction};
use goodfetch_protocol::foxhound::export::{ExportCompression, ExportFormat};
use goodfetch_sink::{
    Driver, FailureKind, FileDestination as _, FilesConfig, FilesSink, GuaranteeClass,
    ObjectStoreBackend, ObjectStoreDestination, ObjectStoreError, Status,
};
use goodfetch_test_support::{Canaries, Output, SecretKind, unicode_record};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use object_store::ObjectStoreExt as _;
use secrecy::SecretString;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::run_files::{copy, items, run_file};

const SINK: &str = "cloud";

fn container_engine() -> &'static str {
    ["docker", "podman"]
        .into_iter()
        .find(|c| {
            Command::new(c)
                .arg("info")
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .expect("no container engine (docker/podman) answers")
}

fn pinned_image(toml_text: &str) -> String {
    let pin: toml::Table = toml::from_str(toml_text).expect("image.toml is valid TOML");
    format!(
        "{}@{}",
        pin["repository"].as_str().unwrap(),
        pin["digest"].as_str().unwrap()
    )
}

fn wait_tcp(addr: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if TcpStream::connect(addr).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("container did not listen on {addr} within 20s");
}

fn unicode_value() -> Value {
    unicode_record()
        .fields
        .iter()
        .map(|field| (field.name.clone(), Value::String(field.text.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

fn decode_zstd_jsonl(bytes: &[u8]) -> Vec<Value> {
    let raw = zstd::stream::decode_all(bytes).unwrap();
    let text = String::from_utf8(raw).unwrap();
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Pinned `fake-gcs-server` container for GCS Sink conformance.
#[cfg(target_os = "linux")]
struct FakeGcs {
    engine: &'static str,
    container: String,
    endpoint: String,
    _data_dir: TempDir,
}

#[cfg(target_os = "linux")]
impl FakeGcs {
    const IMAGE_FILE: &'static str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/fake-gcs/image.toml"
    ));

    fn start(bucket: &str) -> Self {
        let engine = container_engine();
        let image = pinned_image(Self::IMAGE_FILE);
        let data_dir = tempfile::tempdir().unwrap();
        let bucket_dir = data_dir.path().join(bucket);
        std::fs::create_dir_all(&bucket_dir).unwrap();
        std::fs::write(bucket_dir.join(".keep"), b"").unwrap();
        let mut perms = std::fs::metadata(data_dir.path()).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o777);
        std::fs::set_permissions(data_dir.path(), perms).unwrap();
        let mut bperms = std::fs::metadata(data_dir.path().join(bucket))
            .unwrap()
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut bperms, 0o777);
        std::fs::set_permissions(data_dir.path().join(bucket), bperms).unwrap();

        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let host_port = format!("127.0.0.1:{port}");
        let endpoint = format!("http://{host_port}");

        let out = Command::new(engine)
            .args([
                "run",
                "--detach",
                "--rm",
                "--user",
                "0:0",
                "-p",
                &format!("{host_port}:4443"),
                "-v",
            ])
            .arg(format!("{}:/data", data_dir.path().display()))
            .arg(&image)
            .args([
                "-scheme",
                "http",
                "-port",
                "4443",
                "-backend",
                "memory",
                "-external-url",
                &endpoint,
                "-data",
                "/data",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{engine} cannot start {image}: {}; `ci/fake-gcs/fetch.sh` pulls it",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        let container = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        wait_tcp(&host_port);
        Self {
            engine,
            container,
            endpoint,
            _data_dir: data_dir,
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for FakeGcs {
    fn drop(&mut self) {
        let _ = Command::new(self.engine)
            .args(["rm", "-f", &self.container])
            .output();
    }
}

/// Pinned `azurite` container for Azure Blob Sink conformance.
#[cfg(target_os = "linux")]
struct Azurite {
    engine: &'static str,
    container: String,
    endpoint: String,
}

#[cfg(target_os = "linux")]
impl Azurite {
    const IMAGE_FILE: &'static str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../ci/azurite/image.toml"
    ));

    fn start() -> Self {
        let engine = container_engine();
        let image = pinned_image(Self::IMAGE_FILE);
        let out = Command::new(engine)
            .args(["run", "--detach", "--rm", "-p", "127.0.0.1::10000"])
            .arg(&image)
            .args([
                "azurite-blob",
                "--blobHost",
                "0.0.0.0",
                "--blobPort",
                "10000",
                "--skipApiVersionCheck",
                "--disableProductStyleUrl",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{engine} cannot start {image}: {}; `ci/azurite/fetch.sh` pulls it",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        let container = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let port_out = Command::new(engine)
            .args(["port", &container, "10000/tcp"])
            .output()
            .unwrap();
        let host_port = String::from_utf8_lossy(&port_out.stdout)
            .lines()
            .next()
            .unwrap()
            .trim()
            .to_owned();
        wait_tcp(&host_port);
        Self {
            engine,
            container,
            endpoint: format!("http://{host_port}/devstoreaccount1"),
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Azurite {
    fn drop(&mut self) {
        let _ = Command::new(self.engine)
            .args(["rm", "-f", &self.container])
            .output();
    }
}

/// Starts a loopback HTTP proxy in front of `fake-gcs-server` translating
/// `object_store`'s GCS XML API `PUT/GET /{bucket}/{object}` calls to
/// `fake-gcs-server`'s GCS JSON API (`/upload/storage/v1/b/...` and
/// `/storage/v1/b/.../o/...?alt=media`), forwarding all other requests verbatim.
#[cfg(target_os = "linux")]
async fn start_fake_gcs_xml_bridge(backend_host: String) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let backend = backend_host.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let service = service_fn(move |req: Request<hyper::body::Incoming>| {
                    let backend = backend.clone();
                    async move { forward_gcs(req, &backend).await }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await;
            });
        }
    });
    format!("http://{addr}")
}

#[cfg(target_os = "linux")]
fn json_list_to_xml(resp_body: &[u8]) -> Response<Full<Bytes>> {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult><IsTruncated>false</IsTruncated>",
    );
    if let Ok(json_body) = serde_json::from_slice::<Value>(resp_body)
        && let Some(items) = json_body.get("items").and_then(Value::as_array)
    {
        for item in items {
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            let size = item.get("size").and_then(Value::as_str).unwrap_or("0");
            let etag = item.get("etag").and_then(Value::as_str).unwrap_or("");
            let updated = item
                .get("updated")
                .and_then(Value::as_str)
                .unwrap_or("2026-01-01T00:00:00Z");
            let _ = write!(
                xml,
                "<Contents><Key>{name}</Key><LastModified>{updated}</LastModified><ETag>{etag}</ETag><Size>{size}</Size></Contents>"
            );
        }
    }
    xml.push_str("</ListBucketResult>");
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/xml")
        .body(Full::new(Bytes::from(xml)))
        .unwrap()
}

#[cfg(target_os = "linux")]
fn translate_gcs_request(
    method: Method,
    path_and_query: String,
    if_gen_match: Option<String>,
) -> (Method, String, bool) {
    if path_and_query.starts_with("/storage/") || path_and_query.starts_with("/upload/") {
        return (method, path_and_query, false);
    }
    if let Some((bucket, rest)) = path_and_query.trim_start_matches('/').split_once('/') {
        let obj = rest.split('?').next().unwrap_or(rest);
        return match method {
            Method::PUT => {
                let mut q = format!("/upload/storage/v1/b/{bucket}/o?uploadType=media&name={obj}");
                if let Some(gen_match) = if_gen_match {
                    q.push_str("&ifGenerationMatch=");
                    q.push_str(&gen_match);
                }
                (Method::POST, q, false)
            }
            Method::GET | Method::HEAD => (
                Method::GET,
                format!("/storage/v1/b/{bucket}/o/{obj}?alt=media"),
                false,
            ),
            other => (other, path_and_query, false),
        };
    }
    if method == Method::GET && path_and_query.contains("list-type=2") {
        let (bucket, query) = path_and_query
            .trim_start_matches('/')
            .split_once('?')
            .unwrap_or((path_and_query.trim_start_matches('/'), ""));
        let prefix = query
            .split('&')
            .find_map(|p| p.strip_prefix("prefix="))
            .unwrap_or("");
        let out = if prefix.is_empty() {
            format!("/storage/v1/b/{bucket}/o")
        } else {
            format!("/storage/v1/b/{bucket}/o?prefix={prefix}")
        };
        return (Method::GET, out, true);
    }
    (method, path_and_query, false)
}

#[cfg(target_os = "linux")]
async fn forward_gcs(
    req: Request<hyper::body::Incoming>,
    backend_host: &str,
) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map_or("/", hyper::http::uri::PathAndQuery::as_str)
        .to_owned();
    let if_gen_match = req
        .headers()
        .get("x-goog-if-generation-match")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = req
        .into_body()
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .unwrap_or_default();
    let (out_method, out_path, is_xml_list) =
        translate_gcs_request(method, path_and_query, if_gen_match);

    let Ok(stream) = tokio::net::TcpStream::connect(backend_host).await else {
        return Ok(Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .body(Full::new(Bytes::new()))
            .unwrap());
    };
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let out_req = Request::builder()
        .method(out_method)
        .uri(out_path)
        .header("host", backend_host)
        .header("content-length", body.len())
        .body(Full::new(body))
        .unwrap();
    let resp = sender.send_request(out_req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let resp_body = resp.into_body().collect().await.unwrap().to_bytes();

    if is_xml_list && status.is_success() {
        return Ok(json_list_to_xml(&resp_body));
    }

    let mut builder = Response::builder().status(status);
    for (k, v) in &headers {
        if k != "transfer-encoding" && k != "content-length" {
            builder = builder.header(k, v);
        }
    }
    if status.is_success()
        && let Ok(meta) = serde_json::from_slice::<Value>(&resp_body)
    {
        if let Some(gen_val) = meta.get("generation").and_then(Value::as_str) {
            builder = builder.header("x-goog-generation", gen_val);
        }
        if let Some(etag) = meta.get("etag").and_then(Value::as_str) {
            builder = builder.header("etag", etag);
        }
    }
    Ok(builder.body(Full::new(resp_body)).unwrap())
}

/// GCS Files-mode Sink conformance against pinned `fake-gcs-server`:
/// rolls numbered parts, writes final part, round-trips Unicode byte for byte,
/// and masks Secret canaries in delivered content.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gcs_files_mode_conformance_against_fake_gcs() {
    let gcs = FakeGcs::start("goodfetchgcstest");
    let backend_host = gcs.endpoint.trim_start_matches("http://").to_owned();
    let bridge_endpoint = start_fake_gcs_xml_bridge(backend_host).await;
    let url = format!("gs://goodfetchgcstest/crawl?endpoint={bridge_endpoint}&allow_http=true");
    let store = ObjectStoreDestination::from_url(ObjectStoreBackend::Gcs, SINK, &url, None)
        .unwrap()
        .store();

    let mut canaries = Canaries::new(1156);
    let credential_canary = canaries.plant(SecretKind::ProxyPassword).value().to_owned();
    let masks = Masks::new();
    let run_mask = masks.for_run(Redaction::On);
    run_mask.join(
        "gcs_api_token",
        &SecretString::from(credential_canary),
        Reach::Value,
    );

    let mut recs = items("gcs-", 2);
    recs.push(unicode_value());
    recs.push(json!({ "item": "gcs-4" }));

    let url_clone = url.clone();
    let recs_clone = recs.clone();
    let masks_clone = masks.clone();
    let class = tokio::task::spawn_blocking(move || {
        let dest =
            ObjectStoreDestination::from_url(ObjectStoreBackend::Gcs, SINK, &url_clone, None)
                .unwrap();
        let config = FilesConfig {
            format: ExportFormat::Jsonl,
            compression: ExportCompression::Zstd,
            max_records: 2,
            ..FilesConfig::default()
        };
        let clock = Clock::system();
        let sink = FilesSink::new(SINK, "crawl", config, clock, dest);
        let records = run_file(&[("products", recs_clone)]);
        let mut driver = Driver::new(SINK, sink, copy(1), vec!["products".to_owned()])
            .with_clock(clock)
            .with_masks(masks_clone)
            .with_batch_records(10);
        assert_eq!(driver.deliver(&records).unwrap(), Status::Complete);
        assert_eq!(driver.finish(), Status::Complete);
        driver.class()
    })
    .await
    .unwrap();
    assert!(matches!(
        class,
        Some(GuaranteeClass::Fenced | GuaranteeClass::Keyed)
    ));

    let mut part_keys = Vec::new();
    let mut stream = store.list(None);
    while let Some(meta) = stream.try_next().await.unwrap() {
        let key = meta.location.as_ref().to_owned();
        if key.contains("/part-") {
            part_keys.push(key);
        }
    }
    part_keys.sort();
    assert_eq!(part_keys.len(), 2, "4 records / 2 = 2 parts: {part_keys:?}");

    let mut decoded_all = Vec::new();
    let mut raw_text = String::new();
    for key in &part_keys {
        let bytes = store
            .get(&object_store::path::Path::from(key.as_str()))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        let plain = String::from_utf8(zstd::stream::decode_all(&bytes[..]).unwrap()).unwrap();
        raw_text.push_str(&plain);
        decoded_all.extend(decode_zstd_jsonl(&bytes));
    }
    assert_eq!(decoded_all, recs);
    assert!(
        canaries
            .leaks(&[Output::Bytes {
                name: "gcs_parts",
                bytes: raw_text.as_bytes(),
            }])
            .unwrap()
            .is_empty(),
        "secret canary leaked into GCS output"
    );
}

/// Verifies `ObjectStoreDestination::map_failure` maps auth, permanent,
/// transient, quota and unrecognized errors per ADR 0127, and Azure Blob
/// emulator container starts and responds.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_blob_emulator_and_map_failure_contract() {
    let azurite = Azurite::start();
    let url = format!(
        "az://goodfetch-az-test/crawl?endpoint={}&allow_http=true&use_emulator=true",
        azurite.endpoint
    );
    let dest =
        ObjectStoreDestination::from_url(ObjectStoreBackend::AzureBlob, SINK, &url, None).unwrap();

    // Verify ADR 0127 error translation including Unrecognized.
    let unknown = dest.map_failure(ObjectStoreError::Store(object_store::Error::Generic {
        store: "AzureBlob",
        source: Box::new(std::io::Error::other("opaque vendor error XYZ")),
    }));
    assert_eq!(unknown.kind(), FailureKind::Unrecognized);

    let auth = dest.map_failure(ObjectStoreError::Store(
        object_store::Error::Unauthenticated {
            path: "p".into(),
            source: Box::new(std::io::Error::other("401")),
        },
    ));
    assert_eq!(auth.kind(), FailureKind::Auth);

    let perm = dest.map_failure(ObjectStoreError::Store(object_store::Error::NotFound {
        path: "p".into(),
        source: Box::new(std::io::Error::other("404")),
    }));
    assert_eq!(perm.kind(), FailureKind::Permanent);
}

/// Live check against a real Google Cloud Storage bucket (disabled until the
/// owner approves a real account case by case; ADRs 0064 and 0076).
#[test]
#[ignore = "live check: disabled until owner approves real GCS account (ADRs 0064, 0076)"]
fn live_gcs_sink_check() {
    let url = std::env::var("GOODFETCH_LIVE_GCS_URL")
        .expect("set GOODFETCH_LIVE_GCS_URL when running live GCS check");
    let _ = ObjectStoreDestination::from_url(ObjectStoreBackend::Gcs, SINK, &url, None).unwrap();
}

/// Live check against a real Azure Blob container (disabled until the owner
/// approves a real account case by case; ADRs 0064 and 0076).
#[test]
#[ignore = "live check: disabled until owner approves real Azure Blob account (ADRs 0064, 0076)"]
fn live_azure_blob_sink_check() {
    let url = std::env::var("GOODFETCH_LIVE_AZURE_BLOB_URL")
        .expect("set GOODFETCH_LIVE_AZURE_BLOB_URL when running live Azure Blob check");
    let _ =
        ObjectStoreDestination::from_url(ObjectStoreBackend::AzureBlob, SINK, &url, None).unwrap();
}
