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
mod game_icons;
mod ocr;
mod overlay;
mod phrasebook;
mod preview_shot;
mod lan;
mod langs;
mod screens;
mod snapshot;
mod pagescript;
mod speaker;
mod sync;
mod translate;
mod tv;
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


/// Commands that change things or reach outside the app answer the main window only: an overlay
/// window shows text the game showed, and has no business asking for more.
fn main_only(w: &tauri::Webview) -> Result<(), String> {
    if w.label() == "main" { Ok(()) } else { Err("not allowed in this window".into()) }
}

/// Translation memory: on/off, size, hits/misses and the time saved.
#[tauri::command]
fn memory_status(state: tauri::State<AppState>) -> memory::Stats {
    state.memory.lock().unwrap().stats()
}

#[tauri::command]
fn memory_clear(webview: tauri::Webview, state: tauri::State<AppState>) -> Result<(), String> {
    main_only(&webview)?;
    state.memory.lock().unwrap().clear();
    Ok(())
}

#[derive(Serialize, Clone)]
struct TranslationEvent {
    area: String,
    en: String,
    /// The English with the game's button icons in place of its "⟦n⟧" marks, drawn like the
    /// translation (`en` has 🎮 there instead)
    en_segments: Segments,
    segments: Segments,
    stats: String,
    error: Option<String>,
    /// Height of the English text line, as a fraction of the frame height (0 = unknown). The
    /// overlay sizes the Arabic from it so both look the same size.
    line_h: f64,
    /// The English text block (words + icons) as [x, y, w, h] fractions of the frame, so the Arabic
    /// can line up with the game's own box ("below the game's text box" placement).
    text_box: Option<[f64; 4]>,
    /// The game's own box around that text (GTA's help box) as [x, y, w, h] fractions, when its edges
    /// were found: the Arabic box below it takes exactly its left and right edges.
    #[serde(skip_serializing_if = "Option::is_none")]
    game_box: Option<[f64; 4]>,
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
// that kind has the same size. Locked sizes are kept in text-sizes.json for the next sessions, with
// the picture's resolution: a picture of another resolution (another capture card or source, the
// card set to another mode) has its text at other sizes, so they're measured again. So are they
// when the player picks another subtitle size in the game (Small, Medium, Large): several lines in
// a row that are all clearly bigger, or all clearly smaller, than the locked size.

const SIZE_SAMPLES: usize = 5;
/// Lines in a row, all this much off the locked size on the same side, that lock a new size.
/// (A single line can be ~30% off by its letters alone: "we are" has no tall ones. Five in a row
/// aren't; GTA's subtitle sizes are ~20% apart.)
const SIZE_DRIFT_LINES: usize = 5;
const SIZE_DRIFT: f64 = 0.15;

#[derive(Serialize, Deserialize, Default)]
struct SizeLock {
    locked: HashMap<String, f64>,
    /// The picture's width and height the sizes were measured on (None: not known yet)
    #[serde(default)]
    res: Option<(u32, u32)>,
    #[serde(skip)]
    samples: HashMap<String, Vec<f64>>,
    /// Lines since the last one at the locked size, all off it on the same side
    #[serde(skip)]
    drift: HashMap<String, Vec<f64>>,
}

fn sizes_path() -> PathBuf {
    data_dir().join("text-sizes.json")
}

impl SizeLock {
    fn load() -> Self {
        let mut s: Self = std::fs::read_to_string(sizes_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        s.locked.retain(|_, v| Self::plausible(*v));
        s
    }

    /// A line's height as a share of the frame that text can have (a script's mistake, or NaN, isn't).
    fn plausible(v: f64) -> bool {
        v > 0.0 && v <= 0.5
    }

    /// The size to use for this kind of text, given this line's measured height (fractions of the frame).
    fn size(&mut self, kind: &str, measured: f64) -> f64 {
        if !Self::plausible(measured) {
            return self.locked.get(kind).copied().unwrap_or(0.0);
        }
        if let Some(&v) = self.locked.get(kind) {
            let off = measured / v - 1.0;
            let d = self.drift.entry(kind.into()).or_default();
            if off.abs() < SIZE_DRIFT || d.last().is_some_and(|&p| (p / v - 1.0).signum() != off.signum()) {
                d.clear();
            }
            if off.abs() >= SIZE_DRIFT {
                d.push(measured);
            }
            if d.len() < SIZE_DRIFT_LINES {
                return v;
            }
            // the game's subtitle size changed: lock the new one
            let mut sorted = std::mem::take(d);
            sorted.sort_by(|a, b| a.total_cmp(b));
            let median = sorted[sorted.len() / 2];
            self.locked.insert(kind.into(), median);
            self.store();
            return median;
        }
        let v = self.samples.entry(kind.into()).or_default();
        v.push(measured);
        let mut sorted = v.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let median = sorted[sorted.len() / 2];
        if sorted.len() >= SIZE_SAMPLES {
            self.locked.insert(kind.into(), median);
            self.store();
        }
        median
    }

    /// The picture now being read is `w`×`h`: sizes measured on a picture of another resolution are
    /// forgotten, and measured again on the next lines. True when they were.
    fn at_resolution(&mut self, w: u32, h: u32) -> bool {
        if w == 0 || h == 0 || self.res == Some((w, h)) {
            return false;
        }
        // (sizes saved before the resolution was kept: taken as measured on this picture)
        let other = self.res.is_some();
        if other {
            self.locked.clear();
            self.samples.clear();
            self.drift.clear();
        }
        self.res = Some((w, h));
        self.store();
        other
    }

    fn store(&self) {
        // (tests never touch the player's own file)
        if cfg!(test) {
            return;
        }
        let _ = std::fs::create_dir_all(data_dir());
        let _ = std::fs::write(sizes_path(), serde_json::to_string_pretty(self).unwrap_or_default());
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

/// A development build (`cargo tauri dev`, debug): the Show on tab shows the tools for developing
/// the app. Releases never do.
#[tauri::command]
fn dev_build() -> bool {
    cfg!(debug_assertions)
}

/// A newer version of the app, if one was found.
#[tauri::command]
fn update_status(state: tauri::State<AppState>) -> Option<updater::Update> {
    state.update.lock().unwrap().clone()
}

/// Download and check the newer version, put it in place and restart into it.
#[tauri::command]
async fn install_update(app: AppHandle, webview: tauri::Webview, state: tauri::State<'_, AppState>) -> Result<(), String> {
    main_only(&webview)?;
    let u = state.update.lock().unwrap().clone().ok_or("no update available")?;
    tauri::async_runtime::spawn_blocking(move || updater::install(&u)).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))?;
    app.restart()
}

/// The home-network page: on/off, its address and a QR code of it.
#[tauri::command]
fn lan_info(webview: tauri::Webview, state: tauri::State<AppState>) -> Result<lan::LanInfo, String> {
    main_only(&webview)?;
    let c = state.cfg.lock().unwrap().clone();
    Ok(lan::info(&c, &state.shared))
}

/// TVs on the home network waiting to be connected (the VI Translator app on the TV), and whether each is
/// connected already.
#[tauri::command]
async fn find_tvs(webview: tauri::Webview, state: tauri::State<'_, AppState>) -> Result<Vec<tv::FoundTv>, String> {
    main_only(&webview)?;
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || tv::find(&shared.tvs)).await.map_err(|e| e.to_string())
}

/// Connect the TV at `address` with the code it shows. Its name, or why not ("wrong_code",
/// "too_many", "no_answer", "not_sharing").
#[tauri::command]
async fn connect_tv(webview: tauri::Webview, state: tauri::State<'_, AppState>, address: String, code: String) -> Result<String, String> {
    main_only(&webview)?;
    let (c, shared) = (state.cfg.lock().unwrap().clone(), state.shared.clone());
    // the TV loads its page from the home-network sharing, which has to be on (the window turns it on first)
    let running = shared.lan_running.load(std::sync::atomic::Ordering::Relaxed);
    let ip = lan::lan_ip();
    let (true, true, Some(ip)) = (c.lan_page, running, ip) else { return Err(tv::PairError::NotSharing.code().into()) };
    let pc = format!("{ip}:{}", c.lan_port);
    tauri::async_runtime::spawn_blocking(move || tv::pair(&shared.tvs, &address, &code, &pc, &tv::pc_name()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.code().to_string())
}

/// The connected TVs (names only).
#[tauri::command]
fn list_tvs(webview: tauri::Webview, state: tauri::State<AppState>) -> Result<Vec<tv::TvInfo>, String> {
    main_only(&webview)?;
    Ok(state.shared.tvs.list())
}

/// Disconnect a TV: its key stops working, and the TV shows a new code.
#[tauri::command]
fn remove_tv(webview: tauri::Webview, state: tauri::State<AppState>, id: String) -> Result<(), String> {
    main_only(&webview)?;
    state.shared.tvs.remove_id(&id);
    Ok(())
}

/// Opens the home-network page in the default browser (only that address: the page passes none).
#[tauri::command]
fn open_lan_page(webview: tauri::Webview, state: tauri::State<AppState>) -> Result<(), String> {
    main_only(&webview)?;
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

/// The settings: whole for the main window; without secrets (translator keys, the OBS password, the
/// install id) for an overlay, which needs only areas, style and language.
#[tauri::command]
fn get_config(webview: tauri::Webview, state: tauri::State<AppState>) -> Config {
    let c = state.cfg.lock().unwrap();
    if webview.label() == "main" { c.clone() } else { c.redacted() }
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
fn save_config(app: AppHandle, webview: tauri::Webview, state: tauri::State<AppState>, config: Config) -> Result<(), String> {
    main_only(&webview)?;
    // (the profile in use decides the areas, style and pages; a built-in one can't be changed)
    let mut config = config;
    // One lock for the whole merge, write and event: a layout update or a game switch landing
    // between them can't be lost, nor the file and the event come out older than the settings.
    let mut now = state.cfg.lock().unwrap();
    // the built-in profiles are the app's (layout updates may have changed them meanwhile)
    config.builtins = now.builtins.clone();
    config.server_games = now.server_games.clone();
    config.games_version = now.games_version;
    config.layout_version = now.layout_version;
    config.layout_versions = now.layout_versions.clone();
    // (the defaults already moved stay moved: a window that left the field out can't undo it)
    config.defaults_version = config.defaults_version.max(now.defaults_version);
    // sent before the window heard of a game switch the app made by itself: the switch stays
    if config.profile_rev != now.profile_rev {
        config.profile = now.profile.clone();
        config.profile_rev = now.profile_rev;
    }
    // (a settings file that's there but couldn't be read stays as it is: see Config::load)
    config.unreadable = now.unreadable;
    config.normalize();
    *now = config.clone();
    // (the change is in use whether or not the file could be written: the overlays hear of it either way)
    let saved = config.save(&config_path()).map_err(|e| e.to_string());
    app.emit("config", &config.redacted()).map_err(|e| e.to_string())?;
    saved
}

#[tauri::command]
fn list_windows(webview: tauri::Webview) -> Result<Vec<capture::WindowInfo>, String> {
    main_only(&webview)?;
    Ok(capture::list_windows(APP_TITLE))
}

/// Everything that can be a source: windows, capture devices, and OBS's sources (if OBS answers).
#[tauri::command]
async fn list_sources(webview: tauri::Webview, state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    main_only(&webview)?;
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
fn open_frames_folder(webview: tauri::Webview, state: tauri::State<AppState>) -> Result<String, String> {
    main_only(&webview)?;
    let dir = state.saver.folder.lock().unwrap().clone().unwrap_or_else(frames_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    debug_frames::open_folder(&dir).map_err(|e| e.to_string())?;
    Ok(dir.display().to_string())
}

/// Debugging: save a picture of the preview (`area`: where it is in the page). The first one of a run makes a
/// new folder in Pictures\VI Translator\previews, the next ones go to the same folder. The file's path.
/// Next to it, "frame_NNN.png": the game's frame at the size the app captures and reads it (the preview's
/// picture is only as big as the preview on screen).
#[tauri::command]
async fn save_preview(webview: tauri::Webview, area: preview_shot::Area, state: tauri::State<'_, AppState>) -> Result<String, String> {
    main_only(&webview)?;
    let base = pictures_dir().map(|p| p.join(FOLDER).join("previews")).unwrap_or_else(|| data_dir().join("previews"));
    // (the frame first: the one the preview is showing now)
    let frame = state.shared.latest();
    let path = preview_shot::save(&webview, area, &base).await?;
    if let (Some(f), Some(name)) = (frame, path.file_name().and_then(|n| n.to_str())) {
        let full = path.with_file_name(name.replacen("preview_", "frame_", 1));
        debug_frames::save_png_fast(&f, &full).map_err(|e| format!("the full-size frame: {e:#}"))?;
    }
    Ok(path.display().to_string())
}

/// Where snapshots are kept: Pictures\VI Translator\snapshots.
fn snapshots_dir() -> PathBuf {
    pictures_dir().map(|p| p.join(FOLDER).join("snapshots")).unwrap_or_else(|| data_dir().join("snapshots"))
}

/// The translator snapshots use (its local model stays loaded between snapshots).
static SNAP_TR: Mutex<Option<Translator>> = Mutex::new(None);

/// A snapshot of the picture on screen now: translated with the snapshot service; with "Keep every
/// snapshot", its picture is saved (and Azure's translated picture; the app saves its own drawing of
/// the others, see `save_snapshot_render`).
fn snapshot_now(cfg: &Mutex<Config>, shared: &Shared, memory: &Mutex<memory::Memory>) -> anyhow::Result<snapshot::Snapshot> {
    let Some(_busy) = snapshot::Busy::take() else { anyhow::bail!("a snapshot is already being translated") };
    let Some(frame) = shared.latest() else { anyhow::bail!("no picture yet: pick a source first") };
    let private = shared.screen_private.lock().unwrap().clone();
    let c = cfg.lock().unwrap().clone();
    let mut slot = SNAP_TR.lock().unwrap();
    let tr = slot.get_or_insert_with(|| Translator::new(model_dir()));
    let lang = c.target_lang.clone();
    // (the reviewed translations only: the memory's own lines come from whatever service was in use, and
    // the snapshot has its own; looked at line by line, not held while a translator is asked)
    let lookup = |en: &str| -> Option<translate::Segments> {
        let m = memory.lock().unwrap();
        m.get(en, &[], &[]).filter(|(_, src)| src == "reviewed").map(|(segs, _)| translate::polite(&lang, segs))
    };
    let id = debug_frames::timestamp();
    let snap = snapshot::make(&frame, &private, snapshot::Tools { cfg: &c, tr, memory: &lookup }, id.clone())?;
    if c.snap_save {
        let dir = snapshots_dir();
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{id}_original.png")), snapshot::png_bytes(&snapshot::covered(&frame, &private))?)?;
        if let Some((bytes, ext)) = snap.translated_image.as_deref().and_then(snapshot::data_url_bytes) {
            std::fs::write(dir.join(format!("{id}_{}.{ext}", snap.lang)), bytes)?;
        }
    }
    Ok(snap)
}

/// Translate the whole picture now (the Snapshot tab's button and Play's).
#[tauri::command]
async fn take_snapshot(webview: tauri::Webview, state: tauri::State<'_, AppState>) -> Result<snapshot::Snapshot, String> {
    main_only(&webview)?;
    let (cfg, shared, memory) = (state.cfg.clone(), state.shared.clone(), state.memory.clone());
    tauri::async_runtime::spawn_blocking(move || snapshot_now(&cfg, &shared, &memory).map_err(|e| format!("{e:#}")))
        .await
        .map_err(|e| e.to_string())?
}

/// The app's drawing of a snapshot (its picture with the translation over it), saved next to its picture.
#[tauri::command]
fn save_snapshot_render(webview: tauri::Webview, id: String, lang: String, data: String) -> Result<String, String> {
    main_only(&webview)?;
    // (names the app made: a timestamp and a language code)
    let ok = |t: &str, max: usize| !t.is_empty() && t.len() <= max && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok(&id, 40) || !ok(&lang, 10) {
        return Err("bad name".into());
    }
    let (bytes, ext) = snapshot::data_url_bytes(&data).ok_or("not a picture")?;
    let dir = snapshots_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{id}_{lang}.{ext}"));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn open_snapshots_folder(webview: tauri::Webview) -> Result<String, String> {
    main_only(&webview)?;
    let dir = snapshots_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    debug_frames::open_folder(&dir).map_err(|e| e.to_string())?;
    Ok(dir.display().to_string())
}

/// Debugging: open the folder the preview pictures go to (this run's, or the one for all of them).
#[tauri::command]
fn open_previews_folder(webview: tauri::Webview) -> Result<String, String> {
    main_only(&webview)?;
    let dir = preview_shot::FOLDER.lock().unwrap().clone().unwrap_or_else(|| pictures_dir().map(|p| p.join(FOLDER).join("previews")).unwrap_or_else(|| data_dir().join("previews")));
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
    /// The game's box around it (see TranslationEvent::game_box)
    game_box: Option<[f64; 4]>,
    /// Two speakers' lines (see ocr::speaker_lines): each part with its brightness; empty = one line
    parts: Vec<(String, f64)>,
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
    /// The line on screen was sent without the game's coloured words: read on its first frame, while
    /// GTA VI's instructions still fade in with a white glow. Its colours are looked for again until then.
    colors_until: Option<Instant>,
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
    /// A full-screen page took over (or the language changed): hide every area now, and drop the
    /// translations of every line read so far (seq up to `upto`) that are still on their way.
    ClearAll { upto: u64 },
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
    /// translations of lines up to this seq come too late (a page took over since): dropped
    cleared_upto: u64,
}

/// How long the end of an English line is remembered (a translation can come minutes late: the
/// first one, while the offline model downloads).
const ENDED_KEEP: Duration = Duration::from_secs(3600);

impl Presenter {
    /// A translation is ready; false (dropped) when its line was cleared by a page meanwhile.
    fn ready(&mut self, area: &str, seq: u64, en_start: Instant) -> bool {
        if seq <= self.cleared_upto {
            return false;
        }
        self.pending.insert(area.into(), (seq, en_start)); // only the newest ready line waits
        true
    }
    fn ended(&mut self, area: &str, seq: u64, at: Instant) {
        self.ended.insert((area.into(), seq), at);
    }
    fn clear_all(&mut self, upto: u64) -> Vec<Act> {
        self.cleared_upto = self.cleared_upto.max(upto);
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
                Some(s) => {
                    // (its end was never told — the record lost, or a path that forgot it: a newer line
                    // of this area waiting means its English had gone by the time that one started)
                    let end = self.ended.get(&(area.clone(), s.seq)).copied().or_else(|| self.pending.get(&area).filter(|p| p.0 > s.seq).map(|p| p.1));
                    end.is_some_and(|end| {
                        let lag = s.shown_at.saturating_duration_since(s.en_start).min(MAX_LAG);
                        now >= end.max(s.shown_at) + lag.min(end.saturating_duration_since(s.en_start))
                    })
                }
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
        self.ended.retain(|_, at| now.saturating_duration_since(*at) < ENDED_KEEP);
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
                    if p.ready(&ev.area, seq, en_start) {
                        events.retain(|(a, _), _| *a != ev.area); // keep only the newest waiting line
                        events.insert((ev.area.clone(), seq), ev);
                    }
                }
                PMsg::Ended { area, seq, at } => p.ended(&area, seq, at),
                PMsg::ClearAll { upto } => {
                    events.clear();
                    acts.extend(p.clear_all(upto));
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
                        lan::push(&shared, &area, &ev.en, &ev.en_segments, &ev.segments, ev.at);
                        let _ = app.emit("translation", ev);
                    }
                }
                Act::Hide { area } => {
                    shared.on_screen.lock().unwrap().remove(&area);
                    shared.shown.lock().unwrap().remove(&area);
                    let _ = app.emit(
                        "translation",
                        TranslationEvent { area, en: String::new(), en_segments: Vec::new(), segments: Vec::new(), stats: String::new(), error: None, line_h: 0.0, text_box: None, game_box: None, cached: false, at: 0 },
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
        assert_eq!(p.clear_all(2), vec![Act::Hide { area: A.into() }]);
        assert_eq!(p.tick(t + ms(5000)), vec![]); // nothing left waiting
    }

    #[test]
    fn a_translation_of_a_line_cleared_by_a_page_is_dropped() {
        let t = Instant::now();
        let mut p = Presenter::default();
        p.clear_all(2); // a page took over while line 2 was being translated
        assert!(!p.ready(A, 2, t));
        assert_eq!(p.tick(t + ms(100)), vec![]);
        // a line read after the page
        assert!(p.ready(A, 3, t + ms(200)));
        assert_eq!(p.tick(t + ms(300)), vec![Act::Show { area: A.into(), seq: 3 }]);
    }

    #[test]
    fn a_line_whose_end_was_never_told_gives_way_to_the_next() {
        let t = Instant::now();
        let mut p = Presenter::default();
        p.ready(A, 1, t);
        assert_eq!(p.tick(t + ms(500)), vec![Act::Show { area: A.into(), seq: 1 }]);
        // no Ended for 1; line 2's English started at 60 s
        p.ready(A, 2, t + ms(60_000));
        assert_eq!(p.tick(t + ms(60_000)), vec![]); // 1 still has its own time (its lag)
        assert_eq!(p.tick(t + ms(60_600)), vec![Act::Show { area: A.into(), seq: 2 }]);
    }

    #[test]
    fn a_very_late_translation_still_knows_when_its_english_ended() {
        let t = Instant::now();
        let mut p = Presenter::default();
        p.ended(A, 1, t + ms(4_000));
        p.ready(A, 1, t);
        assert_eq!(p.tick(t + ms(200_000)), vec![Act::Show { area: A.into(), seq: 1 }]); // 200 s late: the end is still known
        assert_eq!(p.tick(t + ms(203_100)), vec![Act::Hide { area: A.into() }]); // gone at most MAX_LAG later
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
    // a load that left the game without pages (a script that failed to load) is tried again a few
    // times, spaced out: how many so far, and when the last was
    let mut pages_tries: (u32, Instant) = (0, Instant::now());
    // when a page followed like a menu was last read with OCR (see `settling` below)
    let mut menu_read_at = Instant::now() - Duration::from_secs(10);
    // no picture since (the window closed, the card unplugged, OBS failing)
    let mut no_frame_since: Option<Instant> = None;
    // pause-menu text the translator couldn't translate is asked again now and then
    let mut menu_retry_at = Instant::now();
    // the page on screen last time round (to tell its script when it goes)
    let mut ended_check: Option<String> = None;
    // Another game on screen (while sharing lines): the other games' scripts, and when they last
    // looked at the picture.
    let mut other_pages: HashMap<String, (u64, Option<pagescript::Pages>)> = HashMap::new();
    let mut recognized_at = Instant::now();
    // For sharing lines: the game one of whose pages was on screen this session (that game is
    // really being played), and until when another game the app knows was seen on screen.
    let mut confirmed_game = String::new();
    let mut other_game_until: Option<Instant> = None;
    loop {
        let c = cfg.lock().unwrap().clone();
        let t0 = Instant::now();
        // (the player's, or the game's own: see Config::timing)
        let (interval_ms, stable_ms, clear_after_ms) = c.timing();
        if c.target_lang != last_lang {
            // another language: everything shown and cached is in the old one
            last_lang = c.target_lang.clone();
            let l = langs::get(&c.target_lang);
            *shared.live_lang.lock().unwrap() = serde_json::json!({ "code": l.code, "rtl": l.rtl });
            memory.lock().unwrap().set_lang(&c.target_lang);
            menu_asked.clear();
            menu_states.clear();
            {
                let mut mc = shared.menu_cache.lock().unwrap();
                mc.lang = c.target_lang.clone();
                mc.map.clear();
            }
            shared.menu_failed.lock().unwrap().clear();
            if screen.is_some() {
                hide_screen(&app, &shared, &latest, &mut screen);
            }
            for st in states.values_mut() {
                st.current.clear();
                st.candidate.clear();
            }
            let _ = present.send(PMsg::ClearAll { upto: seq_counter });
        }
        // pause-menu text the translator failed on: asked again when it's read again (every 30 s at most)
        if menu_retry_at.elapsed() >= Duration::from_secs(30) {
            menu_retry_at = Instant::now();
            for en in std::mem::take(&mut *shared.menu_failed.lock().unwrap()) {
                menu_asked.remove(&en);
            }
        }
        screens::use_words_for(&c.game(), &c.target_lang);
        let frame_now = shared.latest();
        if frame_now.is_none() {
            // No picture: once that has lasted as long as an English line may blink out, what was up
            // goes, as if the game took it away (else a page and the last line stay on the OBS page).
            let since = *no_frame_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(clear_after_ms) {
                if screen.is_some() {
                    hide_screen(&app, &shared, &latest, &mut screen);
                }
                if let Some((r, s0)) = reason_on.take() {
                    memory.lock().unwrap().log_line(SCREEN_REASON, &r, unix_ms(s0), unix_ms(since));
                }
                for (id, st) in states.iter_mut() {
                    if st.current_seq != 0 {
                        let at = st.last_seen.unwrap_or(since);
                        let _ = present.send(PMsg::Ended { area: id.clone(), seq: st.current_seq, at });
                        if let Some(s0) = st.current_start {
                            memory.lock().unwrap().log_line(id, &st.current_en, unix_ms(s0), unix_ms(at));
                        }
                    }
                    *st = AreaState::default();
                }
            }
        } else {
            no_frame_since = None;
        }
        if let Some(frame) = frame_now {
            // another resolution: the text sizes are measured again
            sizes.lock().unwrap().at_resolution(frame.w, frame.h);
            // ---- game pages (the game's pages script finds and reads them; pagescript.rs) ----
            // Appear: each page's `present` (cheap) every frame, then its `read`.
            // Stay: `same` every frame — the translation stays exactly as long as the page does — with
            //       the page's hold rules, or followed like a menu (`hold.follow`).
            let now_for = (c.game(), pagescript::GENERATION.load(std::sync::atomic::Ordering::Relaxed));
            let changed = now_for != pages_for;
            let retry = pages.is_none()
                && pages_tries.0 < 3
                && pages_tries.1.elapsed() >= Duration::from_secs(30)
                && pagescript::files(&data_dir(), &now_for.0).0.exists();
            if changed || retry {
                pages_tries = (if changed { 0 } else { pages_tries.0 + 1 }, Instant::now());
                // (the states a menu's tabs were seen in belong to the script that read them)
                menu_states.clear();
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
            let screens_on = !script_pages.is_empty();
            let mut found: Option<screens::ScreenLayout> = None;
            // A page on screen whose switch was just turned off (Game pages) goes now, not when the game
            // closes it.
            if let Some(old) = &screen {
                let on = pages.as_ref().and_then(|p| p.info(&old.id).map(|i| p.switch_on(&c.profile_opts, &i.option)));
                // (a page its script no longer has goes too)
                if on != Some(true) {
                    hide_screen(&app, &shared, &latest, &mut screen);
                }
            }
            if screens_on {
                match &screen {
                    None => {
                        if let Some(p) = &pages {
                            for id in &script_pages {
                                if let Some(k) = p.present(id, &frame, &ocr) {
                                    found = p.read(id, &frame, &ocr, &k);
                                    if found.is_some() {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    // A script's page is up. Without hold rules (captions): still the same? Else read it
                    // again, or it's gone. With them (Mission Failed): kept while its title blinks out
                    // (up to hold_ms), ended once the screen is dark, read again every reread_ms.
                    Some(old) if screens::menu_rects(old).is_none() && pages.as_ref().is_some_and(|p| p.has(&old.id)) => {
                        let p = pages.as_ref().expect("checked");
                        let id = old.id.clone();
                        let info = p.info(&id).cloned().unwrap_or_default();
                        let still = p.same(&id, &frame, &ocr, old) == Some(true);
                        if info.hold_ms == 0 && info.reread_ms == 0 {
                            if !still {
                                match p.present(&id, &frame, &ocr).and_then(|k| p.read(&id, &frame, &ocr, &k)) {
                                    Some(l) => found = Some(l),
                                    None => hide_screen(&app, &shared, &latest, &mut screen),
                                }
                            }
                        } else {
                            if still {
                                title_missing_since = None;
                            }
                            let missing = title_missing_since.get_or_insert_with(Instant::now).elapsed();
                            if !still && ((info.end_when_dark && screens::is_dark(&frame)) || missing > Duration::from_millis(info.hold_ms)) {
                                hide_screen(&app, &shared, &latest, &mut screen);
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
                    // A page followed like a menu (`hold.follow`): gone the moment the script's `same`
                    // says so; its boxes watched every frame, read again when they change.
                    Some(old) if screens::menu_rects(old).is_some() && pages.as_ref().is_some_and(|p| p.has(&old.id)) => {
                        let id = old.id.clone();
                        let (content_rect, buttons_rect) = screens::menu_rects(old).expect("checked");
                        let p = pages.as_ref().expect("checked");
                        let pause_present = |f: &capture::FrameData| p.same(&id, f, &ocr, old) == Some(true);
                        let reread = |f: &capture::FrameData| match p.can(&id, "reread") {
                            true => p.reread(&id, f, &ocr, old),
                            false => p.present(&id, f, &ocr).and_then(|k| p.read(&id, f, &ocr, &k)),
                        };
                        let full_read = |f: &capture::FrameData| p.present(&id, f, &ocr).and_then(|k| p.read(&id, f, &ocr, &k));
                        // Every frame: a menu line whose English changed or went away loses its Arabic
                        // at once, and the menu is read again straight away.
                        let gone = old.menu.iter().any(|m| !screens::region_same(&frame, &m.rect, &m.sig))
                            || old.labels.iter().any(|l| !screens::region_same(&frame, &l.rect, &l.sig))
                            || !screens::region_same(&frame, &buttons_rect, &old.buttons_sig);
                        // Read again only when something changed (or every 3 s, to be safe): re-reading a
                        // still menu only makes OCR's boxes shift a few pixels.
                        let changed = screens::content_changed_in(&frame, &content_rect, &old.content_sig);
                        let due = screen_checked.elapsed() >= Duration::from_secs(3);
                        // Only the content area's picture moved, not a translated box, tab or button row (an
                        // animated picture, a live map, the game behind the menu): read again at most once
                        // a second, not every frame.
                        let settling = changed && !gone && !due && menu_read_at.elapsed() < Duration::from_secs(1);
                        // a state of this tab seen before, matching what's on screen now
                        let tab = format!("{id}/{}", screens::selected_tab_name(old).unwrap_or(""));
                        let known = (gone || changed)
                            .then(|| menu_states.iter().find(|(t, sig, _)| *t == tab && !screens::content_changed_in(&frame, &content_rect, sig)))
                            .flatten()
                            .filter(|(_, sig, _)| *sig != old.content_sig);
                        if !pause_present(&frame) {
                            hide_screen(&app, &shared, &latest, &mut screen);
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
                            menu_read_at = Instant::now();
                            match reread(&frame) {
                                Some(l) => found = Some(l),
                                None => {
                                    // The page wasn't ready (a list's selection still sliding): keep its list rows
                                    // (the same words, a highlight moving past them: not dropped to English for a
                                    // moment), but not other text that changed (a panel showing the next topic);
                                    // and try again shortly, not in 3 s.
                                    let mut l = old.clone();
                                    l.menu.retain(|m| m.item || screens::region_same(&frame, &m.rect, &m.sig));
                                    found = Some(l);
                                    screen_checked = Instant::now() - Duration::from_millis(2800);
                                }
                            }
                        } else if gone && !due {
                            // drop what changed now; read the menu again on the next pass (`due`)
                            let mut l = old.clone();
                            l.menu.retain(|m| screens::region_same(&frame, &m.rect, &m.sig));
                            screen_checked = Instant::now() - Duration::from_secs(10);
                            found = Some(l);
                        } else if settling || (!changed && !due && !gone) {
                            // nothing new on screen (or only the picture behind it moving): just add
                            // translations that arrived meanwhile
                            if old.menu.iter().any(|m| m.ar.is_empty()) || screen_missing {
                                found = Some(old.clone());
                            }
                        } else {
                            screen_checked = Instant::now();
                            menu_read_at = Instant::now();
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
                    // a page no script has any more (the script changed)
                    Some(_) => hide_screen(&app, &shared, &latest, &mut screen),
                }
            } else if screen.is_some() {
                hide_screen(&app, &shared, &latest, &mut screen);
            }
            // Another game on screen? While sharing lines, every few seconds while no page is up, the
            // other games' scripts look at the picture: another game on screen holds its lines back.
            if c.share_lines && screen.is_none() && found.is_none() && recognized_at.elapsed() >= Duration::from_secs(4) {
                recognized_at = Instant::now();
                let generation = pagescript::GENERATION.load(std::sync::atomic::Ordering::Relaxed);
                let here = c.game();
                let mut seen: Option<String> = None;
                for g in c.builtins.iter().map(|p| p.id.clone()).filter(|g| *g != here) {
                    let entry = other_pages.entry(g.clone()).or_insert((u64::MAX, None));
                    if entry.0 != generation {
                        *entry = (generation, pagescript::Pages::for_game(&g, &data_dir()));
                    }
                    if entry.1.as_ref().is_some_and(|p| p.recognize(&frame, &ocr)) {
                        seen = Some(g);
                        break;
                    }
                }
                if seen.is_some() {
                    other_game_until = Some(Instant::now() + Duration::from_secs(90));
                }
            }
            // What lines logged now are, for sharing (see memory::set_share).
            if screen.is_some() || found.is_some() {
                confirmed_game = c.game();
            }
            memory::set_share(&c.game(), confirmed_game == c.game(), other_game_until.is_some_and(|t| Instant::now() < t), c.share_lines);
            // A script's page went: tell it (`ended`), so it can forget what it kept about it.
            {
                let now_id = screen.as_ref().map(|s| s.id.clone()).or_else(|| found.as_ref().map(|l| l.id.clone()));
                if ended_check != now_id {
                    if let (Some(p), Some(gone)) = (&pages, &ended_check) {
                        p.ended(gone, &frame);
                    }
                    ended_check = now_id;
                }
            }
            if let Some(layout) = found.as_mut().filter(|l| !l.menu.is_empty()) {
                // The page's text boxes: the layout's words, then this session's translations, then the
                // translation memory; anything else is sent to the translator and shows on a later read.
                // Other players' names (a friends list): the script marks them private — never
                // translated, sent or kept.
                *shared.screen_private.lock().unwrap() = layout.menu.iter().filter(|m| m.private).map(|m| [m.rect.x, m.rect.y, m.rect.w, m.rect.h]).collect();
                layout.menu.retain(|m| !m.private);
                for m in layout.menu.iter_mut() {
                    if m.fixed {
                        if m.ar.is_empty() {
                            m.ar = m.en.clone();
                        }
                        continue;
                    }
                    // (OCR may spell a line slightly differently from one read to the next; looked at only
                    // when the layout's words don't have it)
                    let cached = || {
                        let cache = shared.menu_cache.lock().unwrap();
                        cache.map.get(&m.en).cloned().or_else(|| cache.map.iter().find(|(k, _)| ocr::similar(k, &m.en)).map(|(_, v)| v.clone()))
                    };
                    let remembered = || {
                        memory
                            .lock()
                            .unwrap()
                            .get(&m.en, &[], &[])
                            .map(|(segs, _)| segs.iter().map(|s| s.0.as_str()).collect::<String>())
                            .filter(|ar| translate::keeps_terms(&m.en, ar))
                    };
                    // a list item is one of the menu's known words, even when OCR garbled it (faint, greyed-out items)
                    // (also headings and short panel lines: the faint "Join Friends" title)
                    if (m.item || m.heading || m.en.split_whitespace().count() <= 3) && !m.en.trim().starts_with('<') {
                        if let Some((en, ar)) = screens::menu_item_word(&m.en) {
                            m.en = en.to_string();
                            m.ar = ar.to_string();
                            continue;
                        }
                    }
                    let built_in = screens::menu_word(&m.en).map(String::from);
                    match built_in.or_else(cached).or_else(remembered) {
                        // a switcher "< Michael >" keeps its arrows
                        Some(ar) if m.en.trim().starts_with('<') && m.en.trim().ends_with('>') => m.ar = format!("< {ar} >"),
                        Some(ar) => m.ar = ar,
                        None => {
                            if !menu_asked.contains(&m.en) && !menu_asked.iter().any(|a| ocr::similar(a, &m.en)) && menu_asked.insert(m.en.clone()) {
                                seq_counter += 1;
                                let _ = jobs.send(Job { area: MENU_TEXT.into(), seq: seq_counter, en: m.en.clone(), phrases: Vec::new(), icons: Vec::new(), line_h: 0.0, en_start: Instant::now(), text_box: None, game_box: None, parts: Vec::new() });
                            }
                        }
                    }
                }
                // Stability from one read to the next, on the same tab: a line read a bit differently
                // keeps its Arabic.
                if let Some(old) = screen.as_ref().filter(|o| o.id == layout.id && screens::menu_rects(o).is_some() && screens::selected_tab_name(o) == screens::selected_tab_name(layout)) {
                    // (only OCR's jitter of a few pixels: a line that moved — a list scrolled, read once
                    // mid-scroll — takes its new place, or its Arabic would stay off by part of a row)
                    let near = |a: &screens::Rect, b: &screens::Rect| (a.y - b.y).abs() < a.h.min(b.h).max(0.001) * 0.25 && (a.x - b.x).abs() < 0.01;
                    // The same line found again (same place, similar words): keep its box, colours and
                    // Arabic exactly, so nothing shifts or changes wording between reads.
                    for m in layout.menu.iter_mut() {
                        // (the selected row's box is the game's white bar, measured exactly: it takes this read's
                        // place, or a bar that moved a few pixels — the selection arriving, a list settling after a
                        // scroll — would show the old box's edge beside it)
                        let selected = m.item && m.bg.eq_ignore_ascii_case("#ffffff");
                        // the picture there hasn't changed: it's the same text, whatever OCR made of it this time
                        // (unless this read found more of the game's icons for it: a first read while the page was
                        // still appearing can miss them, and would keep the line without them)
                        // (and the words largely the same: two paragraphs in the same place on the same background
                        // can look alike to that small picture, and the last topic's would stay)
                        if let Some(o) = old.menu.iter().find(|o| near(&o.rect, &m.rect) && !o.ar.is_empty() && o.icons.len() >= m.icons.len() && screens::same_words(&o.en, &m.en) && screens::region_same(&frame, &o.rect, &o.sig)) {
                            let rect = m.rect.clone();
                            *m = o.clone();
                            if selected {
                                m.rect = rect;
                            }
                            continue;
                        }
                        if let Some(o) = old.menu.iter().find(|o| near(&o.rect, &m.rect) && ocr::similar(&o.en, &m.en)) {
                            // same place and words as before; colours and picture from this read (the
                            // selection's white highlight may have moved onto it or off it)
                            // (its Arabic and the icons it places go together; a read with more icons wins)
                            let (ar, icons) = if o.ar.is_empty() || m.icons.len() > o.icons.len() { (m.ar.clone(), m.icons.clone()) } else { (o.ar.clone(), o.icons.clone()) };
                            let rect = if selected || o.bg != m.bg { m.rect.clone() } else { o.rect.clone() };
                            *m = screens::MenuText { ar, icons, rect, right: o.right, ..m.clone() };
                        }
                    }
                }
                // Remember this state of the tab (and add lines an earlier read of it found but
                // this one missed, if they're still on screen). Only a layout that matches the
                // picture on screen — not one just trimmed while the page changes under it.
                if let Some((content_rect, _)) = screens::menu_rects(layout).filter(|(r, _)| !screens::content_changed_in(&frame, r, &layout.content_sig)) {
                    let tab = format!("{}/{}", layout.id, screens::selected_tab_name(layout).unwrap_or(""));
                    let near = |a: &screens::Rect, b: &screens::Rect| (a.y - b.y).abs() < a.h.min(b.h).max(0.001) * 0.6 && (a.x - b.x).abs() < 0.03;
                    if let Some(e) = menu_states.iter_mut().find(|(t, sig, _)| *t == tab && !screens::content_changed_in(&frame, &content_rect, sig)) {
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
            // The page's fixed words (title, tabs, buttons, the name block): the ones the script or the
            // layout's words didn't translate come from the translator, like the text boxes, and show
            // once translated.
            if let Some(layout) = found.as_mut() {
                // (the lock, not a copy of the cache: the translator only waits a moment to add to it)
                let cache = shared.menu_cache.lock().unwrap();
                let mut ask = |en: &str| {
                    if menu_asked.insert(en.to_string()) {
                        seq_counter += 1;
                        let _ = jobs.send(Job { area: MENU_TEXT.into(), seq: seq_counter, en: en.to_string(), phrases: Vec::new(), icons: Vec::new(), line_h: 0.0, en_start: Instant::now(), text_box: None, game_box: None, parts: Vec::new() });
                    }
                };
                screen_missing = localize_screen(layout, &cache.map, &mut ask);
                drop(cache);
            } else if found.is_some() {
                screen_missing = false;
            }
            if let Some(mut layout) = found {
                // the reason is always drawn in the same size
                // (measured only on a new page / new reason: refreshes reuse the frozen geometry)
                let new_reason = screen.as_ref().is_none_or(|s| s.reason_en != layout.reason_en);
                if new_reason && screens::menu_rects(&layout).is_none() {
                    layout.reason_line_h = sizes.lock().unwrap().size(&format!("__reason_{}@{}", layout.id, pages.as_ref().map_or(0, |p| p.version)), layout.reason_line_h);
                }
                // ...and so is the title (kept centred where the game's title is)
                let th = if screen.is_none() && screens::menu_rects(&layout).is_none() && layout.title.h > 0.0 {
                    sizes.lock().unwrap().size(&format!("__title_{}@{}", layout.id, pages.as_ref().map_or(0, |p| p.version)), layout.title.h)
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
                    // The page replaces normal subtitles: clear what the areas were showing, right away
                    // (each line on screen ends now: logged, and its translation dropped if it's still
                    // on its way — see PMsg::ClearAll).
                    let now = Instant::now();
                    for (id, st) in states.iter_mut() {
                        if st.current_seq != 0 {
                            let _ = present.send(PMsg::Ended { area: id.clone(), seq: st.current_seq, at: now });
                            if let Some(s0) = st.current_start {
                                memory.lock().unwrap().log_line(id, &st.current_en, unix_ms(s0), unix_ms(now));
                            }
                        }
                        st.current_start = None;
                        st.current.clear();
                        st.candidate.clear();
                        st.current_seq = 0;
                        seq_counter += 1;
                        latest.lock().unwrap().insert(id.clone(), seq_counter);
                    }
                    let _ = present.send(PMsg::ClearAll { upto: seq_counter });
                }
                if screen.as_ref() != Some(&layout) {
                    let reason_changed = screen.as_ref().map(|s| s.reason_en != layout.reason_en).unwrap_or(true);
                    if reason_changed {
                        *shared.screen_reason.lock().unwrap() = serde_json::json!([]);
                        if !layout.reason_en.is_empty() {
                            seq_counter += 1;
                            latest.lock().unwrap().insert(SCREEN_REASON.into(), seq_counter);
                            let _ = jobs.send(Job { area: SCREEN_REASON.into(), seq: seq_counter, en: layout.reason_en.clone(), phrases: Vec::new(), icons: Vec::new(), line_h: 0.0, en_start: Instant::now(), text_box: None, game_box: None, parts: Vec::new() });
                        }
                    }
                    *shared.screen.lock().unwrap() = serde_json::to_value(&layout).ok();
                    let _ = app.emit("screen", Some(&layout));
                    shared.screen_active.store(true, std::sync::atomic::Ordering::Relaxed);
                    screen = Some(layout);
                }
            }
            // Log each page's reason line with its time on screen, like subtitle lines.
            let reason_now = screen.as_ref().map(|s| s.reason_en.clone()).filter(|r| !r.is_empty());
            if reason_on.as_ref().map(|(r, _)| r) != reason_now.as_ref() {
                if let Some((r, since)) = reason_on.take() {
                    memory.lock().unwrap().log_line(SCREEN_REASON, &r, unix_ms(since), unix_ms(Instant::now()));
                }
                reason_on = reason_now.map(|r| (r, Instant::now()));
            }
            if screen.is_some() {
                // The pause menu is checked on every new frame (its checks are tiny), so its Arabic
                // goes the moment the English does: pace by the capture rate, not the read interval.
                let pause = screen.is_some();
                let pace = if pause { (1000 / c.capture_fps.clamp(1, 30) as u64).min(interval_ms) } else { interval_ms };
                let spent = t0.elapsed();
                std::thread::sleep(Duration::from_millis(pace.max(30)).saturating_sub(spent));
                continue; // normal areas pause while the page is up
            }

            // an area turned off (or gone): its line ends now (else the presenter waits for it forever)
            states.retain(|id, st| {
                let keep = c.areas.iter().any(|a| a.enabled && &a.id == id);
                if !keep && st.current_seq != 0 {
                    let _ = present.send(PMsg::Ended { area: id.clone(), seq: st.current_seq, at: Instant::now() });
                }
                keep
            });
            for area in c.areas.iter().filter(|a| a.enabled) {
                let st = states.entry(area.id.clone()).or_default();
                let now = Instant::now();
                let crop = Crop::from_frame(&frame.rgba, frame.w, frame.h, area.x, area.y, area.w, area.h);
                let (mut text, mut words, sure) = match &crop {
                    Some(cr) => match ocr.recognize_checked(cr) {
                        Ok((words, sure)) => (ocr::clean(&words), words, sure),
                        Err(_) => (String::new(), Vec::new(), true),
                    },
                    None => (String::new(), Vec::new(), true),
                };
                // Subtitles are centred in the picture: text off-centre (a menu's "Browse" at the bottom
                // right) is not a subtitle, and a panel at the side of the area (a garage's car stats) is
                // left out of the line instead of pushing it off-centre.
                if area.centered_only && !words.is_empty() {
                    let kept = ocr::centered_words(&words, area.x.clamp(0.0, 1.0) * frame.w as f64, frame.w as f64);
                    if kept.len() != words.len() {
                        words = kept;
                        text = ocr::clean(&words);
                    }
                }
                // An area of the game's own box (GTA VI's help): no box around the text, no text here.
                if area.needs_box && !text.is_empty() {
                    let found = crop.as_ref().zip(ocr::text_with_icons_block(crop.as_ref(), &words)).and_then(|(cr, b)| {
                        let mut hs: Vec<u32> = words.iter().map(|w| w.rect.3).collect();
                        hs.sort_unstable();
                        ocr::box_around(cr, b, hs.get(hs.len() / 2).copied().unwrap_or(20))
                    });
                    if found.is_none() {
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
                        && sure
                        && crop.as_ref().is_some_and(|cr| {
                            let (t, icons, _) = ocr::text_with_icons(cr, &words);
                            let line = if icons.is_empty() { &st.candidate } else { &t };
                            let m = memory.lock().unwrap();
                            // (a speaker name in front is not part of what's remembered)
                            m.knows(line) || speaker::without_name(line).is_some_and(|rest| m.knows(rest)) || m.knows(&speaker::sentence_of(line))
                        });
                    // A reading the app isn't sure of (text still animating in: GTA VI's instructions
                    // fade in with a glow) waits three times as long: by then the real text has
                    // replaced it, or it really is what's on screen.
                    let wait = if sure { stable_ms } else { stable_ms * 3 };
                    let settled = !is_new && st.candidate_since.is_some_and(|s| now - s >= Duration::from_millis(wait));
                    // The same line, sent without colours, now shows them: sent again with them (the same
                    // line on screen, not a new one: its start and the lines log stay as they are).
                    let recolor = st.colors_until.is_some_and(|t| now < t)
                        && !st.current.is_empty()
                        && ocr::similar(&st.candidate, &st.current)
                        && crop.as_ref().is_some_and(|cr| !ocr::colored_phrases(cr, &words).is_empty());
                    if st.colors_until.is_some_and(|t| now >= t) {
                        st.colors_until = None;
                    }
                    if ((known || settled) && !ocr::similar(&st.candidate, &st.current)) || recolor {
                        // Stable new text: translate it once. Colours are measured now (and again for a
                        // moment if there were none: see `colors_until`).
                        st.current = st.candidate.clone();
                        let phrases = crop.as_ref().map(|cr| ocr::colored_phrases(cr, &words)).unwrap_or_default();
                        st.colors_until = if phrases.is_empty() && !recolor { Some(now + Duration::from_millis(2000)) } else { None };
                        // Controller button icons inside the text ("Press ✕ to ...") come along as placeholders.
                        let (en_icons, icons_found, block) =
                            crop.as_ref().map(|cr| ocr::text_with_icons(cr, &words)).unwrap_or((String::new(), Vec::new(), None));
                        let (en, icons) = if icons_found.is_empty() { (st.current.clone(), Vec::new()) } else { (en_icons, icons_found) };
                        // stacked subtitle lines (not with button icons: those are instructions)
                        let parts = match &crop {
                            // (subtitles only: an instructions box's sentences read as one text)
                            Some(cr) if icons.is_empty() && area.centered_only => ocr::speaker_lines(cr, &words),
                            _ => Vec::new(),
                        };
                        // the block in frame fractions (the crop starts at the area's top-left)
                        let (cx0, cy0) = ((area.x.clamp(0.0, 1.0) * frame.w as f64) as u32, (area.y.clamp(0.0, 1.0) * frame.h as f64) as u32);
                        let to_frame = |(x, y, w, h): (u32, u32, u32, u32)| {
                            let (fw, fh) = (frame.w as f64, frame.h as f64);
                            [(cx0 + x) as f64 / fw, (cy0 + y) as f64 / fh, w as f64 / fw, h as f64 / fh]
                        };
                        let text_box = block.map(to_frame);
                        // the game's own box (only placed below it: "under_text")
                        let game_box = match (&crop, block) {
                            (Some(cr), Some(b)) if area.placement == "under_text" => {
                                let mut hs: Vec<u32> = words.iter().map(|w| w.rect.3).collect();
                                hs.sort_unstable();
                                ocr::box_around(cr, b, hs.get(hs.len() / 2).copied().unwrap_or(20)).map(to_frame)
                            }
                            _ => None,
                        };
                        seq_counter += 1;
                        latest.lock().unwrap().insert(area.id.clone(), seq_counter);
                        // the previous English line ended when this one appeared
                        let en_start = if recolor { st.current_start.unwrap_or(now) } else { st.candidate_since.unwrap_or(now) };
                        if st.current_seq != 0 {
                            let _ = present.send(PMsg::Ended { area: area.id.clone(), seq: st.current_seq, at: if recolor { now } else { en_start } });
                            if let (Some(s0), false) = (st.current_start, recolor) {
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
                        // (logged without a speaker name in front: the sentence is what gets translated)
                        st.current_en = {
                            let clean = speaker::take_cues(&en).map(|(_, t)| t).unwrap_or_else(|| en.clone());
                            let line = speaker::split(&clean, &phrases).map(|(_, _, rest)| rest).unwrap_or(&clean);
                            speaker::language_tag(line).map(|(_, s)| s).unwrap_or(line).to_string()
                        };
                        st.current_start = Some(en_start);
                        let _ = jobs.send(Job { area: area.id.clone(), seq: seq_counter, en, phrases, icons, line_h, en_start, text_box, game_box, parts });
                    }
                } else if !st.current.is_empty()
                    && st.last_seen.is_some_and(|s| now - s > Duration::from_millis(clear_after_ms))
                {
                    // The English is gone (not just an OCR blink). It really ended when last seen; the
                    // presenter keeps the Arabic up for as long as the English was on screen.
                    if let Some(last) = st.last_seen {
                        let _ = present.send(PMsg::Ended { area: area.id.clone(), seq: st.current_seq, at: last });
                        if let Some(s0) = st.current_start {
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
                    && st.last_seen.is_some_and(|s| now - s > Duration::from_millis(clear_after_ms))
                {
                    // Text that came and went before it was ever translated: forget it, or its old start
                    // time would be reused when similar text shows up minutes later.
                    st.candidate.clear();
                    st.candidate_since = None;
                }
            }
        }
        let spent = t0.elapsed();
        std::thread::sleep(Duration::from_millis(interval_ms.max(50)).saturating_sub(spent));
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
    // Subtitle lines (and a page's reason) first; a page's menu text only when nothing else waits,
    // newest first (the tab on screen now before an old backlog): a menu full of text never holds a
    // subtitle up for more than the one menu line being translated.
    let (mut lines_q, mut menu_q) = (std::collections::VecDeque::new(), std::collections::VecDeque::<Job>::new());
    loop {
        if lines_q.is_empty() && menu_q.is_empty() {
            match jobs.recv() {
                Ok(j) if j.area == MENU_TEXT => menu_q.push_back(j),
                Ok(j) => lines_q.push_back(j),
                Err(_) => return,
            }
        }
        for j in jobs.try_iter() {
            if j.area == MENU_TEXT {
                menu_q.push_back(j);
            } else {
                lines_q.push_back(j);
            }
        }
        let Some(job) = lines_q.pop_front().or_else(|| menu_q.pop_back()) else { continue };
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
        // Translation memory first: a known line needs no translator at all.
        // (the Mission Failed reason too: same game text, same reuse)
        // (from the memory of the job's language only: just after a language switch the memory may
        // not have followed yet, and its lines would be in the other language)
        // Two speakers' lines (white and grey) are translated one by one: as one sentence the
        // translator merges or drops one ("… so... What?" lost the "What?"). Each comes back in its own
        // line, the other speaker's in grey like the game's.
        let (result, hit) = if job.parts.len() > 1 {
            let mut all: translate::Segments = Vec::new();
            let mut hits = true;
            let mut err = None;
            for (i, (en, rel)) in job.parts.iter().enumerate() {
                let (r, h) = translate_spoken(&mut tr, &c, &memory, en, &[], &job.phrases, &status);
                hits &= h;
                match r {
                    Ok(segs) => {
                        if i > 0 {
                            all.push(("\n".into(), None));
                        }
                        // the game's grey, on the plain parts (a highlighted word keeps its colour); the
                        // pages draw "dim" grey whatever the colour setting
                        let grey = (*rel < 0.95).then(|| "dim".to_string());
                        all.extend(segs.into_iter().map(|(t, col)| (t, col.or_else(|| grey.clone()))));
                    }
                    Err(e) => err = Some(e),
                }
            }
            (match err { Some(e) => Err(e), None => Ok(all) }, hits)
        } else {
            translate_spoken(&mut tr, &c, &memory, &job.en, &job.icons, &job.phrases, &status)
        };
        let hit_from = if hit { "memory" } else { "" };
        // time from the English line appearing to its Arabic being ready
        let ready_ms = job.en_start.elapsed().as_secs_f64() * 1000.0;
        let mem_note = {
            let mut m = memory.lock().unwrap();
            if m.lang() == c.target_lang {
                m.record(hit, ready_ms);
            }
            Some(if hit { format!("{hit_from} hit · ready {ready_ms:.0} ms after the English") } else { format!("ready {ready_ms:.0} ms after the English · saved to memory") })
        };
        // pause-menu text: keep it for the menu page (it appears on the next read)
        if job.area == MENU_TEXT {
            let ar: String = result.as_ref().map(|segs| segs.iter().map(|s| s.0.as_str()).collect()).unwrap_or_default();
            let mut mc = shared.menu_cache.lock().unwrap();
            // (asked in another language than the one in use now: the menu asks again in this one)
            if mc.lang != c.target_lang {
                continue;
            }
            if ar.trim().is_empty() {
                // (an error, or nothing: asked again later, when it's read again — see ocr_loop)
                shared.menu_failed.lock().unwrap().push(job.en.clone());
            } else {
                mc.map.insert(job.en.clone(), ar.trim().to_string());
            }
            continue;
        }
        // the English with its icons, for the live translations; and as text with a small marker
        let en_segments = if job.parts.len() > 1 {
            // (two speakers: the English in two lines too)
            let mut s: Segments = Vec::new();
            for (i, p) in job.parts.iter().enumerate() {
                if i > 0 {
                    s.push(("\n".into(), None));
                }
                s.push((p.0.clone(), None));
            }
            with_phrase_colors(s, &job.phrases)
        } else {
            with_phrase_colors(english_segments(&job.en, &job.icons), &job.phrases)
        };
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
                en_segments,
                segments,
                stats: match &mem_note {
                    Some(n) if hit => n.clone(),
                    Some(n) => format!("{} · {n}", tr.stats(&c)),
                    None => tr.stats(&c),
                },
                error: None,
                line_h: job.line_h,
                text_box: job.text_box,
                game_box: job.game_box,
                cached: hit,
                at: unix_ms(en_start),
            },
            Err(e) => TranslationEvent {
                area: job.area,
                en: job.en,
                en_segments,
                segments: Vec::new(),
                stats: tr.stats(&c),
                error: Some(format!("{e:#}")),
                line_h: job.line_h,
                text_box: job.text_box,
                game_box: job.game_box,
                cached: false,
                at: unix_ms(en_start),
            },
        };
        if ev.area == SCREEN_REASON && ev.error.is_none() {
            *shared.screen_reason.lock().unwrap() = serde_json::to_value(&ev.segments).unwrap_or_default();
            lan::push(&shared, SCREEN_REASON, &ev.en, &ev.en_segments, &ev.segments, ev.at);
        }
        if ev.area == SCREEN_REASON || ev.error.is_some() {
            let _ = app.emit("translation", ev); // errors go straight to the log
        } else {
            let _ = present.send(PMsg::Ready { seq, en_start, ev });
        }
    }
}

/// One English line to the target language: from the translation memory when it knows it, else
/// the translator (and then into the memory). True when it came from the memory.
/// `translate_line`, with a speaker name in front ("Dobbs: …") kept out of the translation: only the
/// sentence is translated and remembered, and the name comes back in front in the game's colour, in
/// the target language's letters (see `speaker`).
fn translate_spoken(
    tr: &mut Translator,
    c: &Config,
    memory: &Arc<Mutex<memory::Memory>>,
    en: &str,
    icons: &[String],
    phrases: &[(String, String)],
    status: &dyn Fn(String),
) -> (anyhow::Result<translate::Segments>, bool) {
    // "[laughs]": the sound cues off first (each translated on its own), back where they were at the end
    if let Some((cues, clean)) = speaker::take_cues(en) {
        let shown: Vec<String> = cues
            .before
            .iter()
            .chain(&cues.after_name)
            .chain(&cues.after)
            .map(|cue| {
                speaker::cue_in(&c.target_lang, cue).unwrap_or_else(|| {
                    let (r, _) = translate_line(tr, c, memory, cue, &[], &[], status);
                    r.ok().map(|segs| segs.iter().map(|s| s.0.as_str()).collect::<String>()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| cue.clone())
                })
            })
            .collect();
        // a cue-only line ("Donnie: [laughs]", "[rattling]"): the name (if any) and the cues
        let words = clean.chars().filter(|ch| ch.is_alphabetic()).count();
        let probe = format!("{clean} xx");
        let only_name = speaker::plain_name(&probe).is_some_and(|(_, rest)| rest == "xx");
        if words < 2 || only_name {
            let named = if only_name { speaker::plain_name(&probe).map(|(n, _)| n) } else { None };
            let segs = match &named {
                Some(name) => {
                    let color = phrases.iter().find(|(p, _)| p.starts_with(name.as_str())).map(|(_, col)| col.clone()).unwrap_or_else(|| speaker::PLAIN_NAME_COLOR.to_string());
                    let shown_name = speaker::name_in(&c.target_lang, name, reviewed_name(memory, c, name));
                    let mut s = speaker::with_name(&shown_name, &color, Vec::new());
                    s.truncate(2);
                    s
                }
                None => Vec::new(),
            };
            return (Ok(speaker::with_cues(&cues, &shown, named.is_some(), segs)), true);
        }
        let (result, hit) = translate_spoken(tr, c, memory, &clean, icons, phrases, status);
        let named = speaker::split(&clean, phrases).is_some() || speaker::plain_name(&clean).is_some();
        return (result.map(|segs| speaker::with_cues(&cues, &shown, named, segs)), hit);
    }
    let named = speaker::split(en, phrases).or_else(|| plain_speaker(memory, c, en));
    let (line, phrases) = match &named {
        Some((name, _, rest)) => (*rest, speaker::phrases_without(phrases, name)),
        None => (en, phrases.to_vec()),
    };
    // "In Spanish: …": the sentence alone, the tag in front (as the app knows it, or translated too)
    let (result, hit) = match speaker::language_tag(line) {
        Some((language, sentence)) => {
            let (result, hit) = translate_line(tr, c, memory, sentence, icons, &phrases, status);
            let tag = speaker::language_in(&c.target_lang, &language).or_else(|| {
                let (r, _) = translate_line(tr, c, memory, &format!("In {language}:"), &[], &[], status);
                r.ok().map(|segs| segs.iter().map(|s| s.0.as_str()).collect::<String>())
            });
            match tag {
                Some(tag) => (result.map(|segs| speaker::with_language(&tag, segs)), hit),
                None => (result, hit),
            }
        }
        None => translate_line(tr, c, memory, line, icons, &phrases, status),
    };
    let Some((name, color, _)) = named else { return (result, hit) };
    let mut reviewed = reviewed_name(memory, c, &name);
    // a role ("Cop:") is translated; outside Arabic, by the translator when no reviewed line has it
    if reviewed.is_none() && c.target_lang != "ar" && speaker::is_role(&name) {
        let (r, _) = translate_line(tr, c, memory, &name, &[], &[], status);
        reviewed = r.ok().map(|segs| segs.iter().map(|s| s.0.as_str()).collect());
    }
    let shown = if speaker::is_role(&name) && c.target_lang != "ar" {
        reviewed.map(|r| r.trim().trim_end_matches(|ch: char| matches!(ch, ':' | '.' | '!' | '?' | '،' | '؟' | '…')).trim().to_string()).filter(|r| !r.is_empty()).unwrap_or(name)
    } else {
        speaker::name_in(&c.target_lang, &name, reviewed)
    };
    (result.map(|segs| speaker::with_name(&shown, &color, segs)), hit)
}

/// A "Name:" the colour check missed, taken when the app knows it: one of the game's people, a name
/// with a reviewed translation, or a sentence it knows without the name. (name, colour, rest)
fn plain_speaker<'a>(memory: &Arc<Mutex<memory::Memory>>, c: &Config, en: &'a str) -> Option<(String, String, &'a str)> {
    let (name, rest) = speaker::plain_name(en)?;
    let known = speaker::is_known(&name) || speaker::is_role(&name) || reviewed_name(memory, c, &name).is_some() || memory.lock().unwrap().knows(rest);
    known.then(|| (name, speaker::PLAIN_NAME_COLOR.to_string(), rest))
}

/// A reviewed translation of a name alone ("Dobbs" -> "دوبس") from the downloaded translations.
fn reviewed_name(memory: &Arc<Mutex<memory::Memory>>, c: &Config, name: &str) -> Option<String> {
    let m = memory.lock().unwrap();
    if m.lang() != c.target_lang {
        return None;
    }
    m.get(name, &[], &[]).filter(|(_, src)| src == "reviewed").map(|(segs, _)| segs.iter().map(|s| s.0.as_str()).collect())
}

fn translate_line(
    tr: &mut Translator,
    c: &Config,
    memory: &Arc<Mutex<memory::Memory>>,
    en: &str,
    icons: &[String],
    phrases: &[(String, String)],
    status: &dyn Fn(String),
) -> (anyhow::Result<translate::Segments>, bool) {
    // (from the memory of the job's language only: just after a language switch the memory may
    // not have followed yet, and its lines would be in the other language)
    let mut remembered = {
        let m = memory.lock().unwrap();
        if m.lang() == c.target_lang { m.get(en, icons, phrases) } else { None }
    };
    // (not one that mangled a term that must stay, like GTA+ — made before terms were protected)
    if remembered.as_ref().is_some_and(|(segs, _)| !translate::keeps_terms(en, &segs.iter().map(|s| s.0.as_str()).collect::<String>())) {
        remembered = None;
    }
    // A hand-written phrasebook line beats a machine translation remembered earlier (only an
    // reviewed line ranks above it).
    if remembered.as_ref().is_some_and(|(_, src)| src != "reviewed" && src != "phrasebook") {
        if phrasebook::find(&c.game(), &c.target_lang, en, icons).is_some() {
            remembered = None; // the translator below answers from the phrasebook, and memory is updated
        }
    }
    let hit = remembered.is_some();
    let result = match remembered {
        // (coloured here when the remembered line has no colours but the game highlights a word)
        Some((segs, _)) => Ok(translate::polite(&c.target_lang, tr.recolor(c, segs, phrases, status))),
        None => tr.translate(c, en, phrases, icons, status),
    };
    if let (false, Ok(segs)) = (hit, &result) {
        let mut m = memory.lock().unwrap();
        // (translated for the language in use when the job started: if the player switched since,
        // it belongs to neither memory)
        // (not one that lost a term that must stay, like GTA+: it would be turned down when reused)
        if m.lang() == c.target_lang && translate::keeps_terms(en, &segs.iter().map(|s| s.0.as_str()).collect::<String>()) {
            let source = if phrasebook::find(&c.game(), &c.target_lang, en, icons).is_some() { "phrasebook" } else { c.engine.as_str() };
            m.put(en, segs, source);
        }
    }
    (result, hit)
}

// ------------------------------------------------------------------------------------- test mode

/// `vi-translator --test-image shot.png [--area help]`: OCR + translate one screenshot and print the result.
/// See `--read-video`: frames come from ffmpeg (raw RGBA on its stdout, never saved).
fn read_video(file: &str, fps: f64, from: f64, to: f64) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    let cfg = Config::load(&config_path());
    let areas: Vec<_> = cfg.areas.iter().filter(|a| a.enabled).cloned().collect();
    // the picture's size
    let probe = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0", file])
        .output()?;
    let dims = String::from_utf8_lossy(&probe.stdout).trim().to_string();
    let (w, h) = dims.split_once(',').and_then(|(a, b)| Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?))).ok_or_else(|| anyhow::anyhow!("no video size in {dims:?}"))?;
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-v", "error"]);
    if from > 0.0 {
        cmd.args(["-ss", &format!("{from}")]);
    }
    cmd.args(["-i", file]);
    if to > from {
        cmd.args(["-t", &format!("{}", to - from)]);
    }
    cmd.args(["-an", "-vf", &format!("fps={fps}"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"]).stdout(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut pipe = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no ffmpeg output"))?;
    Ocr::init_thread();
    let ocr = Ocr::new()?;
    let mut frame = vec![0u8; (w * h * 4) as usize];
    let out = std::io::stdout();
    let mut out = out.lock();
    let mut n = 0u64;
    while pipe.read_exact(&mut frame).is_ok() {
        let t = from + n as f64 / fps;
        n += 1;
        for area in &areas {
            let Some(crop) = Crop::from_frame(&frame, w, h, area.x, area.y, area.w, area.h) else { continue };
            let (mut words, sure) = ocr.recognize_checked(&crop)?;
            if area.centered_only {
                words = ocr::centered_words(&words, area.x.clamp(0.0, 1.0) * w as f64, w as f64);
            }
            let en = ocr::clean(&words);
            if en.is_empty() {
                continue;
            }
            let (en_icons, icons, block) = ocr::text_with_icons(&crop, &words);
            let phrases = ocr::colored_phrases(&crop, &words);
            let parts = if icons.is_empty() && area.centered_only { ocr::speaker_lines(&crop, &words) } else { Vec::new() };
            // where the text sits across the frame (areas that read only centred text skip the rest)
            let cx = block.map(|(x, _, bw, _)| (area.x * w as f64 + x as f64 + bw as f64 / 2.0) / w as f64);
            let line = serde_json::json!({
                "t": (t * 100.0).round() / 100.0, "area": area.id, "en": if icons.is_empty() { en } else { en_icons },
                "sure": sure, "icons": icons.len(), "cx": cx, "phrases": phrases,
                "parts": parts.iter().map(|(p, rel)| serde_json::json!([p, (rel * 100.0).round() / 100.0])).collect::<Vec<_>>(),
            });
            writeln!(out, "{line}")?;
        }
        if n % 600 == 0 {
            eprintln!("read-video: {:.0} min", t / 60.0);
            out.flush()?;
        }
    }
    let _ = child.wait();
    eprintln!("read-video: {n} frames");
    Ok(())
}

/// One English line translated outside the running app (--test-image, --translate-frames) the way
/// the app does it: a speaker name or "In Spanish:" kept apart, the memory and the downloaded reviewed
/// lines first, then the translator; harsh words softened.
fn offline_line(
    tr: &mut Translator,
    cfg: &Config,
    mem: &memory::Memory,
    en: &str,
    icons: &[String],
    phrases: &[(String, String)],
    log: &dyn Fn(String),
) -> anyhow::Result<translate::Segments> {
    let one = |tr: &mut Translator, line: &str, phrases: &[(String, String)]| -> anyhow::Result<translate::Segments> {
        Ok(match mem.get(line, icons, phrases) {
            Some((segs, src)) => {
                log(format!("From memory ({src})"));
                translate::polite(&cfg.target_lang, tr.recolor(cfg, segs, phrases, log))
            }
            None => tr.translate(cfg, line, phrases, icons, log)?,
        })
    };
    let named = speaker::split(en, phrases).or_else(|| {
        let (name, rest) = speaker::plain_name(en)?;
        let reviewed = mem.get(&name, &[], &[]).is_some_and(|(_, src)| src == "reviewed");
        (speaker::is_known(&name) || speaker::is_role(&name) || reviewed || mem.knows(rest)).then(|| (name, speaker::PLAIN_NAME_COLOR.to_string(), rest))
    });
    let (line, phrases) = match &named {
        Some((name, color, rest)) => {
            log(format!("Speaker: {name} ({color})"));
            (*rest, speaker::phrases_without(phrases, name))
        }
        None => (en, phrases.to_vec()),
    };
    let segs = match speaker::language_tag(line) {
        Some((language, sentence)) => {
            log(format!("Language: {language}"));
            let segs = one(tr, sentence, &phrases)?;
            let tag = speaker::language_in(&cfg.target_lang, &language).unwrap_or_else(|| format!("In {language}:"));
            speaker::with_language(&tag, segs)
        }
        None => one(tr, line, &phrases)?,
    };
    Ok(match named {
        Some((name, color, _)) => {
            let mut reviewed: Option<String> = mem.get(&name, &[], &[]).filter(|(_, src)| src == "reviewed").map(|(s, _)| s.iter().map(|x| x.0.as_str()).collect());
            let role = speaker::is_role(&name) && cfg.target_lang != "ar";
            if role && reviewed.is_none() {
                reviewed = one(tr, &name, &[]).ok().map(|segs| segs.iter().map(|s| s.0.as_str()).collect());
            }
            let shown = if role {
                reviewed.map(|r| r.trim().trim_end_matches(|ch: char| matches!(ch, ':' | '.' | '!' | '?' | '،' | '؟' | '…')).trim().to_string()).filter(|r| !r.is_empty()).unwrap_or(name)
            } else {
                speaker::name_in(&cfg.target_lang, &name, reviewed)
            };
            speaker::with_name(&shown, &color, segs)
        }
        None => segs,
    })
}

/// `--translate-frames <video> <times file> <out dir>`: the frames at those times (seconds, one per
/// line) read and translated as the app would, each saved as <out dir>/<time>.jpg with a JSON line on
/// stdout: {"t", "file", "w", "h", "areas": [{"id", "segs", "line_h", "text_box", "game_box"}]}.
/// The frames come from ffmpeg in order (2 per second), so a whole stream is decoded once.
fn translate_frames(file: &str, times_file: &str, out_dir: &str) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    let cfg = Config::load(&config_path());
    let areas: Vec<_> = cfg.areas.iter().filter(|a| a.enabled).cloned().collect();
    let mut wanted: Vec<f64> = std::fs::read_to_string(times_file)?.lines().filter_map(|l| l.trim().parse().ok()).collect();
    wanted.sort_by(|a, b| a.total_cmp(b));
    std::fs::create_dir_all(out_dir)?;
    let probe = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0", file])
        .output()?;
    let dims = String::from_utf8_lossy(&probe.stdout).trim().to_string();
    let (w, h) = dims.split_once(',').and_then(|(a, b)| Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?))).ok_or_else(|| anyhow::anyhow!("no video size in {dims:?}"))?;
    let fps = 2.0;
    // (several of these can share a video: each starts at its own first frame)
    let start = wanted.first().map(|t| (t * fps).floor() / fps).unwrap_or(0.0);
    let mut child = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &format!("{start}"), "-i", file, "-an", "-vf", "fps=2", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let mut pipe = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no ffmpeg output"))?;
    Ocr::init_thread();
    let ocr = Ocr::new()?;
    let mut tr = Translator::new(model_dir());
    let mut mem = memory::Memory::load(db_dir(), &cfg.target_lang);
    if let Some(entries) = sync::kept_pack_in(&data_dir(), &cfg.game(), &cfg.target_lang) {
        mem.set_shared(&cfg.target_lang, entries);
    }
    let quiet = |_: String| {};
    let mut frame = vec![0u8; (w * h * 4) as usize];
    let out = std::io::stdout();
    let mut out = out.lock();
    let (mut n, mut next) = (0u64, 0usize);
    while next < wanted.len() && pipe.read_exact(&mut frame).is_ok() {
        let t = start + n as f64 / fps;
        n += 1;
        if (t - wanted[next]).abs() > 0.01 {
            continue;
        }
        next += 1;
        let mut shown = Vec::new();
        for area in &areas {
            let Some(crop) = Crop::from_frame(&frame, w, h, area.x, area.y, area.w, area.h) else { continue };
            let (mut words, sure) = ocr.recognize_checked(&crop)?;
            if area.centered_only {
                words = ocr::centered_words(&words, area.x.clamp(0.0, 1.0) * w as f64, w as f64);
            }
            let en = ocr::clean(&words);
            // (a reading the app isn't sure of, text still fading in, is one it would wait out)
            if en.is_empty() || (!sure && area.id != "subtitles") {
                continue;
            }
            let (en_icons, icons, block) = ocr::text_with_icons(&crop, &words);
            // instructions and help: only lines the reviewed translations know (a half-faded
            // "Follow the p" isn't a line; the app would show the whole one a moment later)
            if area.id != "subtitles" && !mem.knows(if icons.is_empty() { &en } else { &en_icons }) {
                continue;
            }
            let (cx0, cy0) = ((area.x.clamp(0.0, 1.0) * w as f64) as u32, (area.y.clamp(0.0, 1.0) * h as f64) as u32);
            let to_frame = |(x, y, bw, bh): (u32, u32, u32, u32)| [(cx0 + x) as f64 / w as f64, (cy0 + y) as f64 / h as f64, bw as f64 / w as f64, bh as f64 / h as f64];
            // (areas that read only centred text skip a menu's hint at the side, as the app does)
            if area.centered_only {
                if let Some((x, _, bw, _)) = block {
                    if ((cx0 + x + bw / 2) as f64 / w as f64 - 0.5).abs() > 0.04 {
                        continue;
                    }
                }
            }
            let phrases = ocr::colored_phrases(&crop, &words);
            let mut hs: Vec<u32> = words.iter().map(|wd| wd.rect.3).collect();
            hs.sort_unstable();
            let lh = hs.get(hs.len() / 2).copied().unwrap_or(20);
            if area.needs_box && block.and_then(|b| ocr::box_around(&crop, b, lh)).is_none() {
                continue;
            }
            let parts = if icons.is_empty() && area.centered_only { ocr::speaker_lines(&crop, &words) } else { Vec::new() };
            let segs = if parts.len() > 1 {
                let mut all: translate::Segments = Vec::new();
                for (i, (p, rel)) in parts.iter().enumerate() {
                    if i > 0 {
                        all.push(("\n".into(), None));
                    }
                    let grey = (*rel < 0.95).then(|| "dim".to_string());
                    let segs = offline_line(&mut tr, &cfg, &mem, p, &[], &phrases, &quiet)?;
                    all.extend(segs.into_iter().map(|(t, col)| (t, col.or_else(|| grey.clone()))));
                }
                all
            } else {
                let line = if icons.is_empty() { en.clone() } else { en_icons.clone() };
                offline_line(&mut tr, &cfg, &mem, &line, &icons, &phrases, &quiet)?
            };
            let game_box = match block {
                Some(b) if area.placement == "under_text" => ocr::box_around(&crop, b, lh).map(to_frame),
                _ => None,
            };
            shown.push(serde_json::json!({
                "id": area.id, "en": en, "segs": segs, "line_h": lh as f64 / h as f64,
                "text_box": block.map(to_frame), "game_box": game_box,
            }));
        }
        let name = format!("{:07.1}.jpg", t);
        let img = image::RgbaImage::from_raw(w, h, frame.clone()).ok_or_else(|| anyhow::anyhow!("frame"))?;
        image::DynamicImage::ImageRgba8(img).to_rgb8().save_with_format(std::path::Path::new(out_dir).join(&name), image::ImageFormat::Jpeg)?;
        writeln!(out, "{}", serde_json::json!({ "t": t, "file": name, "w": w, "h": h, "areas": shown }))?;
        out.flush()?;
        if next % 100 == 0 {
            eprintln!("translate-frames: {next}/{}", wanted.len());
        }
    }
    let _ = child.kill();
    eprintln!("translate-frames: {next} frames");
    Ok(())
}

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
    let mut words = ocr.recognize(&crop)?;
    if std::env::var_os("VI_TRANSLATOR_DEBUG").is_some() {
        eprintln!("[ocr] words: {:?}", words.iter().map(|w| (&w.text, w.rect)).collect::<Vec<_>>());
    }
    if area.centered_only {
        words = ocr::centered_words(&words, area.x.clamp(0.0, 1.0) * w as f64, w as f64);
    }
    let en = ocr::clean(&words);
    let phrases = ocr::colored_phrases(&crop, &words);
    println!("OCR ({} ms): {en:?}", t.elapsed().as_millis());
    let t2 = Instant::now();
    let (en_icons, icons, block) = ocr::text_with_icons(&crop, &words);
    println!("Text block (crop px): {block:?}");
    if let Some(b) = block {
        let mut hs: Vec<u32> = words.iter().map(|w| w.rect.3).collect();
        hs.sort_unstable();
        let lh = hs.get(hs.len() / 2).copied().unwrap_or(20);
        // the game's own box around it, in frame pixels (the crop starts at the area's top-left)
        let (cx0, cy0) = ((area.x.clamp(0.0, 1.0) * w as f64) as u32, (area.y.clamp(0.0, 1.0) * h as f64) as u32);
        println!("Game box (frame px): {:?}", ocr::box_around(&crop, b, lh).map(|(x, y, bw, bh)| (cx0 + x, cy0 + y, bw, bh)));
    }
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
    // the memory as the app has it (the downloaded reviewed lines too); read only, nothing is saved
    let mut mem = memory::Memory::load(db_dir(), &cfg.target_lang);
    if let Some(entries) = sync::kept_pack_in(&data_dir(), &cfg.game(), &cfg.target_lang) {
        mem.set_shared(&cfg.target_lang, entries);
    }
    let line = |tr: &mut Translator, en: &str, icons: &[String]| offline_line(tr, &cfg, &mem, en, icons, &phrases, &|s| println!("{s}"));
    let t = Instant::now();
    // two speakers' lines (white and grey): each translated on its own, as the app does
    let parts = if icons.is_empty() && area.centered_only { ocr::speaker_lines(&crop, &words) } else { Vec::new() };
    let segs = if parts.len() > 1 {
        println!("Speakers: {parts:?}");
        let mut all: translate::Segments = Vec::new();
        for (i, (en, rel)) in parts.iter().enumerate() {
            if i > 0 {
                all.push(("\n".into(), None));
            }
            let grey = (*rel < 0.95).then(|| "dim".to_string());
            let segs = line(&mut tr, en, &[])?;
            all.extend(segs.into_iter().map(|(t, col)| (t, col.or_else(|| grey.clone()))));
        }
        all
    } else {
        line(&mut tr, &en_icons, &icons)?
    };
    println!("Segments: {}", serde_json::to_string(&segs).unwrap_or_default());
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
    // `--read-pages <folder> [--game <id>] [--script <file.js>] [--full]`: what a pages script reads
    // on every PNG in a folder (saved debug frames), page by page — to try a new page before it's
    // signed and sent to testers. Without --script: the game's script as the app would load it (its
    // own, or a signed download). A --script file needn't be signed (it's yours, on your PC).
    if let Some(dir) = args.iter().position(|a| a == "--read-pages").and_then(|i| args.get(i + 1)) {
        let opt = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
        let full = args.iter().any(|a| a == "--full");
        let run = || -> anyhow::Result<()> {
            Ocr::init_thread();
            let ocr = Ocr::new()?;
            let game = opt("--game").unwrap_or_else(|| Config::load(&config_path()).game());
            // (the language in use, for a script's own words: `--lang xx` to try another)
            screens::use_words_for(&game, &opt("--lang").unwrap_or_else(|| Config::load(&config_path()).target_lang));
            let pages = match opt("--script") {
                Some(file) => {
                    let mut p = pagescript::Pages::load(&std::fs::read_to_string(&file)?)?;
                    p.origin = format!("{file}, version {}", pagescript::version_of(&std::fs::read_to_string(&file)?));
                    p
                }
                None => pagescript::Pages::for_game(&game, &data_dir()).ok_or_else(|| anyhow::anyhow!("no pages script for {game}: give one with --script"))?,
            };
            println!("script: {} · pages: {}", pages.origin, pages.list.iter().map(|i| i.id.as_str()).collect::<Vec<_>>().join(", "));
            for o in &pages.options {
                println!("switch: {} ({}), on by default: {}", o.id, o.name, o.on);
            }
            // (the frames only: not the "_overlay.png" copies saved next to them, with the translation drawn)
            let mut files: Vec<_> = std::fs::read_dir(dir)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "png") && !p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("_overlay.png")))
                .collect();
            files.sort();
            let (mut n, mut found, mut recognised) = (0, 0, 0);
            for path in files {
                let Ok(img) = image::open(&path) else { continue };
                let img = img.to_rgba8();
                let (w, h) = img.dimensions();
                let frame = capture::FrameData { w, h, rgba: img.into_raw(), seq: 1 };
                n += 1;
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let mut any = false;
                for info in &pages.list {
                    let t = Instant::now();
                    let Some(k) = pages.present(&info.id, &frame, &ocr) else { continue };
                    let read = pages.read(&info.id, &frame, &ocr, &k);
                    let ms = t.elapsed().as_millis();
                    any = true;
                    found += 1;
                    match read {
                        None => println!("{name} {}: on screen, but read() found nothing ({ms} ms)", info.id),
                        Some(l) if full => println!("{name} {} ({ms} ms): {}", info.id, serde_json::to_string_pretty(&l).unwrap_or_default()),
                        Some(l) => {
                            let mut parts = Vec::new();
                            if l.title.w > 0.0 {
                                parts.push(format!("title at {:.3},{:.3}", l.title.x, l.title.y));
                            }
                            if !l.reason_en.is_empty() {
                                parts.push(format!("reason {:?}", l.reason_en));
                            }
                            if !l.buttons.is_empty() {
                                parts.push(format!("buttons {:?}", l.buttons.iter().map(|b| b.en.as_str()).collect::<Vec<_>>()));
                            }
                            if !l.labels.is_empty() {
                                parts.push(format!("labels {:?}", l.labels.iter().map(|b| b.en.as_str()).collect::<Vec<_>>()));
                            }
                            if let Some(p) = &l.player {
                                parts.push(format!("player {:?}", p.en));
                            }
                            if !l.menu.is_empty() {
                                let shown: Vec<String> = l.menu.iter().take(12).map(|m| if m.private { "(private)".to_string() } else { m.en.clone() }).collect();
                                parts.push(format!("{} text box(es) {:?}{}", l.menu.len(), shown, if l.menu.len() > 12 { " …" } else { "" }));
                            }
                            println!("{name} {} ({ms} ms): {}", info.id, if parts.is_empty() { "an empty page".to_string() } else { parts.join(" · ") });
                        }
                    }
                }
                if pages.recognize(&frame, &ocr) {
                    recognised += 1;
                }
                if !any {
                    println!("{name}: no page");
                }
            }
            println!("{n} frames, {found} pages on screen, the game recognised on {recognised}");
            Ok(())
        };
        if let Err(e) = run() {
            eprintln!("read-pages: {e:#}");
        }
        return;
    }
    // `--read-video <file> [--fps N] [--from S] [--to S]`: every enabled area of every frame read the
    // way the app reads them (nothing translated), as JSON lines on stdout — to collect a game's lines.
    if let Some(file) = args.iter().position(|a| a == "--read-video").and_then(|i| args.get(i + 1)) {
        let num = |name: &str, def: f64| args.iter().position(|a| a == name).and_then(|j| args.get(j + 1)).and_then(|v| v.parse().ok()).unwrap_or(def);
        if let Err(e) = read_video(file, num("--fps", 2.0), num("--from", 0.0), num("--to", 0.0)) {
            eprintln!("read-video: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--translate-frames") {
        let (Some(video), Some(times), Some(out)) = (args.get(i + 1), args.get(i + 2), args.get(i + 3)) else {
            eprintln!("usage: --translate-frames <video> <times file> <out dir>");
            std::process::exit(2);
        };
        if let Err(e) = translate_frames(video, times, out) {
            eprintln!("translate-frames: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    // --test-snapshot <frame.png> [--service <id>]: a snapshot of a saved frame, its blocks printed
    if let Some(file) = args.iter().position(|a| a == "--test-snapshot").and_then(|i| args.get(i + 1)) {
        let run = || -> anyhow::Result<()> {
            let img = image::open(file)?.to_rgba8();
            let (w, h) = img.dimensions();
            let frame = capture::FrameData { w, h, rgba: img.into_raw(), seq: 0 };
            let mut c = Config::load(&config_path());
            if let Some(s) = args.iter().position(|a| a == "--service").and_then(|j| args.get(j + 1)) {
                c.snap_service = s.clone();
            }
            let mut mem = memory::Memory::load(db_dir(), &c.target_lang);
            if let Some(entries) = sync::kept_pack_in(&data_dir(), &c.game(), &c.target_lang) {
                mem.set_shared(&c.target_lang, entries);
            }
            let lang = c.target_lang.clone();
            let lookup = |en: &str| mem.get(en, &[], &[]).filter(|(_, src)| src == "reviewed").map(|(segs, _)| translate::polite(&lang, segs));
            let mut tr = Translator::new(model_dir());
            let snap = snapshot::make(&frame, &[], snapshot::Tools { cfg: &c, tr: &mut tr, memory: &lookup }, "test".into())?;
            println!("{} block(s), {} ms, service {}", snap.blocks.len(), snap.ms, snap.service);
            for b in &snap.blocks {
                let t: String = b.segs.iter().map(|s| s.0.as_str()).collect();
                println!("[{:.3} {:.3} {:.3} {:.3}] {} line(s) {} fg {} bg {} | {} -> {}", b.rect[0], b.rect[1], b.rect[2], b.rect[3], b.lines, b.align, b.fg, b.bg, b.en, t);
            }
            if let Some(out) = args.iter().position(|a| a == "--json").and_then(|j| args.get(j + 1)) {
                std::fs::write(out, serde_json::to_string(&snap)?)?;
            }
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
        let Some(image) = args.get(i + 1) else {
            eprintln!("usage: --test-image <picture.png> [--area <id>]");
            std::process::exit(2);
        };
        if let Err(e) = test_image(image, area) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    let mut loaded = Config::load(&config_path());
    loaded.debug_save_frames = false; // recording always starts off
    let mut save_now = std::mem::take(&mut loaded.defaults_moved); // (a setting moved to a new default)
    if loaded.install_id.is_empty() {
        // a random id for this install (no account): lets the server count distinct players
        loaded.install_id = uuid::Uuid::new_v4().to_string();
        save_now = true;
    }
    if save_now {
        let _ = loaded.save(&config_path());
    }
    let cfg = Arc::new(Mutex::new(loaded));
    let saver = Arc::new(debug_frames::SaverState::default());
    let memory = Arc::new(Mutex::new(memory::Memory::load(db_dir(), &cfg.lock().unwrap().target_lang)));
    let shared = Arc::new(Shared::default());
    // the TVs connected to show the overlays, and their keys (their own file, not the settings)
    shared.tvs.load(data_dir().join("tvs.json"));
    let update: Arc<Mutex<Option<updater::Update>>> = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        .manage(AppState { cfg: cfg.clone(), shared: shared.clone(), saver: saver.clone(), memory: memory.clone(), update: update.clone() })
        .invoke_handler(tauri::generate_handler![app_version, dev_build, update_status, install_update, lan_info, open_lan_page, find_tvs, connect_tv, list_tvs, remove_tv, list_langs, list_sources, get_config, save_config, page_options, list_windows, list_fonts, get_screen, frame_saver_status, open_frames_folder, save_preview, open_previews_folder, take_snapshot, save_snapshot_render, open_snapshots_folder, memory_status, memory_clear])
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


/// "Press ⟦1⟧ to jump" with its icons -> ["Press ", icon 1, " to jump"]: the English line as segments,
/// each "⟦n⟧" the game's own button icon (as in the translation). A mark with no icon becomes 🎮.
fn english_segments(en: &str, icons: &[String]) -> Segments {
    let mut segs: Segments = Vec::new();
    let mut text = String::new();
    let mut rest = en;
    while let Some(open) = rest.find('⟦') {
        let after = &rest[open + '⟦'.len_utf8()..];
        let Some(close) = after.find('⟧') else { break };
        text.push_str(&rest[..open]);
        let icon = after[..close].parse::<usize>().ok().and_then(|n| icons.get(n.wrapping_sub(1)));
        match icon {
            Some(png) => {
                if !text.is_empty() {
                    segs.push((std::mem::take(&mut text), None));
                }
                segs.push((String::new(), Some(format!("icon:{png}"))));
            }
            None => text.push('🎮'),
        }
        rest = &after[close + '⟧'.len_utf8()..];
    }
    text.push_str(rest);
    if !text.is_empty() {
        segs.push((text, None));
    }
    segs
}

/// The game's coloured words (`phrases`: text and colour, in reading order) as coloured segments of the
/// English, as the translation has them: "Restore the engine" with "engine" in pink. Icon segments and
/// text already coloured stay as they are; a phrase not found in the text is skipped.
fn with_phrase_colors(segs: Segments, phrases: &[(String, String)]) -> Segments {
    let mut out = segs;
    for (phrase, color) in phrases {
        let phrase = phrase.trim();
        if phrase.is_empty() {
            continue;
        }
        let mut next: Segments = Vec::with_capacity(out.len() + 2);
        let mut done = false;
        for (text, c) in out {
            let at = if done || c.is_some() { None } else { text.find(phrase).or_else(|| text.to_lowercase().find(&phrase.to_lowercase()).filter(|&i| text.is_char_boundary(i) && text.is_char_boundary(i + phrase.len()))) };
            match at {
                Some(i) => {
                    let (before, rest) = text.split_at(i);
                    let (word, after) = rest.split_at(phrase.len());
                    if !before.is_empty() {
                        next.push((before.to_string(), None));
                    }
                    next.push((word.to_string(), Some(color.clone())));
                    if !after.is_empty() {
                        next.push((after.to_string(), None));
                    }
                    done = true;
                }
                None => next.push((text, c)),
            }
        }
        out = next;
    }
    out
}

/// "Press ⟦1⟧ to jump" -> "Press 🎮 to jump" (for showing English lines in the log).
fn strip_icon_tokens(s: &str) -> String {
    s.split_whitespace()
        .map(|t| if t.starts_with('⟦') && t.ends_with('⟧') { "🎮" } else { t })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod size_lock_tests {
    use super::*;

    fn measured(l: &mut SizeLock, kind: &str, v: f64) -> f64 {
        (0..SIZE_SAMPLES).map(|_| l.size(kind, v)).last().unwrap()
    }

    #[test]
    fn another_resolution_measures_again() {
        let mut l = SizeLock::default();
        assert!(!l.at_resolution(1920, 1080), "the first picture: nothing to forget");
        assert_eq!(measured(&mut l, "subtitles", 0.03), 0.03);
        assert_eq!(l.size("subtitles", 0.05), 0.03, "locked");
        // same resolution: kept
        assert!(!l.at_resolution(1920, 1080));
        assert_eq!(l.size("subtitles", 0.05), 0.03);
        // another one: measured again on the next lines
        assert!(l.at_resolution(1280, 720));
        assert!(l.locked.is_empty());
        assert_eq!(measured(&mut l, "subtitles", 0.04), 0.04);
        // no picture size (no frame yet): nothing changes
        assert!(!l.at_resolution(0, 0));
        assert_eq!(l.size("subtitles", 0.05), 0.04);
    }

    #[test]
    fn another_subtitle_size_in_the_game_measures_again() {
        let mut l = SizeLock::default();
        assert_eq!(measured(&mut l, "subtitles", 0.030), 0.030);
        // a line or two with short letters, or a big one: the locked size stays
        for v in [0.022, 0.023, 0.030, 0.040, 0.022, 0.031, 0.022, 0.023, 0.024] {
            assert_eq!(l.size("subtitles", v), 0.030, "{v}");
        }
        // off on both sides in turn: not a new size
        for v in [0.022, 0.040, 0.022, 0.040, 0.022, 0.040] {
            assert_eq!(l.size("subtitles", v), 0.030, "{v}");
        }
        assert_eq!(l.size("subtitles", 0.030), 0.030);
        // Large in the game's settings: five bigger lines in a row lock the new size
        for v in [0.037, 0.036, 0.038, 0.036] {
            assert_eq!(l.size("subtitles", v), 0.030);
        }
        assert_eq!(l.size("subtitles", 0.037), 0.037);
        assert_eq!(l.size("subtitles", 0.030), 0.037, "locked again");
        // and back to Small
        let small: Vec<f64> = [0.025, 0.026, 0.024, 0.025, 0.025].iter().map(|&v| l.size("subtitles", v)).collect();
        assert_eq!(small.last(), Some(&0.025));
        // other kinds of text keep their own size
        assert_eq!(measured(&mut l, "help", 0.02), 0.02);
        assert_eq!(l.size("subtitles", 0.024), 0.025);
    }

    #[test]
    fn sizes_saved_before_the_resolution_was_kept_stay() {
        let mut l: SizeLock = serde_json::from_str(r#"{"locked":{"subtitles":0.03}}"#).unwrap();
        assert!(!l.at_resolution(1920, 1080));
        assert_eq!(l.size("subtitles", 0.05), 0.03);
        assert_eq!(l.res, Some((1920, 1080)));
    }
}

#[cfg(test)]
mod english_icon_tests {
    use super::*;

    fn icon(n: &str) -> (String, Option<String>) {
        (String::new(), Some(format!("icon:{n}")))
    }

    #[test]
    fn the_english_keeps_its_icons() {
        let icons = vec!["1.1|data:image/png;base64,AAA".to_string(), "data:image/png;base64,BBB".to_string()];
        assert_eq!(
            english_segments("Press ⟦1⟧ to jump", &icons),
            vec![("Press ".to_string(), None), icon("1.1|data:image/png;base64,AAA"), (" to jump".to_string(), None)]
        );
        // at the start, at the end, side by side
        assert_eq!(english_segments("⟦2⟧ ⟦1⟧", &icons), vec![icon("data:image/png;base64,BBB"), (" ".to_string(), None), icon("1.1|data:image/png;base64,AAA")]);
        // a mark with no icon cut for it: the game pad, as in the text
        assert_eq!(english_segments("Hold ⟦3⟧", &icons), vec![("Hold 🎮".to_string(), None)]);
        // no marks: the line as it is
        assert_eq!(english_segments("Meet me at the docks.", &icons), vec![("Meet me at the docks.".to_string(), None)]);
        assert!(english_segments("", &icons).is_empty());
    }
}


/// The fixed words of a game page that the script or the layout's words didn't translate: each from
/// the translator (via `cache`, filled by MENU_TEXT jobs; `ask` queues one). Until translated, tabs,
/// buttons and the name block keep the English, and no title is drawn. Returns true while any is
/// missing.
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
    // (what the script, or the layout's words, already translated stays; the English until then)
    let untranslated = |ar: &str, en: &str| ar.is_empty() || ar == en;
    if !l.title_en.is_empty() && untranslated(&l.title_ar, &l.title_en) {
        l.title_ar = get(&l.title_en, &mut missing).unwrap_or_default();
    }
    for lb in l.labels.iter_mut() {
        if untranslated(&lb.ar, &lb.en) {
            lb.ar = get(&lb.en, &mut missing).unwrap_or_else(|| lb.en.clone());
        }
    }
    for b in l.buttons.iter_mut() {
        if untranslated(&b.ar, &b.en) {
            b.ar = get(&b.en, &mut missing).unwrap_or_else(|| b.en.clone());
        }
    }
    if let Some(p) = l.player.as_mut() {
        // line by line: what the script translated stays (a time or a number reads the same anyway)
        p.ar.resize(p.en.len(), String::new());
        for (a, e) in p.ar.iter_mut().zip(&p.en) {
            if untranslated(a, e) && e.chars().any(char::is_alphabetic) {
                *a = get(e, &mut missing).unwrap_or_else(|| e.clone());
            } else if a.is_empty() {
                *a = e.clone();
            }
        }
    }
    missing
}

/// Take the profile page down right away (the game's own page is gone).
fn hide_screen(app: &AppHandle, shared: &Shared, latest: &Mutex<HashMap<String, u64>>, screen: &mut Option<screens::ScreenLayout>) {
    *screen = None;
    // its reason may still be with the translator: translate_loop drops it when it comes
    latest.lock().unwrap().remove(SCREEN_REASON);
    shared.screen_active.store(false, std::sync::atomic::Ordering::Relaxed);
    *shared.screen.lock().unwrap() = None;
    shared.screen_private.lock().unwrap().clear();
    *shared.screen_reason.lock().unwrap() = serde_json::json!([]);
    let _ = app.emit("screen", None::<screens::ScreenLayout>);
}

#[cfg(test)]
mod english_color_tests {
    use super::*;

    #[test]
    fn the_games_coloured_words_are_coloured_in_the_english() {
        let segs = with_phrase_colors(english_segments("Restore the engine", &[]), &[("engine".into(), "#ff86bf".into())]);
        assert_eq!(segs, vec![("Restore the ".to_string(), None), ("engine".to_string(), Some("#ff86bf".to_string()))]);
    }

    #[test]
    fn icons_stay_and_a_phrase_not_found_is_skipped() {
        let icons = vec!["data:x".to_string()];
        let segs = with_phrase_colors(english_segments("Press ⟦1⟧ to follow Nate", &icons), &[("Nate".into(), "#ff86bf".into()), ("Lucia".into(), "#ff86bf".into())]);
        assert_eq!(segs, vec![("Press ".to_string(), None), (String::new(), Some("icon:data:x".to_string())), (" to follow ".to_string(), None), ("Nate".to_string(), Some("#ff86bf".to_string()))]);
    }
}
