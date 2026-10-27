//! The rewrite of model-typed dict defaults, as a function of the text
//! `datamodel-code-generator` writes.

use super::rewrite;

const MODELS: &str =
    "class Mode(RootModel[Plain | Deep]):\n    root: Plain | Deep\n\n\nclass Sample(BaseModel):\n";

/// A model-typed field's dict default, on one line or wrapped, plain or
/// `Annotated`, is constructed through a factory; a map's dict default, a
/// list default and a scalar one are left as written, and a file with none
/// is not rewritten.
#[test]
fn a_model_fields_dict_default_is_constructed_and_nothing_else_is() {
    let file = format!(
        "{MODELS}    mode: Annotated[Mode | None, Field(validate_default=True)] = {{\n        \"kind\": \"plain\"\n    }}\n    \"\"\"How.\"\"\"\n    other: Mode | None = {{\"kind\": \"deep\", \"depth\": 1}}\n    tags: dict[str, str] | None = {{\"kind\": \"sample\"}}\n    seen: list[Mode] | None = []\n    pages: int | None = 3\n"
    );

    assert_eq!(
        rewrite(&file),
        Some(format!(
            "{MODELS}    mode: Annotated[Mode | None, Field(validate_default=True)] = Field(default_factory=lambda: Mode.model_validate({{\n        \"kind\": \"plain\"\n    }}))\n    \"\"\"How.\"\"\"\n    other: Mode | None = Field(default_factory=lambda: Mode.model_validate({{\"kind\": \"deep\", \"depth\": 1}}))\n    tags: dict[str, str] | None = {{\"kind\": \"sample\"}}\n    seen: list[Mode] | None = []\n    pages: int | None = 3\n"
        ))
    );
    assert_eq!(
        rewrite(&format!("{MODELS}    pages: int | None = 3\n")),
        None
    );
}

/// A type the file does not define as a model, though written like one,
/// keeps its dict.
#[test]
fn a_type_the_file_does_not_define_keeps_its_dict() {
    let file = format!("{MODELS}    other: Elsewhere | None = {{\"kind\": \"plain\"}}\n");

    assert_eq!(rewrite(&file), None);
}
