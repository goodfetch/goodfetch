// Only a setting that can be cleared takes `None` (ADR 0083):
// `timeout.navigation` always has a value.
use goodfetch_collie_client::settings::{Settings, timeout};

fn main() {
    let _settings = Settings::new().set(timeout::NAVIGATION, None);
}
