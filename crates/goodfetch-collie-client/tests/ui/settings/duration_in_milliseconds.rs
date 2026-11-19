// A duration setting takes a `Duration`, never a bare number whose unit a
// reader guesses (ADRs 0080 and 0083).
use goodfetch_collie_client::settings::{Settings, timeout};

fn main() {
    let _settings = Settings::new().set(timeout::NAVIGATION, 30_000_u64);
}
