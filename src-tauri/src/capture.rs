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
pub use platform::{client_rect, find_window, focused_client_rect, foreground_window, grab_once, grab_screen_region, list_windows};

/// One captured frame of the window's client area, RGBA.
pub struct FrameData {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub seq: u64,
}

/// Menu text translations, for one target language (the translator's answers in another are dropped).
#[derive(Default)]
pub struct MenuCache {
    pub lang: String,
    pub map: std::collections::HashMap<String, String>,
}

#[derive(Default)]
pub struct Shared {
    pub frame: Mutex<Option<Arc<FrameData>>>,
    /// HWND of the window being captured (0 = none).
    pub hwnd: AtomicIsize,
    pub seq: AtomicU64,
    pub status: Mutex<String>,
    /// A game page (a menu, a "mission failed" page …) is on screen; the full-window overlay shows it.
    pub screen_active: std::sync::atomic::AtomicBool,
    /// The profile screen currently shown (layout JSON) and its translated reason line, for windows
    /// that open after the screen appeared.
    pub screen: Mutex<Option<serde_json::Value>>,
    pub screen_reason: Mutex<serde_json::Value>,
    /// Area id -> {"en": ..., "ar": ...} currently shown by that area's overlay (for saved frames).
    pub on_screen: Mutex<std::collections::HashMap<String, serde_json::Value>>,
    /// Area id -> the full translation event now shown (segments, size, place), for the OBS page.
    pub shown: Mutex<std::collections::HashMap<String, serde_json::Value>>,
    /// Pause-menu text translated by the translator this session: English -> translation, in one language.
    pub menu_cache: Mutex<MenuCache>,
    /// …and the menu text it couldn't translate (asked again later).
    pub menu_failed: Mutex<Vec<String>>,
    /// Live preview, separate from the full frames read for text: a frame (≤ PREVIEW_W wide) at
    /// the preview rate, made only while someone watches (see `watching`), and its JPEG.
    pub preview_raw: Mutex<Option<Arc<FrameData>>>,
    pub preview_jpeg: Mutex<Option<(u64, Arc<Vec<u8>>)>>,
    /// Or, from a capture card that sends YUY2: the card's own frame as it came, no conversion or
    /// JPEG (the window turns it into colour on the GPU, the way OBS does). Numbered with `preview_seq`.
    pub preview_yuv: Mutex<Option<(u64, Arc<YuvFrame>)>>,
    pub preview_seq: AtomicU64,
    pub preview_clients: AtomicUsize,
    /// when the app's window last asked for a preview frame (ms since the app started)
    pub preview_pulled_ms: AtomicU64,
    /// the app window is minimised or hidden: no preview frames
    pub preview_paused: std::sync::atomic::AtomicBool,
    /// Recent translations for the home-network page (see lan.rs), newest last, and their ids.
    pub live_log: Mutex<std::collections::VecDeque<serde_json::Value>>,
    pub live_seq: AtomicU64,
    /// the target language for that page: {"code": "ar", "rtl": true}
    pub live_lang: Mutex<serde_json::Value>,
    /// the home-network page is being served
    pub lan_running: std::sync::atomic::AtomicBool,
    /// why the phone page couldn't start listening (shown in the window)
    pub lan_error: Mutex<Option<String>>,
    /// TVs connected to show the overlays (the VI Translator app on the TV), with their keys
    pub tvs: crate::tv::Tvs,
    /// the header's "Live translations" switch is off: the home-network page gets no lines meanwhile
    pub live_off: std::sync::atomic::AtomicBool,
    /// ms between full frames (for reading text) and between preview frames (0 = default)
    pub capture_ms: AtomicU64,
    pub preview_ms: AtomicU64,
}

/// Preview frames are at most this wide: a 1080p game is shown at full size, a 4K one at half.
pub const PREVIEW_W: u32 = 1920;

/// How a card's YUV numbers map to colour: which standard, and whether black is 16 or 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YuvColor {
    pub bt709: bool,
    pub full_range: bool,
}

/// A YUY2 frame: 2 bytes per pixel, each pair of pixels stored as Y0 U Y1 V, rows `w * 2` bytes.
pub struct YuvFrame {
    pub w: u32,
    pub h: u32,
    pub color: YuvColor,
    pub data: Vec<u8>,
}

/// YUY2 → RGBA (for reading text; the preview does the same on the GPU).
#[cfg_attr(not(windows), allow(dead_code))] // (the Windows capture card reader's)
pub fn yuy2_to_rgba(src: &[u8], w: u32, h: u32, c: YuvColor) -> Vec<u8> {
    // R = Y + kr·V, G = Y − kgu·U − kgv·V, B = Y + kb·U, in fixed point (×1024)
    let (kr, kgu, kgv, kb) = if c.bt709 { (1.5748, 0.1873, 0.4681, 1.8556) } else { (1.402, 0.3441, 0.7141, 1.772) };
    let (ys, yo, cs): (f64, i32, f64) = if c.full_range { (1.0, 0, 1.0) } else { (255.0 / 219.0, 16, 255.0 / 224.0) };
    let f = |k: f64| (k * cs * 1024.0).round() as i32;
    let (ym, cr, cgu, cgv, cb) = ((ys * 1024.0).round() as i32, f(kr), f(kgu), f(kgv), f(kb));
    let clamp = |v: i32| ((v + 512) >> 10).clamp(0, 255) as u8; // rounded
    let n = (w * h) as usize;
    let mut out = vec![0u8; n * 4];
    for (i, q) in src[..n * 2].chunks_exact(4).enumerate() {
        let (u, v) = (q[1] as i32 - 128, q[3] as i32 - 128);
        let (dr, dg, db) = (cr * v, cgu * u + cgv * v, cb * u);
        for (j, y) in [q[0], q[2]].into_iter().enumerate() {
            let y = (y as i32 - yo) * ym;
            let o = (i * 2 + j) * 4;
            out[o] = clamp(y + dr);
            out[o + 1] = clamp(y - dg);
            out[o + 2] = clamp(y + db);
            out[o + 3] = 255;
        }
    }
    out
}

/// Milliseconds since the app started (a steady clock for "when did X last happen").
fn app_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

impl Shared {
    pub fn latest(&self) -> Option<Arc<FrameData>> {
        self.frame.lock().unwrap().clone()
    }
    /// Time for the next full frame (the one text is read from)?
    pub fn full_due(&self, last: Instant) -> bool {
        last.elapsed() >= Duration::from_millis(self.capture_ms.load(Ordering::Relaxed).max(30))
    }
    /// Is anyone watching the preview: an open stream, or the window asked for a frame in the last 2 s?
    pub fn watching(&self) -> bool {
        self.preview_clients.load(Ordering::Relaxed) > 0
            || app_ms().saturating_sub(self.preview_pulled_ms.load(Ordering::Relaxed)) < 2000
                && self.preview_pulled_ms.load(Ordering::Relaxed) > 0
    }
    /// The window just asked for a preview frame.
    pub fn mark_preview_pulled(&self) {
        self.preview_pulled_ms.store(app_ms().max(1), Ordering::Relaxed);
    }
    /// Time for the next preview frame — and is anyone watching?
    pub fn preview_due(&self, last: Instant) -> bool {
        self.watching()
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
        // (every reader of a frame takes its size from w and h: an empty or short one is no frame)
        if w == 0 || h == 0 || rgba.len() < w as usize * h as usize * 4 {
            return;
        }
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        *self.frame.lock().unwrap() = Some(Arc::new(FrameData { w, h, rgba, seq }));
    }
    /// Store a preview frame (small RGBA; the preview thread turns it into JPEG).
    pub fn put_preview(&self, f: FrameData) {
        *self.preview_raw.lock().unwrap() = Some(Arc::new(f));
    }
    /// Store a raw YUY2 preview frame (a capture card's own picture).
    #[cfg_attr(not(windows), allow(dead_code))] // (the Windows capture card reader's)
    pub fn put_preview_yuv(&self, f: YuvFrame) {
        let seq = self.preview_seq.fetch_add(1, Ordering::Relaxed) + 1;
        *self.preview_yuv.lock().unwrap() = Some((seq, Arc::new(f)));
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
    if f == 1 {
        // already small enough: copy the rows as they are (RGBA out)
        let row = (w * 4) as usize;
        let mut out = Vec::with_capacity(row * h as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&src[y * pitch..y * pitch + row]);
        }
        for p in out.chunks_exact_mut(4) {
            if bgr {
                p.swap(0, 2);
            }
            p[3] = 255;
        }
        return FrameData { w, h, rgba: out, seq: 0 };
    }
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


#[cfg(test)]
mod yuv_tests {
    use super::*;

    #[test]
    fn yuy2_limited_bt709_colours() {
        let c = YuvColor { bt709: true, full_range: false };
        // white, black, then BT.709 limited-range red (Y 63, U 102, V 240)
        let px = |y: u8, u: u8, v: u8| yuy2_to_rgba(&[y, u, y, v], 2, 1, c)[..4].to_vec();
        assert_eq!(px(235, 128, 128), [255, 255, 255, 255]);
        assert_eq!(px(16, 128, 128), [0, 0, 0, 255]);
        let r = px(63, 102, 240);
        assert!(r[0] > 245 && r[1] < 10 && r[2] < 10, "red came out as {r:?}");
    }
}
