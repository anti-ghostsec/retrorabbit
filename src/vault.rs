//! The only thing RetroRabbit ever writes to disk: your channel lineup, and only
//! when you press Save, encrypted with a passphrase (Argon2id + XChaCha20-Poly1305).
//!
//! File layout: "RRV1" | salt (16) | nonce (24) | ciphertext

use crate::schedule::Lineup;
use argon2::Argon2;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, rand_core::RngCore};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::path::PathBuf;
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"RRV1";

pub fn default_path() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("retrorabbit").join("lineup.rrv")
}

fn key(pass: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::default().hash_password_into(pass.as_bytes(), salt, &mut *key).map_err(|e| e.to_string())?;
    Ok(key)
}

pub fn seal(lineup: &Lineup, pass: &str) -> Result<Vec<u8>, String> {
    if pass.is_empty() {
        return Err("enter a passphrase first".into());
    }
    let plain = Zeroizing::new(serde_json::to_vec(lineup).map_err(|e| e.to_string())?);
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let key = key(pass, &salt)?;
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct = cipher.encrypt(&nonce, plain.as_slice()).map_err(|_| "encryption failed")?;
    let mut out = Vec::with_capacity(4 + 16 + 24 + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn open(data: &[u8], pass: &str) -> Result<Lineup, String> {
    if data.len() < 44 || &data[..4] != MAGIC {
        return Err("not a RetroRabbit lineup file".into());
    }
    let key = key(pass, &data[4..20])?;
    let cipher = XChaCha20Poly1305::new((&*key).into());
    let plain = Zeroizing::new(
        cipher
            .decrypt(XNonce::from_slice(&data[20..44]), &data[44..])
            .map_err(|_| "wrong passphrase (or the file is damaged)")?,
    );
    serde_json::from_slice(&plain).map_err(|e| e.to_string())
}

pub fn save(lineup: &Lineup, pass: &str, path: &PathBuf) -> Result<(), String> {
    let data = seal(lineup, pass)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("rrv.tmp");
    write_private(&tmp, &data)?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn load(pass: &str, path: &PathBuf) -> Result<Lineup, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    open(&data, pass)
}

fn write_private(path: &PathBuf, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(data).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let lineup = Lineup { media_roots: vec!["/v".into()], volume: 50.0, ..Default::default() };
        let sealed = seal(&lineup, "hunter2").unwrap();
        assert!(!sealed.windows(2).any(|w| w == b"/v"));
        let back = open(&sealed, "hunter2").unwrap();
        assert_eq!(back.media_roots, lineup.media_roots);
        assert!(open(&sealed, "wrong").is_err());
    }
}
