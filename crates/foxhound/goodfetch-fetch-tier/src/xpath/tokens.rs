//! The lexical structure of an `XPath` 1.0 expression (§3.7): its tokens,
//! with the rules that tell a name test from an operator name, a function
//! name, a node type or an axis name by what precedes and follows it.

use super::syntax::{Arithmetic, Axis, Comparison, NodeTest};

/// One token (§3.7).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Token {
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    Dot,
    DotDot,
    At,
    Comma,
    DoubleColon,
    Slash,
    DoubleSlash,
    Pipe,
    Plus,
    Minus,
    Compare(Comparison),
    And,
    Or,
    Multiplicative(Arithmetic),
    NameTest {
        prefix: Option<String>,
        /// `None` for `*`.
        local: Option<String>,
    },
    NodeType(NodeTest),
    FunctionName {
        prefix: Option<String>,
        local: String,
    },
    AxisName(Axis),
    Literal(String),
    Number(f64),
    Variable(String),
}

impl Token {
    /// Whether it is an operator, after which `*` and a name read as a
    /// name test (§3.7).
    const fn is_operator(&self) -> bool {
        matches!(
            self,
            Self::Slash
                | Self::DoubleSlash
                | Self::Pipe
                | Self::Plus
                | Self::Minus
                | Self::Compare(_)
                | Self::And
                | Self::Or
                | Self::Multiplicative(_)
        )
    }

    /// How an error names it.
    pub(super) fn shown(&self) -> String {
        match self {
            Self::LeftParen => "`(`".to_owned(),
            Self::RightParen => "`)`".to_owned(),
            Self::LeftBracket => "`[`".to_owned(),
            Self::RightBracket => "`]`".to_owned(),
            Self::Dot => "`.`".to_owned(),
            Self::DotDot => "`..`".to_owned(),
            Self::At => "`@`".to_owned(),
            Self::Comma => "`,`".to_owned(),
            Self::DoubleColon => "`::`".to_owned(),
            Self::Slash => "`/`".to_owned(),
            Self::DoubleSlash => "`//`".to_owned(),
            Self::Pipe => "`|`".to_owned(),
            Self::Plus => "`+`".to_owned(),
            Self::Minus => "`-`".to_owned(),
            Self::Compare(_) => "a comparison".to_owned(),
            Self::And => "`and`".to_owned(),
            Self::Or => "`or`".to_owned(),
            Self::Multiplicative(_) => "an operator".to_owned(),
            Self::NameTest { local, .. } => {
                format!("`{}`", local.as_deref().unwrap_or("*"))
            }
            Self::NodeType(_) => "a node type".to_owned(),
            Self::FunctionName { local, .. } => format!("`{local}(`"),
            Self::AxisName(_) => "an axis".to_owned(),
            Self::Literal(literal) => format!("the literal `{literal}`"),
            Self::Number(number) => format!("the number `{number}`"),
            Self::Variable(name) => format!("`${name}`"),
        }
    }
}

/// Whether `c` is `XPath`'s white space.
const fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// Whether `c` may start an `NCName` (XML 1.0, fifth edition,
/// `NameStartChar` without `:`).
const fn starts_name(c: char) -> bool {
    matches!(c,
        'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// Whether `c` may continue an `NCName` (`NameChar` without `:`).
const fn continues_name(c: char) -> bool {
    starts_name(c)
        || matches!(c,
            '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// The tokens of `text`, disambiguated by §3.7's rules.
pub(super) fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let at = |index: usize| chars.get(index).copied();
    let mut tokens: Vec<Token> = Vec::new();
    let mut index = 0;
    loop {
        while at(index).is_some_and(is_space) {
            index += 1;
        }
        let Some(c) = at(index) else {
            return Ok(tokens);
        };
        // After an operand, `*` multiplies and a name is an operator.
        let after_operand = tokens.last().is_some_and(|last| {
            !(matches!(
                last,
                Token::At
                    | Token::DoubleColon
                    | Token::LeftParen
                    | Token::LeftBracket
                    | Token::Comma
            ) || last.is_operator())
        });
        let name = |from: usize| {
            let mut end = from;
            while at(end).is_some_and(continues_name) {
                end += 1;
            }
            (chars[from..end].iter().collect::<String>(), end)
        };
        let (token, next) = match c {
            '(' => (Token::LeftParen, index + 1),
            ')' => (Token::RightParen, index + 1),
            '[' => (Token::LeftBracket, index + 1),
            ']' => (Token::RightBracket, index + 1),
            ',' => (Token::Comma, index + 1),
            '@' => (Token::At, index + 1),
            '|' => (Token::Pipe, index + 1),
            '+' => (Token::Plus, index + 1),
            '-' => (Token::Minus, index + 1),
            '=' => (Token::Compare(Comparison::Equal), index + 1),
            '!' if at(index + 1) == Some('=') => (Token::Compare(Comparison::NotEqual), index + 2),
            '<' if at(index + 1) == Some('=') => {
                (Token::Compare(Comparison::LessOrEqual), index + 2)
            }
            '<' => (Token::Compare(Comparison::Less), index + 1),
            '>' if at(index + 1) == Some('=') => {
                (Token::Compare(Comparison::GreaterOrEqual), index + 2)
            }
            '>' => (Token::Compare(Comparison::Greater), index + 1),
            '/' if at(index + 1) == Some('/') => (Token::DoubleSlash, index + 2),
            '/' => (Token::Slash, index + 1),
            ':' if at(index + 1) == Some(':') => (Token::DoubleColon, index + 2),
            '.' if at(index + 1) == Some('.') => (Token::DotDot, index + 2),
            '.' if !at(index + 1).is_some_and(|next| next.is_ascii_digit()) => {
                (Token::Dot, index + 1)
            }
            '.' | '0'..='9' => number(&chars, index)?,
            '"' | '\'' => {
                let close = (index + 1..chars.len())
                    .find(|&end| chars[end] == c)
                    .ok_or_else(|| format!("its literal opened by `{c}` never closes"))?;
                (
                    Token::Literal(chars[index + 1..close].iter().collect()),
                    close + 1,
                )
            }
            '$' if at(index + 1).is_some_and(starts_name) => {
                let (local, end) = name(index + 1);
                (Token::Variable(local), end)
            }
            '$' => return Err("a `$` names no variable".to_owned()),
            '*' if after_operand => (Token::Multiplicative(Arithmetic::Multiply), index + 1),
            '*' => (
                Token::NameTest {
                    prefix: None,
                    local: None,
                },
                index + 1,
            ),
            c if starts_name(c) => {
                let (first, end) = name(index);
                if after_operand {
                    let operator = match first.as_str() {
                        "and" => Token::And,
                        "or" => Token::Or,
                        "mod" => Token::Multiplicative(Arithmetic::Modulo),
                        "div" => Token::Multiplicative(Arithmetic::Divide),
                        _ => return Err(format!("`{first}` stands where an operator is due")),
                    };
                    (operator, end)
                } else {
                    named(&chars, first, end)?
                }
            }
            c => return Err(format!("`{c}` is no part of XPath")),
        };
        tokens.push(token);
        index = next;
    }
}

/// The number that starts at `index`: digits with an optional fraction, or
/// a fraction alone.
fn number(chars: &[char], index: usize) -> Result<(Token, usize), String> {
    let digits_from = |mut end: usize| {
        while chars.get(end).is_some_and(char::is_ascii_digit) {
            end += 1;
        }
        end
    };
    let mut end = digits_from(index);
    if chars.get(end) == Some(&'.') {
        end = digits_from(end + 1);
    }
    let digits: String = chars[index..end].iter().collect();
    let number = digits
        .parse::<f64>()
        .map_err(|_| format!("`{digits}` is no number"))?;
    Ok((Token::Number(number), end))
}

/// The token a name `first`, read up to `end`, starts: a name test, a node
/// type, a function name or an axis name, by what follows it (§3.7).
fn named(chars: &[char], first: String, mut end: usize) -> Result<(Token, usize), String> {
    let at = |index: usize| chars.get(index).copied();
    let mut prefix = None;
    let mut local = Some(first);
    if at(end) == Some(':') && at(end + 1) != Some(':') {
        prefix = local.take();
        if at(end + 1) == Some('*') {
            end += 2;
        } else if at(end + 1).is_some_and(starts_name) {
            let mut close = end + 1;
            while at(close).is_some_and(continues_name) {
                close += 1;
            }
            local = Some(chars[end + 1..close].iter().collect());
            end = close;
        } else {
            return Err("a `:` names no local name".to_owned());
        }
    }
    let mut ahead = end;
    while at(ahead).is_some_and(is_space) {
        ahead += 1;
    }
    let token = match (prefix, local) {
        (None, Some(local)) if at(ahead) == Some('(') => match local.as_str() {
            "node" => Token::NodeType(NodeTest::Node),
            "text" => Token::NodeType(NodeTest::Text),
            "comment" => Token::NodeType(NodeTest::Comment),
            "processing-instruction" => Token::NodeType(NodeTest::Instruction(None)),
            _ => Token::FunctionName {
                prefix: None,
                local,
            },
        },
        (Some(prefix), Some(local)) if at(ahead) == Some('(') => Token::FunctionName {
            prefix: Some(prefix),
            local,
        },
        (None, Some(local)) if at(ahead) == Some(':') && at(ahead + 1) == Some(':') => {
            Token::AxisName(
                Axis::named(&local).ok_or_else(|| format!("no axis is named `{local}`"))?,
            )
        }
        (prefix, local) => Token::NameTest { prefix, local },
    };
    Ok((token, end))
}
