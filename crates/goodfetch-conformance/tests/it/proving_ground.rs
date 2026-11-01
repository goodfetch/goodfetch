//! The local copy of the Proving ground, started for one test from the
//! commit's executable on the test's seed, on port 0 of the loopback
//! address, and stopped when the test ends.

use goodfetch_test_support::{
    PROVING_GROUND_DEADLINE, Process, TestEnv, Tool, reported_address_within, tools,
};

/// A running `serve`.
pub(crate) struct LocalCopy {
    process: Process,
    address: String,
}

impl LocalCopy {
    /// Starts `serve` on `seed`, waiting for its first line as long as test
    /// support's own Proving ground does.
    pub(crate) fn start(env: &TestEnv, seed: u64) -> Self {
        let tools = tools().unwrap();
        let mut command = env.process_command(tools.path(Tool::ProvingGround).unwrap());
        command.args(["serve", "--seed", &seed.to_string(), "--copy", "local"]);
        command.args(["--address", "127.0.0.1:0", "--network-profile", "none"]);
        let process = env.spawn(&mut command).unwrap();
        let address = reported_address_within(&mut process.child(), PROVING_GROUND_DEADLINE)
            .unwrap()
            .address;
        Self { process, address }
    }

    /// The address `serve` reported.
    pub(crate) fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for LocalCopy {
    fn drop(&mut self) {
        // Stopped before the test's teardown checks that nothing is alive.
        let _ = self.process.stop();
    }
}
