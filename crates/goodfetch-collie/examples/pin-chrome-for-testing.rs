//! Writes `src/pins.json`, the digests of the Chrome for Testing build
//! Collie pins (ADRs 0024 and 0098), as `goodfetch_collie::pinned_artifacts`
//! reads them.
//!
//! - `download <build>`: fetches each platform's archive from Google's
//!   Chrome for Testing storage (about 150 MB each), and records the
//!   archive's SHA-256 and the digest of the files it holds. This is the
//!   one way an archive digest is known; run it when the pin moves.
//! - `tree <build> <platform> <folder>`: records the files' digest of an
//!   already unpacked build at `<folder>` (its `chrome-<platform>` folder,
//!   such as Playwright's copy of the same build), with no archive digest:
//!   such a pin launches from a cache that holds the build and is never
//!   downloaded.
//! - `alps <build> <executable>`: records the ALPS payload the build sends
//!   (ADR 0042), launching its `executable` headless against a local TLS
//!   server that accepts ALPS for `h2`, so the egress relay sends a site
//!   the same. Run it whenever the pin moves: the build's other entries
//!   are kept, and a build with no payload recorded fails every launch's
//!   reading of the pins rather than sending another.
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::print_stdout,
    reason = "a developer's tool run by hand, never in the engine"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use goodfetch_engine::browser_cache::{BuildPlatform, content_digest};
use goodfetch_engine::pinned_download::Digest;
use serde_json::{Value, json};

fn pins_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pins.json")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut platforms = BTreeMap::new();
    let kept: Value = fs::read_to_string(pins_file())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    let kept = &kept["chrome_for_testing"];
    let mut application_settings = kept["application_settings"].clone();
    let build = match arguments.as_slice() {
        [command, build, executable] if command == "alps" => {
            application_settings = captured(Path::new(executable))?;
            if let Some(kept) = kept["platforms"].as_object() {
                for (platform, digests) in kept {
                    platforms.insert(platform.clone(), digests.clone());
                }
            }
            build.clone()
        }
        [command, build] if command == "download" => {
            for platform in BuildPlatform::CHROME_FOR_TESTING
                .into_iter()
                .filter_map(BuildPlatform::chrome_for_testing_name)
            {
                platforms.insert(platform.to_owned(), downloaded(build, platform)?);
            }
            build.clone()
        }
        [command, build, platform, folder] if command == "tree" => {
            let digest = tree_digest(platform, Path::new(folder))?;
            platforms.insert(
                platform.clone(),
                json!({ "archive_digest": null, "digest": digest.to_string() }),
            );
            build.clone()
        }
        _ => {
            return Err(
                "usage: pin-chrome-for-testing download <build> | tree <build> <platform> <folder> | alps <build> <executable>"
                    .into(),
            );
        }
    };
    if kept["build"].as_str() != Some(build.as_str()) && arguments[0] != "alps" {
        application_settings = Value::Null;
    }
    let pins = json!({ "chrome_for_testing": {
        "build": build,
        "application_settings": application_settings,
        "platforms": platforms,
    } });
    let mut text = serde_json::to_string_pretty(&pins)?;
    text.push('\n');
    fs::write(pins_file(), text)?;
    println!("wrote {}", pins_file().display());
    Ok(())
}

/// The ALPS payload, in lowercase hex by ALPN protocol, that the browser
/// at `executable` sends a local TLS server it is told to trust.
fn captured(executable: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let profile = tempfile::tempdir()?;
    let settings = runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let mut browser = std::process::Command::new(executable)
            .args([
                "--headless=new",
                "--ignore-certificate-errors",
                "--use-mock-keychain",
                "--password-store=basic",
                "--no-first-run",
                "--no-default-browser-check",
            ])
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .arg(format!("https://localhost:{port}/"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let captured = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            goodfetch_page::capture_application_settings(listener),
        )
        .await;
        let _ = browser.kill();
        let _ = browser.wait();
        Ok::<_, Box<dyn std::error::Error>>(captured??)
    })?;
    println!("captured the ALPS payload for {:?}", settings.keys());
    Ok(settings
        .into_iter()
        .map(|(protocol, payload)| {
            let hex = payload.iter().fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            });
            (protocol, Value::String(hex))
        })
        .collect::<serde_json::Map<_, _>>()
        .into())
}

/// The digest of the files of the unpacked `chrome-<platform>` folder at
/// `folder`, as the archive lays them out.
fn tree_digest(platform: &str, folder: &Path) -> io::Result<Digest> {
    let scratch = tempfile::tempdir()?;
    link_tree(folder, &scratch.path().join(format!("chrome-{platform}")))?;
    content_digest(scratch.path())
}

/// Downloads `platform`'s archive of `build`, and its two digests.
fn downloaded(build: &str, platform: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!(
        "https://storage.googleapis.com/chrome-for-testing-public/{build}/{platform}/chrome-{platform}.zip"
    );
    println!("downloading {url}");
    let mut archive = Vec::new();
    ureq::get(&url)
        .call()?
        .into_body()
        .into_reader()
        .read_to_end(&mut archive)?;
    let scratch = tempfile::tempdir()?;
    zip::ZipArchive::new(io::Cursor::new(&archive))?.extract(scratch.path())?;
    Ok(json!({
        "archive_digest": Digest::of(&archive).to_string(),
        "digest": content_digest(scratch.path())?.to_string(),
    }))
}

/// `source`'s tree at `target`, each file a hard link or a copy.
fn link_tree(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let to = target.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(fs::read_link(entry.path())?, &to)?;
        } else if kind.is_dir() {
            link_tree(&entry.path(), &to)?;
        } else if fs::hard_link(entry.path(), &to).is_err() {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}
