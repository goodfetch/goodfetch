// A setting whose values are a shape the protocol defines takes the
// protocol's type, never untyped JSON whose fields go unchecked (ADR 0083).
use goodfetch_collie_client::settings::{Settings, session};

fn main() {
    let _settings = Settings::new().set(
        session::PAGE_CHECK,
        serde_json::json!({ "login_url": "/login" }),
    );
}
