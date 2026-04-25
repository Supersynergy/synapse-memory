// client-verify.rs — embeddable license verifier for Synapse client binaries.
//
// Add to crate Cargo.toml:
//   jsonwebtoken = "9.3"
//   obfstr = "0.4"
//   sha2 = "0.10"
//   anyhow = "1"
//   serde = { version = "1", features = ["derive"] }
//   [target.'cfg(target_os="linux")'.dependencies]
//   libc = "0.2"
//
// Usage from main():
//   let lic = client_verify::verify_license(&jwt, &hw_fp())?;

use anyhow::{anyhow, bail, Result};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use obfstr::obfstr as s;
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize, Clone)]
pub struct Claims {
    pub sub: String, pub cid: String, pub fp: String,
    pub iat: i64,    pub exp: i64,
}

/// Embedded Ed25519 public key (PEM). Obfuscated at rest via obfstr; decoded only at use.
fn embedded_pubkey_pem() -> String {
    // Replace at build-time via build.rs reading PUB_KEY_PEM env.
    s!("-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA/FxYUTzRxZrmjxQ6s2Ulynf/RqFUmiynSPn1pHA7Luw=\n-----END PUBLIC KEY-----\n").to_string()
}

#[allow(dead_code)]
pub fn license_endpoint() -> String {
    s!("https://license.synapse.example/activate").to_string()
}

#[cfg(target_os = "linux")]
fn anti_debug() -> Result<()> {
    // ptrace(PTRACE_TRACEME) returns -1 if already traced.
    unsafe {
        let r = libc::ptrace(libc::PTRACE_TRACEME, 0, std::ptr::null_mut::<libc::c_void>(), std::ptr::null_mut::<libc::c_void>());
        if r < 0 { bail!("trace check failed"); }
    }
    // Also scan /proc/self/status for TracerPid.
    if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("TracerPid:") {
                if rest.trim() != "0" { bail!("debugger attached"); }
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn anti_debug() -> Result<()> {
    // sysctl KERN_PROC | KERN_PROC_PID -> kp_proc.p_flag & P_TRACED
    use std::mem::size_of;
    const CTL_KERN: i32 = 1;
    const KERN_PROC: i32 = 14;
    const KERN_PROC_PID: i32 = 1;
    const P_TRACED: i32 = 0x00000800;

    #[repr(C)] #[derive(Default)]
    struct KInfoProc { pad: [u8; 432] } // sized envelope; we read p_flag at offset 32

    unsafe {
        let mut mib: [i32; 4] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, libc::getpid()];
        let mut info = KInfoProc::default();
        let mut size = size_of::<KInfoProc>();
        let r = libc::sysctl(mib.as_mut_ptr(), 4,
            &mut info as *mut _ as *mut libc::c_void, &mut size,
            std::ptr::null_mut(), 0);
        if r != 0 { return Ok(()); } // best-effort
        let p_flag = i32::from_ne_bytes([info.pad[32], info.pad[33], info.pad[34], info.pad[35]]);
        if p_flag & P_TRACED != 0 { bail!("debugger attached"); }
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn anti_debug() -> Result<()> { Ok(()) }

pub fn verify_license(jwt: &str, hw_fp: &str) -> Result<Claims> {
    anti_debug()?;
    let pem = embedded_pubkey_pem();
    let key = DecodingKey::from_ed_pem(pem.as_bytes())
        .map_err(|e| anyhow!("bad embedded pubkey: {e}"))?;
    let mut v = Validation::new(Algorithm::EdDSA);
    v.leeway = 30;
    v.validate_exp = true;
    let data = decode::<Claims>(jwt, &key, &v)
        .map_err(|e| anyhow!("jwt verify failed: {e}"))?;
    let c = data.claims;
    if c.fp != hw_fp { bail!("hardware fingerprint mismatch"); }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    if c.exp <= now { bail!("license expired"); }
    Ok(c)
}

/// Compute a stable hardware fingerprint. Combine machine-id + cpu brand + primary MAC.
pub fn hw_fingerprint() -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    #[cfg(target_os = "linux")] {
        if let Ok(s) = std::fs::read_to_string("/etc/machine-id") { h.update(s.as_bytes()); }
        if let Ok(s) = std::fs::read_to_string("/sys/class/dmi/id/product_uuid") { h.update(s.as_bytes()); }
    }
    #[cfg(target_os = "macos")] {
        if let Ok(out) = std::process::Command::new("/usr/sbin/ioreg")
            .args(["-rd1","-c","IOPlatformExpertDevice"]).output() {
            h.update(&out.stdout);
        }
    }
    let digest = h.finalize();
    Ok(hex_lower(&digest))
}

fn hex_lower(b: &[u8]) -> String {
    const T: &[u8;16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len()*2);
    for &x in b { s.push(T[(x>>4) as usize] as char); s.push(T[(x&0xf) as usize] as char); }
    s
}
