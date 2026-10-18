//! A crate whose snapshot test has no stored snapshot.

#[cfg(test)]
mod tests {
    #[test]
    fn greeting() {
        insta::assert_snapshot!("greeting", "hello");
    }
}
