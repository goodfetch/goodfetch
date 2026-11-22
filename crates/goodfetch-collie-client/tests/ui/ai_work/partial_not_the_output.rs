// A value of a declared type is complete (ADRs 0079 and 0117): a stopped
// task's partial answer is typed as partial, not as the declared output,
// so a program cannot take it for the output.
use goodfetch_collie_client::Ended;

fn answer(ended: Ended<String>) -> String {
    match ended {
        Ended::Finished { output } => output,
        Ended::Stopped { partial, .. } => partial,
    }
}

fn main() {
    let _ = answer;
}
