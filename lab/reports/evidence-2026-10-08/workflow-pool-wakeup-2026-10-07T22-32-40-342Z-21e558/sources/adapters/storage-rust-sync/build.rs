fn main() {
    let native = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.native")
        .canonicalize()
        .expect("run the shared SQLite build first");
    println!("cargo:rustc-link-search=native={}", native.display());
    println!("cargo:rustc-link-lib=dylib=automan_sqlite");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", native.display());
    println!(
        "cargo:rerun-if-changed={}/libautoman_sqlite.dylib",
        native.display()
    );
    println!(
        "cargo:rerun-if-changed={}/libautoman_sqlite.so",
        native.display()
    );
}
