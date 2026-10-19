//! The handler of Collie's sample operation, `sample.echo`, which carries the
//! catalog's pipeline until the areas that own each contract replace it
//! (spec #130); it goes with the sample entries.

use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::sample::{ENTRIES, SampleEchoParams, SampleEchoResult};

/// The sample operations Collie carries.
pub(crate) fn contribution() -> Contribution {
    let echo = ENTRIES
        .operations
        .iter()
        .find(|operation| operation.name == "sample.echo")
        .expect("invariant: the sample entries hold `sample.echo`");
    Contribution::new("sample").with_handler(Handler::new(echo, echo_text))
}

async fn echo_text(
    params: SampleEchoParams,
    _: CallContext,
) -> Result<SampleEchoResult, CallError> {
    Ok(SampleEchoResult { text: params.text })
}
