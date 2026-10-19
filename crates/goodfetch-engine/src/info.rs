//! The engine's own operations, `engine.*`, which every Product's
//! composition lists first.

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::engine::{ENGINE_INFO, EngineInfo, EngineInfoParams};

use crate::composition::{CallContext, CallError, Contribution, Handler};

/// What the engine adds to every composition.
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-engine").with_handler(Handler::new(&ENGINE_INFO, engine_info))
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
