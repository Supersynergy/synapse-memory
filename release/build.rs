// build.rs — propagates SYNAPSE_PUBKEY_PEM and SYNAPSE_INTEGRITY_HASH from the
// build-environment into the compiled binary so client-verify can read them via
// option_env!()/env!(). When unset, dev-fallbacks in the source apply.
use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=SYNAPSE_PUBKEY_PEM");
    println!("cargo:rerun-if-env-changed=SYNAPSE_INTEGRITY_HASH");
    println!("cargo:rerun-if-env-changed=SYNAPSE_CUSTOMER_ID");

    if let Ok(pem) = env::var("SYNAPSE_PUBKEY_PEM") {
        println!("cargo:rustc-env=SYNAPSE_PUBKEY_PEM={pem}");
    }
    if let Ok(h) = env::var("SYNAPSE_INTEGRITY_HASH") {
        // BLAKE3 hex of stripped binary `.text` section, computed in build-secure.sh
        // pass 1; embedded in pass 2 so client can self-verify on startup.
        println!("cargo:rustc-env=SYNAPSE_INTEGRITY_HASH={h}");
    }
    if let Ok(c) = env::var("SYNAPSE_CUSTOMER_ID") {
        println!("cargo:rustc-env=SYNAPSE_CUSTOMER_ID={c}");
    }
}
