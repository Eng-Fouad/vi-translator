//! The translation memory and the lines log on disk, encrypted: one record per line, each sealed on
//! its own (AES-256-GCM, random nonce) so the files can still be appended to a line at a time.
//! A line is base64(nonce ‖ ciphertext); the record's kind ("memory/ar", "lines-log") is bound in as
//! associated data, so a record can't be moved from one file to another.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use sha2::{Digest, Sha256};

/// The key for local files: derived from the app's key, so it differs from the shared packs' key.
fn key() -> [u8; 32] {
    static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    *KEY.get_or_init(|| {
        let mut h = Sha256::new();
        h.update(crate::sync::key());
        h.update(b"vi-translator local files v1");
        h.finalize().into()
    })
}

/// Seal one record (a line of JSON) as one line of text.
pub fn seal(kind: &str, plain: &str) -> String {
    seal_with(&key(), kind, plain)
}

/// Open a line written by `seal`; None if it isn't one (or was changed).
pub fn open(kind: &str, line: &str) -> Option<String> {
    open_with(&key(), kind, line)
}

fn seal_with(key: &[u8; 32], kind: &str, plain: &str) -> String {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = cipher.encrypt(&nonce, Payload { msg: plain.as_bytes(), aad: kind.as_bytes() }).expect("encrypts");
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    base64::engine::general_purpose::STANDARD.encode(out)
}

fn open_with(key: &[u8; 32], kind: &str, line: &str) -> Option<String> {
    let raw = base64::engine::general_purpose::STANDARD.decode(line.trim()).ok()?;
    if raw.len() < 12 + 16 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let plain = cipher.decrypt(Nonce::from_slice(&raw[..12]), Payload { msg: &raw[12..], aad: kind.as_bytes() }).ok()?;
    String::from_utf8(plain).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seals_and_opens_one_record() {
        let k = [7u8; 32];
        let line = seal_with(&k, "memory/ar", r#"{"en":"hi","ar":"مرحبا","source":"local"}"#);
        // (not "hi" itself: random base64 has it now and then)
        assert!(!line.contains("\"en\"") && !line.contains("مرحبا") && !line.contains('\n'), "nothing readable, one line");
        assert_eq!(open_with(&k, "memory/ar", &line).as_deref(), Some(r#"{"en":"hi","ar":"مرحبا","source":"local"}"#));
        assert!(open_with(&k, "lines-log", &line).is_none(), "another kind must not open");
        assert!(open_with(&[8u8; 32], "memory/ar", &line).is_none(), "another key must not open");
        assert_ne!(seal_with(&k, "memory/ar", "x"), seal_with(&k, "memory/ar", "x"), "a new nonce every time");
        assert!(open_with(&k, "memory/ar", "not base64 !").is_none());
    }
}
