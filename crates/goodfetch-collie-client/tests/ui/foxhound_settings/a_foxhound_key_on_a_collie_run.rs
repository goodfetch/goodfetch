// A setting only Foxhound carries is no Collie key (ADR 0095): Collie's
// `politeness` has no `domain`, so a Collie Run's settings cannot name
// `politeness.domain.concurrency`.
use goodfetch_collie_client::settings::{Settings, politeness};

fn main() {
    let _settings = Settings::new().set(politeness::domain::CONCURRENCY, 2);
}
