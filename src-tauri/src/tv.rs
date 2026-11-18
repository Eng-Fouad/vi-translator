//! TVs that show the overlays over the game: the VI Translator app on the TV on a Google TV or Android TV.
//!
//! Connecting starts here, in the main window: the app finds the TVs on the home network (the TV
//! app answers on `TV_PORT` while it waits to be connected), and the user types the code the TV
//! shows. The TV checks the code; if it matches, it keeps this PC's address and a key made for it,
//! and from then on asks the home-network server (lan.rs) for the TV page with that key. A TV
//! without a key, or with a key removed here, gets nothing.
//!
//! The keys are kept in their own file (tvs.json), not in the settings: the settings window saves
//! the whole settings back and never needs to see them.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Where the TV app answers while it waits to be connected.
pub const TV_PORT: u16 = 8792;
/// Sent with every request to a TV: a web page can't send it to the TV from a browser without the
/// TV agreeing first (it never does), so only this app can connect a TV.
const HEADER: &str = "X-VIT";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PairedTv {
    /// the TV app's own id (the same TV connected again replaces its old key)
    pub id: String,
    pub name: String,
    pub token: String,
}

/// A connected TV, for the window (never its key).
#[derive(Serialize, Clone, Debug)]
pub struct TvInfo {
    pub id: String,
    pub name: String,
}

/// A TV on the home network that runs the TV app.
#[derive(Serialize, Clone, Debug)]
pub struct FoundTv {
    pub address: String,
    pub id: String,
    pub name: String,
    /// already connected to this PC
    pub connected: bool,
}

#[derive(Default)]
pub struct Tvs {
    path: Mutex<Option<PathBuf>>,
    list: Mutex<Vec<PairedTv>>,
}

impl Tvs {
    /// Read the connected TVs from `path`, and keep them there from now on.
    pub fn load(&self, path: PathBuf) {
        let list: Vec<PairedTv> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        *self.list.lock().unwrap() = list.into_iter().filter(|t| !t.token.is_empty()).collect();
        *self.path.lock().unwrap() = Some(path);
    }

    fn save(&self, list: &[PairedTv]) {
        if let Some(p) = self.path.lock().unwrap().as_ref() {
            if let Ok(b) = serde_json::to_vec_pretty(list) {
                let tmp = p.with_extension("json.tmp");
                if std::fs::write(&tmp, b).is_ok() {
                    let _ = std::fs::rename(&tmp, p);
                }
            }
        }
    }

    /// Is this a key of a connected TV?
    pub fn allowed(&self, token: &str) -> bool {
        !token.is_empty() && self.list.lock().unwrap().iter().any(|t| same(t.token.as_bytes(), token.as_bytes()))
    }

    pub fn list(&self) -> Vec<TvInfo> {
        self.list.lock().unwrap().iter().map(|t| TvInfo { id: t.id.clone(), name: t.name.clone() }).collect()
    }

    fn add(&self, tv: PairedTv) {
        let mut l = self.list.lock().unwrap();
        l.retain(|t| t.id != tv.id);
        l.push(tv);
        self.save(&l);
    }

    /// Removed here: the TV's key stops working (the TV notices and shows a new code).
    pub fn remove_id(&self, id: &str) -> bool {
        let mut l = self.list.lock().unwrap();
        let before = l.len();
        l.retain(|t| t.id != id);
        let changed = l.len() != before;
        if changed {
            self.save(&l);
        }
        changed
    }

    /// The TV disconnected itself.
    pub fn remove_token(&self, token: &str) -> bool {
        let mut l = self.list.lock().unwrap();
        let before = l.len();
        l.retain(|t| !same(t.token.as_bytes(), token.as_bytes()));
        let changed = l.len() != before;
        if changed {
            self.save(&l);
        }
        changed
    }
}

/// Compared in the same time whatever the bytes (a key can't be guessed a byte at a time).
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(Deserialize)]
struct Hello {
    app: String,
    id: String,
    name: String,
}

fn client(total: Duration) -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_millis(400))
        .timeout(total)
        .no_proxy()
        .build()
        .ok()
}

/// The TV app at `ip`, if it answers there.
fn hello(c: &reqwest::blocking::Client, ip: Ipv4Addr) -> Option<(String, Hello)> {
    let address = format!("{ip}:{TV_PORT}");
    let r = c.get(format!("http://{address}/vit-tv")).header(HEADER, "1").send().ok()?;
    if !r.status().is_success() {
        return None;
    }
    let h: Hello = r.json().ok()?;
    (h.app == "vi-translator-tv" && !h.id.is_empty()).then_some((address, h))
}

/// The TVs waiting to be connected on this PC's network (its /24): every address at once, a
/// couple of seconds in all.
pub fn find(tvs: &Tvs) -> Vec<FoundTv> {
    let Some(mine) = crate::lan::lan_ip().and_then(|ip| ip.parse::<Ipv4Addr>().ok()) else { return Vec::new() };
    let Some(c) = client(Duration::from_millis(1500)) else { return Vec::new() };
    let [a, b, n, me] = mine.octets();
    let found = Mutex::new(Vec::new());
    let others: Vec<u8> = (1..=254u8).filter(|i| *i != me).collect();
    std::thread::scope(|s| {
        for chunk in others.chunks(8) {
            let (c, found) = (&c, &found);
            s.spawn(move || {
                for i in chunk {
                    if let Some((address, h)) = hello(c, Ipv4Addr::new(a, b, n, *i)) {
                        found.lock().unwrap().push((address, h));
                    }
                }
            });
        }
    });
    let connected: Vec<String> = tvs.list().into_iter().map(|t| t.id).collect();
    let mut out: Vec<FoundTv> = found
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|(address, h)| FoundTv { connected: connected.contains(&h.id), address, id: h.id, name: h.name })
        .collect();
    out.sort_by(|x, y| x.name.cmp(&y.name).then(x.address.cmp(&y.address)));
    out
}

#[derive(Deserialize)]
struct Paired {
    id: String,
    name: String,
}

/// Why connecting didn't work (the window says it in the user's language).
#[derive(Debug, PartialEq)]
pub enum PairError {
    /// not the code the TV shows
    WrongCode,
    /// too many wrong codes: the TV shows a new one
    TooMany,
    /// no answer, or not the TV app
    NoAnswer,
    /// the home-network sharing isn't running (the TV would have nothing to load)
    NotSharing,
}

impl PairError {
    pub fn code(&self) -> &'static str {
        match self {
            PairError::WrongCode => "wrong_code",
            PairError::TooMany => "too_many",
            PairError::NoAnswer => "no_answer",
            PairError::NotSharing => "not_sharing",
        }
    }
}

/// Connect the TV at `address` (as `find` gave it) with the code it shows: it gets this PC's
/// address (`pc`, as "ip:port") and a new key, and is kept as connected. Its name, when it worked.
pub fn pair(tvs: &Tvs, address: &str, code: &str, pc: &str, pc_name: &str) -> Result<String, PairError> {
    // (an address on the home network as find() gives it: the TV app's port, an IPv4 address)
    let ok_address = address
        .strip_suffix(&format!(":{TV_PORT}"))
        .and_then(|ip| ip.parse::<Ipv4Addr>().ok())
        .is_some_and(|ip| ip.is_private() || ip.is_link_local());
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if !ok_address || code.len() != 6 {
        return Err(if ok_address { PairError::WrongCode } else { PairError::NoAnswer });
    }
    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let c = client(Duration::from_secs(5)).ok_or(PairError::NoAnswer)?;
    let body = serde_json::json!({ "code": code, "pc": pc, "pc_name": pc_name, "token": token });
    let r = c.post(format!("http://{address}/vit-tv/pair")).header(HEADER, "1").json(&body).send().map_err(|_| PairError::NoAnswer)?;
    match r.status().as_u16() {
        200 => {
            let p: Paired = r.json().map_err(|_| PairError::NoAnswer)?;
            let name = if p.name.trim().is_empty() { "TV".to_string() } else { p.name.chars().take(60).collect() };
            tvs.add(PairedTv { id: p.id.chars().take(64).collect(), name: name.clone(), token });
            Ok(name)
        }
        403 => Err(PairError::WrongCode),
        429 => Err(PairError::TooMany),
        _ => Err(PairError::NoAnswer),
    }
}

/// This PC's name, as the TV shows it ("Connected to …").
pub fn pc_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "PC".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_kept_and_removed() {
        let dir = std::env::temp_dir().join(format!("vit-tvs-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tvs.json");
        let tvs = Tvs::default();
        tvs.load(path.clone());
        assert!(!tvs.allowed("") && !tvs.allowed("x"));
        tvs.add(PairedTv { id: "tv1".into(), name: "Living room".into(), token: "k1".into() });
        tvs.add(PairedTv { id: "tv1".into(), name: "Living room".into(), token: "k2".into() }); // connected again
        assert!(!tvs.allowed("k1") && tvs.allowed("k2"));
        let again = Tvs::default();
        again.load(path.clone());
        assert!(again.allowed("k2"), "kept in the file");
        assert_eq!(again.list().len(), 1);
        assert!(again.remove_token("k2") && !again.allowed("k2"));
        tvs.add(PairedTv { id: "tv2".into(), name: "Bedroom".into(), token: "k3".into() });
        assert!(tvs.remove_id("tv2") && !tvs.allowed("k3"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_home_addresses_and_six_digit_codes() {
        let tvs = Tvs::default();
        assert_eq!(pair(&tvs, "8.8.8.8:8792", "123456", "x", "pc"), Err(PairError::NoAnswer));
        assert_eq!(pair(&tvs, "192.168.1.5:80", "123456", "x", "pc"), Err(PairError::NoAnswer));
        assert_eq!(pair(&tvs, "192.168.1.5:8792", "12345", "x", "pc"), Err(PairError::WrongCode));
    }
}
