//! Chromium's renderers sandboxed on macOS and Windows (spec #564, ticket
//! #1618), as `browser_sandbox` proves them on Linux: each OS is asked
//! about each renderer of a browser Collie launched, and about the browser
//! itself, which runs unsandboxed, as the positive control (TESTING.md,
//! Confinement).
//!
//! macOS answers through `sandbox_check`, which says whether a process
//! runs under a Seatbelt profile. Windows answers through the integrity
//! level of the process's token: a sandboxed renderer's token is lowered
//! to Untrusted, where an unsandboxed one keeps its user's Medium (or
//! High, on a runner whose user is an administrator).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test lists processes on its own thread, never an async worker"
)]
#![expect(
    unsafe_code,
    reason = "the OS says whether a process is sandboxed only through its C interface"
)]

use std::process::Command;

use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, json};

use crate::page::{opened, started_with};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 1618;

/// What the OS says of a sandboxed renderer.
#[cfg(target_os = "macos")]
const SANDBOXED: &[&str] = &["Seatbelt"];
/// What the OS says of a sandboxed renderer: Chromium lowers its token to
/// Untrusted once it has started.
#[cfg(windows)]
const SANDBOXED: &[&str] = &["untrusted"];

/// What the OS says of the unsandboxed browser.
#[cfg(target_os = "macos")]
const UNSANDBOXED: &[&str] = &["no sandbox"];
/// What the OS says of the unsandboxed browser: its user's level.
#[cfg(windows)]
const UNSANDBOXED: &[&str] = &["medium", "high"];

/// Each running process as its ID and command line.
#[cfg(target_os = "macos")]
fn processes() -> Vec<(u32, String)> {
    let listed = Command::new("ps")
        .args(["-axww", "-o", "pid=,args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, args) = line.trim_start().split_once(' ')?;
            Some((pid.parse().ok()?, args.to_owned()))
        })
        .collect()
}

/// Each running process as its ID and command line.
#[cfg(windows)]
fn processes() -> Vec<(u32, String)> {
    let listed = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }",
        ])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, args) = line.trim_start().split_once(' ')?;
            Some((pid.parse().ok()?, args.to_owned()))
        })
        .collect()
}

/// The test's browser processes, those whose profile is in its folder,
/// which a command line names by the folder name's last, ASCII, part.
fn browser_processes(env: &TestEnv) -> Vec<(u32, String)> {
    let name = env.folder().file_name().unwrap().to_string_lossy();
    let marker = name.rsplit(' ').next().unwrap().to_owned();
    processes()
        .into_iter()
        .filter(|(_, line)| line.contains(&marker) && line.contains("--user-data-dir="))
        .collect()
}

/// Whether the process `pid` runs under a Seatbelt profile.
#[cfg(target_os = "macos")]
fn confinement_of(pid: u32) -> String {
    use std::ffi::{c_char, c_int};

    unsafe extern "C" {
        /// `<sandbox.h>`'s private check, which libSystem exports: 1 when
        /// the process `pid` is sandboxed, 0 when it is not, with no
        /// operation and `SANDBOX_FILTER_NONE` (0) as its filter.
        fn sandbox_check(pid: i32, operation: *const c_char, filter: c_int, ...) -> c_int;
    }

    let pid = i32::try_from(pid).unwrap();
    // SAFETY: a null operation with `SANDBOX_FILTER_NONE` asks whether the
    // process is sandboxed at all, and reads no further argument.
    let checked = unsafe { sandbox_check(pid, std::ptr::null(), 0) };
    match checked {
        1 => "Seatbelt".to_owned(),
        0 => "no sandbox".to_owned(),
        other => format!(
            "sandbox_check answered {other}: {}",
            std::io::Error::last_os_error()
        ),
    }
}

/// The integrity level of the process `pid`'s token.
#[cfg(windows)]
fn confinement_of(pid: u32) -> String {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TokenIntegrityLevel,
    };
    use windows_sys::Win32::System::SystemServices::{
        SECURITY_MANDATORY_HIGH_RID, SECURITY_MANDATORY_LOW_RID, SECURITY_MANDATORY_MEDIUM_RID,
        SECURITY_MANDATORY_SYSTEM_RID, SECURITY_MANDATORY_UNTRUSTED_RID,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: `OpenProcess` takes no pointer; a null handle is its failure.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return format!("OpenProcess: {}", std::io::Error::last_os_error());
    }
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `process` is open with the right to query it, and `token` is
    // a valid place for the token's handle.
    let token_opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) };
    let token_opened = (token_opened != 0)
        .then_some(())
        .ok_or_else(|| format!("OpenProcessToken: {}", std::io::Error::last_os_error()));
    // SAFETY: `process` is open and closed once, here.
    unsafe { CloseHandle(process) };
    if let Err(error) = token_opened {
        return error;
    }
    // The label and the SID it points to, in a buffer aligned for the
    // label, which holds a SID of up to fourteen subauthorities.
    let mut buffer = [0_u64; 16];
    let mut length = 0_u32;
    // SAFETY: `token` is open with `TOKEN_QUERY`, and `buffer` holds as
    // many bytes as the length passed says.
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            buffer.as_mut_ptr().cast(),
            u32::try_from(size_of_val(&buffer)).unwrap(),
            &raw mut length,
        )
    };
    let read = (read != 0)
        .then_some(())
        .ok_or_else(|| format!("GetTokenInformation: {}", std::io::Error::last_os_error()));
    // SAFETY: `token` is open and closed once, here.
    unsafe { CloseHandle(token) };
    if let Err(error) = read {
        return error;
    }
    // SAFETY: `GetTokenInformation` wrote a `TOKEN_MANDATORY_LABEL` at the
    // start of `buffer`, aligned for it, whose SID lies in `buffer` too; a
    // label's SID has at least one subauthority, the level's.
    let level = unsafe {
        let label = &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        *GetSidSubAuthority(label.Label.Sid, u32::from(count) - 1)
    };
    match i32::try_from(level) {
        Ok(SECURITY_MANDATORY_UNTRUSTED_RID) => "untrusted".to_owned(),
        Ok(SECURITY_MANDATORY_LOW_RID) => "low".to_owned(),
        Ok(SECURITY_MANDATORY_MEDIUM_RID) => "medium".to_owned(),
        Ok(SECURITY_MANDATORY_HIGH_RID) => "high".to_owned(),
        Ok(SECURITY_MANDATORY_SYSTEM_RID) => "system".to_owned(),
        _ => format!("integrity level {level:#x}"),
    }
}

/// Every renderer of a launched Chromium is sandboxed: on macOS, under a
/// Seatbelt profile; on Windows, with its token lowered to Untrusted. The
/// browser itself, which the OS reports unsandboxed, is the positive
/// control, and it was started without `--no-sandbox`.
/// Covers: story:4-85, setting:security.browser_sandbox, operation:run.new_page, operation:page.goto
#[test]
fn every_renderer_of_a_launched_chromium_is_sandboxed() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "sandboxed",
        &Map::new(),
        |_| {},
    );
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "sandboxed" })));
    let went = engine.ask(
        3,
        "page.goto",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("catalog.test", "/catalog/1"),
        }),
    );

    let listed = browser_processes(&env);
    let browsers: Vec<String> = listed
        .iter()
        .filter(|(_, line)| !line.contains("--type="))
        .map(|(pid, _)| confinement_of(*pid))
        .collect();
    let renderers: Vec<(u32, String)> = listed
        .iter()
        .filter(|(_, line)| line.contains("--type=renderer"))
        .map(|(pid, _)| (*pid, confinement_of(*pid)))
        .collect();
    // A renderer that exited since it was listed, as the blank page's does
    // once the navigation gives the site its own, has nothing to ask.
    let still_running: Vec<u32> = browser_processes(&env)
        .into_iter()
        .map(|(pid, _)| pid)
        .collect();
    let renderers: Vec<String> = renderers
        .into_iter()
        .filter(|(pid, _)| still_running.contains(pid))
        .map(|(_, confinement)| confinement)
        .collect();
    engine.ask(4, "run.finish", &json!({ "run_id": "sandboxed" }));
    let _ = engine.finish();

    assert!(went.get("result").is_some(), "{went}");
    assert_eq!(
        (
            browsers.len(),
            browsers
                .iter()
                .all(|confinement| UNSANDBOXED.contains(&confinement.as_str())),
            !renderers.is_empty(),
            renderers
                .iter()
                .all(|confinement| SANDBOXED.contains(&confinement.as_str())),
            listed.iter().any(|(_, line)| line.contains("--no-sandbox")),
        ),
        (1, true, true, true, false),
        "browsers {browsers:?}, renderers {renderers:?}, processes {listed:#?}"
    );
}
