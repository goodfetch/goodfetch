//! The engine's own operations, `engine.*`, `run.*` and `config.*`, which
//! every Product's composition lists first.

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo, EngineInfoParams};

use crate::composition::{CallContext, CallError, Contribution, Handler};
use crate::{config, run};

/// What the engine adds to every composition.
pub fn contribution() -> Contribution {
    run::handlers().into_iter().chain(config::handlers()).fold(
        Contribution::new("goodfetch-engine").with_handler(Handler::new(&ENGINE_INFO, engine_info)),
        Contribution::with_handler,
    )
}

async fn engine_info(_: EngineInfoParams, context: CallContext) -> Result<EngineInfo, CallError> {
    let identity = context.identity();
    Ok(EngineInfo {
        product: identity.product,
        product_version: identity.product_version.clone(),
        protocol_version: PROTOCOL_VERSION,
        product_major: identity.product_version.major(),
    })
}
