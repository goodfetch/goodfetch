//! A crate that is not a Product's executable crate: its tests implement no
//! case on the `cli` surface.

#[cfg(test)]
mod tests {
    mod client_conformance {
        mod gone {
            #[test]
            fn attached() {}
        }
    }
}
