use goodfetch_protocol::connection::Form;
use goodfetch_protocol::page::functions::{LOCATOR_EVALUATE_ALL, PAGE_EVALUATE};
use goodfetch_protocol::page::waits::{LOCATOR_WAIT_FOR_FUNCTION, PAGE_WAIT_FOR_FUNCTION};
use rstest::rstest;
use serde_json::json;

use super::{Thrown, error_message, failed, init_script};

/// A source that is one function, comments and a closing `;` around it
/// allowed, is called with its argument: it takes one, which the script
/// sent passes it.
#[rstest]
#[case::arrow("() => { window.mark = 1; }")]
#[case::arrow_with_one_parameter("mark => { window.mark = mark; }")]
#[case::arrow_with_an_expression_body("(mark) => (window.mark = mark)")]
#[case::function("function () { window.mark = 1; }")]
#[case::named_async_function("async function patch(mark) { window.mark = mark; }")]
#[case::generator("function* marks() { yield 1; }")]
#[case::comment_before("// marks the window\n() => { window.mark = 1; }")]
#[case::block_comment_before("/* marks */ async () => { window.mark = 1; }")]
#[case::comment_and_semicolon_after("() => { window.mark = 1; }; // done")]
#[case::braces_in_strings_and_comments(
    "function () { const close = '}'; /* } */ window.mark = `${close}`; }"
)]
fn a_function_is_called_with_its_argument(#[case] source: &str) {
    let script = init_script(source, Some(&json!({ "mark": "passed" }))).ok();
    assert!(
        script
            .as_deref()
            .is_some_and(|script| script.contains(r#"{\"mark\":\"passed\"}"#)),
        "{source:?} gave {script:?}"
    );
}

/// A source that is more than one function, or none, runs as it is, and
/// is refused an argument.
#[rstest]
#[case::statement("window.mark = 1;")]
#[case::declaration_then_call("function patch() { window.mark = 1; }\npatch();")]
#[case::declaration_then_call_on_one_line("function patch() { window.mark = 1; }; patch();")]
#[case::called_at_once("(() => { window.mark = 1; })();")]
#[case::arrow_then_statement("() => 1; window.mark = 1;")]
#[case::identifier_statement("patch();")]
fn a_script_runs_as_it_is_and_takes_no_argument(#[case] source: &str) {
    assert_eq!(init_script(source, None).ok().as_deref(), Some(source));
    assert!(init_script(source, Some(&json!(1))).is_err());
}

/// An error's message is its description's first line past its name, with
/// or without one, and none for the name alone.
#[rstest]
#[case::named(
    Some("TypeError"),
    "TypeError: the page said no\n    at <anonymous>:1:7",
    "the page said no"
)]
#[case::name_alone(Some("Error"), "Error\n    at <anonymous>:1:7", "")]
#[case::other_name(
    Some("RangeError"),
    "SyntaxError: Unexpected token",
    "SyntaxError: Unexpected token"
)]
#[case::no_name(None, "Uncaught 42", "Uncaught 42")]
fn an_errors_message_is_its_first_line_past_its_name(
    #[case] name: Option<&str>,
    #[case] description: &str,
    #[case] message: &str,
) {
    assert_eq!(error_message(name, description), message);
}

/// A `ReferenceError`'s fix spells the call of the operation that ran,
/// in the form the program declares: a locator's page function takes its
/// matches first, so the fix's function takes the argument second.
#[rstest]
#[case::page_evaluate(PAGE_EVALUATE.name, Form::Typescript, "page.evaluate((x) => …, x)")]
#[case::page_evaluate_python(PAGE_EVALUATE.name, Form::Python, "page.evaluate(\"(x) => …\", x)")]
#[case::locator_evaluate_all(
    LOCATOR_EVALUATE_ALL.name,
    Form::Typescript,
    "locator.evaluateAll((elements, x) => …, x)"
)]
#[case::locator_evaluate_all_python(
    LOCATOR_EVALUATE_ALL.name,
    Form::Python,
    "locator.evaluate_all(\"(elements, x) => …\", x)"
)]
#[case::page_wait_for_function(
    PAGE_WAIT_FOR_FUNCTION.name,
    Form::Typescript,
    "page.waitForFunction((x) => …, x)"
)]
#[case::page_wait_for_function_python(
    PAGE_WAIT_FOR_FUNCTION.name,
    Form::Python,
    "page.wait_for_function(\"(x) => …\", x)"
)]
#[case::locator_wait_for_function(
    LOCATOR_WAIT_FOR_FUNCTION.name,
    Form::Typescript,
    "locator.waitForFunction((element, x) => …, x)"
)]
#[case::locator_wait_for_function_python(
    LOCATOR_WAIT_FOR_FUNCTION.name,
    Form::Python,
    "locator.wait_for_function(\"(element, x) => …\", x)"
)]
fn a_reference_errors_fix_spells_the_call_of_its_operation(
    #[case] operation: &'static str,
    #[case] form: Form,
    #[case] call: &str,
) {
    let thrown = Thrown {
        name: Some("ReferenceError".to_owned()),
        message: "x is not defined".to_owned(),
        stack: None,
    };

    let fix = failed(thrown, (form, operation))
        .details()
        .and_then(|details| details["fix"].as_str().map(ToOwned::to_owned))
        .unwrap_or_default();

    assert!(
        fix.contains("`x`") && fix.contains(&format!("`{call}`")),
        "{fix}"
    );
}
