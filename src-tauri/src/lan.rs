//! Live translations on other devices: a small read-only web page on the home network
//! (http://<this PC>:<port>/), for a phone, tablet or another computer, and the overlays for a TV
//! (http://<this PC>:<port>/tv, drawn by the VI Translator app on the TV over the game picture).
//!
//! Off by default. While "lan_page" is on, it listens on all addresses but serves only the pages,
//! their fonts and drawing code, the list of recent translations and what the overlays show now:
//! never settings beyond the overlays' own places and style, the picture or the OBS page (those
//! stay on the 127.0.0.1-only server in obs_page.rs).

use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::AppHandle;

use crate::capture::Shared;
use crate::config::Config;
use crate::obs_page::{read_request, respond, serve_asset, Slot};
use vi_translator_core::tv_server;

/// This PC's address on the home network.
pub use vi_translator_core::net::lan_ip;
use vi_translator_core::net::host_is_address;

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
    // the TV app's page (/tv, /tv.json, /tv/forget) and a Samsung TV offering its code: the core's
    // (the same as a phone serves), with the page's files from inside the executable
    let Some(s) = tv_server::route(&req, s, cfg, shared, &|s, file| serve_asset(app, s, file)) else { return };
    let (path, query) = (req.path.as_str(), req.query.as_str());
    match path {
        "/" | "/live" => serve_asset(app, s, "live.html"),
        "/live.json" => {
            let after: u64 = query.split('&').find_map(|kv| kv.strip_prefix("after=")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let lines: Vec<serde_json::Value> =
                shared.live_log.lock().unwrap().iter().filter(|l| l["id"].as_u64().unwrap_or(0) > after).cloned().collect();
            let live = !shared.live_off.load(std::sync::atomic::Ordering::Relaxed);
            // ("run": new each time the app starts; line ids start again then, and the page with it)
            let (snap, control, ui) = {
                let c = cfg.lock().unwrap();
                (c.snap_lan, c.lan_control.then(|| controls(&c)), c.ui_lang.clone())
            };
            // ("ui": the app's own interface language, which the page takes for its buttons and notices)
            let body = serde_json::json!({ "run": crate::obs_page::build_id(), "lang": shared.live_lang.lock().unwrap().clone(), "lines": lines, "live": live, "snap": snap, "control": control, "ui": ui, "perf": crate::perf_status() });
            respond(s, "200 OK", "application/json; charset=utf-8", body.to_string().as_bytes());
        }
        "/common.js" => serve_asset(app, s, "common.js"),
        "/snapshot.js" => serve_asset(app, s, "snapshot.js"),
        // the interface's words, in the app's languages (for the page's own text)
        "/i18n.js" => serve_asset(app, s, "i18n.js"),
        p if p.starts_with("/i18n/") && p.ends_with(".js") && p[6..p.len() - 3].chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && p.len() <= 20 => {
            serve_asset(app, s, p.trim_start_matches('/'))
        }
        // a switch changed on this page (a POST from its own address, when the app allows it):
        // {"what": "overlays" | "live" | "area" | "page" | "lang", "id": "...", "on": true}
        "/control" if req.method == "POST" && req.origin == format!("http://{}", req.host) => {
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&req.body) else {
                return respond(s, "400 Bad Request", "text/plain", b"bad request");
            };
            let (what, id, on) = (v["what"].as_str().unwrap_or(""), v["id"].as_str().unwrap_or(""), v["on"].as_bool());
            let Some(on) = on else { return respond(s, "400 Bad Request", "text/plain", b"bad request") };
            let mut c = cfg.lock().unwrap();
            if !c.lan_control {
                return respond(s, "403 Forbidden", "text/plain; charset=utf-8", b"Changes from this page are switched off in VI Translator.");
            }
            let ok = match what {
                "overlays" => {
                    c.overlays_enabled = on;
                    true
                }
                "lang" if crate::langs::LANGS.iter().any(|l| l.code == id) => {
                    c.target_lang = id.to_string();
                    true
                }
                "live" => {
                    c.live_translations = on;
                    shared.live_off.store(!on, std::sync::atomic::Ordering::Relaxed);
                    true
                }
                // (only the profile's own areas, and the switches its game's script declares)
                "area" if c.areas.iter().any(|a| a.id == id) => c.set_shown("area", id, on),
                "page" if page_options(&c.game()).iter().any(|o| o.id == id) => c.set_shown("page", id, on),
                _ => false,
            };
            if !ok {
                return respond(s, "404 Not Found", "text/plain", b"not found");
            }
            let _ = c.save(&crate::config_path());
            // (the window keeps its own copy of the settings: told, so its next save doesn't undo this)
            use tauri::Emitter;
            let _ = app.emit_to("main", "lan-control", serde_json::json!({
                "overlays_enabled": c.overlays_enabled, "live_translations": c.live_translations, "shown": c.shown, "target_lang": c.target_lang,
                "profile": c.profile, "areas": c.areas.iter().map(|a| (a.id.clone(), a.enabled)).collect::<std::collections::BTreeMap<_, _>>(),
                "more": c.profile_opts.more,
            }));
            let _ = app.emit("config", &c.redacted());
            respond(s, "200 OK", "application/json; charset=utf-8", controls(&c).to_string().as_bytes())
        }
        // a snapshot (the whole picture translated), asked for by this page (a POST from its own
        // address: another website can't make one), when the app allows it
        "/snapshot" if req.method == "POST" && req.origin == format!("http://{}", req.host) => {
            if !cfg.lock().unwrap().snap_lan {
                return respond(s, "403 Forbidden", "text/plain; charset=utf-8", b"Snapshots from this page are switched off in VI Translator.");
            }
            use tauri::{Emitter, Manager};
            let state = app.state::<crate::AppState>();
            match crate::snapshot_now(&state.cfg, &state.shared, &state.memory) {
                Ok(snap) => {
                    // (the app keeps it and shows it in its Snapshot tab)
                    let _ = app.emit_to("main", "snapshot", &snap);
                    respond(s, "200 OK", "application/json; charset=utf-8", serde_json::to_string(&snap).unwrap_or_default().as_bytes())
                }
                Err(e) => respond(s, "500 Internal Server Error", "text/plain; charset=utf-8", format!("{e:#}").as_bytes()),
            }
        }
        // the pages' fonts and nothing else
        p if p.starts_with("/fonts/") && !p.contains("..") || p == "/fonts.css" => serve_asset(app, s, p.trim_start_matches('/')),
        _ => respond(s, "404 Not Found", "text/plain", b"not found"),
    }
}

/// What the home-network page can switch: the overlays, the live translations, the profile's read areas and
/// its game's pages (with each one's state).
fn controls(c: &Config) -> serde_json::Value {
    let opts = page_options(&c.game());
    serde_json::json!({
        // the language the game is translated into, and the ones it can be
        "lang": c.target_lang,
        "langs": crate::langs::LANGS.iter().map(|l| serde_json::json!({ "code": l.code, "name": l.name, "native": l.native })).collect::<Vec<_>>(),
        "overlays": c.overlays_enabled,
        "live": c.live_translations,
        "areas": c.areas.iter().map(|a| serde_json::json!({ "id": a.id, "name": a.name, "on": a.enabled })).collect::<Vec<_>>(),
        // (a page's name as the script gives it: text, or {"en": …, "ar": …} for the page to pick from)
        "pages": opts.iter().map(|o| serde_json::json!({ "id": o.id, "name": o.name, "on": c.profile_opts.more.get(&o.id).copied().unwrap_or(o.on) })).collect::<Vec<_>>(),
    })
}

/// The switches a game's pages script declares (loaded once per game and script version).
fn page_options(game: &str) -> Vec<crate::pagescript::OptionInfo> {
    static CACHE: Mutex<Option<(String, u64, Vec<crate::pagescript::OptionInfo>)>> = Mutex::new(None);
    let generation = crate::pagescript::GENERATION.load(std::sync::atomic::Ordering::Relaxed);
    let mut cache = CACHE.lock().unwrap();
    if let Some((g, n, list)) = cache.as_ref() {
        if g == game && *n == generation {
            return list.clone();
        }
    }
    let list = crate::pagescript::Pages::for_game(game, &crate::data_dir()).map(|p| p.options.clone()).unwrap_or_default();
    *cache = Some((game.to_string(), generation, list.clone()));
    list
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
