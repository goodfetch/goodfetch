//! The fenced code blocks of a rendering, as Markdown reads them: a run of
//! three or more backticks or tildes opens one, indented or not (inside an
//! MDX component too), and a run of the same character at least as long,
//! alone on its line, closes it; a block never closed runs to the end.

use crate::products::Product;

use super::render::Line;

/// One fenced block.
pub(super) struct Block {
    /// The first word of its info string.
    pub(super) language: String,
    /// The words after it, but metadata such as `title="…"` or `{1,3}`.
    pub(super) markers: Vec<String>,
    /// Its opening fence's line in the source.
    pub(super) line: usize,
    /// Its lines, without the fence's indentation.
    pub(super) body: Vec<BodyLine>,
    /// The Products whose renderings carry it, Collie first.
    pub(super) carriers: Vec<Product>,
    /// The Product the part it sits in is marked as naming.
    pub(super) names: Option<Product>,
}

impl Block {
    pub(super) fn marked(&self, marker: &str) -> bool {
        self.markers.iter().any(|found| found == marker)
    }
}

pub(super) struct BodyLine {
    pub(super) text: String,
    /// Its line in the source.
    pub(super) source: usize,
}

/// An open fence: its character, its length and its indentation.
struct Fence {
    character: char,
    length: usize,
    indent: usize,
}

/// Every fenced block of `lines`, in order.
pub(super) fn fenced(lines: &[Line]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut open: Option<(Fence, Block)> = None;
    for line in lines {
        let trimmed = line.text.trim_start();
        let indent = line.text.len() - trimmed.len();
        if let Some((fence, mut block)) = open.take() {
            let run = run_of(trimmed, fence.character);
            if run >= fence.length && trimmed[run..].trim().is_empty() {
                blocks.push(block);
            } else {
                block.body.push(BodyLine {
                    text: without_indent(&line.text, fence.indent).to_owned(),
                    source: line.source,
                });
                open = Some((fence, block));
            }
            continue;
        }
        let Some(character) = trimmed.chars().next().filter(|c| matches!(c, '`' | '~')) else {
            continue;
        };
        let length = run_of(trimmed, character);
        let info = &trimmed[length..];
        if length < 3 || (character == '`' && info.contains('`')) {
            continue;
        }
        let mut words = info_words(info).into_iter();
        let language = words.next().unwrap_or_default();
        let markers = words
            .filter(|word| !word.contains('=') && !word.starts_with('{'))
            .collect();
        open = Some((
            Fence {
                character,
                length,
                indent,
            },
            Block {
                language,
                markers,
                line: line.source,
                body: Vec::new(),
                carriers: line.carriers.clone(),
                names: line.names,
            },
        ));
    }
    blocks.extend(open.map(|(_, block)| block));
    blocks
}

/// How many times `character` starts `text`.
fn run_of(text: &str, character: char) -> usize {
    text.chars().take_while(|found| *found == character).count()
}

/// `text` without up to `indent` leading spaces.
fn without_indent(text: &str, indent: usize) -> &str {
    let spaces = text.len() - text.trim_start_matches(' ').len();
    &text[spaces.min(indent)..]
}

/// The words of an info string, a quoted value with its spaces kept whole.
fn info_words(info: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for character in info.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                word.push(character);
            }
            _ if character.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            _ => word.push(character),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}
