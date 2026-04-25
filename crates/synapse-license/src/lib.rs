use anyhow::Result;
use base64::Engine;
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    pub customer_id: String,
    pub tier: String,
    pub expires_at: u64,
    pub hw_fingerprint: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    customer_id: String,
    tier: String,
    exp: u64,
    hw_fingerprint: String,
}

#[derive(Debug, Error)]
pub enum LicenseError {
    #[error("JWT decode/verify failed: {0}")]
    InvalidToken(String),
    #[error("License expired")]
    Expired,
    #[error("Hardware fingerprint mismatch")]
    HwMismatch,
}

#[derive(Debug, Serialize, Deserialize)]
struct LicenseCache {
    last_valid_ts: u64,
    hw_fingerprint: String,
}

fn cache_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("synapse")
        .join("license-cache.json")
}

fn load_cache() -> Option<LicenseCache> {
    let path = cache_path();
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_cache(cache: &LicenseCache) {
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(data) = serde_json::to_string(cache) {
        let _ = std::fs::write(path, data);
    }
}

const OFFLINE_GRACE_SECS: u64 = 30 * 24 * 3600;

/// Verify a license JWT.
/// `public_key_raw` must be the raw 32-byte Ed25519 public key (not DER-wrapped).
pub fn verify_license(jwt: &str, public_key_raw: &[u8]) -> Result<License> {
    let x = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public_key_raw);
    let decoding_key = DecodingKey::from_ed_components(&x)
        .map_err(|e| LicenseError::InvalidToken(e.to_string()))?;

    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.validate_exp = false; // we validate manually for grace logic

    let token_data = jsonwebtoken::decode::<Claims>(jwt, &decoding_key, &validation)
        .map_err(|e| LicenseError::InvalidToken(e.to_string()))?;

    let claims = token_data.claims;
    let now = Utc::now().timestamp() as u64;

    // Check expiry with 30-day offline grace
    if now > claims.exp {
        let cache = load_cache();
        let grace_ok = cache
            .as_ref()
            .map(|c| now - c.last_valid_ts <= OFFLINE_GRACE_SECS)
            .unwrap_or(false);
        if !grace_ok {
            return Err(LicenseError::Expired.into());
        }
    }

    // Hardware fingerprint check
    let local_fp = current_hw_fingerprint();
    if local_fp != claims.hw_fingerprint {
        return Err(LicenseError::HwMismatch.into());
    }

    // Cache successful verification
    save_cache(&LicenseCache {
        last_valid_ts: now,
        hw_fingerprint: local_fp.clone(),
    });

    Ok(License {
        customer_id: claims.customer_id,
        tier: claims.tier,
        expires_at: claims.exp,
        hw_fingerprint: local_fp,
    })
}

pub fn current_hw_fingerprint() -> String {
    let machine_uid = get_machine_uid();
    let mac = get_first_mac();
    let cpu = get_cpu_brand();

    let combined = format!("{machine_uid}|{mac}|{cpu}");
    let hash = blake3::hash(combined.as_bytes());
    hash.to_hex().to_string()
}

fn get_machine_uid() -> String {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if let Ok(out) = Command::new("system_profiler")
            .args(["SPHardwareDataType"])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.contains("Hardware UUID") {
                    if let Some(uid) = line.split(':').nth(1) {
                        return uid.trim().to_string();
                    }
                }
            }
        }
        "unknown-macos".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/etc/machine-id")
            .unwrap_or_else(|_| "unknown-linux".to_string())
            .trim()
            .to_string()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "unknown-platform".to_string()
    }
}

fn get_first_mac() -> String {
    // Platform-agnostic: read from /sys on Linux, system_profiler on macOS
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let iface = name.to_string_lossy();
                if iface == "lo" {
                    continue;
                }
                let path = format!("/sys/class/net/{}/address", iface);
                if let Ok(mac) = std::fs::read_to_string(&path) {
                    let mac = mac.trim().to_string();
                    if !mac.is_empty() && mac != "00:00:00:00:00:00" {
                        return mac;
                    }
                }
            }
        }
        "00:00:00:00:00:00".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if let Ok(out) = Command::new("ifconfig").arg("en0").output() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("ether ") {
                    return rest.trim().to_string();
                }
            }
        }
        "00:00:00:00:00:00".to_string()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "00:00:00:00:00:00".to_string()
    }
}

fn get_cpu_brand() -> String {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if let Ok(out) = Command::new("sysctl").args(["-n", "machdep.cpu.brand_string"]).output() {
            return String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        "unknown-cpu".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in content.lines() {
                if line.starts_with("model name") {
                    if let Some(val) = line.split(':').nth(1) {
                        return val.trim().to_string();
                    }
                }
            }
        }
        "unknown-cpu".to_string()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "unknown-cpu".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header};
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    struct TestKeypair {
        pkcs8_der: Vec<u8>,
        pub_raw: Vec<u8>,
    }

    fn make_keypair() -> TestKeypair {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let pub_raw = pair.public_key().as_ref().to_vec();
        TestKeypair {
            pkcs8_der: pkcs8.as_ref().to_vec(),
            pub_raw,
        }
    }

    fn mint_jwt(kp: &TestKeypair, claims: &Claims) -> String {
        let enc_key = EncodingKey::from_ed_der(&kp.pkcs8_der);
        jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), claims, &enc_key).unwrap()
    }

    fn make_claims(exp: u64, hw: &str) -> Claims {
        Claims {
            customer_id: "cust-001".to_string(),
            tier: "pro".to_string(),
            exp,
            hw_fingerprint: hw.to_string(),
        }
    }

    #[test]
    fn valid_license_verifies() {
        let kp = make_keypair();
        let fp = current_hw_fingerprint();
        let exp = (Utc::now().timestamp() as u64) + 86400 * 365;
        let claims = make_claims(exp, &fp);
        let jwt = mint_jwt(&kp, &claims);

        let result = verify_license(&jwt, &kp.pub_raw);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        let lic = result.unwrap();
        assert_eq!(lic.tier, "pro");
        assert_eq!(lic.customer_id, "cust-001");
    }

    #[test]
    fn tampered_jwt_rejected() {
        let kp = make_keypair();
        let fp = current_hw_fingerprint();
        let exp = (Utc::now().timestamp() as u64) + 86400 * 365;
        let claims = make_claims(exp, &fp);
        let jwt = mint_jwt(&kp, &claims);

        // Flip a character in the signature (last segment)
        let mut parts: Vec<&str> = jwt.splitn(3, '.').collect();
        let mut sig = parts[2].to_string();
        let last = sig.pop().unwrap_or('A');
        sig.push(if last == 'A' { 'B' } else { 'A' });
        parts[2] = Box::leak(sig.into_boxed_str());
        let tampered = parts.join(".");

        let result = verify_license(&tampered, &kp.pub_raw);
        assert!(result.is_err(), "expected Err for tampered JWT");
    }
}
