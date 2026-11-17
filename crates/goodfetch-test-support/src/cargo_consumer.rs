//! The crates.io side of the install harness (TESTING.md, Installed
//! packages): a temporary consumer project builds against what `cargo
//! package` put in a crate's `.crate` file, as a user's project builds
//! against the crate crates.io serves, so a file the package leaves out
//! fails the consumer's build rather than the user's.
//!
//! `cargo_consumer` packages the crate offline without verifying it, unpacks
//! the `.crate` file into the test's folder, and writes a binary project
//! there, its own workspace, whose one dependency is the unpacked crate by
//! path and whose `main.rs` the test gives, then builds it offline. Cargo
//! runs with the test's cleared map plus what it needs to find its toolchain
//! (`PATH`, Cargo's and rustup's homes, the toolchain the test was built
//! with), each build into a target folder of its own under the test's
//! folder. The consumer's build is returned, success or not, for the test
//! to judge; only a step before it that fails is an error.
#![expect(
    clippy::disallowed_methods,
    reason = "test support runs Cargo and reads and writes files outside any async worker (ADR 0062)"
)]

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};

use crate::env::TestEnv;
use crate::variables::{TOOLCHAIN_VARIABLES, Variables, WINDOWS_USER_VARIABLE};

/// The consumer project's package and executable name.
const CONSUMER: &str = "goodfetch-consumer";

/// A consumer project built against a packaged crate.
#[derive(Debug)]
pub struct Consumer {
    folder: PathBuf,
    build: CargoRun,
    executable: PathBuf,
}

impl Consumer {
    /// The consumer project's folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Its `cargo build`.
    pub fn build(&self) -> &CargoRun {
        &self.build
    }

    /// The executable its build writes, which exists once the build
    /// succeeded.
    pub fn executable(&self) -> &Path {
        &self.executable
    }
}

/// What a Cargo command left: its exit status and output.
#[derive(Debug)]
pub struct CargoRun {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

impl CargoRun {
    fn of(output: Output) -> Self {
        Self {
            status: output.status,
            stdout: lossy(output.stdout),
            stderr: lossy(output.stderr),
        }
    }

    /// Whether it exited with status 0.
    pub fn success(&self) -> bool {
        self.status.success()
    }

    /// Its exit status.
    pub fn status(&self) -> ExitStatus {
        self.status
    }

    /// Its standard output.
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// Its standard error, where Cargo writes its progress and the
    /// compiler's errors.
    pub fn stderr(&self) -> &str {
        &self.stderr
    }
}

/// Why no consumer could be built.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConsumerError {
    /// `cargo package` failed.
    #[error("`cargo package` of {package} failed: {stderr}")]
    Package {
        /// The crate's folder.
        package: PathBuf,
        /// What Cargo wrote on stderr.
        stderr: String,
    },
    /// `cargo package` wrote not exactly one `.crate` file.
    #[error("`cargo package` wrote {count} `.crate` files in {folder}, not one")]
    CrateFiles {
        /// The folder it writes them in.
        folder: PathBuf,
        /// How many it holds.
        count: usize,
    },
    /// The `.crate` file cannot be unpacked.
    #[error("{crate_file} cannot be unpacked: {source}")]
    Unpack {
        /// The `.crate` file.
        crate_file: PathBuf,
        /// What reading it failed on.
        source: io::Error,
    },
    /// The packaged manifest names no package.
    #[error("{manifest} names no package: {reason}")]
    Manifest {
        /// The packaged `Cargo.toml`.
        manifest: PathBuf,
        /// What reading it failed on.
        reason: String,
    },
    /// A file of the consumer project cannot be written, or Cargo cannot
    /// start.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Packages the crate in `package` with `cargo package`, and builds a
/// consumer project against the packaged files whose `src/main.rs` is
/// `main_rs`, in a folder of the test's own.
///
/// # Errors
///
/// [`ConsumerError`] when packaging, unpacking or writing the project
/// fails, or Cargo cannot start. A consumer whose build fails is no error:
/// [`Consumer::build`] says so.
pub fn cargo_consumer(
    env: &TestEnv,
    package: &Path,
    main_rs: &str,
) -> Result<Consumer, ConsumerError> {
    let variables = Variables::from_process();
    let work = env.folder().join("cargo consumer");
    std::fs::create_dir_all(&work)?;

    let package_target = work.join("package target");
    let packaged = cargo(env, &variables)
        .arg("package")
        .arg("--manifest-path")
        .arg(package.join("Cargo.toml"))
        .args(["--offline", "--no-verify", "--allow-dirty", "--target-dir"])
        .arg(&package_target)
        .output()?;
    if !packaged.status.success() {
        return Err(ConsumerError::Package {
            package: package.to_path_buf(),
            stderr: lossy(packaged.stderr),
        });
    }
    let crate_file = only_crate_file(&package_target.join("package"))?;
    let unpacked = unpack(&crate_file, &work.join("unpacked"))?;
    let name = package_name(&unpacked.join("Cargo.toml"))?;

    let folder = work.join("consumer");
    write_project(&folder, &name, &unpacked, main_rs)?;
    let target = folder.join("target");
    let build = cargo(env, &variables)
        .arg("build")
        .arg("--manifest-path")
        .arg(folder.join("Cargo.toml"))
        .args(["--offline", "--target-dir"])
        .arg(&target)
        .output()?;
    let executable = target
        .join("debug")
        .join(format!("{CONSUMER}{}", std::env::consts::EXE_SUFFIX));
    Ok(Consumer {
        folder,
        build: CargoRun::of(build),
        executable,
    })
}

/// `cargo` with the test's map and what Cargo needs to find its toolchain:
/// the test's `HOME` is its own, so Cargo's and rustup's homes default to
/// the user's, where the toolchain is.
fn cargo(env: &TestEnv, variables: &Variables) -> Command {
    let mut command = env.process_command("cargo");
    for name in TOOLCHAIN_VARIABLES {
        if let Some(value) = variables.get(name) {
            command.env(name, value);
        }
    }
    let user = variables.get(if cfg!(windows) {
        WINDOWS_USER_VARIABLE
    } else {
        "HOME"
    });
    for (name, folder) in [("CARGO_HOME", ".cargo"), ("RUSTUP_HOME", ".rustup")] {
        if variables.get(name).is_none()
            && let Some(user) = user
        {
            command.env(name, Path::new(user).join(folder));
        }
    }
    command
}

/// The one `.crate` file `cargo package` wrote in `folder`.
fn only_crate_file(folder: &Path) -> Result<PathBuf, ConsumerError> {
    let mut crate_files = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "crate")
        {
            crate_files.push(path);
        }
    }
    match <[PathBuf; 1]>::try_from(crate_files) {
        Ok([crate_file]) => Ok(crate_file),
        Err(crate_files) => Err(ConsumerError::CrateFiles {
            folder: folder.to_path_buf(),
            count: crate_files.len(),
        }),
    }
}

/// Unpacks `crate_file`, a gzipped tar whose one folder is
/// `<name>-<version>`, into `into`, and returns that folder.
fn unpack(crate_file: &Path, into: &Path) -> Result<PathBuf, ConsumerError> {
    let unpack_error = |source| ConsumerError::Unpack {
        crate_file: crate_file.to_path_buf(),
        source,
    };
    let file = File::open(crate_file).map_err(unpack_error)?;
    tar::Archive::new(flate2::read::GzDecoder::new(file))
        .unpack(into)
        .map_err(unpack_error)?;
    let stem = crate_file.file_stem().ok_or_else(|| {
        unpack_error(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the file has no name",
        ))
    })?;
    Ok(into.join(stem))
}

/// The `package.name` of a manifest.
fn package_name(manifest: &Path) -> Result<String, ConsumerError> {
    let manifest_error = |reason: String| ConsumerError::Manifest {
        manifest: manifest.to_path_buf(),
        reason,
    };
    let text =
        std::fs::read_to_string(manifest).map_err(|error| manifest_error(error.to_string()))?;
    let table: toml::Table = text
        .parse()
        .map_err(|error: toml::de::Error| manifest_error(error.to_string()))?;
    table
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| manifest_error("it has no `package.name`".to_owned()))
}

/// Writes the consumer project: its manifest, a workspace of its own whose
/// one dependency is the unpacked crate by path, and `src/main.rs`.
fn write_project(
    folder: &Path,
    dependency: &str,
    unpacked: &Path,
    main_rs: &str,
) -> Result<(), ConsumerError> {
    let path = unpacked.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not Unicode", unpacked.display()),
        )
    })?;
    // A JSON string is a TOML basic string: the path's backslashes and
    // quotes come out escaped.
    let path = serde_json::to_string(path).map_err(io::Error::other)?;
    let manifest = format!(
        "[package]\nname = \"{CONSUMER}\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n\
         [dependencies]\n{dependency} = {{ path = {path} }}\n\n[workspace]\n"
    );
    std::fs::create_dir_all(folder.join("src"))?;
    std::fs::write(folder.join("Cargo.toml"), manifest)?;
    std::fs::write(folder.join("src").join("main.rs"), main_rs)?;
    Ok(())
}

/// `bytes` as text, each invalid UTF-8 sequence replaced.
fn lossy(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}
