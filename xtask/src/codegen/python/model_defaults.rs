//! The defaults `datamodel-code-generator` writes as an object's bare
//! dict (`transform: Annotated[FieldTransform | None, ...] = {"kind":
//! "text"}`), which a type checker rejects for a model-typed field: each is
//! rewritten as a factory that constructs the model from that object,
//! `Field(default_factory=lambda: FieldTransform.model_validate({...}))`,
//! so every instance gets its own and pyright and mypy accept it. A field
//! whose type is no model of the file, such as a `dict[str, str]`, keeps
//! its dict. The caller formats the rewritten file with ruff again.

/// What [`rewrite`] does to a generator's output, part of each output's
/// recipe so a change here makes the committed files stale.
pub(crate) const REWRITE: &str =
    "model-defaults: Field(default_factory=lambda: Model.model_validate({...}))";

/// `models` with each model-typed field's dict default constructed through
/// a factory; `None` when it holds none.
pub(crate) fn rewrite(models: &str) -> Option<String> {
    let lines: Vec<&str> = models.split_inclusive('\n').collect();
    let mut out = String::with_capacity(models.len());
    let mut changed = false;
    let mut at = 0;
    while at < lines.len() {
        let end = statement_end(&lines, at);
        let statement: String = lines[at..end].concat();
        match rewritten(&statement, models) {
            Some(new) => {
                out.push_str(&new);
                changed = true;
            }
            None => out.push_str(&statement),
        }
        at = end;
    }
    changed.then_some(out)
}

/// Whether `line` begins a class-body field: four spaces, then a name and
/// `:`.
fn begins_field(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("    ") else {
        return false;
    };
    let name: String = rest
        .chars()
        .take_while(|character| character.is_alphanumeric() || *character == '_')
        .collect();
    !name.is_empty()
        && !name.starts_with(|character: char| character.is_ascii_digit())
        && rest[name.len()..].starts_with(": ")
}

/// The line after the statement that begins at `start`: a field runs on
/// over every line indented deeper and every closing bracket at its own
/// indentation; any other line stands alone.
fn statement_end(lines: &[&str], start: usize) -> usize {
    if !begins_field(lines[start]) {
        return start + 1;
    }
    let mut end = start + 1;
    while let Some(line) = lines.get(end) {
        let continues = line.starts_with("     ")
            || ["    }", "    ]", "    )"]
                .iter()
                .any(|closing| line.starts_with(closing));
        if !continues {
            break;
        }
        end += 1;
    }
    end
}

/// The field `statement` with its dict default constructed, when its type
/// is a model `models` defines; `None` otherwise.
fn rewritten(statement: &str, models: &str) -> Option<String> {
    if !begins_field(statement) {
        return None;
    }
    let (name, rest) = statement[4..].split_once(": ")?;
    let assign = top_level(rest, " = ")?;
    let (annotation, value) = (&rest[..assign], rest[assign + 3..].trim_end());
    if !value.starts_with('{') || !value.ends_with('}') {
        return None;
    }
    let model = model_of(annotation)?;
    let class = format!("class {model}(");
    if !models.lines().any(|line| line.starts_with(&class)) {
        return None;
    }
    Some(format!(
        "    {name}: {annotation} = Field(default_factory=lambda: {model}.model_validate({value}))\n"
    ))
}

/// The one model `annotation` names, besides `None`: its own type, or an
/// `Annotated`'s first argument; `None` for any other type.
fn model_of(annotation: &str) -> Option<&str> {
    let typed = match annotation.strip_prefix("Annotated[") {
        Some(inner) => &inner[..top_level(inner, ",")?],
        None => annotation,
    };
    let mut named = typed
        .split(" | ")
        .map(str::trim)
        .filter(|part| *part != "None");
    let model = named.next()?;
    let is_name = model
        .chars()
        .all(|character| character.is_alphanumeric() || character == '_')
        && model.starts_with(|character: char| character.is_ascii_uppercase());
    (named.next().is_none() && is_name).then_some(model)
}

/// Where `needle` first stands in `text` outside brackets and strings.
fn top_level(text: &str, needle: &str) -> Option<usize> {
    let mut depth = 0_i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (at, character) in text.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth -= 1,
            _ if depth == 0 && text[at..].starts_with(needle) => return Some(at),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests;
