//! Live translations on other devices: a small read-only web page on the home network
//! (http://<this PC>:<port>/), for a phone, tablet or another computer, and the overlays for a TV
//! (http://<this PC>:<port>/tv, drawn by the VI Translator app on the TV over the game picture).
//!
//! Off by default. While "lan_page" is on, it listens on all addresses but serves only the pages,
//! their fonts and drawing code, the list of recent translations and what the overlays show now:
//! never settings beyond the overlays' own places and style, the picture or the OBS page (those
//! stay on the 127.0.0.1-only server in obs_page.rs).

use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::AppHandle;

use crate::capture::Shared;
use crate::config::Config;
use crate::obs_page::{read_request, respond, serve_asset, Slot};

/// How many recent lines the page can fetch.
const KEEP: usize = 200;

/// Remember a line that was just shown, for the page.
pub fn push(shared: &Shared, area: &str, en: &str, segments: &[(String, Option<String>)], at_ms: u64) {
    // "Live translations" switched off: the list pauses (and switching it back on doesn't dump what came
    // in meanwhile). Independent of the overlays: with this on, the list keeps filling with them off.
    if shared.live_off.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let text: String = segments
        .iter()
        .map(|(t, c)| if c.as_deref().is_some_and(|c| c.starts_with("icon:")) { "🎮" } else { t.as_str() })
        .collect();
    if text.trim().is_empty() {
        return;
    }
    let id = shared.live_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let mut log = shared.live_log.lock().unwrap();
    log.push_back(serde_json::json!({ "id": id, "at": at_ms, "area": area, "en": en, "text": text.trim() }));
    while log.len() > KEEP {
        log.pop_front();
    }
}

/// This PC's address on the home network (the one used to reach the internet; no packet is sent).
pub fn lan_ip() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

#[derive(Serialize)]
pub struct LanInfo {
    pub enabled: bool,
    pub running: bool,
    pub url: Option<String>,
    /// QR code of the address, as SVG
    pub qr: Option<String>,
    /// why the page isn't running although it's on (the port is taken …)
    pub error: Option<String>,
}

pub fn info(c: &Config, shared: &Shared) -> LanInfo {
    let running = shared.lan_running.load(std::sync::atomic::Ordering::Relaxed);
    let url = (c.lan_page && running).then(|| lan_ip().map(|ip| format!("http://{ip}:{}/", c.lan_port))).flatten();
    let qr = url.as_ref().and_then(|u| {
        let code = qrcode::QrCode::new(u.as_bytes()).ok()?;
        Some(code.render::<qrcode::render::svg::Color>().min_dimensions(180, 180).quiet_zone(true).build())
    });
    let error = (c.lan_page && !running).then(|| shared.lan_error.lock().unwrap().clone()).flatten();
    LanInfo { enabled: c.lan_page, running, url, qr, error }
}

/// Runs for the app's lifetime: starts and stops the listener as the setting changes.
pub fn run(app: AppHandle, cfg: Arc<Mutex<Config>>, shared: Arc<Shared>) {
    use std::sync::atomic::Ordering;
    let mut listener: Option<(TcpListener, u16)> = None;
    loop {
        let (on, port) = {
            let c = cfg.lock().unwrap();
            shared.live_off.store(!c.live_translations, Ordering::Relaxed);
            (c.lan_page, c.lan_port)
        };
        match (&listener, on) {
            (Some((_, p)), true) if *p == port => {}
            (_, true) => {
                listener = match TcpListener::bind(("0.0.0.0", port)) {
                    Ok(l) => {
                        let _ = l.set_nonblocking(true);
                        *shared.lan_error.lock().unwrap() = None;
                        Some((l, port))
                    }
                    // (shown in the window: "Starting…" forever would hide it)
                    Err(e) => {
                        *shared.lan_error.lock().unwrap() = Some(format!("port {port}: {e}"));
                        None
                    }
                };
            }
            (Some(_), false) => listener = None,
            (None, false) => {}
        }
        shared.lan_running.store(listener.is_some(), Ordering::Relaxed);
        match &listener {
            Some((l, _)) => {
                // accept everything waiting, then look at the setting again
                static ACTIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                while let Ok((s, _)) = l.accept() {
                    // (too many at once: closed)
                    let Some(slot) = Slot::take(&ACTIVE) else { continue };
                    let _ = s.set_nonblocking(false);
                    let (a, c, sh) = (app.clone(), cfg.clone(), shared.clone());
                    std::thread::spawn(move || {
                        let _slot = slot;
                        handle(&a, &c, &sh, s);
                    });
                }
                std::thread::sleep(Duration::from_millis(60));
            }
            None => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

fn handle(app: &AppHandle, cfg: &Mutex<Config>, shared: &Shared, s: TcpStream) {
    let Some(req) = read_request(&s) else { return };
    // Asked for by an address (this PC's, as the phone opens it), not a name: a website can't point a
    // name of its own at this PC and read the page from the browser ("DNS rebinding").
    if !host_is_address(&req.host) {
        return respond(s, "403 Forbidden", "text/plain", b"not allowed");
    }
    let (path, query) = (req.path.as_str(), req.query.as_str());
    match path {
        "/" | "/live" => serve_asset(app, s, "live.html"),
        "/live.json" => {
            let after: u64 = query.split('&').find_map(|kv| kv.strip_prefix("after=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let lines: Vec<serde_json::Value> =
                shared.live_log.lock().unwrap().iter().filter(|l| l["id"].as_u64().unwrap_or(0) > after).cloned().collect();
            let live = !shared.live_off.load(std::sync::atomic::Ordering::Relaxed);
            // ("run": new each time the app starts; line ids start again then, and the page with it)
            let body = serde_json::json!({ "run": crate::obs_page::build_id(), "lang": shared.live_lang.lock().unwrap().clone(), "lines": lines, "live": live });
            respond(s, "200 OK", "application/json; charset=utf-8", body.to_string().as_bytes());
        }
        // the TV app's page: the overlays over the whole picture, as on the OBS page, for a TV
        // connected in the main window only (its key in t=)
        "/tv" | "/tv.json" | "/tv/forget" if !shared.tvs.allowed(param(query, "t")) => {
            respond(s, "403 Forbidden", "text/plain", b"not connected")
        }
        "/tv" => serve_asset(app, s, "tv.html"),
        "/tv.json" => tv_state(cfg, shared, s, param(query, "h")),
        // the TV disconnected itself: its key is forgotten
        "/tv/forget" => {
            shared.tvs.remove_token(param(query, "t"));
            respond(s, "200 OK", "text/plain", b"ok")
        }
        // a Samsung TV showing a code offers it here (it can't be called: see tv.rs); only the
        // TV app sends X-VIT (a web page can't, and gets nothing)
        "/vit-tv/offer" if req.method == "POST" && req.vit => {
            let (status, body) = shared.tvs.offer(&req.body);
            let status = match status {
                200 => "200 OK",
                400 => "400 Bad Request",
                403 => "403 Forbidden",
                429 => "429 Too Many Requests",
                _ => "404 Not Found",
            };
            respond(s, status, "application/json", body.as_bytes())
        }
        "/common.js" => serve_asset(app, s, "common.js"),
        // the pages' fonts and nothing else
        p if p.starts_with("/fonts/") && !p.contains("..") || p == "/fonts.css" => serve_asset(app, s, p.trim_start_matches('/')),
        _ => respond(s, "404 Not Found", "text/plain", b"not found"),
    }
}

/// A query parameter's value ("" when it isn't there).
fn param<'a>(query: &'a str, name: &str) -> &'a str {
    query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v)).unwrap_or("")
}

/// What the overlays show now, for the TV page, with its version in `X-State`: when the page
/// already has this version (`seen`), the answer waits up to a second for a change (a new line is
/// on the TV at once, and nothing is sent while nothing changes), then is 204 if there was none.
fn tv_state(cfg: &Mutex<Config>, shared: &Shared, mut s: TcpStream, seen: &str) {
    use std::hash::{Hash, Hasher};
    use std::io::Write;
    let start = std::time::Instant::now();
    loop {
        let body = crate::obs_page::overlay_state(cfg, shared).to_string();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        body.hash(&mut h);
        let version = format!("{:016x}", h.finish());
        if version != seen {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nX-State: {version}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.set_nodelay(true);
            let _ = s.write_all(head.as_bytes()).and_then(|_| s.write_all(body.as_bytes()));
            return;
        }
        if start.elapsed() >= Duration::from_secs(1) {
            let _ = s.write_all(b"HTTP/1.1 204 No Content\r\nCache-Control: no-store\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A Host header that is an IP address or "localhost" (with or without a port).
fn host_is_address(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map(|(ip, _)| ip).unwrap_or("")
    } else {
        host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host)
    };
    name.eq_ignore_ascii_case("localhost") || name.parse::<std::net::IpAddr>().is_ok()
}

#[cfg(test)]
mod host_tests {
    // Every answer written by hand ends its header lines in CRLF: a line break inside the literal
    // would be a bare LF on the wire (browsers take it, strict HTTP clients don't).
    #[test]
    fn hand_written_answers_use_crlf() {
        for (file, src) in [("lan.rs", include_str!("lan.rs")), ("obs_page.rs", include_str!("obs_page.rs"))] {
            for line in src.lines() {
                // (the patterns split, so this line isn't one)
                let starts_answer = [concat!("\"HTTP", "/1.1 "), concat!("\"--", "frame")].iter().any(|a| line.contains(a));
                if starts_answer {
                    assert!(line.contains("\\r\\n"), "{file}: a header line without \\r\\n: {}", line.trim());
                }
            }
        }
    }

    #[test]
    fn the_phone_page_answers_addresses_only() {
        for ok in ["192.168.1.20:8791", "192.168.1.20", "127.0.0.1:8791", "localhost:8791", "[::1]:8791", "[fe80::1]:8791"] {
            assert!(super::host_is_address(ok), "{ok}");
        }
        for bad in ["evil.example:8791", "evil.example", "", "192.168.1.20.evil.example:8791", "[evil]:8791"] {
            assert!(!super::host_is_address(bad), "{bad}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_lines_with_icons_as_a_game_pad() {
        let shared = Shared::default();
        for i in 0..(KEEP + 5) {
            push(&shared, "subtitles", &format!("Line {i}"), &[(format!("سطر {i}"), None)], 1000 + i as u64);
        }
        push(&shared, "help", "Press X", &[("اضغط ".into(), None), (String::new(), Some("icon:data:x".into()))], 9);
        push(&shared, "help", "nothing", &[(" ".into(), None)], 10); // blank: not kept
        let log = shared.live_log.lock().unwrap();
        assert_eq!(log.len(), KEEP);
        assert_eq!(log.back().unwrap()["text"], "اضغط 🎮");
        let ids: Vec<u64> = log.iter().map(|l| l["id"].as_u64().unwrap()).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "ids increase");
        assert_eq!(log.front().unwrap()["en"], "Line 6");
    }

    #[test]
    fn the_tv_state_has_the_overlays_only() {
        let c = Mutex::new(Config { deepl_key: "secret-deepl-key".into(), claude_key: "secret-claude-key".into(), ..Config::default() });
        let shared = Shared::default();
        *shared.status.lock().unwrap() = "status-for-troubleshooting".into();
        let v = crate::obs_page::overlay_state(&c, &shared);
        let text = v.to_string();
        assert!(!text.contains("secret-") && !text.contains("status-for-troubleshooting"), "{text}");
        for key in ["areas", "style", "shown", "screen", "overlays", "lang"] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert!(v.get("status").is_none() && v.get("frame").is_none() && v.get("preview").is_none());
    }

    #[test]
    fn qr_code_for_the_address() {
        let c = Config { lan_page: true, ..Config::default() };
        let shared = Shared::default();
        shared.lan_running.store(true, std::sync::atomic::Ordering::Relaxed);
        let i = info(&c, &shared);
        if i.url.is_some() {
            assert!(i.qr.as_deref().is_some_and(|s| s.starts_with("<?xml") || s.contains("<svg")));
        }
        let off = info(&Config::default(), &shared);
        assert!(off.url.is_none() && off.qr.is_none());
    }
}
