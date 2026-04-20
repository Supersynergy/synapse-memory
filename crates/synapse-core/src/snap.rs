//! .brainpack export/import: zstd(sqlite file), optionally Ed25519-signed.

use crate::error::{Error, Result};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

const MAGIC: &[u8; 4] = b"BPK1";
const VERSION: u8 = 1;
// VERSION 2 = signed pack: after compressed body, 32-byte pubkey + 64-byte sig over BLAKE3(body)
const VERSION_SIGNED: u8 = 2;

/// Write a signed .brainpack file. Appends pubkey+signature after body.
pub fn export_signed(
    db_path: impl AsRef<Path>,
    out: impl AsRef<Path>,
    level: i32,
    signing_key: &SigningKey,
) -> Result<()> {
    let compressed = build_compressed(db_path, level)?;
    let body_hash = blake3::hash(&compressed);
    let sig = crate::sign::sign_bytes(signing_key, body_hash.as_bytes());
    let vk = signing_key.verifying_key();
    let mut file = std::fs::File::create(out)?;
    use std::io::Write;
    file.write_all(MAGIC)?;
    file.write_all(&[VERSION_SIGNED])?;
    file.write_all(&(level as u32).to_le_bytes())?;
    // raw_len stored as placeholder (0 for signed packs — verified after decompress)
    let raw_len_pos = 0u64;
    file.write_all(&raw_len_pos.to_le_bytes())?;
    // 32 bytes hash of compressed body
    file.write_all(body_hash.as_bytes())?;
    file.write_all(&compressed)?;
    // trailer: 32-byte pubkey + 64-byte sig
    file.write_all(vk.as_bytes())?;
    file.write_all(&sig)?;
    Ok(())
}

/// Verify and import a signed .brainpack. Returns the embedded public key.
pub fn import_signed(pack: impl AsRef<Path>, out: impl AsRef<Path>, expected_vk: Option<&VerifyingKey>) -> Result<VerifyingKey> {
    let raw = std::fs::read(pack)?;
    if &raw[0..4] != MAGIC {
        return Err(Error::Other("bad magic".into()));
    }
    if raw[4] != VERSION_SIGNED {
        return Err(Error::Other("not a signed brainpack (version mismatch)".into()));
    }
    let _level = u32::from_le_bytes(raw[5..9].try_into().unwrap());
    // raw_len at [9..17] is 0 for signed packs
    let hash = &raw[17..49];
    // body: everything between header (49 bytes) and trailer (96 bytes)
    if raw.len() < 49 + 96 {
        return Err(Error::Other("signed brainpack too short".into()));
    }
    let body = &raw[49..raw.len() - 96];
    let pubkey_bytes: [u8; 32] = raw[raw.len()-96..raw.len()-64].try_into().unwrap();
    let sig_bytes: [u8; 64] = raw[raw.len()-64..].try_into().unwrap();
    let vk = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| Error::Other(format!("bad pubkey in pack: {e}")))?;
    // verify body hash
    let actual = blake3::hash(body);
    if actual.as_bytes() != hash {
        return Err(Error::Other("blake3 body hash mismatch".into()));
    }
    // verify signature
    crate::sign::verify_bytes(&vk, actual.as_bytes(), &sig_bytes)?;
    // optionally verify against expected key
    if let Some(exp) = expected_vk {
        if exp.as_bytes() != &pubkey_bytes {
            return Err(Error::Other("public key in pack does not match expected".into()));
        }
    }
    let data = zstd::decode_all(body)?;
    std::fs::write(out, data)?;
    Ok(vk)
}

fn build_compressed(db_path: impl AsRef<Path>, level: i32) -> Result<Vec<u8>> {
    let tmp = tempfile::NamedTempFile::new()?;
    let src = Connection::open(db_path.as_ref())?;
    let mut dst = Connection::open(tmp.path())?;
    let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
    backup.run_to_completion(128, std::time::Duration::from_millis(0), None)?;
    drop(backup);
    drop(dst);
    let data = std::fs::read(tmp.path())?;
    Ok(zstd::encode_all(&data[..], level)?)
}

/// Write a .brainpack file from a live Store path.
/// Does a proper SQLite backup to avoid WAL drift.
pub fn export(db_path: impl AsRef<Path>, out: impl AsRef<Path>, level: i32) -> Result<()> {
    let tmp = tempfile::NamedTempFile::new()?;
    let src = Connection::open(db_path.as_ref())?;
    let mut dst = Connection::open(tmp.path())?;
    let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
    backup.run_to_completion(128, std::time::Duration::from_millis(0), None)?;
    drop(backup);
    drop(dst);
    let data = std::fs::read(tmp.path())?;
    let blake = blake3::hash(&data);
    let compressed = zstd::encode_all(&data[..], level)?;
    let mut file = std::fs::File::create(out)?;
    use std::io::Write;
    file.write_all(MAGIC)?;
    file.write_all(&[VERSION])?;
    file.write_all(&(level as u32).to_le_bytes())?;
    file.write_all(&(data.len() as u64).to_le_bytes())?;
    file.write_all(blake.as_bytes())?;
    file.write_all(&compressed)?;
    Ok(())
}

/// Merge two brainpacks by URI-matching docs and merging meta_crdt per doc.
/// Writes a new brainpack to `out_pack`.
pub fn merge_packs(
    pack_a: impl AsRef<Path>,
    pack_b: impl AsRef<Path>,
    out_pack: impl AsRef<Path>,
    level: i32,
) -> Result<()> {
    let db_a = tempfile::NamedTempFile::new()?;
    let db_b = tempfile::NamedTempFile::new()?;
    let db_out = tempfile::NamedTempFile::new()?;
    import_magic(&pack_a, db_a.path())?;
    import_magic(&pack_b, db_b.path())?;

    // Copy A as base
    {
        let src = Connection::open(db_a.path())?;
        let mut dst = Connection::open(db_out.path())?;
        let backup = rusqlite::backup::Backup::new(&src, &mut dst)?;
        backup.run_to_completion(128, std::time::Duration::from_millis(0), None)?;
    }

    // Merge B docs into out: match by URI, merge meta_crdt
    let conn_b = Connection::open(db_b.path())?;
    let mut conn_out = Connection::open(db_out.path())?;

    let mut stmt = conn_b.prepare(
        "SELECT uri, title, text, meta, ts, blake3, sig, meta_crdt FROM docs"
    )?;
    let rows: Vec<(Option<String>, Option<String>, String, Option<String>, i64, Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>)> = stmt.query_map([], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
    })?.collect::<rusqlite::Result<_>>()?;

    let tx = conn_out.transaction()?;
    for (uri, title, text, meta, ts, blake3, sig, crdt_b) in rows {
        let existing_id: Option<(i64, Option<Vec<u8>>)> = if let Some(ref u) = uri {
            tx.query_row(
                "SELECT id, meta_crdt FROM docs WHERE uri = ?1",
                params![u],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ).optional()?
        } else {
            tx.query_row(
                "SELECT id, meta_crdt FROM docs WHERE blake3 = ?1",
                params![blake3],
                |r| Ok((r.get(0)?, r.get(1)?)),
            ).optional()?
        };

        if let Some((id, crdt_a)) = existing_id {
            if let Some(ref b_state) = crdt_b {
                let merged: Vec<u8> = match crdt_a {
                    Some(ref a_state) => crate::crdt::merge_meta(a_state, b_state)?,
                    None => b_state.clone(),
                };
                tx.execute("UPDATE docs SET meta_crdt = ?1 WHERE id = ?2", params![merged, id])?;
            }
        } else {
            tx.execute(
                "INSERT OR IGNORE INTO docs(uri,title,text,meta,ts,blake3,sig,meta_crdt) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![uri, title, text, meta, ts, blake3, sig, crdt_b],
            )?;
        }
    }
    tx.commit()?;
    drop(conn_out);

    export(db_out.path(), out_pack, level)
}

/// Import a brainpack ignoring file extension — uses magic bytes to determine format.
pub fn import_magic(pack: impl AsRef<Path>, out: impl AsRef<Path>) -> Result<()> {
    let raw = std::fs::read(pack.as_ref())?;
    if raw.len() < 5 {
        return Err(crate::Error::Other("file too short".into()));
    }
    if &raw[0..4] != MAGIC {
        return Err(crate::Error::Other(format!(
            "bad magic — expected BPK1, got {:?}. Extension is ignored; content determines format.",
            &raw[0..4]
        )));
    }
    if raw[4] == VERSION_SIGNED {
        // signed pack: just decompress body (skip sig verification for raw restore)
        let body = &raw[49..raw.len() - 96];
        let data = zstd::decode_all(body)?;
        std::fs::write(out, data)?;
        Ok(())
    } else {
        import_raw(&raw, out)
    }
}

fn import_raw(raw: &[u8], out: impl AsRef<Path>) -> Result<()> {
    let _version = raw[4];
    let _level = u32::from_le_bytes(raw[5..9].try_into().unwrap());
    let raw_len = u64::from_le_bytes(raw[9..17].try_into().unwrap()) as usize;
    let hash = &raw[17..49];
    let body = &raw[49..];
    let data = zstd::decode_all(body)?;
    if data.len() != raw_len {
        return Err(crate::Error::Other("size mismatch".into()));
    }
    let actual = blake3::hash(&data);
    if actual.as_bytes() != hash {
        return Err(crate::Error::Other("blake3 mismatch".into()));
    }
    std::fs::write(out, data)?;
    Ok(())
}

/// Restore a .brainpack into a fresh db file at `out`.
/// Extension is ignored — content (magic bytes) determines format.
/// Accepts .syn, .synapse, .brainpack, .bp — all equivalent.
pub fn import(pack: impl AsRef<Path>, out: impl AsRef<Path>) -> Result<()> {
    import_magic(pack, out)
}

/// Encrypt an existing .brainpack file with age passphrase. Writes `.brainpack.age`.
pub fn encrypt_pack(pack: impl AsRef<Path>, out: impl AsRef<Path>, passphrase: &str) -> Result<()> {
    use age::secrecy::SecretString;
    use std::io::Write as _;
    let data = std::fs::read(pack)?;
    let encryptor = age::Encryptor::with_user_passphrase(SecretString::new(passphrase.to_string().into()));
    let mut output = vec![];
    let mut writer = encryptor.wrap_output(&mut output)
        .map_err(|e| Error::Other(e.to_string()))?;
    writer.write_all(&data)?;
    writer.finish().map_err(|e| Error::Other(e.to_string()))?;
    std::fs::write(out, output)?;
    Ok(())
}

/// Decrypt an age-encrypted .brainpack file.
pub fn decrypt_pack(enc_pack: impl AsRef<Path>, out: impl AsRef<Path>, passphrase: &str) -> Result<()> {
    use age::secrecy::SecretString;
    use std::io::Read as _;
    let data = std::fs::read(enc_pack)?;
    let decryptor = match age::Decryptor::new(&data[..]).map_err(|e| Error::Other(e.to_string()))? {
        age::Decryptor::Passphrase(d) => d,
        _ => return Err(Error::Other("expected passphrase-encrypted pack".into())),
    };
    let mut reader = decryptor
        .decrypt(&age::secrecy::SecretString::new(passphrase.to_string().into()), None)
        .map_err(|e| Error::Other(e.to_string()))?;
    let mut plaintext = vec![];
    reader.read_to_end(&mut plaintext)?;
    std::fs::write(out, plaintext)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PutRequest, Store};

    #[test]
    fn roundtrip() {
        let db = tempfile::NamedTempFile::new().unwrap();
        {
            let mut s = Store::open(db.path()).unwrap();
            s.put(&PutRequest { text: "round trip test".into(), ..Default::default() }).unwrap();
        }
        let pack = tempfile::NamedTempFile::new().unwrap();
        export(db.path(), pack.path(), 3).unwrap();
        let restored = tempfile::NamedTempFile::new().unwrap();
        import(pack.path(), restored.path()).unwrap();
        let s = Store::open(restored.path()).unwrap();
        assert_eq!(s.stats().unwrap().docs, 1);
    }

    #[test]
    fn signed_roundtrip() {
        use ed25519_dalek::SigningKey;
        use rand::rngs::OsRng;
        let sk = SigningKey::generate(&mut OsRng);
        let db = tempfile::NamedTempFile::new().unwrap();
        {
            let mut s = Store::open(db.path()).unwrap();
            s.put(&PutRequest { text: "signed pack test".into(), ..Default::default() }).unwrap();
        }
        let pack = tempfile::NamedTempFile::new().unwrap();
        export_signed(db.path(), pack.path(), 3, &sk).unwrap();
        let restored = tempfile::NamedTempFile::new().unwrap();
        let vk = import_signed(pack.path(), restored.path(), None).unwrap();
        assert_eq!(vk.as_bytes(), sk.verifying_key().as_bytes());
        let s = Store::open(restored.path()).unwrap();
        assert_eq!(s.stats().unwrap().docs, 1);
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let db = tempfile::NamedTempFile::new().unwrap();
        {
            let mut s = Store::open(db.path()).unwrap();
            s.put(&PutRequest { text: "encrypted pack".into(), ..Default::default() }).unwrap();
        }
        let pack = tempfile::NamedTempFile::new().unwrap();
        export(db.path(), pack.path(), 3).unwrap();
        let enc = tempfile::NamedTempFile::new().unwrap();
        encrypt_pack(pack.path(), enc.path(), "s3cr3t").unwrap();
        let dec = tempfile::NamedTempFile::new().unwrap();
        decrypt_pack(enc.path(), dec.path(), "s3cr3t").unwrap();
        let restored = tempfile::NamedTempFile::new().unwrap();
        import(dec.path(), restored.path()).unwrap();
        let s = Store::open(restored.path()).unwrap();
        assert_eq!(s.stats().unwrap().docs, 1);
    }
}
