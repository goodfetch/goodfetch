//! The columns a SQLite store's SQL text dump leaves without a value: every
//! column of every `CREATE TABLE` that holds `NULL` in each of the table's
//! `INSERT` rows, or whose table has none (ADR 0066). It reads the statements
//! `sqlite3`'s `.dump` writes, through a tokenizer that knows SQL's quotes
//! and comments, so a `;` or `,` inside a value never splits it.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, PartialEq, Eq)]
enum Token {
    /// A keyword or bare name.
    Word(String),
    /// A name in `"…"`, `` `…` `` or `[…]`.
    Quoted(String),
    /// A string literal.
    Text,
    /// A number, or anything else that is one value's token.
    Other,
    Punctuation(char),
}

impl Token {
    fn is_word(&self, keyword: &str) -> bool {
        matches!(self, Self::Word(word) if word.eq_ignore_ascii_case(keyword))
    }

    fn name(&self) -> Option<&str> {
        match self {
            Self::Word(name) | Self::Quoted(name) => Some(name),
            _ => None,
        }
    }
}

fn tokens(text: &str) -> Vec<Token> {
    let characters: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while let Some(&character) = characters.get(index) {
        let next = characters.get(index + 1).copied();
        match character {
            _ if character.is_whitespace() => index += 1,
            '-' if next == Some('-') => {
                while characters
                    .get(index)
                    .is_some_and(|&current| current != '\n')
                {
                    index += 1;
                }
            }
            '/' if next == Some('*') => {
                index += 2;
                while index < characters.len()
                    && !(characters[index] == '*' && characters.get(index + 1) == Some(&'/'))
                {
                    index += 1;
                }
                index += 2;
            }
            '\'' => {
                index = quoted(&characters, index, '\'').1;
                tokens.push(Token::Text);
            }
            '"' | '`' | '[' => {
                let close = if character == '[' { ']' } else { character };
                let (name, end) = quoted(&characters, index, close);
                tokens.push(Token::Quoted(name));
                index = end;
            }
            _ if character.is_alphabetic() || character == '_' => {
                let start = index;
                while characters.get(index).is_some_and(|&current| {
                    current.is_alphanumeric() || current == '_' || current == '$'
                }) {
                    index += 1;
                }
                tokens.push(Token::Word(characters[start..index].iter().collect()));
            }
            _ if character.is_ascii_digit() => {
                while characters
                    .get(index)
                    .is_some_and(|&current| current.is_alphanumeric() || current == '.')
                {
                    index += 1;
                }
                tokens.push(Token::Other);
            }
            _ => {
                tokens.push(Token::Punctuation(character));
                index += 1;
            }
        }
    }
    tokens
}

/// The text between the quote at `start` and its closing `close`, where a
/// doubled `close` stands for itself, and the index after the closing one.
fn quoted(characters: &[char], start: usize, close: char) -> (String, usize) {
    let mut text = String::new();
    let mut index = start + 1;
    while let Some(&character) = characters.get(index) {
        if character == close {
            if close != ']' && characters.get(index + 1) == Some(&close) {
                text.push(close);
                index += 2;
                continue;
            }
            return (text, index + 1);
        }
        text.push(character);
        index += 1;
    }
    (text, index)
}

/// The token lists between commas at the outermost level of `tokens`.
fn split_top_level(tokens: &[Token]) -> Vec<&[Token]> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Punctuation('(') => depth += 1,
            Token::Punctuation(')') => depth = depth.saturating_sub(1),
            Token::Punctuation(',') if depth == 0 => {
                parts.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&tokens[start..]);
    parts
}

/// The tokens inside the parenthesis that opens at `tokens[open]`, and the
/// index after the one that closes it.
fn parenthesized(tokens: &[Token], open: usize) -> Option<(&[Token], usize)> {
    if tokens.get(open) != Some(&Token::Punctuation('(')) {
        return None;
    }
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Token::Punctuation('(') => depth += 1,
            Token::Punctuation(')') => {
                depth -= 1;
                if depth == 0 {
                    return Some((&tokens[open + 1..index], index + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// A table name at `tokens[index]`, possibly after its schema's name and a
/// dot, and the index after it.
fn table_name(tokens: &[Token], index: usize) -> Option<(String, usize)> {
    let first = tokens.get(index)?.name()?;
    if tokens.get(index + 1) == Some(&Token::Punctuation('.')) {
        let second = tokens.get(index + 2)?.name()?;
        return Some((second.to_owned(), index + 3));
    }
    Some((first.to_owned(), index + 1))
}

/// Words that open a table constraint rather than a column definition.
const CONSTRAINTS: [&str; 5] = ["CONSTRAINT", "PRIMARY", "UNIQUE", "CHECK", "FOREIGN"];

/// A `CREATE TABLE` statement's table and columns.
fn created_table(statement: &[Token]) -> Option<(String, Vec<String>)> {
    let mut index = 1;
    if statement
        .get(index)
        .is_some_and(|token| token.is_word("TEMP") || token.is_word("TEMPORARY"))
    {
        index += 1;
    }
    if !statement.first()?.is_word("CREATE") || !statement.get(index)?.is_word("TABLE") {
        return None;
    }
    index += 1;
    if statement
        .get(index)
        .is_some_and(|token| token.is_word("IF"))
    {
        index += 3;
    }
    let (table, index) = table_name(statement, index)?;
    let (body, _) = parenthesized(statement, index)?;
    let columns = split_top_level(body)
        .into_iter()
        .filter_map(|definition| {
            let first = definition.first()?;
            if CONSTRAINTS.iter().any(|keyword| first.is_word(keyword)) {
                return None;
            }
            first.name().map(str::to_owned)
        })
        .collect();
    Some((table, columns))
}

/// An `INSERT` statement's table, its named columns if it names them, and
/// whether each value of each row is `NULL`.
struct Insert {
    table: String,
    columns: Option<Vec<String>>,
    rows: Vec<Vec<bool>>,
}

fn insert(statement: &[Token]) -> Option<Insert> {
    if !statement.first()?.is_word("INSERT") {
        return None;
    }
    let mut index = 1;
    if statement
        .get(index)
        .is_some_and(|token| token.is_word("OR"))
    {
        index += 2;
    }
    if !statement.get(index)?.is_word("INTO") {
        return None;
    }
    let (table, mut index) = table_name(statement, index + 1)?;
    let mut columns = None;
    if let Some((names, after)) = parenthesized(statement, index) {
        columns = Some(
            split_top_level(names)
                .into_iter()
                .filter_map(|name| name.first()?.name().map(str::to_owned))
                .collect(),
        );
        index = after;
    }
    if !statement.get(index)?.is_word("VALUES") {
        return None;
    }
    index += 1;
    let mut rows = Vec::new();
    while let Some((values, after)) = parenthesized(statement, index) {
        rows.push(
            split_top_level(values)
                .into_iter()
                .map(|value| matches!(value, [token] if token.is_word("NULL")))
                .collect(),
        );
        index = after;
        if statement.get(index) == Some(&Token::Punctuation(',')) {
            index += 1;
        }
    }
    Some(Insert {
        table,
        columns,
        rows,
    })
}

/// Every column of the dumps' tables, as `<table>.<column>`, that no row of
/// any dump gives a value other than `NULL`.
pub(super) fn uncovered<'dump>(dumps: impl Iterator<Item = &'dump str>) -> Vec<String> {
    let mut tables: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut filled: BTreeSet<(String, String)> = BTreeSet::new();
    let mut inserts = Vec::new();
    for dump in dumps {
        let tokens = tokens(dump);
        for statement in tokens.split(|token| *token == Token::Punctuation(';')) {
            if let Some((table, columns)) = created_table(statement) {
                tables.insert(table, columns);
            } else if let Some(insert) = insert(statement) {
                inserts.push(insert);
            }
        }
    }
    for insert in inserts {
        let Some(columns) = insert
            .columns
            .as_ref()
            .or_else(|| tables.get(&insert.table))
        else {
            continue;
        };
        for row in &insert.rows {
            for (column, null) in columns.iter().zip(row) {
                if !null {
                    filled.insert((insert.table.clone(), column.clone()));
                }
            }
        }
    }
    tables
        .iter()
        .flat_map(|(table, columns)| {
            columns
                .iter()
                .filter(|column| !filled.contains(&(table.clone(), (*column).clone())))
                .map(move |column| format!("{table}.{column}"))
        })
        .collect()
}
