use goodfetch_protocol::catalog::{OperationKind, Product};
use goodfetch_protocol::entries::operations_of;

use super::composition;

#[test]
fn foxhounds_composition_has_exactly_one_handler_for_each_operation_of_foxhounds_catalog() {
    let mut handled: Vec<&str> = composition().operations().collect();
    // A read sends no message, so the engine has no handler for it (issue
    // 2798).
    let mut catalog: Vec<&str> = operations_of(Product::Foxhound)
        .filter(|op| op.kind != OperationKind::Read)
        .map(|op| op.name)
        .collect();

    handled.sort_unstable();
    catalog.sort_unstable();
    assert_eq!(handled, catalog);
}

#[test]
fn foxhounds_composition_contains_all_of_collies() {
    let collie = goodfetch_collie::composition();
    let foxhound = composition();

    let collie_contributions: Vec<&str> = collie.contributions().collect();
    let foxhound_contributions: Vec<&str> = foxhound
        .contributions()
        .take(collie_contributions.len())
        .collect();
    let missing: Vec<&str> = collie
        .operations()
        .filter(|operation| !foxhound.operations().any(|handled| handled == *operation))
        .collect();
    assert_eq!(
        (foxhound_contributions, missing),
        (collie_contributions, vec![])
    );
}
