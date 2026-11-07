//! A Checkpoint the engine cannot write, on Collie's executable over stdio
//! (spec #213's Seam 1; ADR 0116): with the Run's folder on a full disk, or
//! a location on a read-only filesystem, `run.checkpoint` fails with
//! `file_write_failed`, side `engine`, naming the OS's reason, returns no
//! Checkpoint and leaves no file. Each filesystem is a real one, a small
//! disk image `hdiutil` attaches for the test and detaches after it, so the
//! case runs on macOS; Linux needs a mount it cannot make unprivileged.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "a test outside any async worker, which makes, fills and reads real filesystems (ADR 0062)"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::MESSAGE_DEADLINE;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use goodfetch_test_client::{Attached, hello};

use crate::COLLIE;

/// A disk image attached at a folder of the test's, detached when dropped.
struct DiskImage {
    mount: PathBuf,
}

impl DiskImage {
    /// A fresh HFS+ image of `megabytes`, attached at `name` in the test's
    /// folder, read-only when `read_only`.
    fn attach(env: &TestEnv, name: &str, megabytes: u32, read_only: bool) -> Self {
        let image = env.folder().join(format!("{name}.dmg"));
        let mount = env.folder().join(name);
        std::fs::create_dir_all(&mount).unwrap();
        let created = Command::new("hdiutil")
            .args([
                "create", "-quiet", "-fs", "HFS+", "-layout", "NONE", "-size",
            ])
            .arg(format!("{megabytes}m"))
            .arg("-volname")
            .arg(name)
            .arg(&image)
            .status()
            .unwrap();
        assert!(created.success(), "hdiutil could not create {name}");
        let mut attach = Command::new("hdiutil");
        attach.args(["attach", "-quiet", "-nobrowse", "-mountpoint"]);
        attach.arg(&mount);
        if read_only {
            attach.arg("-readonly");
        }
        let attached = attach.arg(&image).status().unwrap();
        assert!(attached.success(), "hdiutil could not attach {name}");
        Self { mount }
    }
}

impl Drop for DiskImage {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet", "-force"])
            .arg(&self.mount)
            .status();
    }
}

/// Fills the filesystem of `folder` with a file of zeros until no space is
/// left, then frees 16 KiB: room to create a file, not to write a
/// Checkpoint, so the write fails part way and its file must go.
fn fill(folder: &Path) {
    let mut filler = std::fs::File::create(folder.join("filler")).unwrap();
    let block = vec![0_u8; 64 * 1024];
    while filler.write_all(&block).is_ok() {}
    let small = [0_u8; 512];
    while filler.write_all(&small).is_ok() {}
    let full = filler.metadata().unwrap().len();
    filler.set_len(full - 16 * 1024).unwrap();
    filler.sync_all().unwrap();
}

/// The names of the files in `folder`, sorted.
fn files(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Opens the Connection and starts the Run `run_id`.
fn started(engine: &mut Attached, run_id: &str) {
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    let reply = engine.ask(1, "run.start", &json!({ "run_id": run_id }));
    assert!(reply.get("result").is_some(), "{reply}");
}

/// What `file_write_failed` said, without the folder its `path` names and,
/// unless `os_code`, the OS's name: a full disk may fail the file's creation
/// with `ENOSPC` or SQLite's write with no OS name.
fn failure(reply: &Value, os_code: bool) -> Value {
    let mut data = reply["error"]["data"].clone();
    let fields = data.as_object_mut().unwrap();
    fields.remove("path");
    if !os_code {
        fields.remove("os_code");
    }
    data
}

/// Covers: story:3-30, error:file_write_failed
#[test]
fn a_checkpoint_of_a_run_whose_folder_is_on_a_full_disk_fails_and_leaves_no_file() {
    let env = TestEnv::new();
    let disk = DiskImage::attach(&env, "full disk", 4, false);
    let home = disk.mount.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let home_for_engine = home.clone();
    let mut engine = Attached::start_with(&env, COLLIE, MESSAGE_DEADLINE, |command| {
        command.env("GOODFETCH_HOME", &home_for_engine);
    });
    started(&mut engine, "filled");
    let folder = HomeLayout::of(&home).run_folder("filled");
    let before = files(&folder);
    fill(&disk.mount);

    let failed = engine.ask(2, "run.checkpoint", &json!({ "run_id": "filled" }));

    let after = files(&folder);
    std::fs::remove_file(disk.mount.join("filler")).unwrap();
    let _ = engine.finish();
    assert_eq!(
        (failure(&failed, false), after),
        (
            json!({
                "code": "file_write_failed",
                "retryable": false,
                "reason": "storage_full",
                "side": "engine",
            }),
            before,
        )
    );
}

/// Covers: story:3-30, error:file_write_failed
#[test]
fn a_checkpoint_to_a_read_only_filesystem_fails_and_leaves_no_file() {
    let env = TestEnv::new();
    let disk = DiskImage::attach(&env, "read only", 2, true);
    let location = disk.mount.join("kept.sqlite").display().to_string();
    let mut engine = Attached::start(&env, COLLIE);
    started(&mut engine, "refused");

    let failed = engine.ask(
        2,
        "run.checkpoint",
        &json!({ "run_id": "refused", "location": location }),
    );

    let _ = engine.finish();
    let left: Vec<String> = files(&disk.mount)
        .into_iter()
        .filter(|name| !name.starts_with('.') || name.ends_with(".partial"))
        .collect();
    assert_eq!(
        (failure(&failed, true), left),
        (
            json!({
                "code": "file_write_failed",
                "retryable": false,
                "reason": "read_only_filesystem",
                "os_code": "EROFS",
                "side": "engine",
            }),
            Vec::<String>::new(),
        )
    );
}
