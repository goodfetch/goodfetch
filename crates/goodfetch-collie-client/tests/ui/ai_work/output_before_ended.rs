// AI work's answer sits in how it ended (ADR 0117): the result has no
// `output` of its own, so a program cannot read one without first meeting
// the early end that has none.
use goodfetch_collie_client::AiWorkResult;

fn answer(result: AiWorkResult<String>) -> String {
    result.output
}

fn main() {
    let _ = answer;
}
