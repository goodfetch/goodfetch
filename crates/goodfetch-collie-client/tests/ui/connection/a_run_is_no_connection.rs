// A helper typed for the Connection both Products' connections implement
// (ADRs 0095 and 0099) takes no other value: a Run is no Connection.
use goodfetch_collie_client::{Connection, Run};

fn start_on(_connection: &impl Connection) {}

fn helper(run: &Run) {
    start_on(run);
}

fn main() {}
