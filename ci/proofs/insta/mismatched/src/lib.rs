//! A crate whose snapshot test has a stored snapshot that differs.

#[cfg(test)]
mod tests {
    #[test]
    fn greeting() {
        insta::assert_snapshot!("greeting", "hello");
    }
}
