//! The evaluation of a checked expression over a [`Tree`] (`XPath` 1.0, §2
//! to §4), as a browser's `document.evaluate` reads an HTML document: a
//! name test without a prefix finds an HTML element by its local name in
//! any letter case and no element of another namespace, an attribute of an
//! HTML element likewise, and `xmlns` attributes are on no axis. Strings
//! are measured in characters.

use super::syntax::{Arithmetic, Axis, Comparison, Expr, Function, NodeTest, Path, Start, Step};
use super::tree::{HTML, Kind, Tree, XML, XMLNS};

/// `XPath`'s white space.
const SPACE: [char; 4] = [' ', '\t', '\r', '\n'];

/// What an expression answers.
#[derive(Clone, Debug)]
pub(super) enum Value {
    /// A node-set, its indices in document order, each once.
    Nodes(Vec<usize>),
    Boolean(bool),
    Number(f64),
    String(String),
}

/// The context an expression is evaluated in.
#[derive(Clone, Copy, Debug)]
pub(super) struct Context {
    pub(super) node: usize,
    pub(super) position: usize,
    pub(super) size: usize,
}

/// What `expr` answers over `tree` in `context`.
pub(super) fn evaluate(tree: &Tree, expr: &Expr, context: Context) -> Value {
    match expr {
        Expr::Or(operands) => Value::Boolean(
            operands
                .iter()
                .any(|operand| boolean(&evaluate(tree, operand, context))),
        ),
        Expr::And(operands) => Value::Boolean(
            operands
                .iter()
                .all(|operand| boolean(&evaluate(tree, operand, context))),
        ),
        Expr::Compare(first, rest) => {
            let mut left = evaluate(tree, first, context);
            for (operator, operand) in rest {
                let right = evaluate(tree, operand, context);
                left = Value::Boolean(compare(tree, *operator, &left, &right));
            }
            left
        }
        Expr::Arithmetic(first, rest) => {
            let mut left = number(tree, &evaluate(tree, first, context));
            for (operator, operand) in rest {
                let right = number(tree, &evaluate(tree, operand, context));
                left = match operator {
                    Arithmetic::Add => left + right,
                    Arithmetic::Subtract => left - right,
                    Arithmetic::Multiply => left * right,
                    Arithmetic::Divide => left / right,
                    Arithmetic::Modulo => left % right,
                };
            }
            Value::Number(left)
        }
        Expr::Negate(operand) => Value::Number(-number(tree, &evaluate(tree, operand, context))),
        Expr::Union(operands) => {
            let mut united = Vec::new();
            for operand in operands {
                united.extend(nodes(evaluate(tree, operand, context)));
            }
            united.sort_unstable();
            united.dedup();
            Value::Nodes(united)
        }
        Expr::Path(path) => Value::Nodes(located(tree, path, context)),
        Expr::Filter(primary, predicates) => {
            let mut found = nodes(evaluate(tree, primary, context));
            for predicate in predicates {
                found = filtered(tree, found, predicate);
            }
            Value::Nodes(found)
        }
        Expr::Literal(literal) => Value::String(literal.clone()),
        Expr::Number(number) => Value::Number(*number),
        Expr::Call(function, arguments) => called(tree, *function, arguments, context),
    }
}

/// The nodes of a value the check found to be a node-set.
fn nodes(value: Value) -> Vec<usize> {
    match value {
        Value::Nodes(nodes) => nodes,
        Value::Boolean(_) | Value::Number(_) | Value::String(_) => Vec::new(),
    }
}

/// The nodes `path` locates from `context`, in document order.
fn located(tree: &Tree, path: &Path, context: Context) -> Vec<usize> {
    let mut found = match &path.start {
        Start::Root => vec![0],
        Start::Context => vec![context.node],
        Start::Nodes(start) => nodes(evaluate(tree, start, context)),
    };
    for step in &path.steps {
        found = stepped(tree, &found, step);
    }
    found
}

/// The nodes `step` reaches from each of `from`, in document order.
fn stepped(tree: &Tree, from: &[usize], step: &Step) -> Vec<usize> {
    let mut seen = vec![false; tree.len()];
    let mut reached = Vec::new();
    for &node in from {
        // In the axis's own order: a reverse axis's nearest node first.
        let mut candidates: Vec<usize> = along(tree, node, step.axis)
            .into_iter()
            .filter(|&candidate| tested(tree, candidate, step.axis, &step.test))
            .collect();
        for predicate in &step.predicates {
            candidates = filtered(tree, candidates, predicate);
        }
        for candidate in candidates {
            if !seen[candidate] {
                seen[candidate] = true;
                reached.push(candidate);
            }
        }
    }
    reached.sort_unstable();
    reached
}

/// The nodes of `candidates`, in their order, for which `predicate` holds
/// at its proximity position.
fn filtered(tree: &Tree, candidates: Vec<usize>, predicate: &Expr) -> Vec<usize> {
    let size = candidates.len();
    candidates
        .into_iter()
        .enumerate()
        .filter(|&(at, node)| {
            let position = at + 1;
            match evaluate(
                tree,
                predicate,
                Context {
                    node,
                    position,
                    size,
                },
            ) {
                Value::Number(number) => at_position(number, position),
                value => boolean(&value),
            }
        })
        .map(|(_, node)| node)
        .collect()
}

/// Whether a number predicate holds at `position`: only when it is that
/// position exactly.
#[expect(
    clippy::float_cmp,
    reason = "a number predicate holds only at exactly its position"
)]
fn at_position(number: f64, position: usize) -> bool {
    number == counted(position)
}

/// The nodes on `axis` from `node`, in the axis's order.
fn along(tree: &Tree, node: usize, axis: Axis) -> Vec<usize> {
    let here = tree.node(node);
    let not_attribute = |index: &usize| !tree.is_attribute(*index);
    let ancestors = || std::iter::successors(here.parent, |&parent| tree.node(parent).parent);
    let siblings = || {
        here.parent
            .filter(|_| !tree.is_attribute(node))
            .map(|parent| tree.node(parent).children.as_slice())
            .unwrap_or_default()
    };
    match axis {
        Axis::Child => here.children.clone(),
        Axis::Descendant => (node + 1..here.end).filter(not_attribute).collect(),
        Axis::DescendantOrSelf => std::iter::once(node)
            .chain((node + 1..here.end).filter(not_attribute))
            .collect(),
        Axis::Parent => here.parent.into_iter().collect(),
        Axis::Ancestor => ancestors().collect(),
        Axis::AncestorOrSelf => std::iter::once(node).chain(ancestors()).collect(),
        Axis::FollowingSibling => siblings()
            .iter()
            .copied()
            .filter(|&sibling| sibling > node)
            .collect(),
        Axis::PrecedingSibling => siblings()
            .iter()
            .rev()
            .copied()
            .filter(|&sibling| sibling < node)
            .collect(),
        Axis::Following => (here.end..tree.len()).filter(not_attribute).collect(),
        Axis::Preceding => {
            let held: Vec<usize> = ancestors().collect();
            (0..node)
                .rev()
                .filter(|index| not_attribute(index) && !held.contains(index))
                .collect()
        }
        Axis::Attribute => here
            .attributes
            .iter()
            .copied()
            .filter(|&attribute| {
                !matches!(&tree.node(attribute).kind, Kind::Attribute(name, _) if name.namespace == XMLNS)
            })
            .collect(),
        // A browser's DOM has no namespace nodes.
        Axis::Namespace => Vec::new(),
        Axis::Itself => vec![node],
    }
}

/// Whether the node at `index` passes `test` on `axis`.
fn tested(tree: &Tree, index: usize, axis: Axis, test: &NodeTest) -> bool {
    let kind = &tree.node(index).kind;
    let principal = |kind: &Kind| match axis {
        Axis::Attribute => matches!(kind, Kind::Attribute(..)),
        Axis::Namespace => false,
        _ => matches!(kind, Kind::Element(_)),
    };
    match test {
        NodeTest::Node => true,
        NodeTest::Text => matches!(kind, Kind::Text(_)),
        NodeTest::Comment => matches!(kind, Kind::Comment(_)),
        NodeTest::Instruction(target) => matches!(
            kind,
            Kind::Instruction { target: found, .. } if target.as_ref().is_none_or(|target| target == found)
        ),
        NodeTest::Principal => principal(kind),
        NodeTest::Name(local) => match kind {
            Kind::Element(name) if principal(kind) => {
                name.namespace == HTML && name.local.eq_ignore_ascii_case(local)
            }
            Kind::Attribute(name, _) if principal(kind) => {
                let on_html = tree
                    .node(index)
                    .parent
                    .is_some_and(|element| matches!(&tree.node(element).kind, Kind::Element(owner) if owner.namespace == HTML));
                name.namespace.is_empty()
                    && if on_html {
                        name.local.eq_ignore_ascii_case(local)
                    } else {
                        name.local == *local
                    }
            }
            _ => false,
        },
    }
}

/// Whether `left` and `right` compare by `operator` (§3.4).
fn compare(tree: &Tree, operator: Comparison, left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Nodes(left), Value::Nodes(right)) => left.iter().any(|&one| {
            let one = Value::String(tree.string_value(one));
            right
                .iter()
                .any(|&other| atomic(operator, &one, &Value::String(tree.string_value(other))))
        }),
        (Value::Nodes(nodes), Value::Boolean(_)) => {
            atomic(operator, &Value::Boolean(!nodes.is_empty()), right)
        }
        (Value::Boolean(_), Value::Nodes(nodes)) => {
            atomic(operator, left, &Value::Boolean(!nodes.is_empty()))
        }
        (Value::Nodes(nodes), _) => nodes
            .iter()
            .any(|&node| atomic(operator, &Value::String(tree.string_value(node)), right)),
        (_, Value::Nodes(nodes)) => nodes
            .iter()
            .any(|&node| atomic(operator, left, &Value::String(tree.string_value(node)))),
        _ => atomic(operator, left, right),
    }
}

/// Whether two values, neither a node-set, compare by `operator`.
#[expect(
    clippy::float_cmp,
    reason = "XPath compares numbers exactly, as IEEE 754 does"
)]
fn atomic(operator: Comparison, left: &Value, right: &Value) -> bool {
    let (one, other) = (atomic_number(left), atomic_number(right));
    let equal = operator == Comparison::Equal;
    match operator {
        Comparison::Equal | Comparison::NotEqual => match (left, right) {
            (Value::Boolean(_), _) | (_, Value::Boolean(_)) => {
                (atomic_boolean(left) == atomic_boolean(right)) == equal
            }
            // NaN equals nothing and differs from everything, so `!=` is
            // no negation of `=` for numbers.
            (Value::Number(_), _) | (_, Value::Number(_)) => {
                if equal {
                    one == other
                } else {
                    one != other
                }
            }
            _ => (atomic_string(left) == atomic_string(right)) == equal,
        },
        Comparison::Less => one < other,
        Comparison::LessOrEqual => one <= other,
        Comparison::Greater => one > other,
        Comparison::GreaterOrEqual => one >= other,
    }
}

/// A count as `XPath`'s number.
#[expect(
    clippy::cast_precision_loss,
    reason = "a count of nodes or characters is far below 2^52"
)]
fn counted(count: usize) -> f64 {
    count as f64
}

fn atomic_boolean(value: &Value) -> bool {
    match value {
        Value::Nodes(nodes) => !nodes.is_empty(),
        Value::Boolean(boolean) => *boolean,
        Value::Number(number) => *number != 0.0 && !number.is_nan(),
        Value::String(string) => !string.is_empty(),
    }
}

fn atomic_number(value: &Value) -> f64 {
    match value {
        Value::Nodes(_) => f64::NAN,
        Value::Boolean(boolean) => f64::from(u8::from(*boolean)),
        Value::Number(number) => *number,
        Value::String(string) => parsed(string),
    }
}

fn atomic_string(value: &Value) -> String {
    match value {
        Value::Nodes(_) => String::new(),
        Value::Boolean(boolean) => boolean.to_string(),
        Value::Number(number) => shown(*number),
        Value::String(string) => string.clone(),
    }
}

/// `boolean()` of `value`.
fn boolean(value: &Value) -> bool {
    atomic_boolean(value)
}

/// `number()` of `value`.
fn number(tree: &Tree, value: &Value) -> f64 {
    match value {
        Value::Nodes(_) => parsed(&string(tree, value)),
        _ => atomic_number(value),
    }
}

/// `string()` of `value`: a node-set's first node's string-value.
fn string(tree: &Tree, value: &Value) -> String {
    match value {
        Value::Nodes(nodes) => nodes
            .first()
            .map_or_else(String::new, |&node| tree.string_value(node)),
        _ => atomic_string(value),
    }
}

/// The number a string reads as (§4.4): optional white space, an optional
/// minus, digits with an optional fraction, optional white space; NaN for
/// anything else.
fn parsed(text: &str) -> f64 {
    let text = text.trim_matches(SPACE);
    let digits = text.strip_prefix('-').unwrap_or(text);
    let mut parts = digits.splitn(2, '.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    let all_digits = |part: &str| part.bytes().all(|byte| byte.is_ascii_digit());
    let readable = all_digits(whole)
        && fraction.is_none_or(all_digits)
        && !(whole.is_empty() && fraction.is_none_or(str::is_empty));
    if !readable {
        return f64::NAN;
    }
    text.parse().unwrap_or(f64::NAN)
}

/// How a number reads as a string (§4.2): `NaN`, `Infinity`, an integer
/// with no fraction, else a decimal with no exponent.
fn shown(number: f64) -> String {
    if number.is_nan() {
        "NaN".to_owned()
    } else if number.is_infinite() {
        if number > 0.0 {
            "Infinity".to_owned()
        } else {
            "-Infinity".to_owned()
        }
    } else if number == 0.0 {
        "0".to_owned()
    } else {
        number.to_string()
    }
}

/// `XPath`'s `round()`: to the nearest integer, a half towards positive
/// infinity, keeping a negative zero.
fn rounded(number: f64) -> f64 {
    if number.is_nan() || number.is_infinite() {
        return number;
    }
    if (-0.5..0.0).contains(&number) {
        return -0.0;
    }
    (number + 0.5).floor()
}

/// `XPath`'s white space collapsed and trimmed.
fn normalized(text: &str) -> String {
    text.split(SPACE)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// What `function` answers for `arguments` in `context` (§4).
fn called(tree: &Tree, function: Function, arguments: &[Expr], context: Context) -> Value {
    let argument = |at: usize| {
        arguments
            .get(at)
            .map(|argument| evaluate(tree, argument, context))
    };
    // An argument's value, or the context node's when it is left out.
    let or_context = || argument(0).unwrap_or_else(|| Value::Nodes(vec![context.node]));
    let text = |at: usize| {
        argument(at)
            .map(|value| string(tree, &value))
            .unwrap_or_default()
    };
    // The first node of a node-set argument, or the context node.
    let first = || nodes(or_context()).first().copied();
    match function {
        Function::Last => Value::Number(counted(context.size)),
        Function::Position => Value::Number(counted(context.position)),
        Function::Count => Value::Number(counted(nodes(or_context()).len())),
        Function::Id => Value::Nodes(identified(tree, or_context())),
        Function::LocalName | Function::NamespaceUri | Function::Name => {
            Value::String(first().map_or_else(String::new, |node| named(tree, function, node)))
        }
        Function::String => Value::String(string(tree, &or_context())),
        Function::Concat => Value::String((0..arguments.len()).map(text).collect()),
        Function::StartsWith => Value::Boolean(text(0).starts_with(&text(1))),
        Function::Contains => Value::Boolean(text(0).contains(&text(1))),
        Function::SubstringBefore => {
            let (whole, before) = (text(0), text(1));
            Value::String(
                whole
                    .find(&before)
                    .map(|at| whole[..at].to_owned())
                    .unwrap_or_default(),
            )
        }
        Function::SubstringAfter => {
            let (whole, after) = (text(0), text(1));
            Value::String(
                whole
                    .find(&after)
                    .map(|at| whole[at + after.len()..].to_owned())
                    .unwrap_or_default(),
            )
        }
        Function::Substring => {
            let start = argument(1).map_or(f64::NAN, |start| number(tree, &start));
            let length = argument(2).map(|length| number(tree, &length));
            Value::String(substring(&text(0), start, length))
        }
        Function::StringLength => {
            Value::Number(counted(string(tree, &or_context()).chars().count()))
        }
        Function::NormalizeSpace => Value::String(normalized(&string(tree, &or_context()))),
        Function::Translate => Value::String(translated(&text(0), &text(1), &text(2))),
        Function::Boolean => Value::Boolean(argument(0).is_some_and(|value| boolean(&value))),
        Function::Not => Value::Boolean(!argument(0).is_some_and(|value| boolean(&value))),
        Function::True => Value::Boolean(true),
        Function::False => Value::Boolean(false),
        Function::Lang => Value::Boolean(in_language(tree, context.node, &text(0))),
        Function::Number => Value::Number(number(tree, &or_context())),
        Function::Sum => Value::Number(
            nodes(or_context())
                .iter()
                .map(|&node| parsed(&tree.string_value(node)))
                .sum(),
        ),
        Function::Floor => Value::Number(number(tree, &or_context()).floor()),
        Function::Ceiling => Value::Number(number(tree, &or_context()).ceil()),
        Function::Round => Value::Number(rounded(number(tree, &or_context()))),
    }
}

/// `id()`: the first element with each `id` that `value` names, its
/// node-set's string-values or its string split at white space.
fn identified(tree: &Tree, value: Value) -> Vec<usize> {
    let names = match value {
        Value::Nodes(found) => found
            .iter()
            .map(|&node| tree.string_value(node))
            .collect::<Vec<_>>()
            .join(" "),
        value => string(tree, &value),
    };
    let mut found: Vec<usize> = names
        .split(SPACE)
        .filter_map(|id| tree.element_by_id(id))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

/// `substring()`: the characters of `text` from the rounded `start`, the
/// first at 1, for the rounded `length` or to its end; NaN and infinities
/// compare as IEEE 754 says.
fn substring(text: &str, start: f64, length: Option<f64>) -> String {
    let start = rounded(start);
    let end = length.map_or(f64::INFINITY, |length| start + rounded(length));
    text.chars()
        .enumerate()
        .filter(|&(at, _)| {
            let position = counted(at + 1);
            position >= start && position < end
        })
        .map(|(_, c)| c)
        .collect()
}

/// `translate()`: each character of `text` found in `from` becomes the one
/// at its place in `to`, or goes when `to` is shorter.
fn translated(text: &str, from: &str, to: &str) -> String {
    let (from, to): (Vec<char>, Vec<char>) = (from.chars().collect(), to.chars().collect());
    text.chars()
        .filter_map(|c| match from.iter().position(|&found| found == c) {
            Some(at) => to.get(at).copied(),
            None => Some(c),
        })
        .collect()
}

/// `local-name()`, `namespace-uri()` or `name()` of the node at `index`:
/// an element's or an attribute's, a processing instruction's target as
/// its name, and nothing for another node.
fn named(tree: &Tree, function: Function, index: usize) -> String {
    match &tree.node(index).kind {
        Kind::Element(name) | Kind::Attribute(name, _) => match function {
            Function::LocalName => name.local.clone(),
            Function::NamespaceUri => name.namespace.clone(),
            _ => name.prefix.as_ref().map_or_else(
                || name.local.clone(),
                |prefix| format!("{prefix}:{}", name.local),
            ),
        },
        Kind::Instruction { target, .. } if function != Function::NamespaceUri => target.clone(),
        _ => String::new(),
    }
}

/// `lang()`: whether the nearest `xml:lang` of the context node or an
/// element holding it names `language` or one of its sublanguages. As in
/// a browser, only an attribute in the XML namespace counts (foreign
/// content's `xml:lang`), and an attribute holds no language of its own.
fn in_language(tree: &Tree, index: usize, language: &str) -> bool {
    if tree.is_attribute(index) {
        return false;
    }
    let declared =
        std::iter::successors(Some(index), |&node| tree.node(node).parent).find_map(|node| {
            tree.node(node).attributes.iter().find_map(|&attribute| {
                match &tree.node(attribute).kind {
                    Kind::Attribute(name, value)
                        if name.namespace == XML && name.local == "lang" =>
                    {
                        Some(value.as_str())
                    }
                    _ => None,
                }
            })
        });
    declared.is_some_and(|declared| {
        declared.eq_ignore_ascii_case(language)
            || declared.len() > language.len()
                && declared.as_bytes()[language.len()] == b'-'
                && declared[..language.len()].eq_ignore_ascii_case(language)
    })
}
