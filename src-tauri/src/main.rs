#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod config;
mod debug_frames;
// capture cards: Media Foundation on Windows, nokhwa (V4L2 / AVFoundation) elsewhere
#[cfg_attr(not(windows), path = "device_nokhwa.rs")]
mod device;
mod obs;
mod obs_page;
mod memory;
mod secure;
// installed fonts: DirectWrite on Windows, fontdb elsewhere
#[cfg_attr(not(windows), path = "fonts_fontdb.rs")]
mod fonts;
mod ocr;
mod overlay;
mod phrasebook;
mod lan;
mod langs;
mod screens;
mod pagescript;
mod sync;
mod translate;
mod updater;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::capture::Shared;
use crate::config::Config;
use crate::ocr::{Crop, Ocr};
use crate::translate::{Segments, Translator};

pub const APP_TITLE: &str = "VI Translator";
/// Translation-job id for the reason line of a profile screen.
pub const SCREEN_REASON: &str = "__screen_reason";
/// Translation jobs for pause-menu text (results go to Shared::menu_cache).
pub const MENU_TEXT: &str = "__menu";
const IDENTIFIER: &str = "com.vitranslator.app";
/// The folder name in Documents and Pictures.
const FOLDER: &str = "VI Translator";
/// Where earlier versions kept their files (newest first): moved over on first start.
const OLD_NAMES: [(&str, &str); 2] = [("com.gta6translator.app", "GTA 6 Translator"), ("com.arabicsubs.app", "Arabic Subs")];

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else {
            std::fs::copy(e.path(), dest)?;
        }
    }
    Ok(())
}

/// Move the settings, downloaded model, translation memory and recordings of the app's earlier
/// names ("GTA 6 Translator", "Arabic Subs") to the new folders, once. The newest old folder that
/// exists wins; nothing is moved if the new folder exists.
fn move_old_folders() {
    let appdata = app_data_root();
    let mut pairs = Vec::new();
    for (id, folder) in OLD_NAMES {
        pairs.push((appdata.join(id), data_dir()));
        for dir in [documents_dir(), pictures_dir()].into_iter().flatten() {
            pairs.push((dir.join(folder), dir.join(FOLDER)));
        }
    }
    for (old, new) in pairs {
        if old.is_dir() && !new.exists() {
            // (a folder open in Explorer can't be moved: copy it instead, and leave the old one)
            if std::fs::rename(&old, &new).is_err() {
                if let Err(e) = copy_dir(&old, &new) {
                    eprintln!("could not copy {} to {}: {e}", old.display(), new.display());
                }
            }
        }
    }
}

/// Where apps keep their settings: %APPDATA% on Windows, ~/Library/Application Support on macOS,
/// ~/.local/share on Linux.
fn app_data_root() -> PathBuf {
    #[cfg(windows)]
    return PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into()));
    #[cfg(not(windows))]
    return dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
}
fn data_dir() -> PathBuf {
    app_data_root().join(IDENTIFIER)
}
fn config_path() -> PathBuf {
    data_dir().join("config.json")
}
/// Debugging: where "Save frames" recordings go (one sub-folder per recording).
fn frames_dir() -> PathBuf {
    // Pictures\VI Translator\debug-frames: easy to find, and (unlike AppData) never redirected by Windows
    // into a private copy when the app is started from a packaged app.
    pictures_dir().map(|p| p.join(FOLDER).join("debug-frames")).unwrap_or_else(|| data_dir().join("debug-frames"))
}

/// The user's Pictures folder (follows OneDrive / moved folders).
fn pictures_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return known_folder(&windows::Win32::UI::Shell::FOLDERID_Pictures);
    #[cfg(not(windows))]
    return dirs::picture_dir();
}

/// The user's Documents folder.
fn documents_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return known_folder(&windows::Win32::UI::Shell::FOLDERID_Documents);
    #[cfg(not(windows))]
    return dirs::document_dir();
}

/// Translation memory and lines log: Documents\VI Translator\translation-db (not AppData, which
/// Windows may redirect into a private copy).
fn db_dir() -> PathBuf {
    documents_dir()
        .map(|p| p.join(FOLDER).join("translation-db"))
        .unwrap_or_else(|| data_dir().join("translation-db"))
}

#[cfg(windows)]
fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = p.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(p.0 as *const _));
        path
    }
}
fn model_dir() -> PathBuf {
    data_dir().join("models").join("opus-mt-en-ar")
}

struct AppState {
    cfg: Arc<Mutex<Config>>,
    shared: Arc<Shared>,
    saver: Arc<debug_frames::SaverState>,
    memory: Arc<Mutex<memory::Memory>>,
    /// a newer version found by the updater
    update: Arc<Mutex<Option<updater::Update>>>,
}


/// Translation memory: on/off, size, hits/misses and the time saved.
#[tauri::command]
fn memory_status(state: tauri::State<AppState>) -> memory::Stats {
    let on = state.cfg.lock().unwrap().memory_enabled;
    state.memory.lock().unwrap().stats(on)
}

#[tauri::command]
fn memory_clear(state: tauri::State<AppState>) {
    state.memory.lock().unwrap().clear();
}

/// Forget the locked text sizes: measure them again on the next lines.
#[tauri::command]
fn reset_text_sizes(state: tauri::State<AppState>) {
    state.shared.reset_sizes.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Re-read the memory files.
#[tauri::command]
fn memory_reload(state: tauri::State<AppState>) {
    state.memory.lock().unwrap().reload();
}

#[tauri::command]
fn memory_open_folder() -> Result<(), String> {
    let dir = db_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    debug_frames::open_folder(&dir).map_err(|e| e.to_string())
}

#[derive(Serialize, Clone)]
struct TranslationEvent {
    area: String,
    en: String,
    segments: Segments,
    stats: String,
    error: Option<String>,
    /// Height of the English text line, as a fraction of the frame height (0 = unknown). The
    /// overlay sizes the Arabic from it so both look the same size.
    line_h: f64,
    /// The English text block (words + icons) as [x, y, w, h] fractions of the frame, so the Arabic
    /// can line up with the game's own box ("below the game's text box" placement).
    text_box: Option<[f64; 4]>,
    /// came from the translation memory (shown tinted while testing)
    cached: bool,
    /// when the English line appeared on screen (Unix ms; 0 = unknown)
    at: u64,
}

// ------------------------------------------------------------------------------------- size lock
//
// The game draws each kind of text (subtitles, instructions, the Mission Failed reason) in one fixed
// font size, but the measured height of a line changes with its letters ("we are" vs "Back off!").
// So the size is measured on the first few lines, then locked to their median — every Arabic line of
// that kind has the same size. Locked sizes are kept in text-sizes.json for the next sessions.

const SIZE_SAMPLES: usize = 5;

#[derive(Serialize, Deserialize, Default)]
struct SizeLock {
    locked: HashMap<String, f64>,
    #[serde(skip)]
    samples: HashMap<String, Vec<f64>>,
}

fn sizes_path() -> PathBuf {
    data_dir().join("text-sizes.json")
}

impl SizeLock {
    fn load() -> Self {
        std::fs::read_to_string(sizes_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    /// The size to use for this kind of text, given this line's measured height (fractions of the frame).
    fn size(&mut self, kind: &str, measured: f64) -> f64 {
        if let Some(&v) = self.locked.get(kind) {
            return v;
        }
        if measured <= 0.0 {
            return 0.0;
        }
        let v = self.samples.entry(kind.into()).or_default();
        v.push(measured);
        let mut sorted = v.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let median = sorted[sorted.len() / 2];
        if sorted.len() >= SIZE_SAMPLES {
            self.locked.insert(kind.into(), median);
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(sizes_path(), serde_json::to_string_pretty(self).unwrap_or_default());
        }
        median
    }

    fn reset(&mut self) {
        *self = Self::default();
        let _ = std::fs::remove_file(sizes_path());
    }
}

#[derive(Serialize, Clone)]
struct FrameEvent {
    /// a picture is coming in (the preview itself comes from /preview)
    live: bool,
    /// full frame size (what areas are fractions of)
    w: u32,
    h: u32,
    status: String,
    /// where the live preview streams from
    preview_url: String,
}

// ------------------------------------------------------------------------------------- commands

/// The languages the app can translate into.
#[tauri::command]
fn list_langs() -> Vec<langs::Lang> {
    langs::LANGS.to_vec()
}

/// This build's version (from the release tag).
#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// A newer version of the app, if one was found.
#[tauri::command]
fn update_status(state: tauri::State<AppState>) -> Option<updater::Update> {
    state.update.lock().unwrap().clone()
}

/// Download and check the newer version, put it in place and restart into it.
#[tauri::command]
async fn install_update(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let u = state.update.lock().unwrap().clone().ok_or("no update available")?;
    tauri::async_runtime::spawn_blocking(move || updater::install(&u)).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))?;
    app.restart()
}

/// The home-network page: on/off, its address and a QR code of it.
#[tauri::command]
fn lan_info(state: tauri::State<AppState>) -> lan::LanInfo {
    let c = state.cfg.lock().unwrap().clone();
    lan::info(&c, &state.shared)
}

/// Opens the home-network page in the default browser (only that address: the page passes none).
#[tauri::command]
fn open_lan_page(state: tauri::State<AppState>) -> Result<(), String> {
    let c = state.cfg.lock().unwrap().clone();
    let url = lan::info(&c, &state.shared).url.ok_or("The page is not running")?;
    // Windows: the shell's own "open" (the default browser), as Explorer does for a link
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::core::{HSTRING, w};
        let r = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url.as_str()), None, None, SW_SHOWNORMAL) };
        // (a value above 32 means it worked)
        if r.0 as isize > 32 { Ok(()) } else { Err(format!("could not open {url}")) }
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> Config {
    state.cfg.lock().unwrap().clone()
}

/// The game pages a game's script has, and the switches it declares (for Game pages).
#[tauri::command]
fn page_options(game: String) -> serde_json::Value {
    match pagescript::Pages::for_game(&game, &data_dir()) {
        Some(p) => serde_json::json!({ "pages": p.list.iter().map(|i| serde_json::json!({ "id": i.id, "option": i.option })).collect::<Vec<_>>(), "options": p.options }),
        None => serde_json::json!({ "pages": [], "options": [] }),
    }
}

#[tauri::command]
fn save_config(app: AppHandle, state: tauri::State<AppState>, config: Config) -> Result<(), String> {
    // (the profile in use decides the areas, style and pages; a built-in one can't be changed)
    let mut config = config;
    {
        // the built-in profiles are the app's (layout updates may have changed them meanwhile)
        let now = state.cfg.lock().unwrap();
        config.builtins = now.builtins.clone();
        config.server_games = now.server_games.clone();
        config.games_version = now.games_version;
        config.layout_version = now.layout_version;
        config.layout_versions = now.layout_versions.clone();
    }
    config.normalize();
    *state.cfg.lock().unwrap() = config.clone();
    config.save(&config_path()).map_err(|e| e.to_string())?;
    app.emit("config", &config).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_windows() -> Vec<capture::WindowInfo> {
    capture::list_windows(APP_TITLE)
}

/// Everything that can be a source: windows, capture devices, and OBS's sources (if OBS answers).
#[tauri::command]
async fn list_sources(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let (port, password) = {
        let c = state.cfg.lock().unwrap();
        (c.obs_port, c.obs_password.clone())
    };
    let (windows, devices, obs) = tauri::async_runtime::spawn_blocking(move || {
        let obs = match obs::list(port, &password) {
            Ok((list, canvas)) => serde_json::json!({
                "ok": true,
                "sources": list.into_iter().map(|(n, k)| serde_json::json!({ "name": n, "kind": k })).collect::<Vec<_>>(),
                "canvas": canvas.map(|(w, h)| serde_json::json!({ "w": w, "h": h })),
            }),
            Err(e) => serde_json::json!({ "ok": false, "error": format!("{e:#}") }),
        };
        (capture::list_windows(APP_TITLE), device::list(), obs)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "windows": windows, "devices": devices, "obs": obs }))
}

/// The profile screen on show right now (layout + Arabic reason), for windows opened after it appeared.
#[tauri::command]
fn get_screen(state: tauri::State<AppState>) -> serde_json::Value {
    serde_json::json!({
        "layout": state.shared.screen.lock().unwrap().clone(),
        "reason": state.shared.screen_reason.lock().unwrap().clone(),
    })
}

/// Debugging: state of the frame saver (on/off, frames saved, folder).
#[tauri::command]
fn frame_saver_status(state: tauri::State<AppState>) -> debug_frames::SaverEvent {
    let on = state.cfg.lock().unwrap().debug_save_frames;
    state.saver.event(on)
}

/// Debugging: open the last recording's folder (or the parent folder) in File Explorer.
#[tauri::command]
fn open_frames_folder(state: tauri::State<AppState>) -> Result<String, String> {
    let dir = state.saver.folder.lock().unwrap().clone().unwrap_or_else(frames_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    debug_frames::open_folder(&dir).map_err(|e| e.to_string())?;
    Ok(dir.display().to_string())
}

#[tauri::command]
fn list_fonts() -> Vec<fonts::FontInfo> {
    fonts::list_fonts()
}

// ------------------------------------------------------------------------------------- workers

/// The live preview: turns small preview frames into JPEG (only while the preview is watched; the
/// app's picture asks for them one by one at http://127.0.0.1:<port>/preview), and tells the main window
/// twice a second whether a picture is coming in, its size and the capture status.
fn preview_loop(app: AppHandle, cfg: Arc<Mutex<Config>>, shared: Arc<Shared>) {
    let mut last_raw: Option<Arc<capture::FrameData>> = None;
    let mut last_status = Instant::now() - Duration::from_secs(5);
    loop {
        std::thread::sleep(Duration::from_millis(5));
        let raw = shared.preview_raw.lock().unwrap().clone();
        // someone started watching, but the picture hasn't changed since: start from the last full frame
        let watching = shared.watching();
        let raw = match raw {
            None if watching && shared.preview_jpeg.lock().unwrap().is_none() && shared.preview_yuv.lock().unwrap().is_none() => shared.latest().map(|f| {
                let small = Arc::new(capture::shrink(&f.rgba, f.w, f.h, (f.w * 4) as usize, false));
                *shared.preview_raw.lock().unwrap() = Some(small.clone());
                small
            }),
            r => r,
        };
        if let Some(f) = raw.filter(|f| last_raw.as_ref().is_none_or(|l| !Arc::ptr_eq(l, f))) {
            last_raw = Some(f.clone());
            let t = Instant::now();
            // a fast (SIMD) encoder straight from RGBA: a full 1080p picture in a few ms, so the
            // preview can be full size without falling behind
            let mut jpeg = Vec::with_capacity(f.rgba.len() / 10);
            let enc = jpeg_encoder::Encoder::new(&mut jpeg, 85);
            if enc.encode(&f.rgba, f.w as u16, f.h as u16, jpeg_encoder::ColorType::Rgba).is_ok() {
                shared.put_preview_jpeg(jpeg);
            }
            if std::env::var_os("VI_TRANSLATOR_DEBUG").is_some() {
                eprintln!("[preview] {}x{} JPEG in {:.1} ms", f.w, f.h, t.elapsed().as_secs_f64() * 1000.0);
            }
        }
        if last_status.elapsed() >= Duration::from_millis(500) {
            last_status = Instant::now();
            let visible = app
                .get_webview_window("main")
                .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
            shared.preview_paused.store(!visible, std::sync::atomic::Ordering::Relaxed);
            let frame = shared.latest();
            let status = shared.status.lock().unwrap().clone();
            let port = cfg.lock().unwrap().obs_page_port;
            let (w, h) = frame.as_ref().map(|f| (f.w, f.h)).unwrap_or((0, 0));
            let _ = app.emit_to(
                "main",
                "frame",
                FrameEvent { live: frame.is_some(), w, h, status, preview_url: format!("http://127.0.0.1:{port}/preview") },
            );
            if frame.is_none() {
                *shared.preview_raw.lock().unwrap() = None;
                *shared.preview_jpeg.lock().unwrap() = None;
                *shared.preview_yuv.lock().unwrap() = None;
            }
        }
    }
}

struct Job {
    area: String,
    seq: u64,
    /// English text; may contain button-icon placeholders "⟦n⟧"
    en: String,
    phrases: Vec<(String, String)>,
    /// Button icons cut from the frame (PNG data URLs), in placeholder order
    icons: Vec<String>,
    /// English line height as a fraction of the frame height (0 = unknown)
    line_h: f64,
    /// When this English line first appeared on screen
    en_start: Instant,
    /// The English text block as [x, y, w, h] fractions of the frame
    text_box: Option<[f64; 4]>,
}

#[derive(Default)]
struct AreaState {
    candidate: String,
    candidate_since: Option<Instant>,
    current: String,
    last_seen: Option<Instant>,
    /// translation-job seq of the English line now on screen (0 = none)
    current_seq: u64,
    /// that line as sent to translation (icons as ⟦n⟧) and when it appeared — for the lines log
    current_en: String,
    current_start: Option<Instant>,
}

/// Instant -> Unix milliseconds.
fn unix_ms(t: Instant) -> u64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    now.saturating_sub(t.elapsed()).as_millis() as u64
}

// ------------------------------------------------------------------------------------- presenter
//
// Decides WHEN each area's Arabic is shown and hidden:
//  - an Arabic line stays up as long as its English line was on screen (so it ends as much after the
//    English as it started after it — the translation delay);
//  - when the next line's translation is ready and the current one has had its full time, it
//    switches straight away; if not, the next line waits only until it has;
//  - but a line never outstays its English by more than MAX_LAG: a translation that came very late
//    (e.g. menu text read minutes before it was translated) mustn't hold up the lines after it.

/// The most an Arabic line may stay up after its English left the screen.
const MAX_LAG: Duration = Duration::from_secs(3);

enum PMsg {
    /// A translation is ready to show.
    Ready { seq: u64, en_start: Instant, ev: TranslationEvent },
    /// The English line with this seq left the screen at `at` (gone, or replaced by the next line).
    Ended { area: String, seq: u64, at: Instant },
    /// A full-screen profile page took over: hide every area now.
    ClearAll,
}

struct Shown {
    seq: u64,
    shown_at: Instant,
    en_start: Instant,
}

/// What the presenter wants on screen.
#[derive(Debug, PartialEq)]
enum Act {
    Show { area: String, seq: u64 },
    Hide { area: String },
}

/// The timing rules, kept free of the app so they can be tested.
#[derive(Default)]
struct Presenter {
    showing: HashMap<String, Shown>,
    pending: HashMap<String, (u64, Instant)>,
    ended: HashMap<(String, u64), Instant>,
}

impl Presenter {
    fn ready(&mut self, area: &str, seq: u64, en_start: Instant) {
        self.pending.insert(area.into(), (seq, en_start)); // only the newest ready line waits
    }
    fn ended(&mut self, area: &str, seq: u64, at: Instant) {
        self.ended.insert((area.into(), seq), at);
    }
    fn clear_all(&mut self) -> Vec<Act> {
        self.pending.clear();
        self.showing.drain().map(|(area, _)| Act::Hide { area }).collect()
    }
    fn tick(&mut self, now: Instant) -> Vec<Act> {
        let mut acts = Vec::new();
        let areas: Vec<String> = self.showing.keys().chain(self.pending.keys()).cloned().collect();
        for area in areas {
            // Has the line on screen been up as long as its English was? Its time ends as long after the
            // English ended as the Arabic came after the English started (at most MAX_LAG).
            let done = match self.showing.get(&area) {
                None => true,
                Some(s) => self.ended.get(&(area.clone(), s.seq)).is_some_and(|&end| {
                    let lag = s.shown_at.saturating_duration_since(s.en_start).min(MAX_LAG);
                    now >= end.max(s.shown_at) + lag.min(end.saturating_duration_since(s.en_start))
                }),
            };
            if !done {
                continue;
            }
            if let Some((seq, en_start)) = self.pending.remove(&area) {
                self.showing.insert(area.clone(), Shown { seq, shown_at: now, en_start });
                acts.push(Act::Show { area, seq });
            } else if self.showing.remove(&area).is_some() {
                acts.push(Act::Hide { area });
            }
        }
        self.ended.retain(|_, at| now.saturating_duration_since(*at) < Duration::from_secs(120));
        acts
    }
}

fn present_loop(app: AppHandle, rx: mpsc::Receiver<PMsg>, shared: Arc<Shared>) {
    let mut p = Presenter::default();
    let mut events: HashMap<(String, u64), TranslationEvent> = HashMap::new();
    loop {
        let mut msgs = Vec::new();
        match rx.recv_timeout(Duration::from_millis(40)) {
            Ok(m) => msgs.push(m),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        msgs.extend(rx.try_iter());
        let mut acts = Vec::new();
        for m in msgs {
            match m {
                PMsg::Ready { seq, en_start, ev } => {
                    p.ready(&ev.area, seq, en_start);
                    events.retain(|(a, _), _| *a != ev.area); // keep only the newest waiting line
                    events.insert((ev.area.clone(), seq), ev);
                }
                PMsg::Ended { area, seq, at } => p.ended(&area, seq, at),
                PMsg::ClearAll => {
                    events.clear();
                    acts.extend(p.clear_all());
                }
            }
        }
        acts.extend(p.tick(Instant::now()));
        for act in acts {
            match act {
                Act::Show { area, seq } => {
                    if let Some(ev) = events.remove(&(area.clone(), seq)) {
                        let ar: String = ev
                            .segments
                            .iter()
                            .map(|(t, c)| if c.as_deref().is_some_and(|c| c.starts_with("icon:")) { "🎮" } else { t.as_str() })
                            .collect();
                        shared.on_screen.lock().unwrap().insert(area.clone(), serde_json::json!({ "en": ev.en, "ar": ar.trim() }));
                        shared.shown.lock().unwrap().insert(area.clone(), serde_json::to_value(&ev).unwrap_or_default());
                        lan::push(&shared, &area, &ev.en, &ev.segments, ev.at);
                        let _ = app.emit("translation", ev);
                    }
                }
                Act::Hide { area } => {
                    shared.on_screen.lock().unwrap().remove(&area);
                    shared.shown.lock().unwrap().remove(&area);
                    let _ = app.emit(
                        "translation",
                        TranslationEvent { area, en: String::new(), segments: Vec::new(), stats: String::new(), error: None, line_h: 0.0, text_box: None, cached: false, at: 0 },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod presenter_tests {
    use super::*;
    const A: &str = "subtitles";
    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn arabic_stays_as_long_as_the_english_did() {
        let t = Instant::now();
        let mut p = Presenter::default();
        p.ready(A, 1, t); // English appeared at 0
        assert_eq!(p.tick(t + ms(600)), vec![Act::Show { area: A.into(), seq: 1 }]); // translated after 0.6 s
        p.ended(A, 1, t + ms(3000)); // English was up 3 s
        assert_eq!(p.tick(t + ms(3100)), vec![]); // Arabic only up 2.5 s: stays
        assert_eq!(p.tick(t + ms(3590)), vec![]);
        assert_eq!(p.tick(t + ms(3600)), vec![Act::Hide { area: A.into() }]); // 3 s after it appeared
    }

    #[test]
    fn next_line_waits_until_the_current_one_had_its_time() {
        let t = Instant::now();
        let mut p = Presenter::default();
        p.ready(A, 1, t);
        p.tick(t + ms(800)); // line 1 shown at 0.8 s
        p.ended(A, 1, t + ms(2000)); // replaced by line 2 at 2 s (line 1 was up 2 s)
        p.ready(A, 2, t + ms(2000));
        assert_eq!(p.tick(t + ms(2500)), vec![]); // line 2 ready, but line 1 only shown 1.7 s
        assert_eq!(p.tick(t + ms(2800)), vec![Act::Show { area: A.into(), seq: 2 }]); // switches, no gap
    }

    #[test]
    fn next_line_switches_immediately_when_the_current_one_had_its_time() {
        let t = Instant::now();
        let mut p = Presenter::default();
        p.ready(A, 1, t);
        p.tick(t + ms(500)); // shown at 0.5 s
        p.ended(A, 1, t + ms(1000)); // English up 1 s
        // translation of the next line arrives late (at 3 s): line 1 has had its 1 s long ago
        p.ready(A, 2, t + ms(1000));
        assert_eq!(p.tick(t + ms(3000)), vec![Act::Show { area: A.into(), seq: 2 }]);
    }

    #[test]
    fn a_very_late_translation_does_not_block_the_next_lines() {
        let t = Instant::now();
        let mut p = Presenter::default();
        p.ready(A, 1, t); // menu text seen at 0 ...
        p.tick(t + ms(320_000)); // ... but only translated and shown 320 s later
        p.ended(A, 1, t + ms(321_000)); // its English goes 1 s after that
        p.ready(A, 2, t + ms(321_000));
        assert_eq!(p.tick(t + ms(323_900)), vec![]); // line 1 stays at most MAX_LAG after its English left
        assert_eq!(p.tick(t + ms(324_000)), vec![Act::Show { area: A.into(), seq: 2 }]); // not 320 s later
    }

    #[test]
    fn page_takeover_hides_at_once() {
        let t = Instant::now();
        let mut p = Presenter::default();
        p.ready(A, 1, t);
        p.tick(t + ms(500));
        p.ready(A, 2, t + ms(900));
        assert_eq!(p.clear_all(), vec![Act::Hide { area: A.into() }]);
        assert_eq!(p.tick(t + ms(5000)), vec![]); // nothing left waiting
    }
}

/// Reads every enabled area of the latest frame, waits for text to be stable, then queues it for translation.
fn ocr_loop(
    app: AppHandle,
    cfg: Arc<Mutex<Config>>,
    shared: Arc<Shared>,
    latest: Arc<Mutex<HashMap<String, u64>>>,
    jobs: mpsc::Sender<Job>,
    present: mpsc::Sender<PMsg>,
    memory: Arc<Mutex<memory::Memory>>,
) {
    let sizes = Mutex::new(SizeLock::load());
    Ocr::init_thread();
    let ocr = match Ocr::new() {
        Ok(o) => o,
        Err(e) => {
            let _ = app.emit("status-msg", format!("OCR unavailable: {e}"));
            return;
        }
    };
    let mut states: HashMap<String, AreaState> = HashMap::new();
    let mut seq_counter = 0u64;
    let mut screen: Option<screens::ScreenLayout> = None;
    let mut screen_checked = Instant::now() - Duration::from_secs(10);
    // when the red title was last seen missing (None = it is on screen)
    let mut title_missing_since: Option<Instant> = None;
    // pause-menu text already sent to the translator (once per session)
    let mut menu_asked: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Every state of a pause-menu tab seen so far (tab, picture of the menu area, its lines with
    // Arabic). Some pages alternate by themselves — the Settings controller diagram switches
    // between its "press" and "hold" labels every ~3 s — and a state seen before comes back at
    // once, complete, without reading it again.
    let mut menu_states: Vec<(String, Vec<u8>, Vec<screens::MenuText>)> = Vec::new();
    // target language of what is on screen and in the caches
    let mut last_lang = String::new();
    // a profile page still has pieces waiting for the translator (other languages than Arabic)
    let mut screen_missing = false;
    // the Mission Failed reason now on screen and since when — for the lines log (translation DB)
    let mut reason_on: Option<(String, Instant)> = None;
    // the game's pages script (pagescript.rs), loaded again when the game or the script changes
    let mut pages: Option<pagescript::Pages> = None;
    let mut pages_for: (String, u64) = (String::new(), u64::MAX);
    loop {
        if shared.reset_sizes.swap(false, std::sync::atomic::Ordering::Relaxed) {
            sizes.lock().unwrap().reset();
        }
        let c = cfg.lock().unwrap().clone();
        let t0 = Instant::now();
        if c.target_lang != last_lang {
            // another language: everything shown and cached is in the old one
            last_lang = c.target_lang.clone();
            let l = langs::get(&c.target_lang);
            *shared.live_lang.lock().unwrap() = serde_json::json!({ "code": l.code, "rtl": l.rtl });
            memory.lock().unwrap().set_lang(&c.target_lang);
            menu_asked.clear();
            menu_states.clear();
            shared.menu_cache.lock().unwrap().clear();
            if screen.is_some() {
                hide_screen(&app, &shared, &mut screen);
            }
            for st in states.values_mut() {
                st.current.clear();
                st.candidate.clear();
            }
            let _ = present.send(PMsg::ClearAll);
        }
        let arabic = c.target_lang == "ar";
        if let Some(frame) = shared.latest() {
            // ---- profile screens (GTA V "Mission Failed") ----
            // Appear: full detection (colour + OCR) every frame until the page is found.
            // Stay: a quick colour check every frame — the Arabic stays exactly as long as the red title
            //       does, and disappears the moment it goes (no delay, no flicker from OCR misses).
            // Refresh: once a second, re-read the reason and buttons (they can change: Yes / No ...);
            //       a re-read that misses them keeps the last good ones.
            let now_for = (c.game(), pagescript::GENERATION.load(std::sync::atomic::Ordering::Relaxed));
            if now_for != pages_for {
                pages = pagescript::Pages::for_game(&now_for.0, &data_dir());
                if let (Some(p), true) = (&pages, std::env::var_os("VI_TRANSLATOR_DEBUG").is_some()) {
                    eprintln!("[pages] {} script ({}): {:?}", now_for.0, p.origin, p.list.iter().map(|i| &i.id).collect::<Vec<_>>());
                }
                pages_for = now_for;
            }
            // the script's pages whose switch is on in the profile
            let script_pages: Vec<String> = pages
                .as_ref()
                .map(|p| p.list.iter().filter(|i| p.switch_on(&c.profile_opts, &i.option)).map(|i| i.id.clone()).collect())
                .unwrap_or_default();
            let gta5 = c.game() == "gta5";
            let screens_on = (gta5 && (c.profile_opts.mission_failed || c.profile_opts.pause_menu || c.profile_opts.captions)) || !script_pages.is_empty();
            let mut found: Option<screens::ScreenLayout> = None;
            // A page on screen whose switch was just turned off (Game pages) goes now, not when the game
            // closes it.
            if let Some(old) = &screen {
                let option = pages.as_ref().and_then(|p| p.info(&old.id)).map(|i| i.option.clone()).unwrap_or_else(|| pagescript::option_of(&old.id).to_string());
                let on = match &pages {
                    Some(p) => p.switch_on(&c.profile_opts, &option),
                    None => pagescript::option_on(&c.profile_opts, &option),
                };
                if !on {
                    hide_screen(&app, &shared, &mut screen);
                }
            }
            if screens_on {
                match &screen {
                    None => {
                        // the app's own detection, for the pages the script doesn't have
                        let by_script = |id: &str| pages.as_ref().is_some_and(|p| p.has(id));
                        found = if gta5 && c.profile_opts.mission_failed && !by_script("gta5_mission_failed") { screens::detect_gta5_mission_failed(&frame, &ocr) } else { None };
                        if found.is_none() && gta5 && c.profile_opts.pause_menu && !by_script(screens::PAUSE_ID) {
                            found = screens::detect_gta5_pause(&frame, &ocr);
                        }
                        if found.is_none() && gta5 && c.profile_opts.mission_failed && !by_script(screens::ALERT_ID) && screens::alert_present(&frame) {
                            found = screens::detect_gta5_alert(&frame, &ocr);
                        }
                        // the script's pages (captions, and whatever a newer script brings)
                        if let (None, Some(p)) = (&found, &pages) {
                            for id in &script_pages {
                                if let Some(k) = p.present(id, &frame, &ocr) {
                                    found = p.read(id, &frame, &ocr, &k);
                                    if found.is_some() {
                                        break;
                                    }
                                }
                            }
                        }
                        // no script for the captions: the app's own detection
                        let script_captions = pages.as_ref().is_some_and(|p| p.has(screens::CAPTION_ID));
                        if found.is_none() && gta5 && c.profile_opts.captions && !script_captions && screens::find_caption(&frame).is_some() {
                            found = screens::detect_gta5_caption(&frame, &ocr);
                        }
                    }
                    // A script's page is up. Without hold rules (captions): still the same? Else read it
                    // again, or it's gone. With them (Mission Failed): kept while its title blinks out
                    // (up to hold_ms), ended once the screen is dark, read again every reread_ms.
                    Some(old) if old.id != screens::PAUSE_ID && pages.as_ref().is_some_and(|p| p.has(&old.id)) => {
                        let p = pages.as_ref().expect("checked");
                        let id = old.id.clone();
                        let info = p.info(&id).cloned().unwrap_or_default();
                        let still = p.same(&id, &frame, &ocr, old) == Some(true);
                        if info.hold_ms == 0 && info.reread_ms == 0 {
                            if !still {
                                match p.present(&id, &frame, &ocr).and_then(|k| p.read(&id, &frame, &ocr, &k)) {
                                    Some(l) => found = Some(l),
                                    None => hide_screen(&app, &shared, &mut screen),
                                }
                            }
                        } else {
                            if still {
                                title_missing_since = None;
                            }
                            let missing = title_missing_since.get_or_insert_with(Instant::now).elapsed();
                            if !still && ((info.end_when_dark && screens::is_dark(&frame)) || missing > Duration::from_millis(info.hold_ms)) {
                                hide_screen(&app, &shared, &mut screen);
                                title_missing_since = None;
                            } else if !still {
                                // still fading: keep showing, no re-read
                            } else if info.reread_ms > 0 && screen_checked.elapsed() >= Duration::from_millis(info.reread_ms) {
                                screen_checked = Instant::now();
                                if let Some(mut l) = p.present(&id, &frame, &ocr).and_then(|k| p.read(&id, &frame, &ocr, &k)) {
                                    screens::keep_measured(&mut l, old);
                                    found = Some(l);
                                }
                            }
                        }
                    }
                    // Pause menu: gone the moment its tab bar is; re-read twice a second (the selected
                    // tab moves as the player browses).
                    Some(old) if old.id == screens::PAUSE_ID => {
                        // the script reads the menu when it has it; the app follows it here either way
                        let by_script = pages.as_ref().filter(|p| p.has(screens::PAUSE_ID));
                        let pause_present = |f: &capture::FrameData| match by_script {
                            Some(p) => p.same(screens::PAUSE_ID, f, &ocr, old) == Some(true),
                            None => screens::pause_present(f),
                        };
                        let reread = |f: &capture::FrameData| match by_script {
                            Some(p) if p.can(screens::PAUSE_ID, "reread") => p.reread(screens::PAUSE_ID, f, &ocr, old),
                            Some(p) => p.present(screens::PAUSE_ID, f, &ocr).and_then(|k| p.read(screens::PAUSE_ID, f, &ocr, &k)),
                            None => screens::reread_pause_menu(f, &ocr, old),
                        };
                        let full_read = |f: &capture::FrameData| match by_script {
                            Some(p) => p.present(screens::PAUSE_ID, f, &ocr).and_then(|k| p.read(screens::PAUSE_ID, f, &ocr, &k)),
                            None => screens::detect_gta5_pause(f, &ocr),
                        };
                        // Every frame: a menu line whose English changed or went away loses its Arabic
                        // at once, and the menu is read again straight away.
                        let gone = old.menu.iter().any(|m| !screens::region_same(&frame, &m.rect, &m.sig))
                            || old.labels.iter().any(|l| !screens::region_same(&frame, &l.rect, &l.sig))
                            || !screens::region_same(&frame, &screens::BUTTON_ROW, &old.buttons_sig);
                        // Read again only when something changed (or every 3 s, to be safe): re-reading a
                        // still menu only makes OCR's boxes shift a few pixels.
                        let changed = screens::content_changed(&frame, &old.content_sig);
                        let due = screen_checked.elapsed() >= Duration::from_secs(3);
                        // a state of this tab seen before, matching what's on screen now
                        let tab = screens::selected_tab_name(old).unwrap_or("").to_string();
                        let known = (gone || changed)
                            .then(|| menu_states.iter().find(|(t, sig, _)| *t == tab && !screens::content_changed(&frame, sig)))
                            .flatten()
                            .filter(|(_, sig, _)| *sig != old.content_sig);
                        if !pause_present(&frame) {
                            hide_screen(&app, &shared, &mut screen);
                        } else if let Some((_, sig, menu)) = known {
                            let mut l = old.clone();
                            (l.menu, l.content_sig) = (menu.clone(), sig.clone());
                            screen_checked = Instant::now();
                            found = Some(l);
                        } else if gone && !due && old.labels.iter().all(|l| screens::region_same(&frame, &l.rect, &l.sig)) {
                            // Same tab, its content changed (the controller diagram swapped labels): read
                            // the menu again right now (~150 ms), so the old lines go and the new ones come
                            // together, with no half-empty moment in between.
                            screen_checked = Instant::now();
                            match reread(&frame) {
                                Some(l) => found = Some(l),
                                None => {
                                    let mut l = old.clone();
                                    l.menu.retain(|m| screens::region_same(&frame, &m.rect, &m.sig));
                                    found = Some(l);
                                }
                            }
                        } else if gone && !due {
                            // drop what changed now; read the menu again on the next pass (`due`)
                            let mut l = old.clone();
                            l.menu.retain(|m| screens::region_same(&frame, &m.rect, &m.sig));
                            screen_checked = Instant::now() - Duration::from_secs(10);
                            found = Some(l);
                        } else if !changed && !due && !gone {
                            // nothing new on screen: just add translations that arrived meanwhile
                            if old.menu.iter().any(|m| m.ar.is_empty()) || screen_missing {
                                found = Some(old.clone());
                            }
                        } else {
                            screen_checked = Instant::now();
                            // same tabs as before: a quick re-read of just the menu; else a full read
                            let tabs_same = old.labels.iter().all(|l| screens::region_same(&frame, &l.rect, &l.sig));
                            let read = if tabs_same { reread(&frame) } else { full_read(&frame) };
                            match read {
                                Some(l) => found = Some(l),
                                // mid-transition (tabs sliding): try again in half a second
                                None => screen_checked = Instant::now() - Duration::from_millis(2500),
                            }
                        }
                    }
                    // Centre caption: gone the moment its white text is (checked on every frame, pixels
                    // only); a different caption in its place is read again.
                    // Alert page ("Please reconnect controller"): gone the moment its yellow title is;
                    // the message is read again once a second (it can change), the title stays put.
                    Some(old) if old.id == screens::ALERT_ID => {
                        if !screens::alert_present(&frame) {
                            hide_screen(&app, &shared, &mut screen);
                        } else if screen_checked.elapsed() >= Duration::from_secs(1) {
                            screen_checked = Instant::now();
                            if let Some(mut l) = screens::detect_gta5_alert(&frame, &ocr) {
                                l.title = old.title.clone();
                                if l.reason_en.is_empty() || l.reason_en == old.reason_en {
                                    (l.reason, l.reason_line_h, l.reason_en) = (old.reason.clone(), old.reason_line_h, old.reason_en.clone());
                                }
                                if l.buttons.len() == old.buttons.len() {
                                    (l.buttons, l.buttons_row) = (old.buttons.clone(), old.buttons_row.clone());
                                }
                                found = Some(l);
                            }
                        }
                    }
                    Some(old) if old.id == screens::CAPTION_ID => match screens::find_caption(&frame) {
                        Some(b) if screens::caption_same(&frame, b, old) => {}
                        Some(_) => match screens::detect_gta5_caption(&frame, &ocr) {
                            Some(l) => found = Some(l),
                            None => hide_screen(&app, &shared, &mut screen),
                        },
                        None => hide_screen(&app, &shared, &mut screen),
                    },
                    Some(old) => {
                        let present = screens::title_present(&frame);
                        if present {
                            title_missing_since = None;
                        }
                        // Once shown, keep it: the title can blink out for a moment while the scene behind it
                        // fades. The page is over when the title is gone AND the screen has gone dark (or,
                        // as a safety net, the title has been missing for 5 s).
                        let missing = title_missing_since.get_or_insert_with(Instant::now).elapsed();
                        if !present && (screens::is_dark(&frame) || missing > Duration::from_secs(5)) {
                            hide_screen(&app, &shared, &mut screen);
                            title_missing_since = None;
                        } else if !present {
                            // still fading: keep showing, no re-read
                        } else if screen_checked.elapsed() >= Duration::from_secs(1) {
                            screen_checked = Instant::now();
                            if let Some(mut l) = screens::detect_gta5_mission_failed(&frame, &ocr) {
                                screens::keep_measured(&mut l, old);
                                found = Some(l);
                            }
                        }
                    }
                }
            } else if screen.is_some() {
                hide_screen(&app, &shared, &mut screen);
            }
            if let Some(layout) = found.as_mut().filter(|l| !l.menu.is_empty()) {
                // Pause-menu content: built-in words, then this session's translations, then the
                // translation memory; anything else is sent to the translator and shows on a later read.
                // The Friends tab lists other players' nicknames: never translate, send or keep those.
                let friends_tab = screens::selected_tab_name(layout) == Some("FRIENDS");
                if friends_tab {
                    layout.menu.retain(|m| !m.item);
                }
                for m in layout.menu.iter_mut() {
                    // (OCR may spell a line slightly differently from one read to the next)
                    let cached = {
                        let cache = shared.menu_cache.lock().unwrap();
                        cache.get(&m.en).cloned().or_else(|| cache.iter().find(|(k, _)| ocr::similar(k, &m.en)).map(|(_, v)| v.clone()))
                    };
                    let remembered = || {
                        c.memory_enabled
                            .then(|| memory.lock().unwrap().get(&m.en, &[], &[]))
                            .flatten()
                            .map(|(segs, _)| segs.iter().map(|s| s.0.as_str()).collect::<String>())
                            .filter(|ar| translate::keeps_terms(&m.en, ar))
                    };
                    // a list item is one of the menu's known words, even when OCR garbled it (faint, greyed-out items)
                    // (also headings and short panel lines: the faint "Join Friends" title)
                    if arabic && (m.item || m.heading || m.en.split_whitespace().count() <= 3) && !m.en.trim().starts_with('<') {
                        if let Some((en, ar)) = screens::menu_item_word(&m.en) {
                            m.en = en.to_string();
                            m.ar = ar.to_string();
                            continue;
                        }
                    }
                    let built_in = arabic.then(|| screens::menu_word(&m.en).map(String::from).or_else(|| screens::menu_rule(&m.en))).flatten();
                    match built_in.or(cached).or_else(remembered) {
                        // a switcher "< Michael >" keeps its arrows
                        Some(ar) if m.en.trim().starts_with('<') && m.en.trim().ends_with('>') => m.ar = format!("< {ar} >"),
                        Some(ar) => m.ar = ar,
                        None => {
                            if !menu_asked.iter().any(|a| ocr::similar(a, &m.en)) && menu_asked.insert(m.en.clone()) {
                                seq_counter += 1;
                                let _ = jobs.send(Job { area: MENU_TEXT.into(), seq: seq_counter, en: m.en.clone(), phrases: Vec::new(), icons: Vec::new(), line_h: 0.0, en_start: Instant::now(), text_box: None });
                            }
                        }
                    }
                }
                // Stability from one read to the next, on the same tab: a line read a bit differently
                // keeps its Arabic.
                if let Some(old) = screen.as_ref().filter(|o| o.id == screens::PAUSE_ID && screens::selected_tab_name(o) == screens::selected_tab_name(layout)) {
                    let near = |a: &screens::Rect, b: &screens::Rect| (a.y - b.y).abs() < a.h.min(b.h) * 0.6 && (a.x - b.x).abs() < 0.03;
                    // The same line found again (same place, similar words): keep its box, colours and
                    // Arabic exactly, so nothing shifts or changes wording between reads.
                    for m in layout.menu.iter_mut() {
                        // the picture there hasn't changed: it's the same text, whatever OCR made of it this time
                        if let Some(o) = old.menu.iter().find(|o| near(&o.rect, &m.rect) && !o.ar.is_empty() && screens::region_same(&frame, &o.rect, &o.sig)) {
                            *m = o.clone();
                            continue;
                        }
                        if let Some(o) = old.menu.iter().find(|o| near(&o.rect, &m.rect) && ocr::similar(&o.en, &m.en)) {
                            // same place and words as before; colours and picture from this read (the
                            // selection's white highlight may have moved onto it or off it)
                            let ar = if o.ar.is_empty() { m.ar.clone() } else { o.ar.clone() };
                            *m = screens::MenuText { ar, rect: o.rect.clone(), right: o.right, ..m.clone() };
                        }
                    }
                }
                // Remember this state of the tab (and add lines an earlier read of it found but
                // this one missed, if they're still on screen). Only a layout that matches the
                // picture on screen — not one just trimmed while the page changes under it.
                if layout.id == screens::PAUSE_ID && !screens::content_changed(&frame, &layout.content_sig) {
                    let tab = screens::selected_tab_name(layout).unwrap_or("").to_string();
                    let near = |a: &screens::Rect, b: &screens::Rect| (a.y - b.y).abs() < a.h.min(b.h) * 0.6 && (a.x - b.x).abs() < 0.03;
                    if let Some(e) = menu_states.iter_mut().find(|(t, sig, _)| *t == tab && !screens::content_changed(&frame, sig)) {
                        for o in e.2.iter() {
                            if !o.ar.is_empty() && !layout.menu.iter().any(|m| near(&o.rect, &m.rect)) && screens::region_same(&frame, &o.rect, &o.sig) {
                                layout.menu.push(o.clone());
                            }
                        }
                        (e.1, e.2) = (layout.content_sig.clone(), layout.menu.clone());
                    } else {
                        menu_states.push((tab, layout.content_sig.clone(), layout.menu.clone()));
                        if menu_states.len() > 16 {
                            menu_states.remove(0);
                        }
                    }
                }
            }
            // Other languages than Arabic: the page's fixed words (title, tabs, buttons, the name
            // block) come from the translator, like the menu text, and show once translated.
            if let Some(layout) = found.as_mut().filter(|_| !arabic) {
                let cache = shared.menu_cache.lock().unwrap().clone();
                let mut ask = |en: &str| {
                    if menu_asked.insert(en.to_string()) {
                        seq_counter += 1;
                        let _ = jobs.send(Job { area: MENU_TEXT.into(), seq: seq_counter, en: en.to_string(), phrases: Vec::new(), icons: Vec::new(), line_h: 0.0, en_start: Instant::now(), text_box: None });
                    }
                };
                screen_missing = localize_screen(layout, &cache, &mut ask);
            } else if found.is_some() {
                screen_missing = false;
            }
            if let Some(mut layout) = found {
                // the reason is always drawn in the same size
                // (measured only on a new page / new reason: refreshes reuse the frozen geometry)
                let new_reason = screen.as_ref().is_none_or(|s| s.reason_en != layout.reason_en);
                if new_reason && layout.id != screens::PAUSE_ID {
                    let kind = match layout.id.as_str() {
                        screens::CAPTION_ID => "__screen_caption",
                        screens::ALERT_ID => "__screen_alert",
                        _ => SCREEN_REASON,
                    };
                    layout.reason_line_h = sizes.lock().unwrap().size(kind, layout.reason_line_h);
                }
                // ...and so is the title (kept centred where the game's title is)
                let th = if screen.is_none() && layout.id != screens::PAUSE_ID && layout.id != screens::CAPTION_ID {
                    let kind = if layout.id == screens::ALERT_ID { "__screen_alert_title" } else { "__screen_title" };
                    sizes.lock().unwrap().size(kind, layout.title.h)
                } else {
                    0.0
                };
                if th > 0.0 {
                    layout.title.y += (layout.title.h - th) / 2.0;
                    layout.title.h = th;
                }
                if screen.is_none() {
                    screen_checked = Instant::now();
                    title_missing_since = None;
                    // The page replaces normal subtitles: clear what the areas were showing, right away.
                    for (id, st) in states.iter_mut() {
                        st.current.clear();
                        st.candidate.clear();
                        st.current_seq = 0;
                        seq_counter += 1;
                        latest.lock().unwrap().insert(id.clone(), seq_counter);
                    }
                    let _ = present.send(PMsg::ClearAll);
                }
                if screen.as_ref() != Some(&layout) {
                    let reason_changed = screen.as_ref().map(|s| s.reason_en != layout.reason_en).unwrap_or(true);
                    if reason_changed {
                        *shared.screen_reason.lock().unwrap() = serde_json::json!([]);
                        if !layout.reason_en.is_empty() {
                            seq_counter += 1;
                            latest.lock().unwrap().insert(SCREEN_REASON.into(), seq_counter);
                            match arabic.then(|| screens::reason_rule(&layout.reason_en)).flatten() {
                                // Known GTA phrasing ("X died."): exact Arabic, no translator needed.
                                Some(ar) => {
                                    if c.memory_enabled {
                                        memory.lock().unwrap().put(&layout.reason_en, &vec![(ar.clone(), None)], "phrasebook");
                                    }
                                    *shared.screen_reason.lock().unwrap() = serde_json::json!([[ar.clone(), null]]);
                                    lan::push(&shared, SCREEN_REASON, &layout.reason_en, &[(ar.clone(), None)], unix_ms(Instant::now()));
                                    let _ = app.emit("translation", TranslationEvent {
                                        area: SCREEN_REASON.into(),
                                        en: layout.reason_en.clone(),
                                        segments: vec![(ar, None)],
                                        stats: "built-in GTA V phrase".into(),
                                        error: None,
                                        line_h: 0.0,
                                        text_box: None,
                                        cached: false,
                                        at: unix_ms(Instant::now()),
                                    });
                                }
                                None => {
                                    let _ = jobs.send(Job { area: SCREEN_REASON.into(), seq: seq_counter, en: layout.reason_en.clone(), phrases: Vec::new(), icons: Vec::new(), line_h: 0.0, en_start: Instant::now(), text_box: None });
                                }
                            }
                        }
                    }
                    *shared.screen.lock().unwrap() = serde_json::to_value(&layout).ok();
                    let _ = app.emit("screen", Some(&layout));
                    shared.screen_active.store(true, std::sync::atomic::Ordering::Relaxed);
                    screen = Some(layout);
                }
            }
            // Log each Mission Failed reason with its time on screen, like subtitle lines.
            let reason_now = screen.as_ref().map(|s| s.reason_en.clone()).filter(|r| !r.is_empty());
            if reason_on.as_ref().map(|(r, _)| r) != reason_now.as_ref() {
                if let (true, Some((r, since))) = (c.memory_enabled || c.share_lines, reason_on.take()) {
                    memory.lock().unwrap().log_line(SCREEN_REASON, &r, unix_ms(since), unix_ms(Instant::now()));
                }
                reason_on = reason_now.map(|r| (r, Instant::now()));
            }
            if screen.is_some() {
                // The pause menu is checked on every new frame (its checks are tiny), so its Arabic
                // goes the moment the English does: pace by the capture rate, not the read interval.
                let pause = screen.as_ref().is_some_and(|s| [screens::PAUSE_ID, screens::CAPTION_ID, screens::ALERT_ID].contains(&s.id.as_str()));
                let pace = if pause { (1000 / c.capture_fps.clamp(1, 30) as u64).min(c.interval_ms) } else { c.interval_ms };
                let spent = t0.elapsed();
                std::thread::sleep(Duration::from_millis(pace.max(30)).saturating_sub(spent));
                continue; // normal areas pause while the page is up
            }

            states.retain(|id, _| c.areas.iter().any(|a| a.enabled && &a.id == id));
            for area in c.areas.iter().filter(|a| a.enabled) {
                let st = states.entry(area.id.clone()).or_default();
                let now = Instant::now();
                let crop = Crop::from_frame(&frame.rgba, frame.w, frame.h, area.x, area.y, area.w, area.h);
                let (mut text, words) = match &crop {
                    Some(cr) => match ocr.recognize(cr) {
                        Ok(words) => (ocr::clean(&words), words),
                        Err(_) => (String::new(), Vec::new()),
                    },
                    None => (String::new(), Vec::new()),
                };
                // Subtitles are centred in the picture: text whose middle is off-centre (a menu's
                // "Browse" at the bottom right) is not a subtitle.
                if area.centered_only && !words.is_empty() {
                    let x0 = words.iter().map(|w| w.rect.0).min().unwrap_or(0) as f64;
                    let x1 = words.iter().map(|w| w.rect.0 + w.rect.2).max().unwrap_or(0) as f64;
                    let cx = area.x.clamp(0.0, 1.0) + (x0 + x1) / 2.0 / frame.w as f64;
                    if (cx - 0.5).abs() > 0.04 {
                        text.clear();
                    }
                }
                // Not English at all (another script, icons, a map read as letters): no text here.
                if !ocr::looks_like_words(&text) {
                    text.clear();
                }
                if !text.is_empty() {
                    st.last_seen = Some(now);
                    let mut is_new = false;
                    if !ocr::similar(&text, &st.candidate) {
                        st.candidate = text;
                        st.candidate_since = Some(now);
                        is_new = true;
                    }
                    // A line the translation memory already knows needn't wait to settle: show it on
                    // first sight. Other lines wait `stable_ms` so a half-faded frame isn't translated.
                    let known = is_new
                        && c.memory_enabled
                        && crop.as_ref().is_some_and(|cr| {
                            let (t, icons, _) = ocr::text_with_icons(cr, &words);
                            memory.lock().unwrap().knows(if icons.is_empty() { &st.candidate } else { &t })
                        });
                    let settled = !is_new && st.candidate_since.is_some_and(|s| now - s >= Duration::from_millis(c.stable_ms));
                    if (known || settled) && !ocr::similar(&st.candidate, &st.current) {
                        // Stable new text: translate it once. Colours are measured only now.
                        st.current = st.candidate.clone();
                        let phrases = crop.as_ref().map(|cr| ocr::colored_phrases(cr, &words)).unwrap_or_default();
                        // Controller button icons inside the text ("Press ✕ to ...") come along as placeholders.
                        let (en_icons, icons_found, block) =
                            crop.as_ref().map(|cr| ocr::text_with_icons(cr, &words)).unwrap_or((String::new(), Vec::new(), None));
                        let (en, icons) = if icons_found.is_empty() { (st.current.clone(), Vec::new()) } else { (en_icons, icons_found) };
                        // the block in frame fractions (the crop starts at the area's top-left)
                        let (cx0, cy0) = ((area.x.clamp(0.0, 1.0) * frame.w as f64) as u32, (area.y.clamp(0.0, 1.0) * frame.h as f64) as u32);
                        let text_box = block.map(|(x, y, w, h)| {
                            let (fw, fh) = (frame.w as f64, frame.h as f64);
                            [(cx0 + x) as f64 / fw, (cy0 + y) as f64 / fh, w as f64 / fw, h as f64 / fh]
                        });
                        seq_counter += 1;
                        latest.lock().unwrap().insert(area.id.clone(), seq_counter);
                        // the previous English line ended when this one appeared
                        let en_start = st.candidate_since.unwrap_or(now);
                        if st.current_seq != 0 {
                            let _ = present.send(PMsg::Ended { area: area.id.clone(), seq: st.current_seq, at: en_start });
                            if let (true, Some(s0)) = (c.memory_enabled || c.share_lines, st.current_start) {
                                memory.lock().unwrap().log_line(&area.id, &st.current_en, unix_ms(s0), unix_ms(en_start));
                            }
                        }
                        st.current_seq = seq_counter;
                        // typical English word height (median) -> the Arabic is sized to match
                        let mut hs: Vec<u32> = words.iter().map(|w| w.rect.3).collect();
                        hs.sort_unstable();
                        let measured = hs.get(hs.len() / 2).map(|&h| h as f64 / frame.h as f64).unwrap_or(0.0);
                        // same size for every line of this area
                        let line_h = sizes.lock().unwrap().size(&area.id, measured);
                        st.current_en = en.clone();
                        st.current_start = Some(en_start);
                        let _ = jobs.send(Job { area: area.id.clone(), seq: seq_counter, en, phrases, icons, line_h, en_start, text_box });
                    }
                } else if !st.current.is_empty()
                    && st.last_seen.is_some_and(|s| now - s > Duration::from_millis(c.clear_after_ms))
                {
                    // The English is gone (not just an OCR blink). It really ended when last seen; the
                    // presenter keeps the Arabic up for as long as the English was on screen.
                    if let Some(last) = st.last_seen {
                        let _ = present.send(PMsg::Ended { area: area.id.clone(), seq: st.current_seq, at: last });
                        if let (true, Some(s0)) = (c.memory_enabled || c.share_lines, st.current_start) {
                            memory.lock().unwrap().log_line(&area.id, &st.current_en, unix_ms(s0), unix_ms(last));
                        }
                    }
                    st.current_start = None;
                    st.current.clear();
                    st.candidate.clear();
                    st.candidate_since = None;
                    st.current_seq = 0;
                } else if st.current.is_empty()
                    && !st.candidate.is_empty()
                    && st.last_seen.is_some_and(|s| now - s > Duration::from_millis(c.clear_after_ms))
                {
                    // Text that came and went before it was ever translated: forget it, or its old start
                    // time would be reused when similar text shows up minutes later.
                    st.candidate.clear();
                    st.candidate_since = None;
                }
            }
        }
        let spent = t0.elapsed();
        std::thread::sleep(Duration::from_millis(c.interval_ms.max(50)).saturating_sub(spent));
    }
}

fn translate_loop(
    app: AppHandle,
    cfg: Arc<Mutex<Config>>,
    shared: Arc<Shared>,
    latest: Arc<Mutex<HashMap<String, u64>>>,
    jobs: mpsc::Receiver<Job>,
    present: mpsc::Sender<PMsg>,
    memory: Arc<Mutex<memory::Memory>>,
) {
    let mut tr = Translator::new(model_dir());
    for job in jobs {
        // Skip text that was already replaced on screen while we were busy. (Pause-menu lines
        // aren't an area: each is translated once and kept, so never skip those.)
        if job.area != MENU_TEXT && latest.lock().unwrap().get(&job.area).copied() != Some(job.seq) {
            continue;
        }
        let c = cfg.lock().unwrap().clone();
        let status_app = app.clone();
        let status = move |s: String| {
            let _ = status_app.emit("status-msg", s);
        };
        // Translation memory first (when enabled): a known line needs no translator at all.
        // (the Mission Failed reason too: same game text, same reuse)
        let use_memory = c.memory_enabled;
        let mut remembered = memory.lock().unwrap().get(&job.en, &job.icons, &job.phrases).filter(|(_, src)| use_memory || src == "reviewed");
        // (not one that mangled a term that must stay, like GTA+ — made before terms were protected)
        if remembered.as_ref().is_some_and(|(segs, _)| !translate::keeps_terms(&job.en, &segs.iter().map(|s| s.0.as_str()).collect::<String>())) {
            remembered = None;
        }
        // A hand-written phrasebook line beats a machine translation remembered earlier (only an
        // reviewed line ranks above it).
        if c.game() == "gta5" && c.target_lang == "ar" && remembered.as_ref().is_some_and(|(_, src)| src != "reviewed" && src != "phrasebook") {
            if phrasebook::lookup(&job.en, &job.icons).is_some() {
                remembered = None; // the translator below answers from the phrasebook, and memory is updated
            }
        }
        let hit = remembered.is_some();
        let hit_from = if remembered.is_some() { "memory" } else { "" };
        let result = match remembered {
            // (coloured here when the remembered line has no colours but the game highlights a word)
            Some((segs, _)) => Ok(tr.recolor(&c, segs, &job.phrases, &status)),
            None => tr.translate(&c, &job.en, &job.phrases, &job.icons, &status),
        };
        // time from the English line appearing to its Arabic being ready
        let ready_ms = job.en_start.elapsed().as_secs_f64() * 1000.0;
        let mem_note = if use_memory {
            let mut m = memory.lock().unwrap();
            if let (false, Ok(segs)) = (hit, &result) {
                let source = if c.game() == "gta5" && c.target_lang == "ar" && phrasebook::lookup(&job.en, &job.icons).is_some() { "phrasebook" } else { c.engine.as_str() };
                m.put(&job.en, segs, source);
            }
            m.record(hit, ready_ms);
            Some(if hit { format!("{hit_from} hit · ready {ready_ms:.0} ms after the English") } else { format!("ready {ready_ms:.0} ms after the English · saved to memory") })
        } else {
            None
        };
        // pause-menu text: keep it for the menu page (it appears on the next read)
        if job.area == MENU_TEXT {
            if let Ok(segs) = &result {
                let ar: String = segs.iter().map(|s| s.0.as_str()).collect();
                shared.menu_cache.lock().unwrap().insert(job.en.clone(), ar.trim().to_string());
            }
            continue;
        }
        // show icons in the English line of the log as a small marker
        let job = Job { en: strip_icon_tokens(&job.en), ..job };
        // The Mission Failed reason is only useful while it is current. (Area lines go to the
        // presenter even if a newer line exists: it shows them for their time, then the next.)
        if job.area == SCREEN_REASON && latest.lock().unwrap().get(&job.area).copied() != Some(job.seq) {
            continue;
        }
        let (seq, en_start) = (job.seq, job.en_start);
        let ev = match result {
            Ok(segments) => TranslationEvent {
                area: job.area,
                en: job.en,
                segments,
                stats: match &mem_note {
                    Some(n) if hit => n.clone(),
                    Some(n) => format!("{} · {n}", tr.stats(&c)),
                    None => tr.stats(&c),
                },
                error: None,
                line_h: job.line_h,
                text_box: job.text_box,
                cached: hit,
                at: unix_ms(en_start),
            },
            Err(e) => TranslationEvent {
                area: job.area,
                en: job.en,
                segments: Vec::new(),
                stats: tr.stats(&c),
                error: Some(format!("{e:#}")),
                line_h: job.line_h,
                text_box: job.text_box,
                cached: false,
                at: unix_ms(en_start),
            },
        };
        if ev.area == SCREEN_REASON && ev.error.is_none() {
            *shared.screen_reason.lock().unwrap() = serde_json::to_value(&ev.segments).unwrap_or_default();
            lan::push(&shared, SCREEN_REASON, &ev.en, &ev.segments, ev.at);
        }
        if ev.area == SCREEN_REASON || ev.error.is_some() {
            let _ = app.emit("translation", ev); // errors go straight to the log
        } else {
            let _ = present.send(PMsg::Ready { seq, en_start, ev });
        }
    }
}

// ------------------------------------------------------------------------------------- test mode

/// `vi-translator --test-image shot.png [--area help]`: OCR + translate one screenshot and print the result.
fn test_image(path: &str, area_id: &str) -> anyhow::Result<()> {
    let cfg = Config::load(&config_path());
    let mut area = cfg.areas.iter().find(|a| a.id == area_id).cloned().ok_or_else(|| anyhow::anyhow!("no area {area_id}"))?;
    // `--region x,y,w,h` (fractions) overrides the saved area, so tests do not depend on your layout
    let args: Vec<String> = std::env::args().collect();
    if let Some(r) = args.iter().position(|a| a == "--region").and_then(|j| args.get(j + 1)) {
        let v: Vec<f64> = r.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if let [x, y, w, h] = v[..] {
            (area.x, area.y, area.w, area.h) = (x, y, w, h);
        }
    }
    let img = image::open(path)?.to_rgba8();
    let (w, h) = img.dimensions();
    let crop = Crop::from_frame(img.as_raw(), w, h, area.x, area.y, area.w, area.h).ok_or_else(|| anyhow::anyhow!("empty crop"))?;
    Ocr::init_thread();
    let ocr = Ocr::new()?;
    let t = Instant::now();
    let words = ocr.recognize(&crop)?;
    if std::env::var_os("VI_TRANSLATOR_DEBUG").is_some() {
        eprintln!("[ocr] words: {:?}", words.iter().map(|w| (&w.text, w.rect)).collect::<Vec<_>>());
    }
    let en = ocr::clean(&words);
    let phrases = ocr::colored_phrases(&crop, &words);
    println!("OCR ({} ms): {en:?}", t.elapsed().as_millis());
    let t2 = Instant::now();
    let (en_icons, icons, block) = ocr::text_with_icons(&crop, &words);
    println!("Text block (crop px): {block:?}");
    // with VI_TRANSLATOR_ICON_DIR set, write the cut-out icons there (to check how they look)
    if let Some(dir) = std::env::var_os("VI_TRANSLATOR_ICON_DIR") {
        use base64::Engine as _;
        for (n, url) in icons.iter().enumerate() {
            let (rel, png) = url.split_once('|').unwrap_or(("?", url.as_str()));
            println!("icon {}: {} x the text height", n + 1, rel);
            if let Some(b64) = png.strip_prefix("data:image/png;base64,") {
                let bytes = base64::engine::general_purpose::STANDARD.decode(b64)?;
                let stem = std::path::Path::new(path).file_stem().unwrap_or_default().to_string_lossy().to_string();
                std::fs::write(std::path::Path::new(&dir).join(format!("{stem}_icon{}.png", n + 1)), bytes)?;
            }
        }
    }
    println!("With icons ({} ms, {} icon(s)): {en_icons:?}", t2.elapsed().as_millis(), icons.len());
    if !phrases.is_empty() {
        println!("Highlighted: {phrases:?}");
    }
    let mut tr = Translator::new(model_dir());
    let t = Instant::now();
    let segs = tr.translate(&cfg, &en_icons, &phrases, &icons, &|s| println!("{s}"))?;
    let shown: String = segs
        .iter()
        .map(|(t, c)| match c {
            Some(c) if c.starts_with("icon:") => "🎮".to_string(),
            Some(_) => format!("[{t}]"),
            None => t.clone(),
        })
        .collect();
    println!("Translation ({} ms, {}): {shown}", t.elapsed().as_millis(), cfg.engine);
    Ok(())
}

// ------------------------------------------------------------------------------------- main

fn main() {
    move_old_folders();
    let args: Vec<String> = std::env::args().collect();
    // `--screen-copy "Window title" out.png`: copy that window's area of the screen (overlays included).
    if let Some(i) = args.iter().position(|a| a == "--screen-copy") {
        let (Some(title), Some(out)) = (args.get(i + 1), args.get(i + 2)) else { std::process::exit(2) };
        let hwnd = capture::find_window(title).unwrap_or(0);
        let r = capture::client_rect(hwnd).and_then(|(x, y, w, h)| capture::grab_screen_region(x, y, w, h).map(|px| (w, h, px)));
        match r {
            Some((w, h, px)) => {
                let f = capture::FrameData { w: w as u32, h: h as u32, rgba: px, seq: 0 };
                capture::save_png(&f, std::path::Path::new(out)).expect("save");
                println!("saved {w}x{h} screen copy to {out}");
            }
            None => eprintln!("screen copy failed"),
        }
        return;
    }
    // `--grab "Window title" out.png`: save one frame of a window (for building/testing profiles).
    // `--grab-device "fifine Video Capture" out.png` / `--list-devices`: test a capture card.
    if args.iter().any(|a| a == "--list-devices") {
        for d in device::list() {
            println!("{d}");
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--grab-device") {
        let (Some(name), Some(out)) = (args.get(i + 1), args.get(i + 2)) else {
            eprintln!("usage: --grab-device \"Device name\" out.png");
            std::process::exit(2);
        };
        let t = std::time::Instant::now();
        match device::grab_once(name).and_then(|f| capture::save_png(&f, std::path::Path::new(out)).map(|_| f)) {
            Ok(f) => println!("saved {}x{} frame to {out} ({} ms)", f.w, f.h, t.elapsed().as_millis()),
            Err(e) => {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    // `--grab-obs "Video Capture Device" out.png`: one frame of an OBS source, using the app's saved
    // OBS port and password.
    // `--translate "line" ["line" ...]`: translate text with the configured engine (for testing).
    if let Some(i) = args.iter().position(|a| a == "--translate") {
        let cfg = Config::load(&config_path());
        let mut tr = translate::Translator::new(model_dir());
        for line in &args[i + 1..] {
            match tr.translate(&cfg, line, &[], &[], &|_| {}) {
                Ok(segs) => println!("{line}
  -> {}", segs.iter().map(|s| s.0.as_str()).collect::<String>()),
                Err(e) => println!("{line}
  !! {e:#}"),
            }
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--grab-obs") {
        let (Some(name), Some(out)) = (args.get(i + 1), args.get(i + 2)) else {
            eprintln!("usage: --grab-obs \"OBS source name\" out.png");
            std::process::exit(2);
        };
        let cfg = Config::load(&config_path());
        let t = std::time::Instant::now();
        let r = obs::Obs::connect(cfg.obs_port, &cfg.obs_password).and_then(|mut o| {
            let list = o.inputs()?;
            println!("OBS sources: {}", list.iter().map(|(n, k)| format!("{n} ({k})")).collect::<Vec<_>>().join(", "));
            let t1 = std::time::Instant::now();
            let f = o.snapshot(name)?;
            let full_ms = t1.elapsed().as_millis();
            let t2 = std::time::Instant::now();
            let j = o.preview_jpeg(name)?;
            println!("full frame {}x{} in {full_ms} ms; preview JPEG {} KB in {} ms", f.w, f.h, j.len() / 1024, t2.elapsed().as_millis());
            capture::save_png(&f, std::path::Path::new(out))?;
            Ok(())
        });
        match r {
            Ok(()) => println!("saved to {out} ({} ms total)", t.elapsed().as_millis()),
            Err(e) => {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--grab") {
        let (Some(title), Some(out)) = (args.get(i + 1), args.get(i + 2)) else {
            eprintln!("usage: --grab \"Window title\" out.png");
            std::process::exit(2);
        };
        match capture::grab_once(title).and_then(|f| capture::save_png(&f, std::path::Path::new(out)).map(|_| f)) {
            Ok(f) => println!("saved {}x{} frame to {out}", f.w, f.h),
            Err(e) => {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    // `--test-recolor "<arabic>" "<highlighted english>"`: colour a remembered (uncoloured) line.
    if let Some(i) = args.iter().position(|a| a == "--test-recolor") {
        let (Some(ar), Some(phrase)) = (args.get(i + 1), args.get(i + 2)) else { return };
        let cfg = Config::load(&config_path());
        let mut tr = Translator::new(model_dir());
        let segs = tr.recolor(&cfg, vec![(ar.clone(), None)], &[(phrase.clone(), "#7ed957".into())], &|_| {});
        println!("{segs:?}");
        return;
    }
    // `--compare-pages <folder>`: run the built-in pages script and the Rust detection on every PNG in a
    // folder (e.g. saved debug frames) and report where they differ.
    if let Some(dir) = args.iter().position(|a| a == "--compare-pages").and_then(|i| args.get(i + 1)) {
        let run = || -> anyhow::Result<()> {
            Ocr::init_thread();
            let ocr = Ocr::new()?;
            let pages = pagescript::Pages::for_game("gta5", std::path::Path::new("")).ok_or_else(|| anyhow::anyhow!("no gta5 script"))?;
            println!("script: {}", pages.origin);
            let mut files: Vec<_> = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "png")).collect();
            files.sort();
            let (mut n, mut found, mut differ, mut ms_rust, mut ms_js) = (0, 0, 0, 0u128, 0u128);
            for path in files {
                let Ok(img) = image::open(&path) else { continue };
                let img = img.to_rgba8();
                let (w, h) = img.dimensions();
                let frame = capture::FrameData { w, h, rgba: img.into_raw(), seq: 1 };
                n += 1;
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                // each page, the app's Rust detection and the script, compared in full
                let checks: [(&str, Box<dyn Fn() -> Option<screens::ScreenLayout>>); 4] = [
                    (screens::CAPTION_ID, Box::new(|| screens::find_caption(&frame).and_then(|_| screens::detect_gta5_caption(&frame, &ocr)))),
                    ("gta5_mission_failed", Box::new(|| screens::detect_gta5_mission_failed(&frame, &ocr))),
                    (screens::ALERT_ID, Box::new(|| if screens::alert_present(&frame) { screens::detect_gta5_alert(&frame, &ocr) } else { None })),
                    (screens::PAUSE_ID, Box::new(|| screens::detect_gta5_pause(&frame, &ocr))),
                ];
                // the pause menu's quick re-read (against the Rust read of this same frame)
                if let Some(old) = screens::detect_gta5_pause(&frame, &ocr) {
                    let (a, b) = (screens::reread_pause_menu(&frame, &ocr, &old), pages.reread(screens::PAUSE_ID, &frame, &ocr, &old));
                    let (a, b) = (a.map(|l| serde_json::to_value(l).unwrap()), b.map(|l| serde_json::to_value(l).unwrap()));
                    if a == b {
                        println!("same      {name} pause re-read");
                    } else {
                        differ += 1;
                        println!("DIFFERENT {name} pause re-read: rust {} | script {}", a.map(|v| v.to_string().chars().take(300).collect::<String>()).unwrap_or_default(), b.map(|v| v.to_string().chars().take(300).collect::<String>()).unwrap_or_default());
                    }
                }
                for (id, rust_fn) in checks.iter() {
                    let t = Instant::now();
                    let rust = rust_fn();
                    ms_rust += t.elapsed().as_millis();
                    let t = Instant::now();
                    let js = pages.present(id, &frame, &ocr).and_then(|k| pages.read(id, &frame, &ocr, &k));
                    ms_js += t.elapsed().as_millis();
                    let (a, b) = (rust.as_ref().map(|l| serde_json::to_value(l).unwrap()), js.as_ref().map(|l| serde_json::to_value(l).unwrap()));
                    if a.is_none() && b.is_none() {
                        continue;
                    }
                    found += 1;
                    if a == b {
                        println!("same      {name} {id}: {:?}", rust.as_ref().map(|l| &l.reason_en));
                    } else {
                        differ += 1;
                        println!("DIFFERENT {name} {id}");
                        match (&a, &b) {
                            (Some(serde_json::Value::Object(x)), Some(serde_json::Value::Object(y))) => {
                                for (k, v) in x {
                                    // buttons: say which part of which button differs
                                    if let (true, Some(serde_json::Value::Array(ra)), Some(serde_json::Value::Array(sa))) = (k == "buttons", Some(v), y.get(k)) {
                                        for (i, (rb, sb)) in ra.iter().zip(sa).enumerate() {
                                            for (bk, bv) in rb.as_object().into_iter().flatten() {
                                                if sb.get(bk) != Some(bv) {
                                                    let s = |v: &serde_json::Value| { let t = v.to_string(); if t.len() > 120 { format!("{}… ({} chars)", &t[..120], t.len()) } else { t } };
                                                    println!("    buttons[{i}].{bk}: rust {} | script {}", s(bv), sb.get(bk).map(s).unwrap_or_default());
                                                }
                                            }
                                        }
                                        if ra.len() != sa.len() {
                                            println!("    buttons: rust {} | script {}", ra.len(), sa.len());
                                        }
                                        continue;
                                    }
                                    if let (true, Some(serde_json::Value::Array(ra)), Some(serde_json::Value::Array(sa))) = (k == "menu" || k == "labels", Some(v), y.get(k)) {
                                        for (i, (rb, sb)) in ra.iter().zip(sa).enumerate() {
                                            if rb != sb {
                                                println!("    {k}[{i}]: rust {rb} | script {sb}");
                                            }
                                        }
                                        if ra.len() != sa.len() {
                                            println!("    {k}: rust {} | script {}", ra.len(), sa.len());
                                        }
                                        continue;
                                    }
                                    if y.get(k) != Some(v) {
                                        let short = |v: &serde_json::Value| { let s = v.to_string(); if s.len() > 300 { format!("{}…", &s[..300]) } else { s } };
                                        println!("    {k}: rust {} | script {}", short(v), y.get(k).map(short).unwrap_or_default());
                                    }
                                }
                            }
                            _ => println!("    rust found: {}, script found: {} (script present() gave {:?})", a.is_some(), b.is_some(), pages.present(id, &frame, &ocr)),
                        }
                    }
                }
            }
            println!("{n} frames, {found} pages found, {differ} different; time: rust {ms_rust} ms, script {ms_js} ms");
            Ok(())
        };
        if let Err(e) = run() {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
        return;
    }

    // `--test-screen shot.png`: run the GTA V screen detector on a screenshot and print the layout.
    if let Some(path) = args.iter().position(|a| a == "--test-screen").and_then(|i| args.get(i + 1)) {
        let run = || -> anyhow::Result<()> {
            let img = image::open(path)?.to_rgba8();
            let (w, h) = img.dimensions();
            let frame = capture::FrameData { w, h, rgba: img.into_raw(), seq: 1 };
            Ocr::init_thread();
            let ocr = Ocr::new()?;
            let t = Instant::now();
            let tp = Instant::now();
            let present = screens::title_present(&frame);
            println!("title present: {present} ({} ms)", tp.elapsed().as_millis());
            println!("screen dark: {}", screens::is_dark(&frame));
            println!("pause menu present: {}", screens::pause_present(&frame));
            println!("centre caption: {:?}", screens::find_caption(&frame));
            let found = screens::detect_gta5_mission_failed(&frame, &ocr)
                .or_else(|| screens::detect_gta5_pause(&frame, &ocr))
                .or_else(|| screens::detect_gta5_alert(&frame, &ocr))
                .or_else(|| screens::detect_gta5_caption(&frame, &ocr));
            println!("detect: {} ms", t.elapsed().as_millis());
            let Some(mut l) = found else {
                println!("no Mission Failed screen");
                return Ok(());
            };
            // menu text: the built-in Arabic (the app adds the translator's for the rest)
            for m in l.menu.iter_mut() {
                m.ar = screens::menu_word(&m.en).map(|a| if m.en.trim().starts_with('<') { format!("< {a} >") } else { a.to_string() }).unwrap_or_default();
            }
            // icon images shortened for reading (VI_TRANSLATOR_FULL=1 keeps them, e.g. to render the page)
            if std::env::var_os("VI_TRANSLATOR_FULL").is_none() {
                for b in &mut l.buttons {
                    if let Some(i) = &b.icon {
                        b.icon = Some(format!("<png {} bytes>", i.len()));
                    }
                    b.more_icons = b.more_icons.iter().map(|i| format!("<png {} bytes>", i.len())).collect();
                }
            }
            println!("{}", serde_json::to_string_pretty(&l)?);
            let ar = match screens::reason_rule(&l.reason_en) {
                Some(ar) => format!("{ar}   (built-in rule)"),
                None => {
                    let cfg = Config::load(&config_path());
                    let mut tr = Translator::new(model_dir());
                    let segs = tr.translate(&cfg, &l.reason_en, &[], &[], &|_| {})?;
                    format!("{}   ({})", segs.iter().map(|s| s.0.as_str()).collect::<String>(), cfg.engine)
                }
            };
            println!("reason: {} -> {ar}", l.reason_en);
            Ok(())
        };
        if let Err(e) = run() {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--test-image") {
        let area = args.iter().position(|a| a == "--area").and_then(|j| args.get(j + 1)).map(String::as_str).unwrap_or("subtitles");
        if let Err(e) = test_image(&args[i + 1], area) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    let mut loaded = Config::load(&config_path());
    loaded.debug_save_frames = false; // recording always starts off
    if loaded.install_id.is_empty() {
        // a random id for this install (no account): lets the server count distinct players
        loaded.install_id = uuid::Uuid::new_v4().to_string();
        let _ = loaded.save(&config_path());
    }
    let cfg = Arc::new(Mutex::new(loaded));
    let saver = Arc::new(debug_frames::SaverState::default());
    let memory = Arc::new(Mutex::new(memory::Memory::load(db_dir(), &cfg.lock().unwrap().target_lang)));
    let shared = Arc::new(Shared::default());
    let update: Arc<Mutex<Option<updater::Update>>> = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .manage(AppState { cfg: cfg.clone(), shared: shared.clone(), saver: saver.clone(), memory: memory.clone(), update: update.clone() })
        .invoke_handler(tauri::generate_handler![app_version, update_status, install_update, lan_info, open_lan_page, list_langs, list_sources, get_config, save_config, page_options, list_windows, list_fonts, get_screen, frame_saver_status, open_frames_folder, memory_status, memory_clear, memory_reload, memory_open_folder, reset_text_sizes])
        .setup(move |app| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_title(&format!("VI Translator {}", env!("CARGO_PKG_VERSION")));
            }
            let handle = app.handle().clone();
            let latest = Arc::new(Mutex::new(HashMap::new()));
            let (tx, rx) = mpsc::channel();
            let (ptx, prx) = mpsc::channel::<PMsg>();
            {
                let h = handle.clone();
                let s = shared.clone();
                std::thread::spawn(move || present_loop(h, prx, s));
            }
            {
                // the Arabic-only page for an OBS Browser Source
                let (h, c, s) = (handle.clone(), cfg.clone(), shared.clone());
                std::thread::spawn(move || obs_page::run(h, c, s));
            }
            {
                let (h, c, s) = (app.handle().clone(), cfg.clone(), shared.clone());
                std::thread::spawn(move || lan::run(h, c, s));
            }
            {
                let (h, c, u) = (app.handle().clone(), cfg.clone(), update.clone());
                std::thread::spawn(move || updater::run(h, c, u));
            }

            {
                let (s, c) = (shared.clone(), cfg.clone());
                std::thread::spawn(move || capture::run_manager(s, c));
            }
            {
                let (h, c, s) = (handle.clone(), cfg.clone(), shared.clone());
                std::thread::spawn(move || preview_loop(h, c, s));
            }
            {
                let (h, c, s, l) = (handle.clone(), cfg.clone(), shared.clone(), latest.clone());
                let (p, m) = (ptx.clone(), memory.clone());
                std::thread::spawn(move || ocr_loop(h, c, s, l, tx, p, m));
            }
            {
                let (h, c, s, l) = (handle.clone(), cfg.clone(), shared.clone(), latest.clone());
                let (p, m) = (ptx.clone(), memory.clone());
                std::thread::spawn(move || translate_loop(h, c, s, l, rx, p, m));
            }
            {
                let (h, c, s) = (handle.clone(), cfg.clone(), shared.clone());
                std::thread::spawn(move || overlay::position_loop(h, c, s));
            }
            {
                let (h, c, s, sv) = (handle.clone(), cfg.clone(), shared.clone(), saver.clone());
                std::thread::spawn(move || debug_frames::run(h, c, s, sv, frames_dir()));
            }
            {
                let (h, c, m) = (app.handle().clone(), cfg.clone(), memory.clone());
                std::thread::spawn(move || sync::run(h, c, m, sync::Paths { data: data_dir(), db: db_dir() }));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window quits the app (overlays included).
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { .. } = event {
                    window.app_handle().exit(0);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running VI Translator");

}


/// "Press ⟦1⟧ to jump" -> "Press 🎮 to jump" (for showing English lines in the log).
fn strip_icon_tokens(s: &str) -> String {
    s.split_whitespace()
        .map(|t| if t.starts_with('⟦') && t.ends_with('⟧') { "🎮" } else { t })
        .collect::<Vec<_>>()
        .join(" ")
}


/// The fixed words of a profile page in a language other than Arabic: each from the translator
/// (via `cache`, filled by MENU_TEXT jobs; `ask` queues one). Until translated, tabs, buttons and
/// the name block keep the English, and no title is drawn. Returns true while any is missing.
fn localize_screen(l: &mut screens::ScreenLayout, cache: &HashMap<String, String>, ask: &mut dyn FnMut(&str)) -> bool {
    // "MAP", "MONDAY 05:00": translated as ordinary words ("Map"), not shouted letters
    let words = |en: &str| -> String {
        if en.chars().any(|c| c.is_lowercase()) {
            return en.to_string();
        }
        en.split(' ')
            .map(|w| {
                let mut cs = w.chars();
                cs.next().map(|f| f.to_uppercase().chain(cs.flat_map(char::to_lowercase)).collect()).unwrap_or_default()
            })
            .collect::<Vec<String>>()
            .join(" ")
    };
    let mut missing = false;
    let mut get = |en: &str, missing: &mut bool| -> Option<String> {
        let key = words(en);
        let hit = cache.get(&key).cloned();
        if hit.is_none() {
            ask(&key);
            *missing = true;
        }
        hit
    };
    if !l.title_ar.is_empty() {
        let title = if l.id == screens::ALERT_ID { "Alert" } else { "Mission Failed" };
        l.title_ar = get(title, &mut missing).unwrap_or_default();
    }
    for lb in l.labels.iter_mut() {
        lb.ar = get(&lb.en, &mut missing).unwrap_or_else(|| lb.en.clone());
    }
    for b in l.buttons.iter_mut() {
        b.ar = get(&b.en, &mut missing).unwrap_or_else(|| b.en.clone());
    }
    if let Some(p) = l.player.as_mut() {
        p.ar = p.en.iter().map(|e| get(e, &mut missing).unwrap_or_else(|| e.clone())).collect();
    }
    missing
}

/// Take the profile page down right away (the game's own page is gone).
fn hide_screen(app: &AppHandle, shared: &Shared, screen: &mut Option<screens::ScreenLayout>) {
    *screen = None;
    shared.screen_active.store(false, std::sync::atomic::Ordering::Relaxed);
    *shared.screen.lock().unwrap() = None;
    *shared.screen_reason.lock().unwrap() = serde_json::json!([]);
    let _ = app.emit("screen", None::<screens::ScreenLayout>);
}
