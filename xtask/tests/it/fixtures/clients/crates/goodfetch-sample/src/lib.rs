//! A crate that is neither a Product's executable crate nor a Rust SDK
//! crate: its tests implement no case on the `cli` or `sdk-rust` surface.

#[cfg(test)]
mod tests {
    mod client_conformance {
        mod gone {
            #[test]
            fn attached() {
                let _case = ClientCaseRun::start(None, "gone", RustMode::Linked).unwrap();
            }
        }
    }
}
