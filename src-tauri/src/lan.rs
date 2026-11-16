//! Live translations on other devices: a small read-only web page on the home network
//! (http://<this PC>:<port>/), for a phone, tablet or another computer.
//!
//! Off by default. While "lan_page" is on, it listens on all addresses but serves only the page,
//! its fonts and the list of recent translations: never settings, the picture or the OBS page
//! (those stay on the 127.0.0.1-only server in obs_page.rs).

use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::AppHandle;

use crate::capture::Shared;
use crate::config::Config;
use crate::obs_page::{respond, serve_asset};

/// How many recent lines the page can fetch.
const KEEP: usize = 200;

/// Remember a line that was just shown, for the page.
pub fn push(shared: &Shared, area: &str, en: &str, segments: &[(String, Option<String>)], at_ms: u64) {
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
}

pub fn info(c: &Config, shared: &Shared) -> LanInfo {
    let running = shared.lan_running.load(std::sync::atomic::Ordering::Relaxed);
    let url = (c.lan_page && running).then(|| lan_ip().map(|ip| format!("http://{ip}:{}/", c.lan_port))).flatten();
    let qr = url.as_ref().and_then(|u| {
        let code = qrcode::QrCode::new(u.as_bytes()).ok()?;
        Some(code.render::<qrcode::render::svg::Color>().min_dimensions(180, 180).quiet_zone(true).build())
    });
    LanInfo { enabled: c.lan_page, running, url, qr }
}

/// Runs for the app's lifetime: starts and stops the listener as the setting changes.
pub fn run(app: AppHandle, cfg: Arc<Mutex<Config>>, shared: Arc<Shared>) {
    use std::sync::atomic::Ordering;
    let mut listener: Option<(TcpListener, u16)> = None;
    loop {
        let (on, port) = {
            let c = cfg.lock().unwrap();
            (c.lan_page, c.lan_port)
        };
        match (&listener, on) {
            (Some((_, p)), true) if *p == port => {}
            (_, true) => {
                listener = TcpListener::bind(("0.0.0.0", port)).ok().map(|l| {
                    let _ = l.set_nonblocking(true);
                    (l, port)
                });
            }
            (Some(_), false) => listener = None,
            (None, false) => {}
        }
        shared.lan_running.store(listener.is_some(), Ordering::Relaxed);
        match &listener {
            Some((l, _)) => {
                // accept everything waiting, then look at the setting again
                while let Ok((s, _)) = l.accept() {
                    let _ = s.set_nonblocking(false);
                    let (a, sh) = (app.clone(), shared.clone());
                    std::thread::spawn(move || handle(&a, &sh, s));
                }
                std::thread::sleep(Duration::from_millis(60));
            }
            None => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

fn handle(app: &AppHandle, shared: &Shared, s: TcpStream) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let mut line = String::new();
    if BufReader::new(&s).read_line(&mut line).is_err() {
        return;
    }
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/" | "/live" => serve_asset(app, s, "live.html"),
        "/live.json" => {
            let after: u64 = query.split('&').find_map(|kv| kv.strip_prefix("after=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let lines: Vec<serde_json::Value> =
                shared.live_log.lock().unwrap().iter().filter(|l| l["id"].as_u64().unwrap_or(0) > after).cloned().collect();
            let body = serde_json::json!({ "lang": shared.live_lang.lock().unwrap().clone(), "lines": lines });
            respond(s, "200 OK", "application/json; charset=utf-8", body.to_string().as_bytes());
        }
        // the page's fonts and nothing else
        p if p.starts_with("/fonts/") && !p.contains("..") || p == "/fonts.css" => serve_asset(app, s, p.trim_start_matches('/')),
        _ => respond(s, "404 Not Found", "text/plain", b"not found"),
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
