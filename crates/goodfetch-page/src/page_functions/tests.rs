use rstest::rstest;
use serde_json::json;

use super::init_script;

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
