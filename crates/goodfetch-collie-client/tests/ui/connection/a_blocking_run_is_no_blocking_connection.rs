// The blocking twin refuses it too: a blocking Run is no blocking
// Connection (ADRs 0081 and 0099).
use goodfetch_collie_client::blocking::{Connection, Run};

fn start_on(_connection: &impl Connection) {}

fn helper(run: &Run) {
    start_on(run);
}

fn main() {}
