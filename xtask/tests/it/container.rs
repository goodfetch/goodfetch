//! A true external that ships as an image (TESTING.md, true externals), on
//! Linux: the image a pin file under `ci/` names, run through Docker or
//! Podman, as `ci/container.sh` picks it, its port published on a free
//! loopback port (ADR 0059's isolation: no fixed port). The image's
//! `fetch.sh` beside its pin file (`just fetch`) pulls it before ADR 0043's
//! deny; a missing engine or image fails the test with the command that
//! fixes it, never skips it. The container is removed when the value drops.

use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// How long the server may take to answer.
const START_DEADLINE: Duration = Duration::from_secs(30);

pub(crate) struct Container {
    engine: &'static str,
    id: String,
    url: String,
}

impl Container {
    /// Starts the image `pin` names, `ci/<name>/image.toml`'s text, with
    /// `arguments` after it, its `port` published on loopback, and waits
    /// until `ready`, a path on it, answers 200.
    pub(crate) fn start(pin: &str, port: u16, ready: &str, arguments: &[&str]) -> Self {
        let engine = engine();
        let image = image(pin);
        let output = run(Command::new(engine)
            .args(["run", "--detach", "--rm", "--publish"])
            .arg(format!("127.0.0.1::{port}"))
            .arg(&image)
            .args(arguments));
        assert!(
            output.status.success(),
            "{engine} cannot start {image}: {}; the `fetch.sh` beside its pin file pulls it",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let published = run(Command::new(engine).args(["port", &id, &format!("{port}/tcp")]));
        let mut container = Self {
            engine,
            id,
            url: String::new(),
        };
        let address = String::from_utf8_lossy(&published.stdout)
            .lines()
            .next()
            .map_or_else(
                || panic!("{engine} publishes no port {port}: {}", container.logs()),
                str::to_owned,
            );
        container.url = format!("http://{address}");
        container.wait_answering(ready);
        container
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    fn wait_answering(&self, ready: &str) {
        let started = Instant::now();
        let url = format!("{}{ready}", self.url);
        while ureq::get(&url).call().is_err() {
            assert!(
                started.elapsed() < START_DEADLINE,
                "{url} did not answer within {START_DEADLINE:?}: {}",
                self.logs()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn logs(&self) -> String {
        let output = run(Command::new(self.engine).args(["logs", &self.id]));
        String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr)
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        // Removing it stops it; a failure leaves a container `--rm` removes
        // once it stops, and fails no test.
        let _removed = Command::new(self.engine)
            .args(["rm", "--force", &self.id])
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
        .expect("Docker or Podman answers, to run a test's image")
}

/// `repository@digest` from a pin file's text.
fn image(pin: &str) -> String {
    let pin: toml::Table = toml::from_str(pin).expect("the pin file is TOML");
    let field = |name: &str| {
        pin.get(name)
            .and_then(toml::Value::as_str)
            .unwrap_or_else(|| panic!("the pin file names its {name}"))
            .to_owned()
    };
    format!("{}@{}", field("repository"), field("digest"))
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the container engine starts")
}
