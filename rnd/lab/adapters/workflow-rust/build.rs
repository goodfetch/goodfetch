fn main() {
    let native = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.native")
        .canonicalize()
        .expect("run shared-build first");
    println!("cargo:rustc-link-search=native={}", native.display());
    println!("cargo:rustc-link-lib=dylib=automan_sqlite");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", native.display());
    println!(
        "cargo:rerun-if-changed={}/{}",
        native.display(),
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") { "libautoman_sqlite.dylib" } else { "libautoman_sqlite.so" }
    );
}
