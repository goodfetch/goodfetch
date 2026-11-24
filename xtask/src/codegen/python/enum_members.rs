//! Each closed value union that holds a value `StrEnum` names itself, such
//! as `value`, written so that its member takes the name with a trailing
//! underscore, `value_`, its value unchanged.
//!
//! `datamodel-code-generator` writes a closed `StrEnum` for a `oneOf` of
//! constants, each member named as its value, so a member `value` would
//! hide `Enum.value`. It names the members of an `enum` by its
//! `x-enum-varnames` and documents them by its `x-enum-descriptions`.

use serde_json::{Map, Value};

/// The names a `StrEnum` member cannot take: `Enum`'s own and `str`'s
/// methods, which a member of that name would hide.
const TAKEN: [&str; 49] = [
    "name",
    "value",
    "capitalize",
    "casefold",
    "center",
    "count",
    "encode",
    "endswith",
    "expandtabs",
    "find",
    "format",
    "format_map",
    "index",
    "isalnum",
    "isalpha",
    "isascii",
    "isdecimal",
    "isdigit",
    "isidentifier",
    "islower",
    "isnumeric",
    "isprintable",
    "isspace",
    "istitle",
    "isupper",
    "join",
    "ljust",
    "lower",
    "lstrip",
    "maketrans",
    "partition",
    "removeprefix",
    "removesuffix",
    "replace",
    "rfind",
    "rindex",
    "rjust",
    "rpartition",
    "rsplit",
    "rstrip",
    "split",
    "splitlines",
    "startswith",
    "strip",
    "swapcase",
    "title",
    "translate",
    "upper",
    "zfill",
];

/// `input`, a generator input, with each definition that is a `oneOf` of
/// text constants, one of them a name in [`TAKEN`], as an `enum` whose
/// member of that name takes a trailing underscore; its own keywords, such
/// as its description, kept.
pub(super) fn with_free_member_names(input: &mut Value) {
    let Some(definitions) = input.get_mut("$defs").and_then(Value::as_object_mut) else {
        return;
    };
    for schema in definitions.values_mut() {
        let Some(object) = schema.as_object_mut() else {
            continue;
        };
        let Some(values) = values(object) else {
            continue;
        };
        if !values
            .iter()
            .any(|(value, _)| TAKEN.contains(&value.as_str()))
        {
            continue;
        }
        object.remove("oneOf");
        let (names, descriptions): (Vec<Value>, Vec<Value>) = values
            .iter()
            .map(|(value, description)| {
                let name = if TAKEN.contains(&value.as_str()) {
                    format!("{value}_")
                } else {
                    value.clone()
                };
                (Value::from(name), Value::from(description.clone()))
            })
            .unzip();
        object.insert("type".to_owned(), Value::from("string"));
        object.insert(
            "enum".to_owned(),
            values
                .into_iter()
                .map(|(value, _)| Value::from(value))
                .collect(),
        );
        object.insert("x-enum-varnames".to_owned(), Value::Array(names));
        object.insert("x-enum-descriptions".to_owned(), Value::Array(descriptions));
    }
}

/// The values of a `oneOf` of text constants, each with its description,
/// or `None` for any other schema.
fn values(object: &Map<String, Value>) -> Option<Vec<(String, String)>> {
    object
        .get("oneOf")?
        .as_array()?
        .iter()
        .map(|member| {
            let value = member.get("const")?.as_str()?.to_owned();
            let description = member
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Some((value, description))
        })
        .collect()
}

#[cfg(test)]
mod tests;
