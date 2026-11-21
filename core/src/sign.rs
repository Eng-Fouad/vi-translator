//! Ed25519 signatures: what the app's updates and the game pages scripts delivered in layout
//! updates are signed with (the private key exists only as a secret of the release workflow).

use anyhow::{Result, anyhow};

/// Ed25519 public key that release files and page scripts are signed with (base64 of the 32 raw bytes).
pub const PUBLIC_KEY: &str = "kQ2qBATJSRXwW3PtomkxkIUplaHqPHwoq+z3zJjTpWk=";

/// Does `sig_b64` (base64 Ed25519 signature) sign `data` with `pub_b64`?
pub fn verify(data: &[u8], sig_b64: &str, pub_b64: &str) -> Result<()> {
    use base64::Engine;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let b64 = base64::engine::general_purpose::STANDARD;
    let key: [u8; 32] = b64.decode(pub_b64.trim())?.try_into().map_err(|_| anyhow!("bad public key"))?;
    let sig: [u8; 64] = b64.decode(sig_b64.trim())?.try_into().map_err(|_| anyhow!("bad signature"))?;
    VerifyingKey::from_bytes(&key)?.verify(data, &Signature::from_bytes(&sig)).map_err(|_| anyhow!("the signature does not match"))
}
