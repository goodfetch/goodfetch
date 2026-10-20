//! Every module's entries, in one list: what the emitter writes into the
//! catalog, and what each Product's catalog holds, by the Product mark of
//! where each entry is defined (ADRs 0095 and 0099).

use crate::catalog::{Entries, Operation, Product};
use crate::{browser_cache, connection, engine, foxhound, run, sample, settings};

/// Every module that defines entries.
pub const MODULES: [&Entries; 9] = [
    &connection::ENTRIES,
    &browser_cache::ENTRIES,
    &engine::ENTRIES,
    &run::ENTRIES,
    &sample::ENTRIES,
    &settings::ENTRIES,
    &foxhound::ENTRIES,
    &foxhound::crawl::ENTRIES,
    &foxhound::settings::ENTRIES,
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
                "engine.info",
                "run.start",
                "run.finish",
                "run.abort",
                "sample.echo",
                "config.explain"
            ]
        );
        assert_eq!(
            foxhound,
            [
                "engine.info",
                "run.start",
                "run.finish",
                "run.abort",
                "sample.echo",
                "config.explain",
                "sample.crawl",
                "crawl.start"
            ]
        );
    }
}
