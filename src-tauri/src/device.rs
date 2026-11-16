//! Capture cards and other video devices (UVC) through Media Foundation.
//!
//! The card's picture is the whole game frame (no window, no title bar), read at ~8 frames per
//! second into the same `Shared` frame slot window capture uses, so OCR, profiles and frame saving
//! work unchanged. Windows' Frame Server lets another app (OBS) keep the same card open.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::core::{Interface, PWSTR};

use crate::capture::{FrameData, Shared};

/// Prefix that marks a device (not a window) in the config's source field.
pub const PREFIX: &str = "device:";

fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    ONCE.call_once(|| unsafe {
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    });
}

fn devices() -> Result<Vec<(String, IMFActivate)>> {
    init();
    unsafe {
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 1)?;
        let attrs = attrs.context("no attributes")?;
        attrs.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut n = 0u32;
        MFEnumDeviceSources(&attrs, &mut list, &mut n)?;
        let mut out = Vec::new();
        for i in 0..n as usize {
            if let Some(act) = (*list.add(i)).take() {
                let mut name = PWSTR::null();
                let mut len = 0u32;
                if act.GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut name, &mut len).is_ok() {
                    out.push((name.to_string().unwrap_or_default(), act));
                    CoTaskMemFree(Some(name.0 as *const _));
                }
            }
        }
        CoTaskMemFree(Some(list as *const _));
        Ok(out)
    }
}

/// Names of the video capture devices (capture cards, webcams).
pub fn list() -> Vec<String> {
    devices().map(|d| d.into_iter().map(|(n, _)| n).collect()).unwrap_or_default()
}

const STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

fn size_of(t: &IMFMediaType) -> (u32, u32) {
    let v = unsafe { t.GetUINT64(&MF_MT_FRAME_SIZE) }.unwrap_or(0);
    ((v >> 32) as u32, v as u32)
}

fn fps_of(t: &IMFMediaType) -> f64 {
    let v = unsafe { t.GetUINT64(&MF_MT_FRAME_RATE) }.unwrap_or(0);
    let (n, d) = ((v >> 32) as f64, (v as u32) as f64);
    if d > 0.0 { n / d } else { 0.0 }
}

/// Open the device and set it up to hand us RGB32 frames. Prefers 1920×1080 (plenty for reading
/// text, and what most cards stream), then the highest frame rate, then uncompressed formats.
/// `pick = false` keeps the card's current format (needed while another app is streaming it).
fn open(name: &str, pick: bool) -> Result<(IMFSourceReader, u32, u32)> {
    let act = devices()?
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, a)| a)
        .ok_or_else(|| anyhow!("capture device \"{name}\" not found (unplugged?)"))?;
    unsafe {
        let source: IMFMediaSource = act.ActivateObject().context("could not open the device (in use by another app?)")?;
        let mut attrs: Option<IMFAttributes> = None;
        MFCreateAttributes(&mut attrs, 2)?;
        let attrs = attrs.context("no attributes")?;
        // let the reader decode MJPG / convert NV12, YUY2 … to RGB32 for us
        attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
        let reader = MFCreateSourceReaderFromMediaSource(&source, &attrs)?;

        let mut best: Option<(i64, IMFMediaType)> = None;
        for i in 0.. {
            let Ok(t) = reader.GetNativeMediaType(STREAM, i) else { break };
            let (w, h) = size_of(&t);
            let fps = fps_of(&t).min(60.0);
            let compressed = t.GetGUID(&MF_MT_SUBTYPE).is_ok_and(|g| g == MFVideoFormat_MJPG);
            let area = (w as i64) * (h as i64);
            let size_score = if area <= 1920 * 1080 { area } else { 1920 * 1080 - (area - 1920 * 1080) / 4 };
            let score = size_score * 1000 + (fps * 10.0) as i64 + if compressed { 0 } else { 5 };
            if best.as_ref().is_none_or(|(s, _)| score > *s) {
                best = Some((score, t));
            }
        }
        // Another app (OBS) may already be streaming the card in its own format: then keep that one.
        if let (true, Some((_, t))) = (pick, &best) {
            let _ = reader.SetCurrentMediaType(STREAM, None, t);
        }
        if std::env::var_os("GTA6_TRANSLATOR_DEBUG").is_some() {
            let cur = reader.GetCurrentMediaType(STREAM)?;
            let (w, h) = size_of(&cur);
            eprintln!("[device] pick {pick}: {w}x{h} @ {:.1} fps, subtype {:?}", fps_of(&cur), cur.GetGUID(&MF_MT_SUBTYPE));
        }
        let cur = reader.GetCurrentMediaType(STREAM)?;
        let (w, h) = size_of(&cur);
        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        out.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
        reader.SetCurrentMediaType(STREAM, None, &out).context("the device's picture can't be converted to RGB")?;
        let cur = reader.GetCurrentMediaType(STREAM)?;
        let (w, h) = size_of(&cur);
        if w == 0 || h == 0 {
            return Err(anyhow!("the device reports no picture size"));
        }
        Ok((reader, w, h))
    }
}

/// Copy one RGB32 sample out as top-down BGRX rows (w × 4 bytes each).
fn to_bgrx(sample: &IMFSample, w: u32, h: u32) -> Result<Vec<u8>> {
    unsafe {
        let buf = sample.ConvertToContiguousBuffer()?;
        let mut out = vec![0u8; (w * h * 4) as usize];
        let row = (w * 4) as usize;
        let copy_rows = |scan0: *const u8, pitch: isize, out: &mut [u8]| {
            for y in 0..h as usize {
                let src = std::slice::from_raw_parts(scan0.offset(pitch * y as isize), row);
                out[y * row..(y + 1) * row].copy_from_slice(src);
            }
        };
        if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
            let (mut scan0, mut pitch) = (std::ptr::null_mut(), 0i32);
            b2.Lock2D(&mut scan0, &mut pitch)?;
            copy_rows(scan0, pitch as isize, &mut out);
            let _ = b2.Unlock2D();
        } else {
            let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
            buf.Lock(&mut p, None, Some(&mut len))?;
            if (len as usize) < row * h as usize {
                let _ = buf.Unlock();
                return Err(anyhow!("short frame"));
            }
            copy_rows(p, row as isize, &mut out);
            let _ = buf.Unlock();
        }
        Ok(out)
    }
}

/// A running device capture (a reader thread).
pub struct DeviceCapture {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl DeviceCapture {
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
    pub fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.handle.join();
    }
}

/// Start reading `name` into `shared`. Its status gets "Capturing …" or the error.
pub fn start(name: String, shared: Arc<Shared>) -> DeviceCapture {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let handle = std::thread::spawn(move || {
        let status = &shared.status;
        init();
        // First in our preferred format; if the card won't start streaming in it (another app such as
        // OBS has it running in a different format), again in whatever format it is already sending.
        let mut opened = None;
        let mut why = String::new();
        for pick in [true, false] {
            match open(&name, pick) {
                Ok((reader, w, h)) => {
                    let (mut flags, mut sample) = (0u32, None);
                    match unsafe { reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample)) } {
                        Ok(()) if flags & MF_SOURCE_READERF_ERROR.0 as u32 == 0 => {
                            opened = Some((reader, w, h));
                            break;
                        }
                        Ok(()) => why = format!("flags {flags:#x}"),
                        Err(e) => why = format!("{e}"),
                    }
                }
                Err(e) => why = format!("{e:#}"),
            }
        }
        let Some((reader, w, h)) = opened else {
            if why.contains("0xC00D3704") {
                why = "another app is using it (OBS's Video Capture Device keeps it to itself) — pick the card's OBS source (📺) instead".into();
            }
            *status.lock().unwrap() = format!("Capture device \"{name}\": {why}");
            std::thread::sleep(Duration::from_secs(2)); // the manager retries after this
            return;
        };
        *status.lock().unwrap() = format!("Capturing device \"{name}\" ({w}×{h})");
        let ago = Instant::now() - Duration::from_secs(1);
        let (mut last, mut last_preview) = (ago, ago);
        while !s2.load(Ordering::Relaxed) {
            let (mut flags, mut sample) = (0u32, None);
            // Read every frame the card sends (so the newest one is never stale); keep full frames at
            // the capture rate and small ones at the preview rate.
            let r = unsafe { reader.ReadSample(STREAM, 0, None, Some(&mut flags), None, Some(&mut sample)) };
            if r.is_err() || flags & (MF_SOURCE_READERF_ERROR.0 | MF_SOURCE_READERF_ENDOFSTREAM.0) as u32 != 0 {
                let why = match &r {
                    Err(e) => format!("{e}"),
                    Ok(()) => format!("flags {flags:#x}"),
                };
                *status.lock().unwrap() = format!("Capture device \"{name}\" stopped sending frames ({why})");
                break;
            }
            let Some(sample) = sample else { continue };
            let (full, preview) = (shared.full_due(last), shared.preview_due(last_preview));
            if !full && !preview {
                continue;
            }
            let Ok(mut px) = to_bgrx(&sample, w, h) else { continue };
            if preview {
                last_preview = Instant::now();
                shared.put_preview(crate::capture::shrink(&px, w, h, (w * 4) as usize, true));
            }
            if full {
                last = Instant::now();
                for p in px.chunks_exact_mut(4) {
                    p.swap(0, 2); // BGRX -> RGBA
                    p[3] = 255;
                }
                shared.put_full(w, h, px);
            }
        }
    });
    DeviceCapture { stop, handle }
}

/// One frame from a device (for `--grab-device`).
pub fn grab_once(name: &str) -> Result<Arc<FrameData>> {
    let shared = Arc::new(Shared::default());
    let cap = start(name.to_string(), shared.clone());
    let t = Instant::now();
    let f = loop {
        if let Some(f) = shared.latest() {
            break Ok(f);
        }
        if cap.is_finished() || t.elapsed() > Duration::from_secs(8) {
            let s = shared.status.lock().unwrap().clone();
            break Err(anyhow!("no frame from the device: {s}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    cap.stop();
    f
}
