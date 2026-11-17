//! Window capture on Windows: Windows Graphics Capture captures only the chosen window, even while
//! other windows cover it, so nothing else on screen is ever read.

use std::sync::Arc;
use std::time::{Duration, Instant};

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

use super::{FrameData, Shared, WindowInfo, shrink};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A running window capture.
pub struct WindowCapture(CaptureControl<Handler, BoxError>);

impl WindowCapture {
    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
    pub fn stop(self) {
        let _ = self.0.stop();
    }
}

/// Start capturing the window titled `title`; returns the capture and the window's handle.
pub fn start_window(title: &str, shared: Arc<Shared>) -> anyhow::Result<(WindowCapture, isize)> {
    let win = Window::from_name(title).map_err(|_| anyhow::anyhow!("Window \"{title}\" not found. Open it, or pick another window."))?;
    let hwnd = win.as_raw_hwnd() as isize;
    // Hide the yellow capture border where Windows allows it; fall back if refused.
    let mut last_err = String::new();
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
            Ok(c) => return Ok((WindowCapture(c), hwnd)),
            Err(e) => last_err = format!("Capture failed: {e}"),
        }
    }
    Err(anyhow::anyhow!(last_err))
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

/// The window's client rect, only while it's the one in front.
pub fn focused_client_rect(hwnd: isize) -> Option<(i32, i32, i32, i32)> {
    if hwnd == 0 || foreground_window() != hwnd {
        return None;
    }
    client_rect(hwnd)
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


/// The handle of the window titled `title`.
pub fn find_window(title: &str) -> Option<isize> {
    Window::from_name(title).ok().map(|w| w.as_raw_hwnd() as isize)
}
