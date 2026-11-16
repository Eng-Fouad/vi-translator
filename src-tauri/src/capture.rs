//! Picture sources: a game window (the platform's window capture, see capture_windows.rs /
//! capture_xcap.rs), a capture card (device.rs) or an OBS source (obs.rs), all feeding the same
//! shared frame slot.

use std::sync::atomic::{AtomicIsize, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::Config;

#[cfg_attr(windows, path = "capture_windows.rs")]
#[cfg_attr(not(windows), path = "capture_xcap.rs")]
mod platform;
pub use platform::{client_rect, find_window, foreground_window, grab_once, grab_screen_region, list_windows};

/// One captured frame of the window's client area, RGBA.
pub struct FrameData {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub seq: u64,
}

#[derive(Default)]
pub struct Shared {
    pub frame: Mutex<Option<Arc<FrameData>>>,
    /// HWND of the window being captured (0 = none).
    pub hwnd: AtomicIsize,
    pub seq: AtomicU64,
    pub status: Mutex<String>,
    /// A profile screen (e.g. GTA V "Mission Failed") is on screen; the full-window overlay shows it.
    pub screen_active: std::sync::atomic::AtomicBool,
    /// The profile screen currently shown (layout JSON) and its translated reason line, for windows
    /// that open after the screen appeared.
    pub screen: Mutex<Option<serde_json::Value>>,
    pub screen_reason: Mutex<serde_json::Value>,
    /// Area id -> {"en": ..., "ar": ...} currently shown by that area's overlay (for saved frames).
    pub on_screen: Mutex<std::collections::HashMap<String, serde_json::Value>>,
    /// Area id -> the full translation event now shown (segments, size, place), for the OBS page.
    pub shown: Mutex<std::collections::HashMap<String, serde_json::Value>>,
    /// Pause-menu text translated by the translator this session: English -> Arabic.
    pub menu_cache: Mutex<std::collections::HashMap<String, String>>,
    /// Set by the UI: measure the text sizes again.
    pub reset_sizes: std::sync::atomic::AtomicBool,
    /// Live preview, separate from the full frames read for text: a small frame (≤ 1280 wide) at
    /// the preview rate, made only while someone watches (`preview_clients`), and its JPEG.
    pub preview_raw: Mutex<Option<Arc<FrameData>>>,
    pub preview_jpeg: Mutex<Option<(u64, Arc<Vec<u8>>)>>,
    pub preview_seq: AtomicU64,
    pub preview_clients: AtomicUsize,
    /// the app window is minimised or hidden: no preview frames
    pub preview_paused: std::sync::atomic::AtomicBool,
    /// Recent translations for the home-network page (see lan.rs), newest last, and their ids.
    pub live_log: Mutex<std::collections::VecDeque<serde_json::Value>>,
    pub live_seq: AtomicU64,
    /// the target language for that page: {"code": "ar", "rtl": true}
    pub live_lang: Mutex<serde_json::Value>,
    /// the home-network page is being served
    pub lan_running: std::sync::atomic::AtomicBool,
    /// ms between full frames (for reading text) and between preview frames (0 = default)
    pub capture_ms: AtomicU64,
    pub preview_ms: AtomicU64,
}

/// Preview frames are at most this wide.
pub const PREVIEW_W: u32 = 1280;

impl Shared {
    pub fn latest(&self) -> Option<Arc<FrameData>> {
        self.frame.lock().unwrap().clone()
    }
    /// Time for the next full frame (the one text is read from)?
    pub fn full_due(&self, last: Instant) -> bool {
        last.elapsed() >= Duration::from_millis(self.capture_ms.load(Ordering::Relaxed).max(30))
    }
    /// Time for the next preview frame — and is anyone watching?
    pub fn preview_due(&self, last: Instant) -> bool {
        self.preview_clients.load(Ordering::Relaxed) > 0
            && !self.preview_paused.load(Ordering::Relaxed)
            // ¾ of the interval: frames arrive every ~16.7 ms, and a strict 33 ms test would often just
            // miss one and wait a whole extra frame (30 fps would become ~20)
            && last.elapsed() >= Duration::from_millis(self.preview_ms.load(Ordering::Relaxed).max(16) * 3 / 4)
    }
    /// The preview interval exactly (for sources we pull from, like OBS, rather than ones that push frames).
    pub fn preview_interval(&self) -> Duration {
        Duration::from_millis(self.preview_ms.load(Ordering::Relaxed).max(16))
    }
    /// Store a full frame.
    pub fn put_full(&self, w: u32, h: u32, rgba: Vec<u8>) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        *self.frame.lock().unwrap() = Some(Arc::new(FrameData { w, h, rgba, seq }));
    }
    /// Store a preview frame (small RGBA; the preview thread turns it into JPEG).
    pub fn put_preview(&self, f: FrameData) {
        *self.preview_raw.lock().unwrap() = Some(Arc::new(f));
    }
    /// Store a ready-made preview JPEG (OBS sends one).
    pub fn put_preview_jpeg(&self, jpeg: Vec<u8>) {
        let seq = self.preview_seq.fetch_add(1, Ordering::Relaxed) + 1;
        *self.preview_jpeg.lock().unwrap() = Some((seq, Arc::new(jpeg)));
    }
    pub fn set_status(&self, s: impl Into<String>) {
        *self.status.lock().unwrap() = s.into();
    }
}

/// Shrink a 4-byte-per-pixel image (rows `pitch` bytes apart) to ≤ PREVIEW_W wide, averaging 2×2
/// samples per output pixel. `bgr`: the source is BGRA/BGRX (output is always RGBA).
pub fn shrink(src: &[u8], w: u32, h: u32, pitch: usize, bgr: bool) -> FrameData {
    let f = w.div_ceil(PREVIEW_W).max(1);
    let (ow, oh) = (w / f, h / f);
    let mut out = vec![255u8; (ow * oh * 4) as usize];
    let half = (f / 2) as usize;
    let (r, b) = if bgr { (2, 0) } else { (0, 2) };
    for y in 0..oh as usize {
        let y0 = y * f as usize;
        let y1 = (y0 + half).min(h as usize - 1);
        for x in 0..ow as usize {
            let x0 = x * f as usize * 4;
            let x1 = ((x * f as usize + half).min(w as usize - 1)) * 4;
            let o = (y * ow as usize + x) * 4;
            for (c, sc) in [(0, r), (1, 1), (2, b)] {
                let sum = src[y0 * pitch + x0 + sc] as u32
                    + src[y0 * pitch + x1 + sc] as u32
                    + src[y1 * pitch + x0 + sc] as u32
                    + src[y1 * pitch + x1 + sc] as u32;
                out[o + c] = (sum / 4) as u8;
            }
        }
    }
    FrameData { w: ow, h: oh, rgba: out, seq: 0 }
}

/// Save a frame as PNG.
pub fn save_png(f: &FrameData, path: &std::path::Path) -> anyhow::Result<()> {
    let img = image::RgbaImage::from_raw(f.w, f.h, f.rgba.clone()).ok_or_else(|| anyhow::anyhow!("bad frame"))?;
    img.save(path)?;
    Ok(())
}

/// Keeps a capture running on the window named in the config; restarts it when the title changes
/// or the window is closed and reopened.
pub fn run_manager(shared: Arc<Shared>, cfg: Arc<Mutex<Config>>) {
    let mut active: Option<(String, platform::WindowCapture)> = None;
    let mut device: Option<(String, crate::device::DeviceCapture)> = None;
    let mut obs: Option<(String, crate::obs::ObsCapture)> = None;
    loop {
        let (want, obs_port, obs_password) = {
            let c = cfg.lock().unwrap();
            shared.capture_ms.store(1000 / c.capture_fps.clamp(1, 30) as u64, Ordering::Relaxed);
            shared.preview_ms.store(1000 / c.preview_fps.clamp(1, 60) as u64, Ordering::Relaxed);
            (c.window_title.clone(), c.obs_port, c.obs_password.clone())
        };
        // An OBS source ("obs:<name>"): OBS hands us snapshots of it
        if let Some(name) = want.strip_prefix(crate::obs::PREFIX) {
            if let Some((_, c)) = active.take() {
                let _ = c.stop();
            }
            if let Some((_, d)) = device.take() {
                d.stop();
            }
            let key = format!("{name}|{obs_port}|{obs_password}");
            if obs.as_ref().is_none_or(|(k, o)| *k != key || o.is_finished()) {
                if let Some((_, o)) = obs.take() {
                    o.stop();
                }
                shared.hwnd.store(0, Ordering::Relaxed);
                *shared.frame.lock().unwrap() = None;
                shared.set_status(format!("Connecting to OBS for \"{name}\" ..."));
                obs = Some((key, crate::obs::start(name.to_string(), obs_port, obs_password, shared.clone())));
            }
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        if let Some((_, o)) = obs.take() {
            o.stop();
            *shared.frame.lock().unwrap() = None;
        }
        // A capture card / video device ("device:<name>") instead of a window
        if let Some(name) = want.strip_prefix(crate::device::PREFIX) {
            if let Some((_, c)) = active.take() {
                let _ = c.stop();
            }
            let restart = device.as_ref().is_none_or(|(n, d)| n != name || d.is_finished());
            if restart {
                if let Some((_, d)) = device.take() {
                    d.stop();
                }
                shared.hwnd.store(0, Ordering::Relaxed); // no window: the Arabic goes to OBS / the preview
                *shared.frame.lock().unwrap() = None;
                shared.set_status(format!("Opening capture device \"{name}\" ..."));
                device = Some((name.to_string(), crate::device::start(name.to_string(), shared.clone())));
            }
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        if let Some((_, d)) = device.take() {
            d.stop();
            *shared.frame.lock().unwrap() = None;
        }
        let finished = active.as_ref().map(|(_, c)| c.is_finished()).unwrap_or(true);
        let changed = active.as_ref().map(|(t, _)| *t != want).unwrap_or(true);
        if changed || finished {
            if let Some((_, c)) = active.take() {
                let _ = c.stop();
            }
            shared.hwnd.store(0, Ordering::Relaxed);
            *shared.frame.lock().unwrap() = None;
            match platform::start_window(&want, shared.clone()) {
                Ok((c, hwnd)) => {
                    shared.hwnd.store(hwnd, Ordering::Relaxed);
                    shared.set_status(format!("Capturing \"{want}\""));
                    active = Some((want, c));
                }
                Err(e) => shared.set_status(e.to_string()),
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[derive(Serialize)]
pub struct WindowInfo {
    pub title: String,
    pub process: String,
}

