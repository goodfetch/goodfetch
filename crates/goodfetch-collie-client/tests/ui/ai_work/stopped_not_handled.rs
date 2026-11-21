// `Ended` is exhaustive (ADR 0117): a `match` that handles only the
// finished work does not compile, so a stopped task's partial answer is
// never skipped.
use goodfetch_collie_client::Ended;

fn answer(ended: Ended<String>) -> String {
    match ended {
        Ended::Finished { output } => output,
    }
}

fn main() {
    let _ = answer;
}
