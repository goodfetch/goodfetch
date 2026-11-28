//! Shared container lifecycle guard, Unicode fixture and zstd-JSONL decoder
//! helpers for Sink conformance integration tests (ADR 0026; issues 1152,
//! 1156, 1159, 1162 and 1163).
#![expect(
    clippy::disallowed_methods,
    reason = "integration test container harness runs docker/podman and binds ephemeral loopback ports"
)]
#![expect(
    clippy::disallowed_types,
    reason = "integration test binds ephemeral loopback TCP port"
)]

use std::io::Read as _;
use std::net::TcpListener;
use std::process::Command;

use goodfetch_test_support::unicode_record;
use serde_json::Value;

/// Builds a URL with RFC 3986 percent-encoded `user` and `pass` credentials
/// so Secret canaries containing reserved URL characters (`:`, `@`, `/`, `+`)
/// parse cleanly.
pub(crate) fn url_with_credentials(base_url: &str, user: &str, pass: &str) -> String {
    let mut parsed = url::Url::parse(base_url).expect("valid base URL");
    parsed.set_username(user).expect("set username");
    parsed.set_password(Some(pass)).expect("set password");
    parsed.into()
}

/// Builds the ADR 0067 Unicode test record as a JSON object.
pub(crate) fn unicode_value() -> Value {
    unicode_record()
        .fields
        .iter()
        .map(|field| (field.name.clone(), Value::String(field.text.clone())))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// Decodes a zstd-compressed JSONL Export part into JSON values.
pub(crate) fn decode_zstd_jsonl(bytes: &[u8]) -> Vec<Value> {
    let mut decoder = zstd::stream::read::Decoder::new(bytes).unwrap();
    let mut text = String::new();
    decoder.read_to_string(&mut text).unwrap();
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Allocates a single free ephemeral loopback TCP port (for single-port
/// passive FTP containers without hardcoded port collisions).
#[cfg(target_os = "linux")]
pub(crate) fn allocate_ephemeral_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Detached container handle that removes the container on drop, constructed
/// immediately after `run --detach` so port-lookup or readiness panics never
/// leak containers.
#[cfg(target_os = "linux")]
pub(crate) struct ContainerGuard {
    pub(crate) engine: &'static str,
    pub(crate) id: String,
}

#[cfg(target_os = "linux")]
impl ContainerGuard {
    /// Starts a pinned container from `image_toml` with `run_args` and
    /// optional trailing `cmd_args`, returning the RAII guard immediately.
    pub(crate) fn spawn(image_toml: &str, run_args: &[&str], cmd_args: &[&str]) -> Self {
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
        cmd.args(run_args);
        cmd.arg(&image);
        cmd.args(cmd_args);
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{engine} failed to start {image}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
        let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Self { engine, id }
    }

    /// Queries the published host port for `container_port` (e.g. `"22/tcp"`).
    pub(crate) fn published_port(&self, container_port: &str) -> u16 {
        let port_out = Command::new(self.engine)
            .args(["port", &self.id, container_port])
            .output()
            .unwrap();
        assert!(
            port_out.status.success(),
            "cannot read published port {container_port}: {}",
            String::from_utf8_lossy(&port_out.stderr).trim()
        );
        String::from_utf8_lossy(&port_out.stdout)
            .lines()
            .next()
            .unwrap()
            .rsplit(':')
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap()
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
