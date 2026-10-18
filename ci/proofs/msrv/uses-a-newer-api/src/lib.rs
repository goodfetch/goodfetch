//! `Option::map_or_default`, stable since Rust 1.98.

pub fn length(text: Option<&str>) -> usize {
    text.map_or_default(str::len)
}
