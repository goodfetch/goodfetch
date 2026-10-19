//! A crate whose snapshot test passes beside a snapshot no test references.

#[cfg(test)]
mod tests {
    #[test]
    fn greeting() {
        insta::assert_snapshot!("greeting", "hello");
    }
}
