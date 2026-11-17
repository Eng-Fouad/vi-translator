//! Auto-update: "Version X is available — Update now".
//!
//! On start (after a short wait) and every hour, the app asks GitHub for the latest release of
//! the app. If it is newer than this build, the window shows a banner. "Update now" downloads this
//! platform's executable and its signature, checks the signature against the public key built in
//! below (the private key exists only as a secret of the release workflow), puts the new
//! executable in place of the running one (the old one is moved aside as "<name>.old" and removed
//! on the next start) and restarts. Anything that doesn't check out leaves the app as it is.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::config::Config;

/// Where releases are published.
const RELEASES: &str = "https://api.github.com/repos/Eng-Fouad/vi-translator/releases/latest";
/// Ed25519 public key that release files are signed with (base64 of the 32 raw bytes).
pub(crate) const PUBLIC_KEY: &str = "kQ2qBATJSRXwW3PtomkxkIUplaHqPHwoq+z3zJjTpWk=";
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// The release file for this platform: vi-translator-<windows|linux|macos>-<x64|arm64>[.exe]
pub fn asset_name() -> String {
    let os = if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" };
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    format!("vi-translator-{os}-{arch}{}", if cfg!(windows) { ".exe" } else { "" })
}

/// "1.2.3" or "v1.2.3" -> (1, 2, 3)
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let r = (it.next()??, it.next()??, it.next()??);
    it.next().is_none().then_some(r)
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Update {
    pub version: String,
    pub notes: String,
    #[serde(skip)]
    pub url: String,
    #[serde(skip)]
    pub sig_url: String,
}

/// The newest release, if it is newer than `current` and has a file (and signature) for this platform.
pub fn newer_release(json: &serde_json::Value, current: &str) -> Option<Update> {
    let tag = json["tag_name"].as_str()?;
    if json["draft"].as_bool() == Some(true) || json["prerelease"].as_bool() == Some(true) {
        return None;
    }
    if parse_version(tag)? <= parse_version(current)? {
        return None;
    }
    let assets = json["assets"].as_array()?;
    let url_of = |name: &str| {
        assets.iter().find(|a| a["name"].as_str() == Some(name)).and_then(|a| a["browser_download_url"].as_str()).map(String::from)
    };
    let name = asset_name();
    Some(Update {
        version: tag.trim_start_matches('v').to_string(),
        notes: json["body"].as_str().unwrap_or("").chars().take(2000).collect(),
        url: url_of(&name)?,
        sig_url: url_of(&format!("{name}.sig"))?,
    })
}

/// What a release file's signature (`.sig`) signs: what it's for, the file's name (its system), the
/// version and the file's SHA-256. So a signed file can't pass for another version (an older one)
/// or another system's, and nothing else signed with the same key (a page script) for an update.
pub fn signed_text(asset: &str, version: &str, data: &[u8]) -> String {
    use sha2::Digest as _;
    format!("vi-translator-update-v1\n{asset}\n{version}\n{:x}\n", sha2::Sha256::digest(data))
}

/// Does `sig_b64` (base64 Ed25519 signature) sign `data` with `pub_b64`?
pub fn verify(data: &[u8], sig_b64: &str, pub_b64: &str) -> Result<()> {
    use base64::Engine;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let b64 = base64::engine::general_purpose::STANDARD;
    let key: [u8; 32] = b64.decode(pub_b64.trim())?.try_into().map_err(|_| anyhow!("bad public key"))?;
    let sig: [u8; 64] = b64.decode(sig_b64.trim())?.try_into().map_err(|_| anyhow!("bad signature"))?;
    VerifyingKey::from_bytes(&key)?.verify(data, &Signature::from_bytes(&sig)).map_err(|_| anyhow!("the signature does not match"))
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("vi-translator/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(300))
        .build()
        .expect("http client")
}

/// The latest release, if newer than this build.
pub fn check() -> Result<Option<Update>> {
    let r = http().get(RELEASES).header("Accept", "application/vnd.github+json").timeout(Duration::from_secs(20)).send()?;
    if r.status().as_u16() == 404 {
        return Ok(None); // nothing released yet
    }
    let json: serde_json::Value = r.error_for_status()?.json()?;
    Ok(newer_release(&json, env!("CARGO_PKG_VERSION")))
}

/// Download, verify, swap the executable. Returns the path of the new executable (to restart).
pub fn install(u: &Update) -> Result<PathBuf> {
    let c = http();
    let data = c.get(&u.url).send()?.error_for_status()?.bytes().context("download")?;
    let sig = c.get(&u.sig_url).send()?.error_for_status()?.text().context("download signature")?;
    verify(signed_text(&asset_name(), &u.version, &data).as_bytes(), &sig, PUBLIC_KEY)
        .context("the downloaded update is not signed by VI Translator for this version and system")?;
    let exe = std::env::current_exe()?;
    replace_exe(&exe, &data)?;
    Ok(exe)
}

/// Put `data` in place of the executable at `exe`: the running file is renamed (allowed on
/// Windows while it runs) and the new one written under the original name.
fn replace_exe(exe: &Path, data: &[u8]) -> Result<()> {
    let old = old_path(exe);
    let _ = std::fs::remove_file(&old);
    let tmp = exe.with_extension("new");
    std::fs::write(&tmp, data).context("write the new version")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(exe, &old).context("move the old version aside")?;
    if let Err(e) = std::fs::rename(&tmp, exe) {
        let _ = std::fs::rename(&old, exe); // put the old one back
        bail!("put the new version in place: {e}");
    }
    Ok(())
}

fn old_path(exe: &Path) -> PathBuf {
    let mut p = exe.as_os_str().to_owned();
    p.push(".old");
    PathBuf::from(p)
}

/// Remove what the last update left behind.
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(old_path(&exe));
        let _ = std::fs::remove_file(exe.with_extension("new"));
    }
}

/// Background check: on start (after a minute) and every hour; tells the window when an update exists.
pub fn run(app: AppHandle, cfg: Arc<Mutex<Config>>, found: Arc<Mutex<Option<Update>>>) {
    clean_up();
    std::thread::sleep(Duration::from_secs(60));
    loop {
        if cfg.lock().unwrap().update_check {
            if let Ok(Some(u)) = check() {
                *found.lock().unwrap() = Some(u.clone());
                let _ = app.emit("update-available", &u);
            }
        }
        std::thread::sleep(CHECK_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.10.0"), Some((0, 10, 0)));
        assert!(parse_version("v1.2").is_none() && parse_version("1.2.3-beta").is_none() && parse_version("1.2.3.4").is_none());
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
    }

    fn release(tag: &str, with_sig: bool) -> serde_json::Value {
        let name = asset_name();
        let mut assets = vec![serde_json::json!({ "name": name, "browser_download_url": "https://x/bin" })];
        if with_sig {
            assets.push(serde_json::json!({ "name": format!("{name}.sig"), "browser_download_url": "https://x/sig" }));
        }
        serde_json::json!({ "tag_name": tag, "draft": false, "prerelease": false, "body": "notes", "assets": assets })
    }

    #[test]
    fn only_newer_signed_releases() {
        let u = newer_release(&release("v0.2.0", true), "0.1.0").expect("newer");
        assert_eq!((u.version.as_str(), u.url.as_str(), u.sig_url.as_str()), ("0.2.0", "https://x/bin", "https://x/sig"));
        assert!(newer_release(&release("v0.1.0", true), "0.1.0").is_none(), "same version");
        assert!(newer_release(&release("v0.0.9", true), "0.1.0").is_none(), "older");
        assert!(newer_release(&release("v0.2.0", false), "0.1.0").is_none(), "no signature: no update");
        let mut pre = release("v0.2.0", true);
        pre["prerelease"] = serde_json::json!(true);
        assert!(newer_release(&pre, "0.1.0").is_none(), "pre-release");
    }

    #[test]
    fn signatures() {
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};
        let b64 = base64::engine::general_purpose::STANDARD;
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let public = b64.encode(key.verifying_key().to_bytes());
        let data = b"new version";
        let sig = b64.encode(key.sign(data).to_bytes());
        assert!(verify(data, &sig, &public).is_ok());
        assert!(verify(b"tampered", &sig, &public).is_err(), "changed file");
        assert!(verify(data, &sig, PUBLIC_KEY).is_err(), "signed with another key");
        assert!(verify(data, "not base64!", &public).is_err());
    }

    #[test]
    fn swaps_the_executable() {
        let dir = std::env::temp_dir().join(format!("vit-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("vi-translator.exe");
        std::fs::write(&exe, b"old").unwrap();
        replace_exe(&exe, b"new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod openssl_signature {
    /// A file signed the way the release workflow signs (`signed_text` of it, with openssl pkeyutl
    /// -sign -rawin, base64): set VIT_SIGNED_FILE to its path (its signature is next to it,
    /// "<path>.sig", and the file keeps its release name) and VIT_SIGNED_VERSION to its version.
    #[test]
    #[ignore]
    fn verifies_with_the_built_in_key() {
        // VIT_SIGNED_FILE: a release file (its .sig next to it); VIT_SIGNED_VERSION: its version
        let path = std::env::var("VIT_SIGNED_FILE").expect("VIT_SIGNED_FILE");
        let version = std::env::var("VIT_SIGNED_VERSION").expect("VIT_SIGNED_VERSION");
        let name = std::path::Path::new(&path).file_name().unwrap().to_string_lossy().to_string();
        let data = std::fs::read(&path).unwrap();
        let sig = std::fs::read_to_string(format!("{path}.sig")).unwrap();
        super::verify(super::signed_text(&name, &version, &data).as_bytes(), &sig, super::PUBLIC_KEY).unwrap();
        let mut bad = data.clone();
        bad[0] ^= 1;
        assert!(super::verify(super::signed_text(&name, &version, &bad).as_bytes(), &sig, super::PUBLIC_KEY).is_err());
    }

    #[test]
    fn a_signature_is_for_one_file_version_and_system() {
        use base64::Engine as _;
        use ed25519_dalek::Signer as _;
        let b64 = base64::engine::general_purpose::STANDARD;
        let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let public = b64.encode(key.verifying_key().to_bytes());
        let data = b"the new version".to_vec();
        let sig = b64.encode(key.sign(super::signed_text("vi-translator-windows-x64.exe", "1.2.0", &data).as_bytes()).to_bytes());
        let ok = |asset: &str, version: &str, data: &[u8]| super::verify(super::signed_text(asset, version, data).as_bytes(), &sig, &public).is_ok();
        assert!(ok("vi-translator-windows-x64.exe", "1.2.0", &data));
        assert!(!ok("vi-translator-linux-x64", "1.2.0", &data), "another system's file");
        assert!(!ok("vi-translator-windows-x64.exe", "1.1.0", &data), "another version");
        assert!(!ok("vi-translator-windows-x64.exe", "1.2.0", b"something else"), "another file");
        // a signature of the bare file (what a page script's is) doesn't do
        let raw = b64.encode(key.sign(&data).to_bytes());
        assert!(super::verify(super::signed_text("vi-translator-windows-x64.exe", "1.2.0", &data).as_bytes(), &raw, &public).is_err());
    }
}
