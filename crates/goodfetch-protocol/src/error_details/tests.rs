use std::collections::BTreeSet;

use crate::catalog::Product;
use crate::entries::MODULES;
use crate::error_code::{ErrorCode, ErrorDetails};
use crate::foxhound::error_code as foxhound;
use crate::hook::HookFailedDetails;

/// `PascalCase` to `snake_case`.
fn snake(pascal: &str) -> String {
    pascal
        .chars()
        .enumerate()
        .fold(String::new(), |mut snake, (at, letter)| {
            if letter.is_ascii_uppercase() && at > 0 {
                snake.push('_');
            }
            snake.push(letter.to_ascii_lowercase());
            snake
        })
}

/// The hand-written table's lines, `<path>::<Code>Details => <path>::<CODE>`.
fn table() -> Vec<(&'static str, &'static str)> {
    include_str!("../error_details.rs")
        .lines()
        .filter_map(|line| line.trim().strip_suffix(',')?.split_once(" => "))
        .collect()
}

#[test]
fn each_details_type_names_the_code_it_is_named_for() {
    let table = table();

    assert!(table.len() > 50, "the table has {} lines", table.len());
    for (path, code) in table {
        let name = path.rsplit("::").next().unwrap();
        let named_for = snake(name.strip_suffix("Details").unwrap()).to_uppercase();
        assert_eq!(code.rsplit("::").next().unwrap(), named_for, "{path}");
    }
}

#[test]
fn a_foxhound_code_is_a_value_of_the_same_type() {
    let read = ErrorCode::new("handler_failed").unwrap();

    assert_eq!(read, foxhound::HANDLER_FAILED);
    assert_ne!(read, ErrorCode::HOOK_FAILED);
}

/// Covers: story:11-28, story:11-37, story:11-190
#[test]
fn every_code_of_each_product_has_its_constant() {
    let declared = |product: Product| -> BTreeSet<&str> {
        MODULES
            .into_iter()
            .filter(|entries| Product::of_module(entries.defined_in) == product)
            .flat_map(|entries| entries.error_codes)
            .map(|code| code.name)
            .collect()
    };
    let constants = |codes: &[ErrorCode]| -> BTreeSet<String> {
        codes.iter().map(|code| code.as_str().to_owned()).collect()
    };

    assert_eq!(
        constants(ErrorCode::COLLIE),
        declared(Product::Collie)
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert_eq!(
        constants(foxhound::ALL),
        declared(Product::Foxhound)
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
}

#[test]
fn a_details_type_names_its_code() {
    assert_eq!(HookFailedDetails::CODES, [ErrorCode::HOOK_FAILED]);
}
