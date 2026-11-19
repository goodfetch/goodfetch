//! A handle member's signature in the TypeScript handles (ADR 0099): its
//! parameter list, positional parameters then the options object, and the
//! object it sends; an overload's narrows the types to its forms.

use super::super::handles::{Overload, Parameters};
use super::super::naming::camel;
use super::super::settings_tree::SETTINGS;
use super::{FIELDS, TIMEOUT, TIMEOUT_OF, settings};

/// The words TypeScript reserves, which a positional parameter cannot be
/// named: it takes a trailing `_` in the declaration, which no caller
/// writes (ADR 0099).
const RESERVED_WORDS: [&str; 48] = [
    "with",
    "yield",
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
];

/// A positional parameter's name in a declaration.
fn positional_name(name: &str) -> String {
    let spelled = camel(name);
    if RESERVED_WORDS.contains(&spelled.as_str()) || spelled == "options" {
        format!("{spelled}_")
    } else {
        spelled
    }
}

/// A method's parameter list and the object it makes of it, which starts
/// with the handle's ID as `id`: positional parameters first, then the
/// options object, which holds the named ones and, for a call that can be
/// cancelled, `signal`. Each type is the params type's own,
/// `types.<Params>["<name>"]`, but the one the fields that type a result
/// are declared by, `typed`, which is the member's type parameter. A call's
/// own `timeout` goes through `timeoutOf`, which refuses before sending a
/// value no deadline can be, naming its setting, `timeout_setting`
/// (ADR 0080). A scope's own `settings` take that scope's settings type,
/// `settings_type`, and go through `settingsOnWire`, which sends them by
/// their wire spelling (ADR 0083). `overload` picks the signature
/// declared: the implementation's, or an overload's, which narrows the
/// types to its forms and has no default (ADR 0099).
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is one fact of the member the signature declares"
)]
pub(super) fn signature(
    parameters: &Parameters,
    type_name: &str,
    cancellable: bool,
    id: &str,
    typed: Option<&str>,
    timeout_setting: Option<&str>,
    settings_type: Option<&str>,
    overload: Overload<'_>,
) -> (String, String) {
    let mut declared = Vec::new();
    let mut entries = vec![format!("{}: this.#host.id", camel(id))];
    for name in &parameters.positional {
        let local = positional_name(name);
        if typed == Some(name.as_str()) {
            declared.push(format!("{local}: {FIELDS}"));
        } else {
            let field = format!("types.{type_name}[{:?}]", camel(name));
            declared.push(format!("{local}: {}", narrowed(&field, name, overload)));
        }
        let field = camel(name);
        entries.push(if local == field {
            local
        } else {
            format!("{field}: {local}")
        });
    }
    let mut options = Vec::new();
    for name in &parameters.named {
        let field = camel(name);
        let spelled = narrowed(&format!("types.{type_name}[{field:?}]"), name, overload);
        options.push(format!("readonly {field}: {spelled}"));
        entries.push(format!("{field}: options.{field}"));
    }
    // An overload's options are required when it requires one of them.
    let mut required = !parameters.named.is_empty();
    for name in &parameters.optional {
        let field = camel(name);
        let given = format!("options.{field}");
        let spelled = if let Some(settings) = settings_type.filter(|_| name == SETTINGS) {
            entries.push(format!("{field}: {}({given})", settings::SETTINGS_ON_WIRE));
            settings.to_owned()
        } else {
            entries.push(match timeout_setting.filter(|_| name == TIMEOUT) {
                Some(setting) => format!("{field}: {TIMEOUT_OF}({given}, {setting:?})"),
                None => format!("{field}: {given}"),
            });
            narrowed(&format!("types.{type_name}[{field:?}]"), name, overload)
        };
        if overload.leaves_out(name) {
            continue;
        }
        // A deprecated form's overload requires the parameter it passes.
        if overload.chosen_by(name) {
            required = true;
            options.push(format!("readonly {field}: {spelled}"));
        } else {
            options.push(format!("readonly {field}?: {spelled} | undefined"));
        }
    }
    if cancellable {
        options.push("readonly signal?: AbortSignal | undefined".to_owned());
    }
    if !options.is_empty() {
        let options = options.join("; ");
        // Only the implementation has a default: an overload's options
        // are optional instead when it requires none of them.
        declared.push(if required {
            format!("options: {{ {options} }}")
        } else if matches!(overload, Overload::Every) {
            format!("options: {{ {options} }} = {{}}")
        } else {
            format!("options?: {{ {options} }}")
        });
    }
    let object = format!("{{ {} }}", entries.join(", "));
    // An optional parameter left out is left out of the object, never sent
    // as `undefined`.
    let object = if parameters.optional.is_empty() {
        object
    } else {
        format!("defined({object})")
    };
    (declared.join(", "), object)
}

/// The type `overload` declares for the parameter `param`, whose full type
/// is `full`: a deprecated form's own, which is the deprecated value alone
/// or the parameter passed, or the full type less the deprecated values
/// the current overload leaves out (ADR 0099).
fn narrowed(full: &str, param: &str, overload: Overload<'_>) -> String {
    match overload {
        Overload::Deprecated(form) if form.param == param => match &form.value {
            Some(value) => format!("{value:?}"),
            None => format!("Exclude<{full}, undefined>"),
        },
        Overload::Current(forms) => {
            let left_out: Vec<String> = forms
                .iter()
                .filter(|form| form.param == param)
                .filter_map(|form| form.value.as_ref().map(|value| format!("{value:?}")))
                .collect();
            if left_out.is_empty() {
                full.to_owned()
            } else {
                format!("Exclude<{full}, {}>", left_out.join(" | "))
            }
        }
        Overload::Every | Overload::Deprecated(_) => full.to_owned(),
    }
}
