/**
 * Cross-language ABI stub for synapse-market.
 * Status: NOT EXECUTABLE — requires bun-ffi + synapse_market.dylib (feature = "c-api").
 *
 * Once built (`cargo build --features c-api --release`):
 *   import { dlopen, FFIType, suffix } from "bun:ffi";
 *   const lib = dlopen(`libsynapse_market.${suffix}`, { ... });
 *   // Call smx_range(path, ts_start, ts_end) -> JSON string
 *   const json = lib.symbols.smx_range_json("abi.smx", ts_start, ts_end);
 *   const hash = new Bun.CryptoHasher("blake3").update(json).digest("hex");
 *   console.log(hash);
 *
 * The abi_cross_lang.rs test will compare this hash against the Rust reference hash.
 */
throw new Error("bun-ffi binding not yet built — add feature 'c-api' to synapse-market");
