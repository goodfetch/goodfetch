use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::bail;

use super::at_once;

/// Once the base's build fails, the head's starts no further command, and
/// the build fails with the base's failure alone: the head stopped, it did
/// not fail.
#[test]
fn once_one_build_fails_the_other_starts_no_further_command() {
    let result = at_once(
        |_| -> anyhow::Result<()> { bail!("building the base's goodfetch-frontier/heap failed") },
        |builds| {
            let started = Instant::now();
            let mut commands = 0;
            while builds
                .run(
                    "head",
                    &mut Command::new("true"),
                    "building the head's target",
                )
                .is_ok()
            {
                commands += 1;
                if started.elapsed() > Duration::from_secs(10) {
                    bail!("the head ran {commands} commands after the base failed");
                }
            }
            Ok(())
        },
    );

    let error = result.unwrap_err();
    assert_eq!(
        format!("{error:#}"),
        "building the base's goodfetch-frontier/heap failed"
    );
}
