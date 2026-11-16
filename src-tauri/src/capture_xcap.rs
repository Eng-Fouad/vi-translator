//! Window capture on Linux and macOS, with the xcap crate (X11 / PipeWire on Linux,
//! ScreenCaptureKit / CoreGraphics on macOS).
//!
//! A thread grabs the chosen window at the capture rate (and small preview frames at the preview
//! rate while the preview is watched). Window "handles" are xcap's window ids.
//!
//! Limits: on Linux Wayland, other apps' windows can't be listed or captured; use an OBS source
//! or a capture card there (X11 and XWayland windows work). On macOS the app needs the Screen
//! Recording permission (System Settings > Privacy & Security).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use xcap::Window;

use super::{FrameData, Shared, WindowInfo, shrink};

/// A running window capture.
pub struct WindowCapture {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WindowCapture {
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn find(title: &str) -> Option<Window> {
    let all = Window::all().ok()?;
    let exact = all.iter().position(|w| w.title().ok().as_deref() == Some(title));
    let idx = exact.or_else(|| all.iter().position(|w| w.title().is_ok_and(|t| !t.is_empty() && t.contains(title))))?;
    all.into_iter().nth(idx)
}

fn by_id(id: isize) -> Option<Window> {
    Window::all().ok()?.into_iter().find(|w| w.id().ok().map(|i| i as isize) == Some(id))
}

/// Start capturing the window titled `title`; returns the capture and the window's id.
pub fn start_window(title: &str, shared: Arc<Shared>) -> Result<(WindowCapture, isize)> {
    let win = find(title).ok_or_else(|| anyhow!("Window \"{title}\" not found. Open it, or pick another window."))?;
    let id = win.id().map_err(|e| anyhow!("Capture failed: {e}"))? as isize;
    // one frame now: fails fast when capture isn't allowed (macOS Screen Recording permission)
    win.capture_image().map_err(|e| anyhow!("Capture failed: {e}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::spawn(move || {
        let ago = Instant::now() - Duration::from_secs(1);
        let (mut last, mut last_preview) = (ago, ago);
        let mut misses = 0;
        while !flag.load(Ordering::Relaxed) {
            let full = shared.full_due(last);
            let preview = shared.preview_due(last_preview);
            if full || preview {
                match win.capture_image() {
                    Ok(img) => {
                        misses = 0;
                        let (w, h) = img.dimensions();
                        if preview {
                            last_preview = Instant::now();
                            shared.put_preview(shrink(img.as_raw(), w, h, (w * 4) as usize, false));
                        }
                        if full {
                            last = Instant::now();
                            shared.put_full(w, h, img.into_raw());
                        }
                    }
                    // the window closed (or can't be read any more): end, so the manager restarts
                    Err(_) => {
                        misses += 1;
                        if misses > 20 {
                            break;
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    });
    Ok((WindowCapture { stop, thread: Some(thread) }, id))
}

/// Capture a single frame of a window (used by `--grab` for building and testing profiles).
pub fn grab_once(title: &str) -> Result<Arc<FrameData>> {
    let win = find(title).ok_or_else(|| anyhow!("window \"{title}\" not found"))?;
    let img = win.capture_image().map_err(|e| anyhow!("capture failed: {e}"))?;
    let (w, h) = img.dimensions();
    Ok(Arc::new(FrameData { w, h, rgba: img.into_raw(), seq: 1 }))
}

/// Visible windows that can be captured (excluding this app's own windows).
pub fn list_windows(own_title: &str) -> Vec<WindowInfo> {
    let mut out: Vec<WindowInfo> = Window::all()
        .unwrap_or_default()
        .into_iter()
        .filter(|w| !w.is_minimized().unwrap_or(false))
        .filter_map(|w| {
            let title = w.title().ok()?.trim().to_string();
            if title.is_empty() || title == own_title || title.starts_with("VI Translator overlay") {
                return None;
            }
            Some(WindowInfo { title, process: w.app_name().unwrap_or_default() })
        })
        .collect();
    out.sort_by_key(|w| w.title.to_lowercase());
    out.dedup_by(|a, b| a.title == b.title);
    out
}

/// The window's area on screen: (x, y, width, height).
pub fn client_rect(id: isize) -> Option<(i32, i32, i32, i32)> {
    if id == 0 {
        return None;
    }
    let w = by_id(id)?;
    if w.is_minimized().unwrap_or(false) {
        return None;
    }
    let (ww, hh) = (w.width().ok()? as i32, w.height().ok()? as i32);
    (ww > 50 && hh > 50).then_some((w.x().ok()?, w.y().ok()?, ww, hh))
}

/// The id of the focused window (0 if none is known).
pub fn foreground_window() -> isize {
    Window::all()
        .ok()
        .and_then(|all| all.into_iter().find(|w| w.is_focused().unwrap_or(false)))
        .and_then(|w| w.id().ok())
        .map(|i| i as isize)
        .unwrap_or(0)
}

/// The id of the window titled `title`.
pub fn find_window(title: &str) -> Option<isize> {
    find(title).and_then(|w| w.id().ok()).map(|i| i as isize)
}

/// Copy a region of the screen as RGBA (what the user sees there, overlays included).
pub fn grab_screen_region(x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let m = xcap::Monitor::from_point(x, y).ok()?;
    let (mx, my) = (m.x().ok()?, m.y().ok()?);
    let img = m.capture_region((x - mx).max(0) as u32, (y - my).max(0) as u32, w as u32, h as u32).ok()?;
    Some(img.into_raw())
}
