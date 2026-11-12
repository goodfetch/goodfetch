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
const FOXHOUND_SETTING_KEY: Target<'static> = Target {
    setting_keys: &["sample.echo_repeats", "crawl.sample_pages"],
    ..NOTHING
};
const FOXHOUND_PLUGIN_KIND: Target<'static> = Target {
    plugin_kinds: &["sample_store", "sample_page_filter"],
    ..NOTHING
};
const FOXHOUND_HOOK_STAGE: Target<'static> = Target {
    hook_stages: &["sample_before_echo", "sample_crawled_page"],
    ..NOTHING
};
const FOXHOUND_SHARED_TYPE_VALUE: Target<'static> = Target {
    shared_type_values: &["sample_echoes", "sample_crawled_pages"],
    ..NOTHING
};
const COLLIE_NAMES_ONLY: Target<'static> = Target {
    operations: &["sample.echo"],
    setting_keys: &["sample.echo_repeats"],
    plugin_kinds: &["sample_store"],
    hook_stages: &["sample_before_echo"],
    shared_type_values: &["sample_echoes"],
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
// Row 3: a Recipe that names Collie, started by a Schedule or Goodfetch Cloud.
#[case::collie_recipe_naming_a_foxhound_operation_from_a_schedule(
    Starter::Schedule,
    Some(Product::Collie),
    FOXHOUND_OPERATION,
    Product::Collie
)]
#[case::collie_recipe_setting_a_crawl_key_from_a_schedule(
    Starter::Schedule,
    Some(Product::Collie),
    FOXHOUND_SETTING_KEY,
    Product::Collie
)]
#[case::collie_recipe_from_a_schedule(
    Starter::Schedule,
    Some(Product::Collie),
    NOTHING,
    Product::Collie
)]
// Row 4: no Recipe, started by a Schedule or Goodfetch Cloud.
#[case::schedule_target_setting_a_crawl_key(
    Starter::Schedule,
    None,
    FOXHOUND_SETTING_KEY,
    Product::Foxhound
)]
#[case::schedule_target_enabling_a_foxhound_plugin_kind(
    Starter::Schedule,
    None,
    FOXHOUND_PLUGIN_KIND,
    Product::Foxhound
)]
#[case::schedule_target_enabling_a_foxhound_hook_stage(
    Starter::Schedule,
    None,
    FOXHOUND_HOOK_STAGE,
    Product::Foxhound
)]
#[case::schedule_target_using_a_value_foxhound_adds_to_a_shared_type(
    Starter::Schedule,
    None,
    FOXHOUND_SHARED_TYPE_VALUE,
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
#[case::schedule_target_naming_only_collies_names(
    Starter::Schedule,
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
    Starter::Schedule,
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
    Starter::Schedule,
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
