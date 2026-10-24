//! A local HTTPS stand-in for the Clean-up rules' sources: the daily
//! channel's OCI registry and the Filter lists' authors. It answers fixed
//! files by path, on loopback, under a certificate its own authority signs
//! for every host it stands in for, and records the path of each request;
//! a path it stalls on is read and never answered while the test lasts.
//! Its listener runs on a thread of the test, outside any engine.

use std::collections::BTreeMap;
use std::io::{BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};

use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// The daily channel's repository on the stand-in registry.
const REPOSITORY: &str = "goodfetch/clean-up-rules";

/// How long a stalled answer waits: longer than any test that stalls.
const STALL: std::time::Duration = std::time::Duration::from_secs(600);

/// The answers by path, the paths it stalls on, and the paths asked for,
/// in order.
#[derive(Default)]
struct Served {
    files: BTreeMap<String, Vec<u8>>,
    stalled: Vec<String>,
    asked: Vec<String>,
}

/// The stand-in: its port, its authority's certificate as PEM, and what it
/// serves.
pub(crate) struct StandIn {
    port: u16,
    authority: String,
    served: Arc<Mutex<Served>>,
}

impl StandIn {
    /// Starts it for `hosts`, which the test's `network.resolve` maps to
    /// loopback.
    pub(crate) fn start(hosts: &[&str]) -> Self {
        let authority_key = rcgen::KeyPair::generate().unwrap();
        let mut authority_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        authority_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let authority = authority_params.self_signed(&authority_key).unwrap();
        let issuer = rcgen::Issuer::new(authority_params, authority_key);
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf = rcgen::CertificateParams::new(
            hosts
                .iter()
                .map(|host| (*host).to_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap()
        .signed_by(&leaf_key, &issuer)
        .unwrap();
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
            )
            .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let served = Arc::new(Mutex::new(Served::default()));
        let serving = Arc::clone(&served);
        std::thread::spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let (config, serving) = (Arc::clone(&config), Arc::clone(&serving));
                std::thread::spawn(move || answer(stream, &config, &serving));
            }
        });
        Self {
            port,
            authority: authority.pem(),
            served,
        }
    }

    /// `host` with the stand-in's port.
    pub(crate) fn origin(&self, host: &str) -> String {
        format!("{host}:{}", self.port)
    }

    /// Writes its authority's certificate to `file`, for `tls.trusted_cas`.
    pub(crate) fn write_authority(&self, file: &Path) {
        std::fs::write(file, &self.authority).unwrap();
    }

    /// Serves `body` at `path` from now on.
    pub(crate) fn serve(&self, path: &str, body: Vec<u8>) {
        self.served
            .lock()
            .unwrap()
            .files
            .insert(path.to_owned(), body);
    }

    /// What it serves at `path`, if anything.
    pub(crate) fn served(&self, path: &str) -> Option<Vec<u8>> {
        self.served.lock().unwrap().files.get(path).cloned()
    }

    /// Reads each request for `path` from now on, and never answers it.
    pub(crate) fn stall(&self, path: &str) {
        self.served.lock().unwrap().stalled.push(path.to_owned());
    }

    /// The paths asked for so far, in order.
    pub(crate) fn asked(&self) -> Vec<String> {
        self.served.lock().unwrap().asked.clone()
    }

    /// Publishes the version `files` hold on the daily channel: each file a
    /// layer named by its title, its manifest under the tag `tag`, and
    /// under the version's own digest's tag, `sha256-<hex>`.
    pub(crate) fn publish(&self, tag: &str, digest: &str, files: &[(&str, Vec<u8>)]) {
        let layers: Vec<Value> = files
            .iter()
            .map(|(name, bytes)| {
                let blob = format!("sha256:{:x}", Sha256::digest(bytes));
                self.serve(&format!("/v2/{REPOSITORY}/blobs/{blob}"), bytes.clone());
                json!({
                    "mediaType": "application/octet-stream",
                    "digest": blob,
                    "size": bytes.len(),
                    "annotations": { "org.opencontainers.image.title": name },
                })
            })
            .collect();
        let manifest = json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "artifactType": "application/vnd.goodfetch.clean-up-rules",
            "config": {
                "mediaType": "application/vnd.oci.empty.v1+json",
                "digest": "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
                "size": 2,
            },
            "layers": layers,
        })
        .to_string()
        .into_bytes();
        self.serve(&manifest_path(tag), manifest.clone());
        self.serve(&manifest_path(&digest.replacen(':', "-", 1)), manifest);
    }
}

/// The path of the channel's manifest under `reference`.
pub(crate) fn manifest_path(reference: &str) -> String {
    format!("/v2/{REPOSITORY}/manifests/{reference}")
}

/// The daily channel's repository on the stand-in at `origin`.
pub(crate) fn channel(origin: &str) -> String {
    format!("{origin}/{REPOSITORY}")
}

/// Answers one connection: one request, then the connection closes.
fn answer(stream: TcpStream, config: &Arc<rustls::ServerConfig>, served: &Mutex<Served>) {
    let Ok(connection) = rustls::ServerConnection::new(Arc::clone(config)) else {
        return;
    };
    let mut tls = rustls::StreamOwned::new(connection, stream);
    let Some(path) = requested_path(&mut BufReader::new(&mut tls)) else {
        return;
    };
    let (body, stalled) = {
        let mut served = served.lock().unwrap();
        served.asked.push(path.clone());
        (
            served.files.get(&path).cloned(),
            served.stalled.contains(&path),
        )
    };
    if stalled {
        std::thread::sleep(STALL);
        return;
    }
    let (status, body) = match body {
        Some(body) => ("200 OK", body),
        None => ("404 Not Found", Vec::new()),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = tls.write_all(head.as_bytes());
    let _ = tls.write_all(&body);
    let _ = tls.flush();
    tls.conn.send_close_notify();
    let _ = tls.flush();
}

/// The path of the request `reader` reads, once its head is read whole.
fn requested_path(reader: &mut impl std::io::BufRead) -> Option<String> {
    let mut request_line = String::new();
    reader.read_line(&mut request_line).ok()?;
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) | Err(_) => return None,
            Ok(_) if header.trim().is_empty() => break,
            Ok(_) => {}
        }
    }
    request_line.split_whitespace().nth(1).map(str::to_owned)
}
