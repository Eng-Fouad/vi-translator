//! The Arabic as a web page for an OBS Browser Source: http://127.0.0.1:<port>/obs
//!
//! The page has a transparent background and draws only the Arabic (subtitles, instructions, the
//! Mission Failed page) where the overlays would, sized to the page — so stacked over the capture
//! card in an OBS scene it lands on the game picture. It polls /state ~10 times a second.
//! Listens on 127.0.0.1 only: nothing outside this PC can reach it.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::AppHandle;

use crate::capture::Shared;
use crate::config::Config;

pub(crate) fn respond(mut s: TcpStream, status: &str, mime: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Changes every time the app starts: the page reloads itself when it sees a new one, so an OBS
/// Browser Source always runs the current drawing code (it otherwise keeps the code it first loaded).
fn build_id() -> u64 {
    static ID: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *ID.get_or_init(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0))
}

fn state_json(cfg: &Mutex<Config>, shared: &Shared) -> Vec<u8> {
    let c = cfg.lock().unwrap().clone();
    let v = serde_json::json!({
        "build": build_id(),
        "areas": c.areas,
        "style": c.style,
        "profile_opts": c.profile_opts,
        "memory_tint": c.memory_tint,
        // the header's Overlays switch: off = draw nothing
        "overlays": c.overlays_enabled,
        "lang": c.target_lang,
        "shown": shared.shown.lock().unwrap().clone(),
        "screen": shared.screen.lock().unwrap().clone(),
        "reason": shared.screen_reason.lock().unwrap().clone(),
        // for troubleshooting
        "status": shared.status.lock().unwrap().clone(),
        "frame": shared.latest().map(|f| [f.w, f.h, f.seq as u32]),
        "preview": {
            "clients": shared.preview_clients.load(std::sync::atomic::Ordering::Relaxed),
            "paused": shared.preview_paused.load(std::sync::atomic::Ordering::Relaxed),
            "raw": shared.preview_raw.lock().unwrap().as_ref().map(|f| [f.w, f.h]),
            "jpeg_seq": shared.preview_seq.load(std::sync::atomic::Ordering::Relaxed),
        },
    });
    serde_json::to_vec(&v).unwrap_or_default()
}

fn handle(app: &AppHandle, cfg: &Mutex<Config>, shared: &Shared, s: TcpStream) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let mut line = String::new();
    if BufReader::new(&s).read_line(&mut line).is_err() {
        return;
    }
    // "GET /path?query HTTP/1.1"
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = path.to_string();
    match path.as_str() {
        "/state" => respond(s, "200 OK", "application/json; charset=utf-8", &state_json(cfg, shared)),
        "/preview" | "/preview.jpg" => preview_frame(shared, s, query),
        "/preview.mjpg" => stream_preview(shared, s),
        "/" | "/obs" => serve_asset(app, s, "obs.html"),
        p if !p.contains("..") => serve_asset(app, s, p.trim_start_matches('/')),
        _ => respond(s, "404 Not Found", "text/plain", b"not found"),
    }
}

/// The app's live preview, one frame per request: `/preview?after=<seq>` answers with the newest
/// preview frame once there is one other than `seq` (waiting up to a second; 204 if none came), its
/// number in `X-Seq`. The window asks for the next frame only after showing this one, so frames can
/// never queue up on the way and the picture is never behind (an MJPEG stream, by contrast, keeps
/// sending, and if the window draws a little slower the extra frames pile up in the connection).
///
/// The frame is either a JPEG, or — from a capture card sending YUY2 — the card's own frame, raw
/// (`application/x-yuy2`, size and colour standard in headers), which the window turns into colour
/// on the GPU like OBS does: no encoding or decoding on the way.
fn preview_frame(shared: &Shared, mut s: TcpStream, query: &str) {
    let after: u64 = query.split('&').find_map(|kv| kv.strip_prefix("after=")).and_then(|v| v.parse().ok()).unwrap_or(0);
    let start = std::time::Instant::now();
    loop {
        shared.mark_preview_pulled();
        let jpeg = shared.preview_jpeg.lock().unwrap().clone();
        let yuv = shared.preview_yuv.lock().unwrap().clone();
        // the newer of the two (the source may have changed from one kind to the other)
        let yuv_newer = match (&jpeg, &yuv) {
            (Some((j, _)), Some((y, _))) => y > j,
            (None, Some(_)) => true,
            _ => false,
        };
        let expose = "Access-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: X-Seq, X-Width, X-Height, X-Matrix, X-Range";
        if yuv_newer {
            if let Some((seq, f)) = yuv.filter(|(seq, _)| *seq != after) {
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-yuy2\r\nContent-Length: {}\r\nX-Seq: {seq}\r\nX-Width: {}\r\nX-Height: {}\r\nX-Matrix: {}\r\nX-Range: {}\r\nCache-Control: no-store\r\n{expose}\r\nConnection: close\r\n\r\n",
                    f.data.len(),
                    f.w,
                    f.h,
                    if f.color.bt709 { "709" } else { "601" },
                    if f.color.full_range { "full" } else { "limited" },
                );
                let _ = s.set_nodelay(true);
                let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(&f.data));
                return;
            }
        } else if let Some((seq, jpeg)) = jpeg.filter(|(seq, _)| *seq != after) {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nX-Seq: {seq}\r\nCache-Control: no-store\r\n{expose}\r\nConnection: close\r\n\r\n",
                jpeg.len()
            );
            let _ = s.set_nodelay(true);
            let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(&jpeg));
            return;
        }
        if start.elapsed() >= Duration::from_secs(1) {
            return respond(s, "204 No Content", "text/plain", b"");
        }
        std::thread::sleep(Duration::from_millis(3));
    }
}

/// The live preview as an MJPEG stream (an <img> plays it natively): each new preview JPEG is
/// sent as one part. Preview frames are only made while at least one stream is open.
fn stream_preview(shared: &Shared, mut s: TcpStream) {
    use std::sync::atomic::Ordering;
    let head = "HTTP/1.1 200 OK
Content-Type: multipart/x-mixed-replace; boundary=frame
Cache-Control: no-store
Access-Control-Allow-Origin: *
Connection: close

";
    if s.write_all(head.as_bytes()).is_err() {
        return;
    }
    let _ = s.set_nodelay(true);
    shared.preview_clients.fetch_add(1, Ordering::Relaxed);
    let mut sent = 0u64;
    let mut sent_at = std::time::Instant::now();
    loop {
        // a new frame — or, when the picture hasn't changed for a second (a still game screen), the
        // same one again: keeps the picture there and notices a closed viewer
        let next = shared
            .preview_jpeg
            .lock()
            .unwrap()
            .clone()
            .filter(|(seq, _)| *seq != sent || sent_at.elapsed() >= Duration::from_secs(1));
        match next {
            Some((seq, jpeg)) => {
                sent = seq;
                sent_at = std::time::Instant::now();
                let part = format!("--frame
Content-Type: image/jpeg
Content-Length: {}

", jpeg.len());
                if s.write_all(part.as_bytes()).and_then(|_| s.write_all(&jpeg)).and_then(|_| s.write_all(b"
")).is_err() {
                    break; // the picture was closed
                }
            }
            None => std::thread::sleep(Duration::from_millis(4)),
        }
    }
    shared.preview_clients.fetch_sub(1, Ordering::Relaxed);
}

/// Files of the app's own UI (the page, common.js, fonts), from inside the executable.
pub(crate) fn serve_asset(app: &AppHandle, s: TcpStream, path: &str) {
    match app.asset_resolver().get(path.to_string()) {
        Some(a) => respond(s, "200 OK", a.mime_type(), a.bytes()),
        None => respond(s, "404 Not Found", "text/plain", b"not found"),
    }
}

pub fn run(app: AppHandle, cfg: Arc<Mutex<Config>>, shared: Arc<Shared>) {
    let port = cfg.lock().unwrap().obs_page_port;
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("OBS page: port {port} unavailable: {e}");
            return;
        }
    };
    for s in listener.incoming().flatten() {
        let (app, cfg, shared) = (app.clone(), cfg.clone(), shared.clone());
        std::thread::spawn(move || handle(&app, &cfg, &shared, s));
    }
}
