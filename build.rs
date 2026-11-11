// Compiles the vendored tree-sitter grammar in grammar/ (see grammar/README.md) with the
// flags of the grammar crate's own build script (bindings/rust/build.rs there).
fn main() {
    let dir = std::path::Path::new("grammar");
    let mut c = cc::Build::new();
    c.include(dir)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .flag_if_supported("-Wno-trigraphs")
        .file(dir.join("parser.c"))
        .file(dir.join("scanner.c"));
    // the target's toolchain, not the host's: a cross build from Linux to msvc needs it too
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        c.flag("-utf-8");
    }
    c.compile("tree-sitter-groovy");
    println!("cargo:rerun-if-changed=grammar");
}
