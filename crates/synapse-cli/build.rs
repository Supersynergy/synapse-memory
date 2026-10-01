fn main() {
    // MSVC links the main thread with a 1 MiB stack reserve by default.
    // Embedder init (fastembed/ORT, tokenizers) and deep recursive paths in
    // `synx put`/`hybrid` overflow it — observed as `thread 'main' has
    // overflowed its stack` on windows-x64 CI. Bump the PE stack reserve.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/STACK:16777216");
    }
}
