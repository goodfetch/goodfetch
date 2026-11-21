// A setting whose values are a shape the protocol defines takes the
// protocol's type, so a field it lacks does not compile (ADR 0083).
use goodfetch_collie_client::settings::{Settings, session};
use goodfetch_protocol::settings::session_source::SessionSource;

fn main() {
    let _settings = Settings::new().set(
        session::SOURCE,
        SessionSource::SavedIdentity {
            identity: "shop".to_owned(),
        },
    );
}
