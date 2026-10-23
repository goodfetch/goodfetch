//! Every module's entries, in one list: what the emitter writes into the
//! catalog, and what each Product's catalog holds, by the Product mark of
//! where each entry is defined (ADRs 0095 and 0099).

use crate::catalog::{Entries, ErrorCodeDefinition, Operation, Product, SharedType};
use crate::{
    audit, block, browser_cache, checkpoint, connection, console, daemon, dns, egress, engine,
    events, executables, foxhound, governor, key_value_store, log, ops, page, people, politeness,
    run, sample, secrets, settings, storage, tls, usage, versions,
};

/// Every module that defines entries.
pub const MODULES: [&Entries; 37] = [
    &connection::ENTRIES,
    &log::ENTRIES,
    &console::ENTRIES,
    &daemon::ENTRIES,
    &ops::ENTRIES,
    &governor::ENTRIES,
    &storage::ENTRIES,
    &browser_cache::ENTRIES,
    &dns::ENTRIES,
    &egress::ENTRIES,
    &tls::ENTRIES,
    &engine::ENTRIES,
    &events::ENTRIES,
    &page::ENTRIES,
    &page::proxy::ENTRIES,
    &executables::ENTRIES,
    &run::ENTRIES,
    &checkpoint::ENTRIES,
    &key_value_store::ENTRIES,
    &audit::ENTRIES,
    &people::ENTRIES,
    &secrets::ENTRIES,
    &sample::ENTRIES,
    &settings::ENTRIES,
    &usage::ENTRIES,
    &versions::ENTRIES,
    &politeness::ENTRIES,
    &block::ENTRIES,
    &foxhound::ENTRIES,
    &foxhound::crawl::ENTRIES,
    &foxhound::frontier::ENTRIES,
    &foxhound::outcomes::ENTRIES,
    &foxhound::report::ENTRIES,
    &foxhound::session_pool::ENTRIES,
    &foxhound::settings::ENTRIES,
    &foxhound::block::ENTRIES,
    &foxhound::usage::ENTRIES,
];

/// The operations of `product`'s catalog: Collie's own for Collie, every
/// operation for Foxhound, which contains Collie.
pub fn operations_of(product: Product) -> impl Iterator<Item = &'static Operation> {
    MODULES
        .into_iter()
        .filter(move |entries| product.contains(Product::of_module(entries.defined_in)))
        .flat_map(|entries| entries.operations)
}

/// The Product that carries the value `name` of `shared_type`, such as the
/// Usage kind `requests`; none for a value neither Product knows, which a
/// newer engine wrote.
pub fn product_of_value(shared_type: SharedType, name: &str) -> Option<Product> {
    MODULES.into_iter().find_map(|entries| {
        entries
            .shared_type_values
            .iter()
            .any(|value| value.shared_type == shared_type && value.name == name)
            .then(|| Product::of_module(entries.defined_in))
    })
}

/// The Error code named `name`, whichever module defines it; none for a
/// code neither Product knows.
pub fn error_code(name: &str) -> Option<&'static ErrorCodeDefinition> {
    MODULES
        .into_iter()
        .flat_map(|entries| entries.error_codes)
        .find(|code| code.name == name)
}

#[cfg(test)]
mod tests {
    use super::operations_of;
    use crate::catalog::Product;

    /// Collie's operations, in the catalog's order.
    const COLLIE: [&str; 57] = [
        "connection.set_log_level",
        "daemon.status",
        "daemon.stop",
        "daemon.health",
        "certs.list",
        "certs.accept",
        "certs.forget",
        "engine.info",
        "events.subscribe",
        "run.new_page",
        "run.new_session",
        "session.new_page",
        "session.close",
        "page.goto",
        "page.reload",
        "page.go_back",
        "page.go_forward",
        "page.formats",
        "page.body",
        "locator.click",
        "locator.fill",
        "locator.press",
        "page.route",
        "session.route",
        "route.continue",
        "route.fallback",
        "route.fulfill",
        "route.abort",
        "route.fetch",
        "page.on",
        "response.body",
        "page.wait_for_request_finished",
        "page.request",
        "run.start",
        "run.finish",
        "run.stop",
        "run.abort",
        "run.usage",
        "run.set_progress_message",
        "run.discard",
        "run.checkpoint",
        "run.pause",
        "run.migrate",
        "run.resume",
        "key_value_store.get",
        "key_value_store.set",
        "key_value_store.delete",
        "key_value_store.keys",
        "audit.verify",
        "sign_in.list",
        "sign_in.revoke",
        "console.rotate_key",
        "sample.echo",
        "config.explain",
        "usage.list",
        "usage.set",
        "usage.raise",
    ];

    #[test]
    fn foxhounds_catalog_holds_collies_and_its_own_operations() {
        let collie: Vec<&str> = operations_of(Product::Collie).map(|op| op.name).collect();
        let foxhound: Vec<&str> = operations_of(Product::Foxhound).map(|op| op.name).collect();

        assert_eq!(collie, COLLIE);
        assert_eq!(
            foxhound,
            [
                &COLLIE[..],
                &[
                    "sample.crawl",
                    "crawl.start",
                    "crawl.serve",
                    "run.seeds",
                    "run.add_seed",
                    "run.add_seeds",
                    "crawl_request.enqueue",
                    "run.outcomes",
                    "run.failed_requests",
                ],
            ]
            .concat()
        );
    }
}
