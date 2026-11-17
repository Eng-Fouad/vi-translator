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

/// The project server (a different one can be set in the settings file: `server_url`).
pub const DEFAULT_SERVER: &str = "https://vitranslator.app";
const UPLOAD_EVERY: Duration = Duration::from_secs(10 * 60);
const DOWNLOAD_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_LINES_PER_UPLOAD: usize = 500;
const MAX_LINE_CHARS: usize = 400;

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct State {
    /// how far into the old, plain-text lines-log.jsonl has been shared (bytes); once it has all
    /// been handled the file is removed
    log_offset: u64,
    /// how far into the encrypted lines log (memory::LOG_FILE) has been shared (bytes)
    dat_offset: u64,
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
    Some(c.game())
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
    let mut last = (String::new(), String::new(), false); // game, language, test versions of pages
    loop {
        let c = cfg.lock().unwrap().clone();
        // what was downloaded before works offline; another game or language: its own downloads
        if let Some(g) = game(&c) {
            if last != (g.clone(), c.target_lang.clone(), c.early_pages) {
                last = (g.clone(), c.target_lang.clone(), c.early_pages);
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
/// A log from before encryption (lines-log.jsonl) is finished first, then removed.
fn share_lines(http: &reqwest::blocking::Client, url: &str, c: &Config, game: &str, paths: &Paths) {
    let mut st = load_state(paths);
    let legacy = paths.db.join("lines-log.jsonl");
    if legacy.exists() {
        let done = share_file(http, url, c, game, &legacy, &mut st.log_offset, false);
        if done && std::fs::remove_file(&legacy).is_ok() {
            st.log_offset = 0;
        }
        save_state(paths, &st);
        if !done {
            return; // offline: try again next time, in order
        }
    }
    share_file(http, url, c, game, &paths.db.join(memory::LOG_FILE), &mut st.dat_offset, true);
    save_state(paths, &st);
}

/// Share the whole lines of `path` after `offset` (moving it on). `sealed`: each line is an
/// encrypted record. True when everything up to the end has been handled.
fn share_file(http: &reqwest::blocking::Client, url: &str, c: &Config, game: &str, path: &std::path::Path, offset: &mut u64, sealed: bool) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else { return true };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if *offset > len {
        *offset = 0; // the log was cleared
    }
    if !c.share_lines {
        *offset = len;
        return true;
    }
    let mut text = String::new();
    if f.seek(SeekFrom::Start(*offset)).is_err() || f.read_to_string(&mut text).is_err() {
        return false;
    }
    // whole lines only (one may still be being written)
    let complete = text.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let rows: Vec<LogRow> = text[..complete]
        .lines()
        .filter_map(|l| if sealed { memory::open_log_line(l) } else { Some(l.to_string()) })
        .filter_map(|l| serde_json::from_str::<LogRow>(&l).ok())
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
            _ => return false,
        }
    }
    *offset += complete as u64;
    *offset >= len
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
    // the games the server adds (published like a layout, under the name "games")
    let k = "layout/games".to_string();
    if let Some((bytes, tag)) = fetch(http, &format!("{url}/v1/{k}"), st.versions.get(&k)) {
        if let Ok(list) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let _ = std::fs::write(paths.layout("games"), &bytes);
            st.versions.insert(k, tag.clone());
            apply_games(app, cfg, paths, &st, &list, tag.parse().unwrap_or(0), lang);
        }
    }
    // layout
    let k = format!("layout/{game}");
    if let Some((bytes, tag)) = fetch(http, &format!("{url}/v1/{k}"), st.versions.get(&k)) {
        if let Ok(layout) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let _ = std::fs::write(paths.layout(game), &bytes);
            st.versions.insert(k, tag.clone());
            let (id, early) = { let c = cfg.lock().unwrap(); (c.install_id.clone(), c.early_pages) };
            crate::pagescript::save_from_layout(&paths.data, game, &layout, &id, early);
            apply_layout(app, cfg, &layout, tag.parse().unwrap_or(0), lang, game);
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
    if let (Ok(bytes), Some(tag)) = (std::fs::read(paths.layout("games")), st.versions.get("layout/games")) {
        if let Ok(list) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            apply_games(app, cfg, paths, &st, &list, tag.parse().unwrap_or(0), lang);
        }
    }
    if let (Ok(bytes), Some(tag)) = (std::fs::read(paths.shared(game, lang)), st.versions.get(&format!("data/{game}/{lang}"))) {
        if let Some(entries) = open_pack(&bytes, game, lang, tag) {
            memory.lock().unwrap().set_shared(entries);
        }
    }
    // (another game: the last one's extra words, phrases and OCR fixes no longer apply)
    crate::screens::set_layout_words(Vec::new(), Vec::new(), Vec::new());
    crate::phrasebook::set_extra(game, lang, Vec::new());
    crate::ocr::set_fixes(Default::default());
    if let (Ok(bytes), Some(tag)) = (std::fs::read(paths.layout(game)), st.versions.get(&format!("layout/{game}"))) {
        if let Ok(layout) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            // (test versions turned on or off: the script that's for this install now)
            let (id, early) = { let c = cfg.lock().unwrap(); (c.install_id.clone(), c.early_pages) };
            crate::pagescript::save_from_layout(&paths.data, game, &layout, &id, early);
            apply_layout(app, cfg, &layout, tag.parse().unwrap_or(0), lang, game);
        }
    }
}

/// A new list of games from the server: the games it changed get their saved layout again.
fn apply_games(app: &AppHandle, cfg: &Arc<Mutex<Config>>, paths: &Paths, st: &State, list: &serde_json::Value, version: u32, lang: &str) {
    let changed = {
        let mut c = cfg.lock().unwrap();
        let changed = c.apply_games(list, version);
        if changed.is_empty() {
            return;
        }
        let _ = c.save(&crate::config_path());
        let _ = app.emit("config", &*c);
        changed
    };
    for g in changed {
        if let (Ok(bytes), Some(tag)) = (std::fs::read(paths.layout(&g)), st.versions.get(&format!("layout/{g}"))) {
            if let Ok(layout) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                apply_layout(app, cfg, &layout, tag.parse().unwrap_or(0), lang, &g);
            }
        }
    }
}

pub(crate) fn key() -> [u8; 32] {
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

/// `base` with the fields `patch` names replaced (a value that doesn't fit leaves it all as it was).
fn merged<T: serde::Serialize + serde::de::DeserializeOwned>(base: &T, patch: &serde_json::Value) -> Option<T> {
    let mut v = serde_json::to_value(base).ok()?;
    let (o, p) = (v.as_object_mut()?, patch.as_object()?);
    for (k, x) in p {
        if o.contains_key(k) {
            o.insert(k.clone(), x.clone());
        }
    }
    serde_json::from_value(v).ok()
}

/// Apply a game layout:
/// ```json
/// { "areas": [ { "id": "subtitles", "x": 0.1, "y": 0.8, "w": 0.8, "h": 0.15, "centered_only": true } ],
///   "fonts": { "title_font": "Lalezar", "text_font": "Tajawal" },
///   "words": { "ar": { "menu": { "Hide Menu": "…" }, "buttons": { "Browse": "…" }, "tabs": { "MAP": "…" } } } }
/// ```
/// Words always apply (they only add to or fix the built-in lists). Areas and fonts are applied
/// once per new layout version, and only with "Automatic layout updates" on: the player's own
/// changes stay until the next new layout.
fn apply_layout(app: &AppHandle, cfg: &Arc<Mutex<Config>>, layout: &serde_json::Value, version: u32, lang: &str, game: &str) {
    // (the built-in word lists are Arabic; a layout can carry words for any language)
    let words = |kind: &str| -> Vec<(String, String)> {
        layout["words"][lang][kind]
            .as_object()
            .map(|m| m.iter().filter_map(|(en, ar)| Some((en.clone(), ar.as_str()?.to_string()))).collect())
            .unwrap_or_default()
    };
    let mut c = cfg.lock().unwrap();
    if game == c.game() {
        crate::screens::set_layout_words(words("menu"), words("buttons"), words("tabs"));
        // hand-written translations of the game's lines, and OCR fixes for its fonts
        let phrases = layout["phrases"][lang].as_object().map(|m| m.iter().filter_map(|(en, t)| Some((en.clone(), t.as_str()?.to_string()))).collect()).unwrap_or_default();
        crate::phrasebook::set_extra(game, lang, phrases);
        crate::ocr::set_fixes(crate::ocr::fixes_from(&layout["ocr"]));
    }
    if !c.auto_layout || version <= c.layout_versions.get(game).copied().unwrap_or(0) {
        return;
    }
    // The update tunes the game's built-in profile (the player's own copies are theirs: untouched).
    let Some(p) = c.builtins.iter_mut().find(|p| p.id == game) else { return };
    if let Some(areas) = layout["areas"].as_array() {
        for a in areas {
            let Some(id) = a["id"].as_str().filter(|id| !id.is_empty()) else { continue };
            // a place outside the picture is a mistake: that part is left as it was
            let mut a = a.clone();
            if ["x", "y", "w", "h"].iter().any(|k| a[*k].as_f64().is_some_and(|v| !(0.0..=1.0).contains(&v))) {
                for k in ["x", "y", "w", "h"] {
                    a.as_object_mut().map(|o| o.remove(k));
                }
            }
            match p.areas.iter_mut().find(|x| x.id == id) {
                // any of the area's settings (place, alignment, font, background, size …)
                Some(area) => {
                    if let Some(new) = merged(area, &a) {
                        *area = new;
                    }
                }
                // a new area (with its place)
                None if ["x", "y", "w", "h"].iter().all(|k| a[*k].is_number()) => {
                    if let Some(new) = merged(&crate::config::Area::default(), &a) {
                        p.areas.push(new);
                    }
                }
                None => {}
            }
        }
    }
    // the whole text style (colours, outline, sizes …), only the parts it names
    if layout["style"].is_object() {
        if let Some(new) = merged(&p.style, &layout["style"]) {
            p.style = new;
        }
    }
    // reading timing for the game
    if layout["timing"].is_object() {
        if let Some(new) = merged(&p.timing, &layout["timing"]) {
            p.timing = new;
        }
    }
    if let Some(f) = layout["fonts"]["title_font"].as_str() {
        p.pages.title_font = f.to_string();
    }
    if let Some(f) = layout["fonts"]["text_font"].as_str() {
        p.pages.text_font = f.to_string();
    }
    c.layout_versions.insert(game.to_string(), version);
    c.normalize(); // (if that built-in profile is in use, its new layout applies now)
    let _ = c.save(&crate::config_path());
    let _ = app.emit("config", &*c);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_change_only_what_they_name() {
        let style = crate::config::Style::default();
        let new: crate::config::Style = merged(&style, &serde_json::json!({ "no_such_field": 1 })).unwrap();
        assert_eq!(serde_json::to_value(&new).unwrap(), serde_json::to_value(&style).unwrap());
        let area = crate::config::Area { id: "subtitles".into(), x: 0.1, ..Default::default() };
        let moved: crate::config::Area = merged(&area, &serde_json::json!({ "x": 0.2, "font_scale": 1.1 })).unwrap();
        assert_eq!((moved.id.as_str(), moved.x, moved.font_scale), ("subtitles", 0.2, 1.1));
        assert!(merged(&area, &serde_json::json!({ "x": "far" })).is_none(), "a value that doesn't fit changes nothing");
    }

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
