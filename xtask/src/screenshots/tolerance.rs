//! Where a JavaScript or TypeScript file sets a screenshot comparison's
//! tolerance (ADR 0073): any of `maxDiffPixels`, `maxDiffPixelRatio`,
//! `allowedMismatchedPixels` and `allowedMismatchedPixelRatio`, and a
//! `threshold` key of a comparison's options. `threshold` is a common word
//! (a slow-test report's, an `IntersectionObserver`'s), so it counts only in
//! the options of `toHaveScreenshot`, `toMatchScreenshot` or
//! `toMatchSnapshot`, as their argument or their config key, and in
//! `comparatorOptions`; in a visual test file it counts wherever it is
//! written, so options held in a variable or spread do not hide it.
//! Comments and string values set nothing.

/// The tolerance settings Playwright's and Vitest's comparisons take, apart
/// from `threshold`.
const TOLERANCES: [&str; 4] = [
    "maxDiffPixels",
    "maxDiffPixelRatio",
    "allowedMismatchedPixels",
    "allowedMismatchedPixelRatio",
];

/// The per-pixel tolerance, a tolerance only inside a comparison's options.
const THRESHOLD: &str = "threshold";

/// The calls and config keys whose options object is a comparison's.
const COMPARISONS: [&str; 4] = [
    "toHaveScreenshot",
    "toMatchScreenshot",
    "toMatchSnapshot",
    "comparatorOptions",
];

/// The extensions of the files whose code may set a tolerance.
pub(super) const EXTENSIONS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// Each line (counted from 1) of `text` that sets a tolerance, with the
/// setting's name, in order; `visual_test` counts every `threshold`.
pub(super) fn settings(text: &str, visual_test: bool) -> Vec<(usize, &'static str)> {
    let code = code(text.as_bytes());
    let mut found = Vec::new();
    let mut position = 0;
    while position < code.len() {
        if !starts_identifier(code[position]) {
            position += 1;
            continue;
        }
        let start = position;
        while position < code.len() && continues_identifier(code[position]) {
            position += 1;
        }
        let name = &code[start..position];
        let setting = if let Some(tolerance) = TOLERANCES
            .iter()
            .find(|tolerance| tolerance.as_bytes() == name)
        {
            Some(*tolerance)
        } else if name == THRESHOLD.as_bytes()
            && (visual_test || (is_key(&code, start, position) && in_comparison(&code, start)))
        {
            Some(THRESHOLD)
        } else {
            None
        };
        if let Some(setting) = setting {
            let line = code[..start].split(|&byte| byte == b'\n').count();
            found.push((line, setting));
        }
    }
    found
}

fn starts_identifier(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

fn continues_identifier(byte: u8) -> bool {
    starts_identifier(byte) || byte.is_ascii_digit()
}

/// `text` with every comment and every string that is not an object's key
/// blanked to spaces, line breaks kept, so positions and lines still match.
/// Regular expression literals are read as code.
fn code(text: &[u8]) -> Vec<u8> {
    let mut code = text.to_vec();
    let mut position = 0;
    while position < text.len() {
        match (text[position], text.get(position + 1).copied()) {
            (b'/', Some(b'/')) => {
                let end = text[position..]
                    .iter()
                    .position(|&byte| byte == b'\n')
                    .map_or(text.len(), |offset| position + offset);
                blank(&mut code, position, end);
                position = end;
            }
            (b'/', Some(b'*')) => {
                let end = text[position + 2..]
                    .windows(2)
                    .position(|pair| pair == b"*/")
                    .map_or(text.len(), |offset| position + 2 + offset + 2);
                blank(&mut code, position, end);
                position = end;
            }
            (quote @ (b'"' | b'\'' | b'`'), _) => {
                let end = string_end(text, position, quote);
                let key = quote != b'`' && next_byte(text, end) == Some(b':');
                if !key {
                    blank(
                        &mut code,
                        position + 1,
                        end.saturating_sub(1).max(position + 1),
                    );
                }
                position = end;
            }
            _ => position += 1,
        }
    }
    code
}

/// The position just past the string opened by `quote` at `start`.
fn string_end(text: &[u8], start: usize, quote: u8) -> usize {
    let mut position = start + 1;
    while position < text.len() {
        match text[position] {
            b'\\' => position += 2,
            byte if byte == quote => return position + 1,
            b'\n' if quote != b'`' => return position,
            _ => position += 1,
        }
    }
    text.len()
}

fn blank(code: &mut [u8], start: usize, end: usize) {
    let end = end.min(code.len());
    for byte in &mut code[start..end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

/// The first byte at or after `position` that is not white space.
fn next_byte(code: &[u8], position: usize) -> Option<u8> {
    code.get(position..)?
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
}

/// The position of the last byte before `position` that is not white space.
fn previous(code: &[u8], position: usize) -> Option<usize> {
    code[..position]
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
}

/// Whether the identifier at `start..end` is an object key: `name:`,
/// `"name":` or the shorthand `{ name }`.
fn is_key(code: &[u8], start: usize, end: usize) -> bool {
    let quoted =
        matches!(code.get(end), Some(b'"' | b'\'')) && start > 0 && code[start - 1] == code[end];
    let after = if quoted { end + 1 } else { end };
    match next_byte(code, after) {
        Some(b':') => true,
        Some(b',' | b'}') => {
            previous(code, start).is_some_and(|before| matches!(code[before], b'{' | b','))
        }
        _ => false,
    }
}

/// Whether the object literal around `position` is a comparison's options.
fn in_comparison(code: &[u8], position: usize) -> bool {
    let Some(open) = enclosing(code, position, b'{', b'}') else {
        return false;
    };
    let Some(before) = previous(code, open) else {
        return false;
    };
    let name_end = match code[before] {
        // The value of a config key: `toHaveScreenshot: { … }`.
        b':' => previous(code, before),
        // An argument: `toHaveScreenshot("home.png", { … })`.
        b'(' | b',' => enclosing(code, open, b'(', b')').and_then(|call| previous(code, call)),
        _ => None,
    };
    name_end.is_some_and(|end| {
        let end = if matches!(code[end], b'"' | b'\'') {
            end
        } else {
            end + 1
        };
        let start = code[..end]
            .iter()
            .rposition(|&byte| !continues_identifier(byte))
            .map_or(0, |offset| offset + 1);
        COMPARISONS
            .iter()
            .any(|comparison| comparison.as_bytes() == &code[start..end])
    })
}

/// The position of the `open` byte whose pair encloses `position`.
fn enclosing(code: &[u8], position: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0_usize;
    for index in (0..position).rev() {
        let byte = code[index];
        if byte == close {
            depth += 1;
        } else if byte == open {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
    }
    None
}
