use anyhow::{anyhow, Result};
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

pub fn verify_license(jwt: &str, public_key_der: &[u8]) -> Result<License> {
    let decoding_key = DecodingKey::from_ed_der(public_key_der);

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
                if line.starts_with("ether ") {
                    return line[6..].trim().to_string();
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
    use ed25519_dalek::SigningKey;
    use jsonwebtoken::{EncodingKey, Header};
    use rand::rngs::OsRng;

    fn make_keypair() -> (SigningKey, ed25519_dalek::VerifyingKey) {
        let signing = SigningKey::generate(&mut OsRng);
        let verifying = signing.verifying_key();
        (signing, verifying)
    }

    fn mint_jwt(signing: &SigningKey, claims: &Claims) -> String {
        let der = signing.to_pkcs8_der_bytes();
        let enc_key = EncodingKey::from_ed_der(&der);
        jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), claims, &enc_key).unwrap()
    }

    trait ToPkcs8DerBytes {
        fn to_pkcs8_der_bytes(&self) -> Vec<u8>;
    }

    impl ToPkcs8DerBytes for SigningKey {
        fn to_pkcs8_der_bytes(&self) -> Vec<u8> {
            // Raw 32-byte seed wrapped in PKCS#8 DER for ed25519
            // OID: 1.3.101.112 → 06 03 2B 65 70
            let seed = self.to_bytes();
            let mut der = vec![
                0x30, 0x2e, // SEQUENCE (46 bytes)
                0x02, 0x01, 0x00, // INTEGER 0 (version)
                0x30, 0x05, // SEQUENCE (5 bytes)
                0x06, 0x03, 0x2b, 0x65, 0x70, // OID 1.3.101.112
                0x04, 0x22, // OCTET STRING (34 bytes)
                0x04, 0x20, // OCTET STRING (32 bytes)
            ];
            der.extend_from_slice(&seed);
            der
        }
    }

    fn verifying_key_der(vk: &ed25519_dalek::VerifyingKey) -> Vec<u8> {
        // SubjectPublicKeyInfo DER for Ed25519
        let raw = vk.as_bytes();
        let mut der = vec![
            0x30, 0x2a, // SEQUENCE (42 bytes)
            0x30, 0x05, // SEQUENCE (5 bytes)
            0x06, 0x03, 0x2b, 0x65, 0x70, // OID
            0x03, 0x21, 0x00, // BIT STRING (33 bytes, 0 unused bits)
        ];
        der.extend_from_slice(raw);
        der
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
        let (signing, verifying) = make_keypair();
        let fp = current_hw_fingerprint();
        let exp = (Utc::now().timestamp() as u64) + 86400 * 365;
        let claims = make_claims(exp, &fp);
        let jwt = mint_jwt(&signing, &claims);
        let pub_der = verifying_key_der(&verifying);

        let result = verify_license(&jwt, &pub_der);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result.err());
        let lic = result.unwrap();
        assert_eq!(lic.tier, "pro");
        assert_eq!(lic.customer_id, "cust-001");
    }

    #[test]
    fn tampered_jwt_rejected() {
        let (signing, verifying) = make_keypair();
        let fp = current_hw_fingerprint();
        let exp = (Utc::now().timestamp() as u64) + 86400 * 365;
        let claims = make_claims(exp, &fp);
        let jwt = mint_jwt(&signing, &claims);
        let pub_der = verifying_key_der(&verifying);

        // Flip a character in the signature (last segment)
        let mut parts: Vec<&str> = jwt.splitn(3, '.').collect();
        let mut sig = parts[2].to_string();
        let last = sig.pop().unwrap_or('A');
        sig.push(if last == 'A' { 'B' } else { 'A' });
        parts[2] = Box::leak(sig.into_boxed_str());
        let tampered = parts.join(".");

        let result = verify_license(&tampered, &pub_der);
        assert!(result.is_err(), "expected Err for tampered JWT");
    }
}
