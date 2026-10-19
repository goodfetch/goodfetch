//! Covers: story:14-15, story:14-17, story:14-18, story:14-21
//!
//! One case per row of ADR 0096's table, a case per kind of name only
//! Foxhound carries, and known Run IDs. The Foxhound names are those of the
//! generated table; the Collie names are names it does not hold.

use rstest::rstest;

use super::{RunStart, Starter, Target, product_for_run};
use crate::catalog::Product;

const COLLIE_CLIENT: Starter = Starter::Client {
    needs: Product::Collie,
};
const FOXHOUND_CLIENT: Starter = Starter::Client {
    needs: Product::Foxhound,
};

const NOTHING: Target<'static> = Target {
    operations: &[],
    setting_keys: &[],
    plugin_kinds: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
const FOXHOUND_OPERATION: Target<'static> = Target {
    operations: &["sample.echo", "sample.crawl"],
    ..NOTHING
};
const COLLIE_NAMES_ONLY: Target<'static> = Target {
    operations: &["sample.echo"],
    plugin_kinds: &["sample_store"],
    ..NOTHING
};

fn new_run(
    starter: Starter,
    recipe: Option<Product>,
    target: Target<'static>,
) -> RunStart<'static> {
    RunStart {
        known_run: None,
        starter,
        recipe,
        target,
    }
}

#[rstest]
// Row 1: a Recipe that names Foxhound, whoever starts it.
#[case::foxhound_recipe_from_a_collie_client(
    COLLIE_CLIENT,
    Some(Product::Foxhound),
    NOTHING,
    Product::Foxhound
)]
#[case::foxhound_recipe_from_the_console(
    Starter::Console,
    Some(Product::Foxhound),
    NOTHING,
    Product::Foxhound
)]
#[case::foxhound_recipe_from_a_schedule(
    Starter::Schedule,
    Some(Product::Foxhound),
    NOTHING,
    Product::Foxhound
)]
#[case::foxhound_recipe_from_goodfetch_cloud(
    Starter::GoodfetchCloud,
    Some(Product::Foxhound),
    NOTHING,
    Product::Foxhound
)]
// Row 2: a Client's run gets the Product its `hello` names.
#[case::collie_client_with_no_recipe(COLLIE_CLIENT, None, NOTHING, Product::Collie)]
#[case::foxhound_client_with_no_recipe(FOXHOUND_CLIENT, None, NOTHING, Product::Foxhound)]
#[case::foxhound_client_starting_a_collie_recipe(
    FOXHOUND_CLIENT,
    Some(Product::Collie),
    NOTHING,
    Product::Foxhound
)]
#[case::collie_client_naming_a_foxhound_operation(
    COLLIE_CLIENT,
    None,
    FOXHOUND_OPERATION,
    Product::Collie
)]
// Row 3: a Recipe that names Collie, started by the Console, a Schedule or Goodfetch Cloud.
#[case::collie_recipe_naming_a_foxhound_operation_from_the_console(
    Starter::Console,
    Some(Product::Collie),
    FOXHOUND_OPERATION,
    Product::Collie
)]
#[case::collie_recipe_from_a_schedule(
    Starter::Schedule,
    Some(Product::Collie),
    NOTHING,
    Product::Collie
)]
// Row 4: no Recipe, started by the Console, a Schedule or Goodfetch Cloud.
#[case::console_target_naming_a_foxhound_operation(
    Starter::Console,
    None,
    FOXHOUND_OPERATION,
    Product::Foxhound
)]
#[case::schedule_target_naming_a_foxhound_operation(
    Starter::Schedule,
    None,
    FOXHOUND_OPERATION,
    Product::Foxhound
)]
#[case::goodfetch_cloud_target_naming_a_foxhound_operation(
    Starter::GoodfetchCloud,
    None,
    FOXHOUND_OPERATION,
    Product::Foxhound
)]
#[case::console_target_naming_only_collies_names(
    Starter::Console,
    None,
    COLLIE_NAMES_ONLY,
    Product::Collie
)]
#[case::goodfetch_cloud_target_naming_nothing(
    Starter::GoodfetchCloud,
    None,
    NOTHING,
    Product::Collie
)]
fn new_run_lands_on_the_product_the_table_gives(
    #[case] starter: Starter,
    #[case] recipe: Option<Product>,
    #[case] target: Target<'static>,
    #[case] expected: Product,
) {
    let start = new_run(starter, recipe, target);

    let product = product_for_run(&start);

    assert_eq!(product, expected);
}

#[rstest]
#[case::collie_run_started_again_with_a_foxhound_recipe(
    Product::Collie,
    Starter::Console,
    Some(Product::Foxhound),
    NOTHING
)]
#[case::collie_run_started_again_by_a_foxhound_client(
    Product::Collie,
    FOXHOUND_CLIENT,
    None,
    NOTHING
)]
#[case::collie_run_started_again_naming_a_foxhound_operation(
    Product::Collie,
    Starter::Schedule,
    None,
    FOXHOUND_OPERATION
)]
#[case::foxhound_run_started_again_by_a_collie_client(
    Product::Foxhound,
    COLLIE_CLIENT,
    None,
    NOTHING
)]
#[case::foxhound_run_started_again_with_a_collie_recipe(
    Product::Foxhound,
    Starter::Console,
    Some(Product::Collie),
    NOTHING
)]
fn known_run_id_goes_to_its_runs_own_product(
    #[case] known_run: Product,
    #[case] starter: Starter,
    #[case] recipe: Option<Product>,
    #[case] target: Target<'static>,
) {
    let start = RunStart {
        known_run: Some(known_run),
        ..new_run(starter, recipe, target)
    };

    let product = product_for_run(&start);

    assert_eq!(product, known_run);
}
