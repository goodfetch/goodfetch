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
#![expect(
    clippy::disallowed_methods,
    clippy::print_stdout,
    reason = "a developer's tool run by hand, never in the engine"
)]

use std::collections::BTreeMap;
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
    let build = match arguments.as_slice() {
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
                "usage: pin-chrome-for-testing download <build> | tree <build> <platform> <folder>"
                    .into(),
            );
        }
    };
    let pins = json!({ "chrome_for_testing": { "build": build, "platforms": platforms } });
    let mut text = serde_json::to_string_pretty(&pins)?;
    text.push('\n');
    fs::write(pins_file(), text)?;
    println!("wrote {}", pins_file().display());
    Ok(())
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
