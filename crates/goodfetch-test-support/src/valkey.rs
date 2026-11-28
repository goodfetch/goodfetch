//! A Valkey server (or 3-node Cluster) for a test of the Politeness store
//! (ADRs 0053 and 0059): on Linux, the image `ci/valkey/image.toml` pins,
//! run through Docker or Podman with no network for single-node tests
//! (Unix socket in the test's folder) and loopback host ports for Cluster
//! tests; on macOS, `valkey-server` from Homebrew on a Unix socket.
//! `ci/valkey/fetch.sh` (`just fetch`) pulls the image before ADR 0043's
//! deny; a missing image or engine fails the test with the command that
//! fixes it, never skips it. The container or process is removed when the
//! value drops.
#![expect(
    clippy::disallowed_methods,
    reason = "test support runs the container engine or local valkey-server and polls for the socket on the real clock, outside any async worker and any simulated clock (ADR 0062)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "test support reserves free loopback ports and probes Valkey's TCP listener outside any async worker or Simulation (ADR 0062)"
)]

use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use crate::env::TestEnv;

/// The pin file of the image (Linux).
#[cfg(target_os = "linux")]
const IMAGE_FILE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../ci/valkey/image.toml"
));

/// How long the server may take to listen.
const START_DEADLINE: Duration = Duration::from_secs(20);

/// How often the socket or cluster readiness is polled.
const POLL: Duration = Duration::from_millis(20);

/// How a [`Valkey`] test instance is reached.
enum ValkeyEndpoint {
    Unix(PathBuf),
    Tcp(SocketAddr),
}

/// A Valkey server (container on Linux, Homebrew process on macOS), until
/// the value drops.
pub struct Valkey {
    #[cfg(target_os = "linux")]
    engine: &'static str,
    #[cfg(target_os = "linux")]
    container: String,
    #[cfg(target_os = "macos")]
    child: Option<std::process::Child>,
    endpoint: ValkeyEndpoint,
}

impl Valkey {
    /// Starts a server whose socket is in `env`'s folder.
    ///
    /// # Panics
    ///
    /// If no container engine or Homebrew `valkey-server` answers, the image
    /// is missing, or the server does not listen within its deadline.
    #[must_use]
    pub fn start(env: &TestEnv) -> Self {
        let folder = env.folder().join("valkey");
        std::fs::create_dir_all(&folder).expect("the socket's folder is made");

        #[cfg(target_os = "linux")]
        let valkey = {
            let engine = engine();
            let image = image();
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
                .args(["run", "--detach", "--network", "none", "--user", "0:0"])
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
            Self {
                engine,
                container: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                endpoint: ValkeyEndpoint::Unix(folder.join("valkey.sock")),
            }
        };

        #[cfg(target_os = "macos")]
        let valkey = {
            let socket = folder.join("valkey.sock");
            let child = spawn_macos_unix(&socket);
            Self {
                child: Some(child),
                endpoint: ValkeyEndpoint::Unix(socket),
            }
        };

        valkey.wait_listening();
        valkey
    }

    /// Starts a restartable Valkey server listening on a free loopback TCP
    /// port (`127.0.0.1:<port>`), suitable for Toxiproxy routing and mid-run
    /// container stop/restart outage tests (ADRs 0053 and 0060).
    ///
    /// # Panics
    ///
    /// If no container engine or Homebrew `valkey-server` answers, the image
    /// is missing, or the server does not listen within its deadline.
    #[must_use]
    pub fn start_tcp(_env: &TestEnv) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .expect("a free loopback TCP port is available for Valkey");
        let addr = listener.local_addr().expect("bound listener has addr");
        drop(listener);
        let port = addr.port().to_string();

        #[cfg(target_os = "linux")]
        let valkey = {
            let engine = engine();
            let image = image();
            let output = run(Command::new(engine)
                .args([
                    "run",
                    "--detach",
                    "--network",
                    "host",
                    "--entrypoint",
                    "valkey-server",
                ])
                .arg(&image)
                .args([
                    "--bind",
                    "127.0.0.1",
                    "--port",
                    &port,
                    "--save",
                    "",
                    "--appendonly",
                    "no",
                ]));
            assert!(
                output.status.success(),
                "{engine} cannot start TCP {image}: {}; `ci/valkey/fetch.sh` pulls it",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            Self {
                engine,
                container: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
                endpoint: ValkeyEndpoint::Tcp(addr),
            }
        };

        #[cfg(target_os = "macos")]
        let valkey = {
            let child = spawn_macos_tcp(&port);
            Self {
                child: Some(child),
                endpoint: ValkeyEndpoint::Tcp(addr),
            }
        };

        valkey.wait_listening();
        valkey
    }

    /// The URL the `redis` client connects to.
    #[must_use]
    pub fn url(&self) -> String {
        match &self.endpoint {
            ValkeyEndpoint::Unix(socket) => format!("redis+unix://{}", socket.display()),
            ValkeyEndpoint::Tcp(addr) => format!("redis://{addr}"),
        }
    }

    /// The loopback TCP address when started via [`Valkey::start_tcp`].
    #[must_use]
    pub fn addr(&self) -> Option<SocketAddr> {
        match &self.endpoint {
            ValkeyEndpoint::Unix(_) => None,
            ValkeyEndpoint::Tcp(addr) => Some(*addr),
        }
    }

    /// Stops the Valkey container/process mid-run without removing it, so
    /// [`Valkey::restart`] can start it again at the same endpoint (ADR 0060).
    ///
    /// # Panics
    ///
    /// If stopping the container fails.
    pub fn stop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let output = run(Command::new(self.engine).args(["stop", "-t", "1", &self.container]));
            assert!(
                output.status.success(),
                "{} stop failed: {}",
                self.engine,
                String::from_utf8_lossy(&output.stderr).trim()
            );
            if let ValkeyEndpoint::Unix(socket) = &self.endpoint {
                let _ = std::fs::remove_file(socket);
            }
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            if let ValkeyEndpoint::Unix(socket) = &self.endpoint {
                let _ = std::fs::remove_file(socket);
            }
        }
    }

    /// Starts a previously stopped Valkey container/process again and waits
    /// until it is listening (ADR 0060).
    ///
    /// # Panics
    ///
    /// If starting fails or Valkey does not listen within its deadline.
    pub fn restart(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let output = run(Command::new(self.engine).args(["start", &self.container]));
            assert!(
                output.status.success(),
                "{} start failed: {}",
                self.engine,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        #[cfg(target_os = "macos")]
        {
            let child = match &self.endpoint {
                ValkeyEndpoint::Unix(socket) => spawn_macos_unix(socket),
                ValkeyEndpoint::Tcp(addr) => spawn_macos_tcp(&addr.port().to_string()),
            };
            self.child = Some(child);
        }
        self.wait_listening();
    }

    /// The server's `INFO` of `section`, read with `valkey-cli`, so a test
    /// checks what the server saw.
    ///
    /// # Panics
    ///
    /// If `valkey-cli` fails.
    #[must_use]
    pub fn info(&self, section: &str) -> String {
        #[cfg(target_os = "linux")]
        let output = match &self.endpoint {
            ValkeyEndpoint::Unix(_) => run(Command::new(self.engine)
                .args(["exec", &self.container, "valkey-cli", "-s"])
                .args(["/run/valkey/valkey.sock", "INFO", section])),
            ValkeyEndpoint::Tcp(addr) => run(Command::new(self.engine).args([
                "exec",
                &self.container,
                "valkey-cli",
                "-h",
                "127.0.0.1",
                "-p",
                &addr.port().to_string(),
                "INFO",
                section,
            ])),
        };
        #[cfg(target_os = "macos")]
        let output = match &self.endpoint {
            ValkeyEndpoint::Unix(socket) => run(Command::new("valkey-cli")
                .arg("-s")
                .arg(socket)
                .args(["INFO", section])),
            ValkeyEndpoint::Tcp(addr) => run(Command::new("valkey-cli").args([
                "-h",
                "127.0.0.1",
                "-p",
                &addr.port().to_string(),
                "INFO",
                section,
            ])),
        };
        assert!(
            output.status.success(),
            "valkey-cli INFO {section}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        String::from_utf8_lossy(&output.stdout).replace('\r', "")
    }

    fn wait_listening(&self) {
        let started = Instant::now();
        loop {
            let ready = match &self.endpoint {
                ValkeyEndpoint::Unix(socket) => socket.exists(),
                ValkeyEndpoint::Tcp(addr) => {
                    TcpStream::connect_timeout(addr, Duration::from_millis(100)).is_ok()
                }
            };
            if ready {
                return;
            }
            assert!(
                started.elapsed() < START_DEADLINE,
                "Valkey did not listen within {START_DEADLINE:?}: {}",
                self.logs()
            );
            std::thread::sleep(POLL);
        }
    }

    fn logs(&self) -> String {
        #[cfg(target_os = "linux")]
        {
            let output = run(Command::new(self.engine).args(["logs", &self.container]));
            String::from_utf8_lossy(&output.stdout).into_owned()
                + &String::from_utf8_lossy(&output.stderr)
        }
        #[cfg(target_os = "macos")]
        {
            String::from("(valkey-server process)")
        }
    }
}

#[cfg(target_os = "macos")]
fn spawn_macos_unix(socket: &std::path::Path) -> std::process::Child {
    Command::new("valkey-server")
        .args(["--port", "0", "--unixsocket"])
        .arg(socket)
        .args([
            "--unixsocketperm",
            "700",
            "--save",
            "",
            "--appendonly",
            "no",
        ])
        .spawn()
        .expect("Homebrew `valkey-server` starts (`brew install valkey`)")
}

#[cfg(target_os = "macos")]
fn spawn_macos_tcp(port: &str) -> std::process::Child {
    Command::new("valkey-server")
        .args([
            "--bind",
            "127.0.0.1",
            "--port",
            port,
            "--save",
            "",
            "--appendonly",
            "no",
        ])
        .spawn()
        .expect("Homebrew `valkey-server` starts (`brew install valkey`)")
}

impl Drop for Valkey {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let _removed = Command::new(self.engine)
                .args(["rm", "--force", &self.container])
                .output();
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// A 3-primary Valkey Cluster in a container on loopback ports (Linux, ADR
/// 0059): every primary binds `127.0.0.1` on an ephemeral port pair
/// `(port, port + 10000)` and is removed when dropped.
#[cfg(target_os = "linux")]
pub struct ValkeyCluster {
    engine: &'static str,
    container: String,
    ports: [u16; 3],
}

#[cfg(target_os = "linux")]
impl ValkeyCluster {
    /// Starts a 3-node Cluster on free loopback ports.
    ///
    /// # Panics
    ///
    /// If no container engine answers, the image is missing, or the Cluster
    /// does not reach `cluster_state:ok` within its deadline.
    #[must_use]
    pub fn start(_env: &TestEnv) -> Self {
        let engine = engine();
        let image = image();
        let ports = pick_cluster_ports();
        let [p0, p1, p2] = ports;
        let script = format!(
            concat!(
                "set -e; ",
                "for p in {p0} {p1} {p2}; do ",
                "  mkdir -p /tmp/node-$p; ",
                "  valkey-server --dir /tmp/node-$p --bind 127.0.0.1 --port $p ",
                "    --cluster-enabled yes --cluster-config-file nodes.conf ",
                "    --cluster-node-timeout 2000 ",
                "    --cluster-announce-ip 127.0.0.1 --cluster-announce-port $p ",
                "    --cluster-announce-bus-port $((p + 10000)) ",
                "    --save '' --appendonly no --daemonize yes; ",
                "done; ",
                "valkey-cli --cluster create 127.0.0.1:{p0} 127.0.0.1:{p1} 127.0.0.1:{p2} ",
                "  --cluster-replicas 0 --cluster-yes; ",
                "exec tail -f /dev/null"
            ),
            p0 = p0,
            p1 = p1,
            p2 = p2,
        );
        let output = run(Command::new(engine)
            .args([
                "run",
                "--detach",
                "--rm",
                "--network",
                "host",
                "--entrypoint",
                "sh",
            ])
            .arg(&image)
            .args(["-c", &script]));
        assert!(
            output.status.success(),
            "{engine} cannot start Cluster {image}: {}; `ci/valkey/fetch.sh` pulls it",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let cluster = Self {
            engine,
            container: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            ports,
        };
        cluster.wait_ready();
        cluster
    }

    /// The `redis://127.0.0.1:<port>` seed URLs of all three primaries.
    #[must_use]
    pub fn urls(&self) -> Vec<String> {
        self.ports
            .iter()
            .map(|port| format!("redis://127.0.0.1:{port}"))
            .collect()
    }

    fn wait_ready(&self) {
        let started = Instant::now();
        let port = self.ports[0].to_string();
        while started.elapsed() < START_DEADLINE {
            let output = run(Command::new(self.engine).args([
                "exec",
                &self.container,
                "valkey-cli",
                "-h",
                "127.0.0.1",
                "-p",
                &port,
                "CLUSTER",
                "INFO",
            ]));
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("cluster_state:ok")
            {
                return;
            }
            std::thread::sleep(POLL);
        }
        let logs = run(Command::new(self.engine).args(["logs", &self.container]));
        panic!(
            "Valkey Cluster did not reach cluster_state:ok within {START_DEADLINE:?}: {}{}",
            String::from_utf8_lossy(&logs.stdout),
            String::from_utf8_lossy(&logs.stderr)
        );
    }
}

#[cfg(target_os = "linux")]
impl Drop for ValkeyCluster {
    fn drop(&mut self) {
        let _removed = Command::new(self.engine)
            .args(["rm", "--force", &self.container])
            .output();
    }
}

/// Picks three distinct loopback ports whose cluster bus ports (`port + 10000`)
/// are also unbound.
#[cfg(target_os = "linux")]
fn pick_cluster_ports() -> [u16; 3] {
    use std::net::{Ipv4Addr, TcpListener};

    let mut chosen = Vec::with_capacity(3);
    let mut held = Vec::with_capacity(6);
    for _ in 0..200 {
        if chosen.len() == 3 {
            break;
        }
        let Ok(primary) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else {
            continue;
        };
        let port = primary
            .local_addr()
            .expect("bound listener has addr")
            .port();
        if port > 55_000 || chosen.contains(&port) {
            continue;
        }
        if let Ok(bus) = TcpListener::bind((Ipv4Addr::LOCALHOST, port + 10_000)) {
            chosen.push(port);
            held.push(primary);
            held.push(bus);
        }
    }
    assert_eq!(
        chosen.len(),
        3,
        "could not reserve 3 free cluster port pairs"
    );
    drop(held);
    [chosen[0], chosen[1], chosen[2]]
}

/// The container engine that answers on this host, Docker or Podman, as
/// `ci/container.sh` picks it.
#[cfg(target_os = "linux")]
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
#[cfg(target_os = "linux")]
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
