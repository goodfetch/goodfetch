//! `browser_cache` and `browser_cache_with_lightpanda` called again in a
//! test's folder whose browser still runs from it, as a test that starts
//! two Runs does (fix-queue 106): the second call leaves the running
//! build's files in place. Linux x86-64, where a copy of a running
//! executable cannot be opened for writing (`ETXTBSY`), the Chrome for
//! Testing build sits in `chrome-linux64` and Lightpanda's executable is
//! `lightpanda-x86_64-linux`.
//!
//! And a failed test's folder, which its teardown keeps, holding no bytes of
//! the build of its own (#2664). Where Playwright's build cannot be linked
//! into the tests' temporary folder, every cache links one copy of it, laid
//! out once, whole, and again once removed; where it can, no copy is made
//! (#2306). Those cases run `probe::lays_out_a_browser_cache` as a child, with
//! `PLAYWRIGHT_BROWSERS_PATH` naming a stand-in build, since test support
//! reads Playwright's folder from the process's variables.

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use goodfetch_test_support::{
    CHROME_FOR_TESTING_BUILD, LIGHTPANDA_BUILD, TestEnv, browser_cache,
    browser_cache_with_lightpanda, chrome_for_testing,
};
use serde_json::Value;

use crate::probe;
use crate::probe::CACHE_FILE;

/// The bytes of the files in `folder` that no other folder links: what
/// deleting it gives back.
fn bytes_held_alone(folder: &Path) -> u64 {
    let mut held = 0;
    for entry in std::fs::read_dir(folder).unwrap() {
        let entry = entry.unwrap();
        let metadata = entry.path().symlink_metadata().unwrap();
        if metadata.is_dir() {
            held += bytes_held_alone(&entry.path());
        } else if metadata.is_file() && metadata.nlink() == 1 {
            held += metadata.len();
        }
    }
    held
}

/// A browser-backed test that fails keeps its folder for its evidence, and
/// that folder holds no copy of the build: a post-merge run's failed tests
/// kept about 400 MB each on `/` (#2664).
#[test]
fn a_failed_browser_backed_tests_folder_holds_no_copy_of_the_build() {
    let mut folder = None;
    let mut cache = None;

    let _ = catch_unwind(AssertUnwindSafe(|| {
        let env = TestEnv::new();
        folder = Some(env.folder().to_owned());
        cache = Some(browser_cache(&env).unwrap());
        panic!("the test fails");
    }));

    let folder = folder.unwrap();
    let kept = folder.is_dir();
    let links = cache
        .as_deref()
        .and_then(|cache| chrome_for_testing(cache).metadata().ok())
        .map_or(0, |chrome| chrome.nlink());
    let held = if kept { bytes_held_alone(&folder) } else { 0 };
    if kept {
        std::fs::remove_dir_all(&folder).unwrap();
    }
    assert!(cache.is_some(), "the Browser cache is laid out");
    assert!(kept, "the failed test's folder is kept");
    assert!(links > 1, "the kept build's executable has {links} links");
    assert!(
        held < 1 << 20,
        "the kept folder holds {held} bytes of its own"
    );
}

#[test]
fn a_second_browser_cache_while_its_browser_runs_leaves_the_running_build_in_place() {
    let env = TestEnv::new();
    let first = browser_cache(&env).unwrap();
    let chrome = first
        .join("browser")
        .join("chrome-for-testing")
        .join(CHROME_FOR_TESTING_BUILD)
        .join("files")
        .join("chrome-linux64")
        .join("chrome");
    let mut command = env.process_command(&chrome);
    command.args([
        "--headless=new",
        "--no-sandbox",
        "--use-mock-keychain",
        "--password-store=basic",
        "--no-first-run",
        "--remote-debugging-port=0",
        "about:blank",
    ]);
    command.arg(format!(
        "--user-data-dir={}",
        env.folder().join("profile").display()
    ));
    let browser = env.spawn(&mut command).unwrap();

    let second = browser_cache(&env);

    browser.terminate().unwrap();
    browser.child().wait().unwrap();
    assert_eq!(second.unwrap(), first);
}

#[test]
fn a_second_lightpanda_cache_while_lightpanda_runs_leaves_its_executable_in_place() {
    let env = TestEnv::new();
    let first = browser_cache_with_lightpanda(&env).unwrap();
    let lightpanda = first
        .join("browser")
        .join("lightpanda")
        .join(LIGHTPANDA_BUILD)
        .join("files")
        .join("lightpanda-x86_64-linux");
    let mut command = env.process_command(&lightpanda);
    command
        .arg("mcp")
        .env("LIGHTPANDA_DISABLE_TELEMETRY", "true");
    let browser = env.spawn(&mut command).unwrap();

    let second = browser_cache_with_lightpanda(&env);

    browser.close_stdin_and_wait().unwrap();
    assert_eq!(second.unwrap(), first);
}

/// A stand-in for Playwright's folder of browsers at `browsers`, holding a
/// few small files where Playwright's revision of the pinned build
/// (`chromium-1248`) would be: the build's files, by their path in it.
fn stand_in_playwright(browsers: &Path) -> BTreeMap<String, String> {
    let build = browsers.join("chromium-1248").join("chrome-linux64");
    let files: BTreeMap<String, String> = std::iter::once("chrome".to_owned())
        .chain((0..24).map(|n| format!("locales/{n}.pak")))
        .map(|path| (path.clone(), format!("the bytes of {path}\n").repeat(64)))
        .collect();
    for (path, bytes) in &files {
        let file = build.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    files
}

/// A probe that takes a Browser cache as a test does, its Playwright folder
/// `browsers` and its temporary folder `temporary`.
fn cache_probe(env: &TestEnv, browsers: &Path, temporary: &Path) -> Command {
    let mut command = probe(env, "probe::lays_out_a_browser_cache");
    command
        .env("PLAYWRIGHT_BROWSERS_PATH", browsers)
        .env("TMPDIR", temporary)
        .env("TMP", temporary)
        .env("TEMP", temporary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// The files of the cache a probe took, by their path in the build: each
/// one's inode, links and bytes as the probe saw them.
fn cache_files(output: &Output) -> BTreeMap<String, Value> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the probe failed: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix(CACHE_FILE))
        .map(|line| {
            let file: Value = serde_json::from_str(line).unwrap();
            (file["path"].as_str().unwrap().to_owned(), file)
        })
        .collect()
}

/// Asserts that a probe's cache holds every file of `source`, each with its
/// bytes, and nothing else.
fn assert_whole(cache: &BTreeMap<String, Value>, source: &BTreeMap<String, String>) {
    assert_eq!(
        cache.keys().collect::<Vec<_>>(),
        source.keys().collect::<Vec<_>>()
    );
    for (path, bytes) in source {
        assert_eq!(cache[path]["bytes"], bytes.as_str(), "{path}");
    }
}

/// The entries of `temporary` named as the build's copy is, or as its parts
/// laid out are.
fn copies_in(temporary: &Path) -> Vec<String> {
    let copy = format!("goodfetch-chrome-for-testing-{CHROME_FOR_TESTING_BUILD}-chrome-linux64");
    let mut names: Vec<String> = std::fs::read_dir(temporary)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.trim_start_matches('.').starts_with(&copy))
        .collect();
    names.sort();
    names
}

/// The copy of the build in `temporary`.
fn copy_in(temporary: &Path) -> PathBuf {
    temporary.join(format!(
        "goodfetch-chrome-for-testing-{CHROME_FOR_TESTING_BUILD}-chrome-linux64"
    ))
}

/// A stand-in Playwright folder on `/dev/shm`, a file system of its own, and
/// an empty temporary folder in `env`'s, on another: a hard link from the
/// one to the other fails, as the link from root's `/ms-playwright` fails
/// on the gate host (#2306).
fn unlinkable_build(env: &TestEnv) -> (tempfile::TempDir, BTreeMap<String, String>, PathBuf) {
    let browsers = tempfile::Builder::new()
        .prefix("goodfetch-playwright-")
        .tempdir_in("/dev/shm")
        .unwrap();
    let source = stand_in_playwright(browsers.path());
    let temporary = env.folder().join("tmp");
    std::fs::create_dir(&temporary).unwrap();
    assert_ne!(
        browsers.path().metadata().unwrap().dev(),
        temporary.metadata().unwrap().dev(),
        "the stand-in build and the temporary folder share a file system"
    );
    (browsers, source, temporary)
}

#[test]
fn two_caches_of_a_build_they_cannot_link_link_one_copy_of_it() {
    let env = TestEnv::new();
    let (browsers, source, temporary) = unlinkable_build(&env);

    let first = cache_files(
        &cache_probe(&env, browsers.path(), &temporary)
            .output()
            .unwrap(),
    );
    let second = cache_files(
        &cache_probe(&env, browsers.path(), &temporary)
            .output()
            .unwrap(),
    );

    assert_whole(&first, &source);
    assert_whole(&second, &source);
    let copy = copy_in(&temporary);
    for path in source.keys() {
        assert_eq!(first[path]["inode"], second[path]["inode"], "{path}");
        assert!(first[path]["links"].as_u64().unwrap() >= 2, "{path}");
        let copied = copy.join(path).metadata().unwrap().ino();
        assert_eq!(first[path]["inode"], copied, "{path}");
    }
}

#[test]
fn eight_tests_starting_together_on_an_empty_temporary_folder_each_take_a_whole_cache() {
    let env = TestEnv::new();
    let (browsers, source, temporary) = unlinkable_build(&env);

    let probes: Vec<_> = (0..8)
        .map(|_| {
            cache_probe(&env, browsers.path(), &temporary)
                .spawn()
                .unwrap()
        })
        .collect();
    let caches: Vec<_> = probes
        .into_iter()
        .map(|probe| cache_files(&probe.wait_with_output().unwrap()))
        .collect();

    for cache in &caches {
        assert_whole(cache, &source);
    }
    let copy = copy_in(&temporary);
    let name = copy.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        copies_in(&temporary),
        [name.clone(), format!("{name}.lock")]
    );
}

#[test]
fn a_cache_taken_after_the_builds_copy_is_removed_is_whole() {
    let env = TestEnv::new();
    let (browsers, source, temporary) = unlinkable_build(&env);
    let first = cache_files(
        &cache_probe(&env, browsers.path(), &temporary)
            .output()
            .unwrap(),
    );
    std::fs::remove_dir_all(copy_in(&temporary)).unwrap();

    let second = cache_files(
        &cache_probe(&env, browsers.path(), &temporary)
            .output()
            .unwrap(),
    );

    assert_whole(&first, &source);
    assert_whole(&second, &source);
    assert!(copy_in(&temporary).is_dir());
}

#[test]
fn a_build_the_tests_can_link_is_linked_with_no_copy() {
    let env = TestEnv::new();
    let browsers = env.folder().join("playwright");
    let source = stand_in_playwright(&browsers);
    let temporary = env.folder().join("tmp");
    std::fs::create_dir(&temporary).unwrap();

    let cache = cache_files(&cache_probe(&env, &browsers, &temporary).output().unwrap());

    assert_whole(&cache, &source);
    let build = browsers.join("chromium-1248").join("chrome-linux64");
    for path in source.keys() {
        let linked = build.join(path).metadata().unwrap().ino();
        assert_eq!(cache[path]["inode"], linked, "{path}");
    }
    assert_eq!(copies_in(&temporary), Vec::<String>::new());
}
