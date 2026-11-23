//! A Valkey server for a test of the Politeness store (ADRs 0053 and 0059,
//! Linux): the image `ci/valkey/image.toml` pins, run through Docker or
//! Podman with no network, listening only on a Unix socket in the test's
//! folder (ADR 0059's isolation: no fixed port). Each test gets its own
//! server, so its namespace is its own too. `ci/valkey/fetch.sh` (`just
//! fetch`) pulls the image before ADR 0043's deny; a missing image or engine
//! fails the test with the command that fixes it, never skips it. The
//! container is removed when the value drops.
#![expect(
    clippy::disallowed_methods,
    reason = "test support runs the container engine and polls for the socket on the real clock, outside any async worker and any simulated clock (ADR 0062)"
)]

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use crate::env::TestEnv;

/// The pin file of the image.
const IMAGE_FILE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../ci/valkey/image.toml"
));

/// How long the server may take to listen.
const START_DEADLINE: Duration = Duration::from_secs(20);

/// How often the socket is looked for.
const POLL: Duration = Duration::from_millis(20);

/// A Valkey server in a container, until the value drops.
pub struct Valkey {
    engine: &'static str,
    container: String,
    socket: PathBuf,
}

impl Valkey {
    /// Starts a server whose socket is in `env`'s folder.
    ///
    /// # Panics
    ///
    /// If no container engine answers, the image is missing, or the server
    /// does not listen within its deadline.
    #[must_use]
    pub fn start(env: &TestEnv) -> Self {
        let engine = engine();
        let image = image();
        let folder = env.folder().join("valkey");
        std::fs::create_dir_all(&folder).expect("the socket's folder is made");
        // The server runs as the container's root, so that it may create its
        // socket in a folder the test's user owns, under rootless Podman
        // and Docker alike; the socket is open to every user of the host,
        // as the folder is the test's own.
        let mut permissions = std::fs::metadata(&folder)
            .expect("the socket's folder exists")
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o777);
        std::fs::set_permissions(&folder, permissions).expect("the socket's folder opens");
        let output = run(Command::new(engine)
            .args([
                "run",
                "--detach",
                "--rm",
                "--network",
                "none",
                "--user",
                "0:0",
            ])
            .args(["--entrypoint", "valkey-server", "--volume"])
            .arg(format!("{}:/run/valkey", folder.display()))
            .arg(&image)
            .args(["--port", "0", "--unixsocket", "/run/valkey/valkey.sock"])
            .args([
                "--unixsocketperm",
                "777",
                "--save",
                "",
                "--appendonly",
                "no",
            ]));
        assert!(
            output.status.success(),
            "{engine} cannot start {image}: {}; `ci/valkey/fetch.sh` pulls it",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let valkey = Self {
            engine,
            container: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            socket: folder.join("valkey.sock"),
        };
        valkey.wait_listening();
        valkey
    }

    /// The URL the `redis` client connects to.
    #[must_use]
    pub fn url(&self) -> String {
        format!("redis+unix://{}", self.socket.display())
    }

    /// The server's `INFO` of `section`, read with `valkey-cli` in its
    /// container, so a test checks what the server saw.
    ///
    /// # Panics
    ///
    /// If `valkey-cli` fails.
    #[must_use]
    pub fn info(&self, section: &str) -> String {
        let output = run(Command::new(self.engine)
            .args(["exec", &self.container, "valkey-cli", "-s"])
            .args(["/run/valkey/valkey.sock", "INFO", section]));
        assert!(
            output.status.success(),
            "valkey-cli INFO {section}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        String::from_utf8_lossy(&output.stdout).replace('\r', "")
    }

    fn wait_listening(&self) {
        let started = Instant::now();
        while !self.socket.exists() {
            assert!(
                started.elapsed() < START_DEADLINE,
                "Valkey did not listen within {START_DEADLINE:?}: {}",
                self.logs()
            );
            std::thread::sleep(POLL);
        }
    }

    fn logs(&self) -> String {
        let output = run(Command::new(self.engine).args(["logs", &self.container]));
        String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr)
    }
}

impl Drop for Valkey {
    fn drop(&mut self) {
        // Removing it stops it; a failure leaves a container `--rm` removes
        // once it stops, and fails no test.
        let _removed = Command::new(self.engine)
            .args(["rm", "--force", &self.container])
            .output();
    }
}

/// The container engine that answers on this host, Docker or Podman, as
/// `ci/container.sh` picks it.
fn engine() -> &'static str {
    ["docker", "podman"]
        .into_iter()
        .find(|engine| {
            Command::new(engine)
                .arg("info")
                .output()
                .is_ok_and(|output| output.status.success())
        })
        .expect("Docker or Podman answers, to run Valkey")
}

/// `repository@digest` from the pin file.
fn image() -> String {
    let pin: toml::Table = toml::from_str(IMAGE_FILE).expect("ci/valkey/image.toml is TOML");
    let field = |name: &str| {
        pin.get(name)
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("ci/valkey/image.toml names its {name}"))
            .to_owned()
    };
    format!("{}@{}", field("repository"), field("digest"))
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the container engine starts")
}
