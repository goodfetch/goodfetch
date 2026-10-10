//! Hello-world napi-rs addon. Install friction does not depend on engine logic.
use napi_derive::napi;

#[napi]
pub fn hello() -> String {
    format!("hello from napi addon ({})", env!("BUILD_TARGET"))
}
