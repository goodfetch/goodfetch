//! `browser_cache` called again in a test's folder whose browser still runs
//! from it, as a test that starts two Runs does (fix-queue 106): the second
//! call leaves the running build's files in place. Linux x86-64, where a
//! copy of a running executable cannot be opened for writing (`ETXTBSY`)
//! and the Chrome for Testing build sits in `chrome-linux64`.

use goodfetch_test_support::{CHROME_FOR_TESTING_BUILD, TestEnv, browser_cache};

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
