//! A Model provider that answers every call with the same reply, written
//! against the interface and `goodfetch-protocol`'s types alone, as the
//! Scripted model (#406) implements it.

use std::error::Error;

use futures_util::stream::{self, BoxStream, StreamExt as _};
use goodfetch_model_provider::{CallContext, ModelProvider};
use goodfetch_protocol::model::{ModelRequest, ModelResponse, ModelResponsePart, ModelTokens};

/// Answers each call with `reply`.
struct Canned {
    reply: String,
}

impl ModelProvider for Canned {
    fn call<'a>(
        &'a self,
        _request: &'a ModelRequest,
        _context: CallContext<'a>,
    ) -> BoxStream<'a, Result<ModelResponsePart, Box<dyn Error + Send + Sync>>> {
        let response = ModelResponse {
            text: self.reply.clone(),
            tool_calls: Vec::new(),
            finish_reason: "stop".to_owned(),
            refusal: None,
            usage: ModelTokens {
                input_tokens: 12,
                output_tokens: 3,
                ..ModelTokens::default()
            },
        };
        stream::iter([
            Ok(ModelResponsePart::Text {
                text: self.reply.clone(),
            }),
            Ok(ModelResponsePart::Done { response }),
        ])
        .boxed()
    }
}

/// Where a Foxhound feature takes its provider: any `dyn ModelProvider`.
fn uses(_provider: &dyn ModelProvider) {}

fn main() {
    let canned = Canned {
        reply: "Dune.".to_owned(),
    };
    uses(&canned);
}
