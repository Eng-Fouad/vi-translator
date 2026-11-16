//! Window capture with Windows Graphics Capture: captures only the chosen window, even while other
//! windows cover it, so nothing else on screen is ever read.

use std::sync::atomic::{AtomicIsize, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetForegroundWindow, IsIconic, IsWindow};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::config::Config;

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

type BoxError = Box<dyn std::error::Error + Send + Sync>;

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

struct Handler {
    shared: Arc<Shared>,
    last: Instant,
    last_preview: Instant,
    scratch: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Arc<Shared>;
    type Error = BoxError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let ago = Instant::now() - Duration::from_secs(1);
        Ok(Self { shared: ctx.flags, last: ago, last_preview: ago, scratch: Vec::new() })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
        // Full frames (a 4K frame is ~33 MB) only at the capture rate — plenty for reading text;
        // small preview frames at the (higher) preview rate, while the preview is watched.
        let full = self.shared.full_due(self.last);
        let preview = self.shared.preview_due(self.last_preview);
        if !full && !preview {
            return Ok(());
        }
        let mut buf = frame.buffer_without_title_bar()?;
        let (w, h) = (buf.width(), buf.height());
        if preview {
            self.last_preview = Instant::now();
            let pitch = buf.row_pitch() as usize;
            self.shared.put_preview(shrink(buf.as_raw_buffer(), w, h, pitch, false));
        }
        if full {
            self.last = Instant::now();
            let rgba = buf.as_nopadding_buffer(&mut self.scratch).to_vec();
            self.shared.put_full(w, h, rgba);
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Capture a single frame of a window (used by `--grab` for building and testing profiles).
pub fn grab_once(title: &str) -> anyhow::Result<Arc<FrameData>> {
    let win = Window::from_name(title).map_err(|_| anyhow::anyhow!("window \"{title}\" not found"))?;
    let shared = Arc::new(Shared::default());
    let settings = Settings::new(
        win,
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        shared.clone(),
    );
    let control = Handler::start_free_threaded(settings).map_err(|e| anyhow::anyhow!("capture failed: {e}"))?;
    let start = Instant::now();
    let frame = loop {
        if let Some(f) = shared.latest() {
            break Ok(f);
        }
        if start.elapsed() > Duration::from_secs(5) {
            break Err(anyhow::anyhow!("no frame within 5 s (is the window minimised?)"));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = control.stop();
    frame
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
    let mut active: Option<(String, CaptureControl<Handler, BoxError>)> = None;
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
            match Window::from_name(&want) {
                Ok(win) => {
                    let hwnd = win.as_raw_hwnd() as isize;
                    // Hide the yellow capture border where Windows allows it; fall back if refused.
                    let mut started = None;
                    for border in [DrawBorderSettings::WithoutBorder, DrawBorderSettings::Default] {
                        let settings = Settings::new(
                            win,
                            CursorCaptureSettings::WithoutCursor,
                            border,
                            SecondaryWindowSettings::Default,
                            MinimumUpdateIntervalSettings::Default,
                            DirtyRegionSettings::Default,
                            ColorFormat::Rgba8,
                            shared.clone(),
                        );
                        match Handler::start_free_threaded(settings) {
                            Ok(c) => {
                                started = Some(c);
                                break;
                            }
                            Err(e) => shared.set_status(format!("Capture failed: {e}")),
                        }
                    }
                    if let Some(c) = started {
                        shared.hwnd.store(hwnd, Ordering::Relaxed);
                        shared.set_status(format!("Capturing \"{want}\""));
                        active = Some((want, c));
                    }
                }
                Err(_) => shared.set_status(format!("Window \"{want}\" not found. Open it, or pick another window.")),
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

/// Visible top-level windows that can be captured (excluding this app's own windows).
pub fn list_windows(own_title: &str) -> Vec<WindowInfo> {
    let mut out: Vec<WindowInfo> = Window::enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter(|w| w.is_valid())
        .filter_map(|w| {
            let title = w.title().ok()?.trim().to_string();
            if title.is_empty() || title == own_title {
                return None;
            }
            // invisible system windows that only confuse the list
            let process = w.process_name().unwrap_or_default();
            if matches!(process.to_lowercase().as_str(), "textinputhost.exe" | "shellexperiencehost.exe" | "searchhost.exe" | "startmenuexperiencehost.exe")
                || title == "Program Manager"
            {
                return None;
            }
            Some(WindowInfo { title, process: w.process_name().unwrap_or_default() })
        })
        .collect();
    out.sort_by_key(|w| w.title.to_lowercase());
    out
}

/// Client area of a window in screen pixels: (x, y, width, height).
pub fn client_rect(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    if hwnd == 0 {
        return None;
    }
    let h = HWND(hwnd as *mut _);
    unsafe {
        if !IsWindow(Some(h)).as_bool() || IsIconic(h).as_bool() {
            return None;
        }
        let mut rc = RECT::default();
        GetClientRect(h, &mut rc).ok()?;
        let mut pt = POINT { x: 0, y: 0 };
        let _ = ClientToScreen(h, &mut pt);
        let (w, hh) = (rc.right - rc.left, rc.bottom - rc.top);
        (w > 50 && hh > 50).then_some((pt.x, pt.y, w, hh))
    }
}

pub fn foreground_window() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}


/// Copy a region of the screen (physical pixels) as RGBA — what the user actually sees there,
/// including our Arabic overlay windows on top of the game.
pub fn grab_screen_region(x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, bmp.into());
        let ok = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT).is_ok();
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(mem, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if !ok || lines == 0 {
            return None;
        }
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2); // BGRA -> RGBA
            px[3] = 255;
        }
        Some(buf)
    }
}
