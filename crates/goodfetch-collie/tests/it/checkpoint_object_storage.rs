//! Object-storage Checkpoints on Collie's executable (issue 3472, ADR 0094):
//! `s3://` (RustFS), `gs://` (fake-gcs-server), and `az://` (Azurite), plus
//! secret-reference canary masking (ADR 0067), unresolved credential refusal
//! before any request is sent, and failed/cut-off upload leaving no object.
//!
//! Linux-only, driven through Docker or Podman using the digest-pinned images
//! of `ci/object-storage/images.toml`. A missing container engine or unpulled
//! image fails the test, never skips it.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests manage containers and probe TCP ports on the real clock outside async workers (ADR 0062)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "integration tests reserve free loopback ports and probe emulator listeners outside async workers (ADR 0062)"
)]

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;
use goodfetch_engine::aws_signing::{Credentials as AwsCredentials, Scope, signed_request};
use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::{Attached, HomeLayout, hello};
use goodfetch_test_support::{MESSAGE_DEADLINE, TestEnv};
use http::Method;
use secrecy::SecretString;
use serde_json::{Value, json};
use url::Url;

use crate::COLLIE;

const IMAGES_TOML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../ci/object-storage/images.toml"
));

const START_DEADLINE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(50);

/// Azurite's well-known default account and Base64 key.
const AZURITE_ACCOUNT: &str = "devstoreaccount1";
const AZURITE_KEY: &str =
    "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";

/// Finds a working container engine (`docker` or `podman`); panics if none answers.
fn container_engine() -> &'static str {
    for candidate in ["docker", "podman"] {
        if Command::new(candidate)
            .arg("info")
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return candidate;
        }
    }
    panic!(
        "no container engine (docker or podman) answers; Linux object-storage Checkpoint tests require one"
    );
}

/// Parses `[section]` `repository@digest` from `ci/object-storage/images.toml`.
fn pinned_image(section: &str) -> String {
    let header = format!("[{section}]");
    let mut in_section = false;
    let mut repo = None;
    let mut digest = None;
    for raw in IMAGES_TOML.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_section = line == header;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(val) = line.strip_prefix("repository = ") {
            repo = Some(val.trim_matches('"').to_owned());
        } else if let Some(val) = line.strip_prefix("digest = ") {
            digest = Some(val.trim_matches('"').to_owned());
        }
    }
    let repo = repo.unwrap_or_else(|| panic!("missing repository in [{section}]"));
    let digest = digest.unwrap_or_else(|| panic!("missing digest in [{section}]"));
    format!("{repo}@{digest}")
}

fn free_loopback_port() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.local_addr().unwrap().port()
}

struct Container {
    engine: &'static str,
    id: String,
    port: u16,
}

impl Container {
    fn start(
        section: &str,
        container_port: u16,
        extra_run_args: &[&str],
        cmd_args: &[&str],
    ) -> Self {
        let engine = container_engine();
        let image = pinned_image(section);
        let inspected: Output = Command::new(engine)
            .args(["image", "inspect", &image])
            .output()
            .unwrap();
        assert!(
            inspected.status.success(),
            "image {image} is not pulled; run `ci/object-storage/fetch.sh` or `just fetch`"
        );
        let port = free_loopback_port();
        let publish = format!("127.0.0.1:{port}:{container_port}");
        let mut command = Command::new(engine);
        command
            .args(["run", "--detach", "-p", &publish])
            .args(extra_run_args)
            .arg(&image)
            .args(cmd_args);
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{engine} failed to start {image}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let container = Self { engine, id, port };
        container.wait_tcp();
        container
    }

    fn wait_tcp(&self) {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let deadline = Instant::now() + START_DEADLINE;
        while Instant::now() < deadline {
            if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                return;
            }
            std::thread::sleep(POLL);
        }
        let logs = Command::new(self.engine)
            .args(["logs", &self.id])
            .output()
            .map(|o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            })
            .unwrap_or_default();
        panic!("container {} did not listen on {addr}:\n{logs}", self.id);
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new(self.engine)
            .args(["rm", "-f", &self.id])
            .output();
    }
}

/// Sends a raw HTTP/1.1 request to `127.0.0.1:port` and returns `(status_code, response_text)`.
fn raw_http(port: u16, raw_request: &str) -> (u16, String) {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(raw_request.as_bytes()).unwrap();
    stream.flush().unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    (status, response)
}

fn send_http_request(port: u16, req: &http::Request<Bytes>) -> (u16, String) {
    let uri = req.uri();
    let path_and_query = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    let mut raw = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Length: {}\r\n",
        req.method(),
        path_and_query,
        req.body().len()
    );
    for (name, value) in req.headers() {
        raw.push_str(name.as_str());
        raw.push_str(": ");
        raw.push_str(value.to_str().unwrap());
        raw.push_str("\r\n");
    }
    raw.push_str("\r\n");
    raw.push_str(&String::from_utf8_lossy(req.body()));
    raw_http(port, &raw)
}

fn create_s3_bucket(port: u16, bucket: &str, access_key: &str, secret_key: &str) {
    let url = Url::parse(&format!("http://127.0.0.1:{port}/{bucket}")).unwrap();
    let creds = AwsCredentials {
        access_key_id: access_key.to_owned(),
        secret_access_key: SecretString::from(secret_key.to_owned()),
        session_token: None,
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let req = signed_request(
            Method::PUT,
            &url,
            &[],
            Bytes::new(),
            Scope {
                region: "us-east-1",
                service: "s3",
            },
            &creds,
            SystemTime::now(),
        )
        .unwrap();
        let (status, resp) = send_http_request(port, &req);
        if (200..300).contains(&status) || status == 409 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "failed to create S3 bucket `{bucket}` (status {status}): {resp}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn create_gcs_bucket(port: u16, bucket: &str) {
    let body = format!(r#"{{"name":"{bucket}"}}"#);
    let raw = format!(
        "POST /storage/v1/b HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let (status, resp) = raw_http(port, &raw);
        if (200..300).contains(&status) || status == 409 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "failed to create GCS bucket `{bucket}` (status {status}): {resp}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn create_azure_container(port: u16, container: &str) {
    let script = format!(
        concat!(
            "import base64, email.utils, hashlib, hmac, urllib.request, urllib.error\n",
            "date = email.utils.formatdate(usegmt=True)\n",
            "ver = '2020-10-02'\n",
            "sts = f'PUT\\n\\n\\n\\n\\n\\n\\n\\n\\n\\n\\n\\nx-ms-date:{{date}}\\nx-ms-version:{{ver}}\\n/{AZURITE_ACCOUNT}/{AZURITE_ACCOUNT}/{container}\\nrestype:container'\n",
            "sig = base64.b64encode(hmac.new(base64.b64decode('{AZURITE_KEY}'), sts.encode(), hashlib.sha256).digest()).decode()\n",
            "req = urllib.request.Request('http://127.0.0.1:{port}/{AZURITE_ACCOUNT}/{container}?restype=container', method='PUT', headers={{'x-ms-date': date, 'x-ms-version': ver, 'Authorization': f'SharedKey {AZURITE_ACCOUNT}:{{sig}}', 'Content-Length': '0'}})\n",
            "try:\n",
            "  urllib.request.urlopen(req, timeout=5)\n",
            "except urllib.error.HTTPError as e:\n",
            "  if e.code != 409: raise\n",
        ),
        AZURITE_ACCOUNT = AZURITE_ACCOUNT,
        AZURITE_KEY = AZURITE_KEY,
        container = container,
        port = port,
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let out = Command::new("python3")
            .args(["-c", &script])
            .output()
            .unwrap();
        if out.status.success() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "failed to create Azure container `{container}`: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Runs a full Checkpoint-to-object-storage + delete local run folder +
/// `run.resume` round trip on Collie's executable, returning the checkpoint
/// report and checking that `canaries` appear nowhere in report or logs.
fn round_trip_object_checkpoint(
    env: &TestEnv,
    extra_env: &[(&str, &str)],
    settings: &Value,
    location: &str,
    canaries: &[&str],
) -> Value {
    let mut engine = Attached::start_with(env, COLLIE, MESSAGE_DEADLINE, |command| {
        for (k, v) in extra_env {
            command.env(k, v);
        }
    });
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    let started = engine.ask(1, "run.start", &json!({ "settings": settings }));
    let run_id = started["result"]["run_id"]
        .as_str()
        .unwrap_or_else(|| panic!("run.start failed: {started}"))
        .to_owned();

    let cp_resp = engine.ask(
        2,
        "run.checkpoint",
        &json!({ "run_id": run_id, "location": location }),
    );
    assert!(
        cp_resp.get("result").is_some(),
        "run.checkpoint to {location} failed: {cp_resp}"
    );
    let cp = cp_resp["result"].clone();
    assert_eq!(cp["location"], location);
    assert!(cp["size"].as_u64().unwrap_or(0) > 0, "{cp}");
    assert_eq!(cp["digest"].as_str().map(str::len), Some(64), "{cp}");
    assert_eq!(cp["run_epoch"].as_u64(), Some(1), "{cp}");
    assert!(cp["journal_position"].as_u64().unwrap_or(0) > 0, "{cp}");

    let stopped = engine.ask(3, "run.stop", &json!({ "run_id": run_id }));
    assert!(
        stopped.get("result").is_some(),
        "run.stop failed: {stopped}"
    );

    // Remove the local run folder so `run.resume` must restore strictly from object storage.
    let run_folder = HomeLayout::of(env.home()).run_folder(&run_id);
    if run_folder.exists() {
        std::fs::remove_dir_all(&run_folder).unwrap();
    }

    let resumed = engine.ask(
        4,
        "run.resume",
        &json!({
            "run_id": run_id,
            "checkpoint": location,
            "settings": settings,
        }),
    );
    assert!(
        resumed.get("result").is_some(),
        "run.resume from {location} failed: {resumed}"
    );
    assert_eq!(resumed["result"]["started_as"], "resumed", "{resumed}");

    let finished = engine.ask(5, "run.finish", &json!({ "run_id": run_id }));
    assert!(
        finished.get("result").is_some(),
        "run.finish failed: {finished}"
    );
    let _ = engine.finish();

    // Verify no secret canary appears in the Checkpoint report or engine logs.
    let cp_text = cp.to_string();
    for canary in canaries {
        assert!(
            !cp_text.contains(canary),
            "secret canary leaked in Checkpoint report: {cp_text}"
        );
    }
    let logs_dir = env.home().join("logs");
    if logs_dir.exists() {
        for entry in std::fs::read_dir(&logs_dir).unwrap().flatten() {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                for canary in canaries {
                    assert!(
                        !content.contains(canary),
                        "secret canary leaked in log {}: {content}",
                        entry.path().display()
                    );
                }
            }
        }
    }
    cp
}

/// Covers: story:3-31, setting:checkpoint.object_storage.s3.endpoint, setting:checkpoint.object_storage.s3.region, setting:checkpoint.object_storage.s3.access_key_id, setting:checkpoint.object_storage.s3.secret_access_key, setting:checkpoint.object_storage.s3.session_token
#[test]
fn checkpoint_and_resume_through_rustfs_s3() {
    let canary_id = "canary-rustfs-key-id-9f2a1b";
    let canary_secret = "canary-rustfs-secret-7e4c8d1a";
    let rustfs = Container::start(
        "rustfs",
        9000,
        &[
            "-e",
            &format!("RUSTFS_ACCESS_KEY={canary_id}"),
            "-e",
            &format!("RUSTFS_SECRET_KEY={canary_secret}"),
        ],
        &["/data"],
    );
    create_s3_bucket(rustfs.port, "checkpoints", canary_id, canary_secret);

    let env = TestEnv::new();
    let settings = json!({
        "checkpoint": {
            "object_storage": {
                "s3": {
                    "endpoint": format!("http://127.0.0.1:{}", rustfs.port),
                    "region": "us-east-1",
                    "access_key_id": "env:TEST_CP_S3_KEY_ID",
                    "secret_access_key": "env:TEST_CP_S3_SECRET",
                }
            }
        }
    });
    let _cp = round_trip_object_checkpoint(
        &env,
        &[
            ("TEST_CP_S3_KEY_ID", canary_id),
            ("TEST_CP_S3_SECRET", canary_secret),
        ],
        &settings,
        "s3://checkpoints/runs/r1/checkpoint.sqlite",
        &[canary_secret],
    );
}

/// Covers: story:3-31, setting:checkpoint.object_storage.gs.endpoint, setting:checkpoint.object_storage.gs.access_token
#[test]
fn checkpoint_and_resume_through_fake_gcs() {
    let canary_token = "canary-gcs-oauth-token-3c9e7f12";
    let gcs = Container::start("fake_gcs", 4443, &[], &["-scheme", "http", "-port", "4443"]);
    create_gcs_bucket(gcs.port, "checkpoints");

    let env = TestEnv::new();
    let settings = json!({
        "checkpoint": {
            "object_storage": {
                "gs": {
                    "endpoint": format!("http://127.0.0.1:{}", gcs.port),
                    "access_token": "env:TEST_CP_GCS_TOKEN",
                }
            }
        }
    });
    let _cp = round_trip_object_checkpoint(
        &env,
        &[("TEST_CP_GCS_TOKEN", canary_token)],
        &settings,
        "gs://checkpoints/runs/r1/checkpoint.sqlite",
        &[canary_token],
    );
}

/// Covers: story:3-31, setting:checkpoint.object_storage.az.endpoint, setting:checkpoint.object_storage.az.account, setting:checkpoint.object_storage.az.account_key, setting:checkpoint.object_storage.az.sas_token
#[test]
fn checkpoint_and_resume_through_azurite_blob() {
    let azurite = Container::start(
        "azurite",
        10000,
        &[],
        &[
            "azurite-blob",
            "--blobHost",
            "0.0.0.0",
            "--blobPort",
            "10000",
            "--loose",
        ],
    );
    create_azure_container(azurite.port, "checkpoints");

    let env = TestEnv::new();
    let settings = json!({
        "checkpoint": {
            "object_storage": {
                "az": {
                    "endpoint": format!("http://127.0.0.1:{}/{AZURITE_ACCOUNT}", azurite.port),
                    "account": AZURITE_ACCOUNT,
                    "account_key": "env:TEST_CP_AZ_KEY",
                }
            }
        }
    });
    let _cp = round_trip_object_checkpoint(
        &env,
        &[("TEST_CP_AZ_KEY", AZURITE_KEY)],
        &settings,
        "az://checkpoints/runs/r1/checkpoint.sqlite",
        &[AZURITE_KEY],
    );
}

/// Covers: story:3-31
#[test]
fn unresolved_credential_fails_before_sending_any_request() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_bg = Arc::clone(&hits);
    let stop = Arc::new(AtomicUsize::new(0));
    let stop_bg = Arc::clone(&stop);
    let server = std::thread::spawn(move || {
        while stop_bg.load(Ordering::Relaxed) == 0 {
            if let Ok((stream, _)) = listener.accept() {
                hits_bg.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });

    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    let started = engine.ask(
        1,
        "run.start",
        &json!({
            "settings": {
                "checkpoint": {
                    "object_storage": {
                        "gs": {
                            "endpoint": format!("http://127.0.0.1:{port}"),
                            "access_token": "env:MISSING_CHECKPOINT_SECRET_FOR_TEST",
                        }
                    }
                }
            }
        }),
    );
    let run_id = started["result"]["run_id"].as_str().unwrap().to_owned();
    let failed = engine.ask(
        2,
        "run.checkpoint",
        &json!({
            "run_id": run_id,
            "location": "gs://checkpoints/never-sent.sqlite",
        }),
    );
    assert_eq!(
        failed["error"]["data"]["code"], "file_write_failed",
        "{failed}"
    );
    let _ = engine.ask(3, "run.finish", &json!({ "run_id": run_id }));
    let _ = engine.finish();

    stop.store(1, Ordering::Relaxed);
    let _ = server.join();
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "no HTTP request may be sent when credential resolution fails"
    );
}

/// Covers: story:3-31
#[test]
fn failed_upload_returns_file_write_failed_and_leaves_no_object() {
    let gcs = Container::start("fake_gcs", 4443, &[], &["-scheme", "http", "-port", "4443"]);
    // Intentionally do NOT create bucket `missing-bucket` so upload fails.
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    let started = engine.ask(
        1,
        "run.start",
        &json!({
            "settings": {
                "checkpoint": {
                    "object_storage": {
                        "gs": {
                            "endpoint": format!("http://127.0.0.1:{}", gcs.port),
                        }
                    }
                }
            }
        }),
    );
    let run_id = started["result"]["run_id"].as_str().unwrap().to_owned();
    let failed = engine.ask(
        2,
        "run.checkpoint",
        &json!({
            "run_id": run_id,
            "location": "gs://missing-bucket/runs/r1/checkpoint.sqlite",
        }),
    );
    assert_eq!(
        failed["error"]["data"]["code"], "file_write_failed",
        "{failed}"
    );
    let (status, _) = raw_http(
        gcs.port,
        &format!(
            "GET /storage/v1/b/missing-bucket/o/runs%2Fr1%2Fcheckpoint.sqlite?alt=media HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            gcs.port
        ),
    );
    assert_eq!(status, 404, "no object may exist after a failed upload");
    let _ = engine.ask(3, "run.finish", &json!({ "run_id": run_id }));
    let _ = engine.finish();
}
