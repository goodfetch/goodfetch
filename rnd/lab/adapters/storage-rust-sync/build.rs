fn main() {
    let native = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.native")
        .canonicalize()
        .expect("run the shared SQLite build first");
    println!("cargo:rustc-link-search=native={}", native.display());
    println!("cargo:rustc-link-lib=dylib=goodfetch_sqlite");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", native.display());
    println!(
        "cargo:rerun-if-changed={}/{}",
        native.display(),
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") { "libgoodfetch_sqlite.dylib" } else { "libgoodfetch_sqlite.so" }
    );
    println!(
        "cargo:rerun-if-changed={}/libgoodfetch_sqlite.so",
        native.display()
    );
}
