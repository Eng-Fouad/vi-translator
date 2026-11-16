//! Background sync with the VI Translator server:
//! - shares the English lines the player saw (setting "Help improve translations", on by
//!   default): in batches about every 10 minutes, never one line at a time;
//! - downloads translations and layout updates (read areas, fonts, menu words) in the background.
//!
//! Nothing runs while no server is set. Only subtitles, instructions and screen captions are
//! shared (never menus, where player names appear), with a random install id, no account.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::config::Config;
use crate::memory::{self, Memory};

/// The project server. Empty: sync is off (set once the server is online).
pub const DEFAULT_SERVER: &str = "";
const UPLOAD_EVERY: Duration = Duration::from_secs(10 * 60);
const DOWNLOAD_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_LINES_PER_UPLOAD: usize = 500;
const MAX_LINE_CHARS: usize = 400;

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct State {
    /// how far into lines-log.jsonl has been shared (bytes)
    log_offset: u64,
    /// ETag of each download
    versions: HashMap<String, String>,
}

pub struct Paths {
    /// settings folder (AppData): downloads and sync state
    pub data: PathBuf,
    /// translation-db folder: the lines log
    pub db: PathBuf,
}

impl Paths {
    fn state(&self) -> PathBuf {
        self.data.join("sync-state.json")
    }
    fn shared(&self, game: &str, lang: &str) -> PathBuf {
        self.data.join(format!("shared-{game}-{lang}.bin"))
    }
    fn layout(&self, game: &str) -> PathBuf {
        self.data.join(format!("layout-{game}.json"))
    }
}

fn load_state(p: &Paths) -> State {
    std::fs::read_to_string(p.state()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_state(p: &Paths, s: &State) {
    let _ = std::fs::create_dir_all(&p.data);
    let _ = std::fs::write(p.state(), serde_json::to_string_pretty(s).unwrap_or_default());
}

fn server(c: &Config) -> Option<String> {
    let s = if c.server_url.trim().is_empty() { DEFAULT_SERVER } else { c.server_url.trim() };
    (!s.is_empty()).then(|| s.trim_end_matches('/').to_string())
}

fn game(c: &Config) -> Option<String> {
    (!c.profile.is_empty() && c.profile != "none").then(|| c.profile.clone())
}

/// Use what was downloaded before (works offline), then keep it up to date.
pub fn run(app: AppHandle, cfg: Arc<Mutex<Config>>, memory: Arc<Mutex<Memory>>, paths: Paths) {

    let http = reqwest::blocking::Client::builder()
        .user_agent(concat!("vi-translator/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client");
    let mut first = true;
    let (mut next_upload, mut next_download) = (Instant::now(), Instant::now());
    let mut last = (String::new(), String::new()); // game, language
    loop {
        let c = cfg.lock().unwrap().clone();
        // what was downloaded before works offline; another game or language: its own downloads
        if let Some(g) = game(&c) {
            if last != (g.clone(), c.target_lang.clone()) {
                last = (g.clone(), c.target_lang.clone());
                // (give the app a moment to switch its memory to the new language first)
                std::thread::sleep(Duration::from_secs(2));
                load_cached(&app, &cfg, &memory, &paths, &g, &c.target_lang);
                next_download = Instant::now();
            }
        }
        if first {
            first = false;
            std::thread::sleep(Duration::from_secs(20)); // let the app start first
        }
        if let (Some(url), Some(g)) = (server(&c), game(&c)) {
            if Instant::now() >= next_upload {
                next_upload = Instant::now() + UPLOAD_EVERY;
                share_lines(&http, &url, &c, &g, &paths);
            }
            if Instant::now() >= next_download {
                next_download = Instant::now() + DOWNLOAD_EVERY;
                download(&http, &url, &app, &cfg, &memory, &paths, &g, &c.target_lang);
            }
        }
        std::thread::sleep(Duration::from_secs(30));
    }
}

// ------------------------------------------------------------------------------------- sharing

#[derive(Deserialize)]
struct LogRow {
    area: String,
    en: String,
    start: u64,
    end: u64,
}

#[derive(Serialize)]
struct UploadLine<'a> {
    en: &'a str,
    area: &'a str,
    start: u64,
    end: u64,
}

/// Send the lines logged since the last upload. With sharing off, nothing is sent and the
/// lines seen meanwhile are skipped for good (turning it on later doesn't send old lines).
fn share_lines(http: &reqwest::blocking::Client, url: &str, c: &Config, game: &str, paths: &Paths) {
    let mut st = load_state(paths);
    let log = paths.db.join("lines-log.jsonl");
    let Ok(mut f) = std::fs::File::open(&log) else { return };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if st.log_offset > len {
        st.log_offset = 0; // the log was cleared
    }
    if !c.share_lines {
        st.log_offset = len;
        save_state(paths, &st);
        return;
    }
    let mut text = String::new();
    if f.seek(SeekFrom::Start(st.log_offset)).is_err() || f.read_to_string(&mut text).is_err() {
        return;
    }
    // whole lines only (one may still be being written)
    let complete = text.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let rows: Vec<LogRow> = text[..complete]
        .lines()
        .filter_map(|l| serde_json::from_str::<LogRow>(l).ok())
        // menus can hold player names: never shared
        .filter(|r| !r.area.starts_with("__menu") && !r.area.starts_with("__free"))
        .filter(|r| r.en.chars().count() <= MAX_LINE_CHARS && looks_like_text(&r.en))
        .collect();
    for chunk in rows.chunks(MAX_LINES_PER_UPLOAD) {
        let lines: Vec<UploadLine> = chunk.iter().map(|r| UploadLine { en: &r.en, area: &r.area, start: r.start, end: r.end.max(r.start) }).collect();
        let body = serde_json::json!({ "game": game, "clientVersion": env!("CARGO_PKG_VERSION"), "lines": lines });
        let sent = http.post(format!("{url}/v1/lines")).header("X-Install-Id", &c.install_id).json(&body).send();
        match sent {
            Ok(r) if r.status().is_success() => {}
            // the server refused these lines (too long, malformed): don't send them again
            Ok(r) if r.status().as_u16() == 400 || r.status().as_u16() == 413 => {}
            // offline, busy or rate-limited: try again next time, from here
            _ => return,
        }
    }
    st.log_offset += complete as u64;
    save_state(paths, &st);
}

/// Real words, not an OCR misread of icons ("bjlwg", "ulnJiJl hjluJ9"): a sentence ("Hi.",
/// "Go to the guard.") or words that include a common English one ("C'mon mister").
fn looks_like_text(en: &str) -> bool {
    const COMMON: [&str; 60] = [
        "the", "a", "an", "to", "you", "i", "it", "is", "we", "me", "my", "your", "and", "of", "in", "on", "for",
        "this", "that", "what", "go", "get", "no", "yes", "not", "don't", "i'm", "it's", "c'mon", "come", "let's",
        "here", "there", "he", "she", "they", "with", "at", "up", "be", "are", "was", "just", "now", "all", "can",
        "do", "out", "him", "her", "us", "so", "but", "if", "how", "where", "who", "why", "man", "hey",
    ];
    let key = memory::key_of(en);
    let words: Vec<&str> = key
        .split(' ')
        // letters only (an icon {1} or a misread "hjluj9" is not a word)
        .filter(|w| w.chars().any(|c| c.is_alphabetic()) && w.chars().all(|c| c.is_alphabetic() || c == '\''))
        .filter(|w| w.chars().any(|c| "aeiouy".contains(c)) || w.len() == 1)
        .collect();
    let sentence = en.trim_end().ends_with(['.', '!', '?']) && words.iter().any(|w| w.len() >= 2);
    sentence || words.iter().any(|w| COMMON.contains(w))
}

// ------------------------------------------------------------------------------------- downloads

fn download(
    http: &reqwest::blocking::Client,
    url: &str,
    app: &AppHandle,
    cfg: &Arc<Mutex<Config>>,
    memory: &Arc<Mutex<Memory>>,
    paths: &Paths,
    game: &str,
    lang: &str,
) {
    let mut st = load_state(paths);
    let _ = std::fs::create_dir_all(&paths.data);
    // translations
    let k = format!("data/{game}/{lang}");
    if let Some((bytes, tag)) = fetch(http, &format!("{url}/v1/{k}"), st.versions.get(&k)) {
        if let Some(entries) = open_pack(&bytes, game, lang, &tag) {
            let _ = std::fs::write(paths.shared(game, lang), &bytes);
            st.versions.insert(k, tag);
            memory.lock().unwrap().set_shared(entries);
        }
    }
    // layout
    let k = format!("layout/{game}");
    if let Some((bytes, tag)) = fetch(http, &format!("{url}/v1/{k}"), st.versions.get(&k)) {
        if let Ok(layout) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let _ = std::fs::write(paths.layout(game), &bytes);
            st.versions.insert(k, tag.clone());
            apply_layout(app, cfg, &layout, tag.parse().unwrap_or(0), lang);
        }
    }
    save_state(paths, &st);
}

/// GET with If-None-Match: Some((body, version)) when there is something new.
fn fetch(http: &reqwest::blocking::Client, url: &str, have: Option<&String>) -> Option<(Vec<u8>, String)> {
    let mut req = http.get(url);
    if let Some(v) = have {
        req = req.header("If-None-Match", format!("\"{v}\""));
    }
    let r = req.send().ok()?;
    if !r.status().is_success() {
        return None; // 304 (unchanged), 404 (nothing yet), errors
    }
    let tag = r.headers().get("ETag")?.to_str().ok()?.trim_start_matches("W/").trim_matches('"').to_string();
    Some((r.bytes().ok()?.to_vec(), tag))
}

fn load_cached(app: &AppHandle, cfg: &Arc<Mutex<Config>>, memory: &Arc<Mutex<Memory>>, paths: &Paths, game: &str, lang: &str) {
    let st = load_state(paths);
    if let (Ok(bytes), Some(tag)) = (std::fs::read(paths.shared(game, lang)), st.versions.get(&format!("data/{game}/{lang}"))) {
        if let Some(entries) = open_pack(&bytes, game, lang, tag) {
            memory.lock().unwrap().set_shared(entries);
        }
    }
    if let (Ok(bytes), Some(tag)) = (std::fs::read(paths.layout(game)), st.versions.get(&format!("layout/{game}"))) {
        if let Ok(layout) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            apply_layout(app, cfg, &layout, tag.parse().unwrap_or(0), lang);
        }
    }
}

fn key() -> [u8; 32] {
    // (debug builds can test against a local server's key)
    if cfg!(debug_assertions) {
        if let Some(k) = std::env::var("VI_TRANSLATOR_DATA_KEY").ok().and_then(|s| b64(&s)).filter(|k| k.len() == 32) {
            return k.try_into().unwrap();
        }
    }
    const A: [u8; 32] = [
        0xa6, 0x23, 0x37, 0x91, 0x51, 0x53, 0x7d, 0x05, 0xdc, 0xc4, 0x89, 0x9c, 0x51, 0x02, 0xd6, 0x5e, 0xa3, 0xd1, 0x81,
        0xfb, 0xfb, 0x5d, 0xa5, 0xf7, 0x3f, 0xa3, 0x04, 0x6e, 0x9d, 0x14, 0x15, 0x36,
    ];
    const B: [u8; 32] = [
        0x2a, 0x6c, 0x1a, 0x44, 0xa3, 0x4c, 0xc9, 0x81, 0x8c, 0xda, 0x59, 0x9d, 0x86, 0x63, 0x4c, 0xfa, 0x40, 0x3b, 0x53,
        0xd3, 0x98, 0x6c, 0x93, 0x62, 0x4a, 0xf1, 0xb8, 0x7c, 0xc8, 0x41, 0x7c, 0x18,
    ];
    std::array::from_fn(|i| A[i] ^ B[i])
}

fn b64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s.trim()).ok()
}

/// "G6T1" | nonce (12) | AES-256-GCM(gzip(JSON lines)), associated data "game/lang/version".
pub fn open_pack(file: &[u8], game: &str, lang: &str, version: &str) -> Option<Vec<memory::Entry>> {
    open_pack_with(&key(), file, game, lang, version)
}

fn open_pack_with(key: &[u8; 32], file: &[u8], game: &str, lang: &str, version: &str) -> Option<Vec<memory::Entry>> {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    if file.len() < 4 + 12 + 16 || &file[..4] != b"G6T1" {
        return None;
    }
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(key).ok()?;
    let aad = format!("{game}/{lang}/{version}");
    let plain = cipher
        .decrypt(aes_gcm::Nonce::from_slice(&file[4..16]), Payload { msg: &file[16..], aad: aad.as_bytes() })
        .ok()?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(&plain[..]).read_to_string(&mut text).ok()?;
    Some(
        text.lines()
            .filter_map(|l| serde_json::from_str::<memory::Entry>(l).ok())
            .map(|mut e| {
                e.en = memory::key_of(&e.en);
                e.source = "reviewed".into();
                e
            })
            .filter(|e| !e.en.is_empty() && !e.ar.trim().is_empty())
            .collect(),
    )
}

// ------------------------------------------------------------------------------------- layouts

/// Apply a game layout:
/// ```json
/// { "areas": [ { "id": "subtitles", "x": 0.1, "y": 0.8, "w": 0.8, "h": 0.15, "centered_only": true } ],
///   "fonts": { "title_font": "Lalezar", "text_font": "Tajawal" },
///   "words": { "ar": { "menu": { "Hide Menu": "…" }, "buttons": { "Browse": "…" }, "tabs": { "MAP": "…" } } } }
/// ```
/// Words always apply (they only add to or fix the built-in lists). Areas and fonts are applied
/// once per new layout version, and only with "Automatic layout updates" on: the player's own
/// changes stay until the next new layout.
fn apply_layout(app: &AppHandle, cfg: &Arc<Mutex<Config>>, layout: &serde_json::Value, version: u32, lang: &str) {
    // (the built-in word lists are Arabic; a layout can carry words for any language)
    let words = |kind: &str| -> Vec<(String, String)> {
        layout["words"][lang][kind]
            .as_object()
            .map(|m| m.iter().filter_map(|(en, ar)| Some((en.clone(), ar.as_str()?.to_string()))).collect())
            .unwrap_or_default()
    };
    crate::screens::set_layout_words(words("menu"), words("buttons"), words("tabs"));

    let mut c = cfg.lock().unwrap();
    if !c.auto_layout || version <= c.layout_version {
        return;
    }
    if let Some(areas) = layout["areas"].as_array() {
        for a in areas {
            let Some(area) = a["id"].as_str().and_then(|id| c.areas.iter_mut().find(|x| x.id == id)) else { continue };
            let num = |k: &str| a[k].as_f64().filter(|v| (0.0..=1.0).contains(v));
            if let (Some(x), Some(y), Some(w), Some(h)) = (num("x"), num("y"), num("w"), num("h")) {
                (area.x, area.y, area.w, area.h) = (x, y, w, h);
            }
            if let Some(v) = a["centered_only"].as_bool() {
                area.centered_only = v;
            }
        }
    }
    if let Some(f) = layout["fonts"]["title_font"].as_str() {
        c.profile_opts.title_font = f.to_string();
    }
    if let Some(f) = layout["fonts"]["text_font"].as_str() {
        c.profile_opts.text_font = f.to_string();
    }
    c.layout_version = version;
    let _ = c.save(&crate::config_path());
    let _ = app.emit("config", &*c);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_is_not_shared() {
        for junk in ["bjlwg", "hjluJ9", "ulnJiJl hjluJ9", "kiflwg", "", "...", "J"] {
            assert!(!looks_like_text(junk), "{junk:?}");
        }
        for text in ["Go to the guard.", "Hi.", "Run!", "Press ⟦1⟧ to bring up the phone.", "C'mon mister"] {
            assert!(looks_like_text(text), "{text:?}");
        }
    }

    /// A pack made like the server makes it (PackCrypto.seal) opens; a wrong version doesn't.
    #[test]
    fn opens_server_packs() {
        use aes_gcm::aead::{Aead, KeyInit, Payload};
        use std::io::Write;
        let key = [7u8; 32];
        let jsonl = "{\"en\":\"press {1} to bring up the phone\",\"ar\":\"اضغط {1} لإظهار الهاتف.\",\"source\":\"reviewed\"}\n";
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(jsonl.as_bytes()).unwrap();
        let zipped = gz.finish().unwrap();
        let nonce = [9u8; 12];
        let ct = aes_gcm::Aes256Gcm::new_from_slice(&key)
            .unwrap()
            .encrypt(aes_gcm::Nonce::from_slice(&nonce), Payload { msg: &zipped, aad: b"gta5/ar/3" })
            .unwrap();
        let file = [b"G6T1".as_slice(), &nonce, &ct].concat();
        let e = open_pack_with(&key, &file, "gta5", "ar", "3").expect("opens");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].en, "press {1} to bring up the phone");
        assert_eq!(e[0].source, "reviewed");
        assert!(open_pack_with(&key, &file, "gta5", "ar", "2").is_none(), "another version must not open");
        assert!(open_pack_with(&key, &file, "gta5", "hi", "3").is_none(), "another language must not open");
        assert!(open_pack_with(&[8u8; 32], &file, "gta5", "ar", "3").is_none(), "another key must not open");
    }
}
