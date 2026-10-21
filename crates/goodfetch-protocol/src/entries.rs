//! Every module's entries, in one list: what the emitter writes into the
//! catalog, and what each Product's catalog holds, by the Product mark of
//! where each entry is defined (ADRs 0095 and 0099).

use crate::catalog::{Entries, Operation, Product};
use crate::{
    block, browser_cache, checkpoint, connection, console, daemon, dns, engine, events,
    executables, foxhound, page, run, sample, settings,
};

/// Every module that defines entries.
pub const MODULES: [&Entries; 20] = [
    &connection::ENTRIES,
    &console::ENTRIES,
    &daemon::ENTRIES,
    &browser_cache::ENTRIES,
    &dns::ENTRIES,
    &engine::ENTRIES,
    &events::ENTRIES,
    &page::ENTRIES,
    &executables::ENTRIES,
    &run::ENTRIES,
    &checkpoint::ENTRIES,
    &sample::ENTRIES,
    &settings::ENTRIES,
    &block::ENTRIES,
    &foxhound::ENTRIES,
    &foxhound::crawl::ENTRIES,
    &foxhound::frontier::ENTRIES,
    &foxhound::outcomes::ENTRIES,
    &foxhound::settings::ENTRIES,
    &foxhound::block::ENTRIES,
];

/// The operations of `product`'s catalog: Collie's own for Collie, every
/// operation for Foxhound, which contains Collie.
pub fn operations_of(product: Product) -> impl Iterator<Item = &'static Operation> {
    MODULES
        .into_iter()
        .filter(move |entries| product.contains(Product::of_module(entries.defined_in)))
        .flat_map(|entries| entries.operations)
}

#[cfg(test)]
mod tests {
    use super::operations_of;
    use crate::catalog::Product;

    #[test]
    fn foxhounds_catalog_holds_collies_and_its_own_operations() {
        let collie: Vec<&str> = operations_of(Product::Collie).map(|op| op.name).collect();
        let foxhound: Vec<&str> = operations_of(Product::Foxhound).map(|op| op.name).collect();

        assert_eq!(
            collie,
            [
                "daemon.status",
                "daemon.stop",
                "engine.info",
                "events.subscribe",
                "run.new_page",
                "page.goto",
                "page.reload",
                "page.go_back",
                "page.go_forward",
                "page.body",
                "run.start",
                "run.finish",
                "run.abort",
                "run.checkpoint",
                "run.pause",
                "run.migrate",
                "run.resume",
                "sample.echo",
                "config.explain"
            ]
        );
        assert_eq!(
            foxhound,
            [
                "daemon.status",
                "daemon.stop",
                "engine.info",
                "events.subscribe",
                "run.new_page",
                "page.goto",
                "page.reload",
                "page.go_back",
                "page.go_forward",
                "page.body",
                "run.start",
                "run.finish",
                "run.abort",
                "run.checkpoint",
                "run.pause",
                "run.migrate",
                "run.resume",
                "sample.echo",
                "config.explain",
                "sample.crawl",
                "crawl.start",
                "crawl.serve",
                "run.seeds",
                "crawl_request.enqueue",
                "run.outcomes",
                "run.failed_requests"
            ]
        );
    }
}
