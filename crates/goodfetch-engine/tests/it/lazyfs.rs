//! Power cuts of the engine's own renamed files (ADR 0002, Linux): the
//! Goodfetch home is mounted through LazyFS, a file is written through the
//! engine's interface, LazyFS drops every write that was never synced, and
//! the home is mounted again, as after a reboot. The Audit log's head file
//! then holds what was written. LazyFS 0.3.1 passes a
//! rename straight to its backing folder and has no folder sync, so these
//! tests show a file's own sync; the folder's sync after the rename is the
//! engine's `durable` module's. Only the Release suite runs them, on Linux
//! x64 (`just lazyfs-test`); nextest's profiles leave them out elsewhere.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the files back as a reboot would, outside any async worker"
)]

use goodfetch_engine::audit::{AuditLog, Origin};
use goodfetch_engine::home::Home;
use goodfetch_protocol::audit::{Actor, AuditVerified, SECRET_REFERENCE_USED};
use goodfetch_protocol::connection::Surface;
use goodfetch_test_support::{LazyFs, TestEnv};
use serde_json::json;

/// Entries recorded before a power cut still verify after it: the head file
/// that names the last of them was synced before `record` answered.
#[test]
fn an_audit_log_recorded_on_a_mounted_home_verifies_intact_after_clear_cache() {
    let env = TestEnv::for_lazyfs();
    let home = Home::new(env.home().to_path_buf());
    let mounted = LazyFs::mount(&env, env.home()).unwrap();
    let origin = Origin {
        actor: Actor::ClientSurface {
            surface: Surface::Cli,
        },
        surface: None,
        layer: None,
        run_id: None,
    };
    for index in 1..=2 {
        AuditLog::of(&home, None)
            .record(
                &origin,
                &SECRET_REFERENCE_USED,
                json!({ "reference": format!("env:SECRET_{index}"), "domain": "example.com" }),
                "2026-01-05T09:00:00Z".parse().unwrap(),
            )
            .unwrap();
    }

    mounted.clear_cache().unwrap();
    drop(mounted);
    let _remounted = LazyFs::mount(&env, env.home()).unwrap();

    assert_eq!(
        AuditLog::of(&home, None).verify().unwrap(),
        AuditVerified {
            intact: true,
            entries: 2,
            first_broken: None,
        }
    );
}
