fn main() {
    // MSVC default main-thread stack is 1 MiB; embedder init can overflow it.
    // Bump the PE stack reserve.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/STACK:16777216");
    }
}
