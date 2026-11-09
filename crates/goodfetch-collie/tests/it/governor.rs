//! The Resource governor of Collie's executable over stdio (spec #213's
//! Seam 1; ADR 0040), in a cgroup with a memory limit: the user's own
//! service manager runs the engine in a transient scope of its own, as a
//! container runtime runs a container's processes in its own cgroup, which
//! sits deep below the hierarchy's root, so only a governor that reads its
//! own cgroup's path finds the limit. The limit is then changed in place,
//! as Kubernetes' in-place Pod resize changes it. These tests need the
//! user's systemd, which the Pre-merge suite's Linux host runs (TESTING.md,
//! Storage, network and OS services).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the user's session from its own process, and runs the user's service manager"
)]

use std::process::{Command, Stdio};

use goodfetch_protocol::catalog::Product;
use goodfetch_test_support::{MESSAGE_DEADLINE, TestEnv};
use serde_json::{Value, json};

use goodfetch_test_client::{Attached, hello};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

const MIB: u64 = 1 << 20;

/// How many samples the test reads after the resize before it judges the
/// last: the governor samples every 100 ms here, so never a measure.
const SAMPLES_AFTER_A_RESIZE: usize = 20;

/// The variables through which a command reaches the user's own service
/// manager.
const USER_SESSION: [&str; 3] = ["PATH", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"];

fn in_session(command: &mut Command) -> &mut Command {
    for name in USER_SESSION {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
}

/// The name of the test's scope, which the test home's folder names.
fn unit(env: &TestEnv) -> String {
    let folder = env.home().file_name().unwrap().to_string_lossy();
    let name: String = folder.chars().filter(char::is_ascii_alphanumeric).collect();
    format!("goodfetch-test-governor-{name}-{}", std::process::id())
}

/// Fails the test at once, naming what is missing, when the user's service
/// manager cannot run a transient scope with a memory limit.
fn needs_user_scopes(env: &TestEnv) {
    let probe = in_session(&mut env.process_command("systemd-run"))
        .args([
            "--user",
            "--scope",
            "--quiet",
            "-p",
            "MemoryMax=64M",
            "true",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|error| panic!("`systemd-run` did not run: {error}"));
    assert!(
        probe.status.success(),
        "the user's systemd cannot run a scope with a memory limit ({}): the test needs a \
         user session (`loginctl enable-linger`) whose memory controller is delegated",
        String::from_utf8_lossy(&probe.stderr).trim()
    );
}

/// The next `engine.system_info` the engine sends.
fn next_sample(engine: &mut Attached) -> Value {
    loop {
        let message = engine
            .next()
            .expect("the engine ended before its next sample");
        let events = message["params"]["events"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(sample) = events
            .into_iter()
            .find(|event| event["event"] == "engine.system_info")
        {
            engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "samples", "credits": 1 } }));
            return sample["data"].clone();
        }
    }
}

/// In a scope limited to 256 MiB, the Memory allowance is that limit, read
/// from the engine's own cgroup; raised in place to 384 MiB, the next
/// samples carry the new limit; the engine's own memory, as PSS, is in use.
///
/// Covers: story:3-56, story:3-58, event:engine.system_info, operation:events.subscribe
#[test]
fn in_a_memory_limited_cgroup_the_allowance_is_its_limit_and_follows_a_change_in_place() {
    let env = TestEnv::new();
    env.user_configuration("governor.sample_interval = 100\n");
    needs_user_scopes(&env);
    let unit = unit(&env);
    let mut command = env.process_command("systemd-run");
    in_session(&mut command)
        .args(["--user", "--scope", "--quiet", "--unit", &unit])
        .args(["-p", "MemoryMax=256M", "--"])
        .arg(COLLIE)
        .arg("attached-engine");
    let mut engine = Attached::spawned(&env, MESSAGE_DEADLINE, &mut command);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "samples", "method": "events.subscribe",
        "params": { "system_info": true },
    }));
    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": "samples", "credits": 1 } }));

    let limited = next_sample(&mut engine);
    let resized = in_session(&mut env.process_command("systemctl"))
        .args(["--user", "set-property", "--runtime"])
        .arg(format!("{unit}.scope"))
        .arg("MemoryMax=384M")
        .status()
        .unwrap();
    // The resize reaches the cgroup before `systemctl` returns, so the next
    // sample but one, taken after it, carries it; a few more allow for a
    // sample under way meanwhile.
    let mut raised = next_sample(&mut engine);
    for _ in 0..SAMPLES_AFTER_A_RESIZE {
        if raised["memory"]["allowance"] != limited["memory"]["allowance"] {
            break;
        }
        raised = next_sample(&mut engine);
    }
    let (_, exited) = engine.finish();

    let used = limited["memory"]["used"].as_u64().unwrap();
    assert_eq!(
        (
            &limited["memory"]["allowance"],
            &limited["memory"]["allowance_source"],
            resized.success(),
            &raised["memory"]["allowance"],
            &raised["memory"]["allowance_source"],
            used > 0 && used < 256 * MIB,
            exited,
        ),
        (
            &json!(256 * MIB),
            &json!("cgroup"),
            true,
            &json!(384 * MIB),
            &json!("cgroup"),
            true,
            true,
        )
    );
}
