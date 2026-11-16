//! `browser_cache` and `browser_cache_with_lightpanda` called again in a
//! test's folder whose browser still runs from it, as a test that starts
//! two Runs does (fix-queue 106): the second call leaves the running
//! build's files in place. Linux x86-64, where a copy of a running
//! executable cannot be opened for writing (`ETXTBSY`), the Chrome for
//! Testing build sits in `chrome-linux64` and Lightpanda's executable is
//! `lightpanda-x86_64-linux`.
//!
//! And a failed test's folder, which its teardown keeps, holding no bytes of
//! the build of its own (#2664).

use std::os::unix::fs::MetadataExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use goodfetch_test_support::{
    CHROME_FOR_TESTING_BUILD, LIGHTPANDA_BUILD, TestEnv, browser_cache,
    browser_cache_with_lightpanda, chrome_for_testing,
};

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
