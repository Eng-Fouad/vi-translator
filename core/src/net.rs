//! The small HTTP server pieces the TV page (tv_server.rs) and the desktop's own pages share:
//! reading a request, answering it, capping connections, and this device's home-network address.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub fn respond(mut s: TcpStream, status: &str, mime: &str, body: &[u8]) {
    // (no CORS header: the pages are same-origin, and other websites must not read these answers)
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// A request's target and the headers that matter here.
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub host: String,
    pub origin: String,
    /// sent with the X-VIT header (only the VI Translator apps send it: a web page can't, without asking)
    pub vit: bool,
    /// the body, up to 4 KB (empty when longer)
    pub body: Vec<u8>,
}

/// Read a request's first line, headers and a small body: at most 8 KB, 3 s per read (a client
/// that sends more, or nothing, is dropped).
pub fn read_request(s: &TcpStream) -> Option<Request> {
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let mut r = BufReader::new(s).take(8192);
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    // "GET /path?query HTTP/1.1"
    let method = line.split_whitespace().next().unwrap_or("").to_string();
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let (path, query) = target.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((target, String::new()));
    let (mut host, mut origin, mut vit, mut length) = (String::new(), String::new(), false, 0usize);
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).ok()? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = h.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => host = value.trim().to_string(),
                "origin" => origin = value.trim().to_string(),
                "x-vit" => vit = true,
                "content-length" => length = value.trim().parse().unwrap_or(usize::MAX),
                _ => {}
            }
        }
    }
    let mut body = Vec::new();
    if (1..=4096).contains(&length) {
        body.resize(length, 0);
        if r.read_exact(&mut body).is_err() {
            body.clear();
        }
    }
    Some(Request { method, path, query, host, origin, vit, body })
}

/// Connections being served at once, at most (each has its own thread).
pub const MAX_CONNECTIONS: usize = 64;

/// Counts a connection while it's served (see `MAX_CONNECTIONS`).
pub struct Slot(&'static AtomicUsize);

impl Slot {
    pub fn take(count: &'static AtomicUsize) -> Option<Slot> {
        if count.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
            count.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Slot(count))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Changes every time the app starts: the pages reload themselves when they see a new one, so an
/// OBS Browser Source or a TV always runs the current drawing code (it otherwise keeps the code it
/// first loaded).
pub fn build_id() -> u64 {
    static ID: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *ID.get_or_init(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0))
}

/// This device's address on the home network (the one used to reach the internet; no packet is sent).
pub fn lan_ip() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

/// An HTTP client builder for the core's downloads and translators. HTTPS is checked by the
/// system's own certificate check, except on Android, where Mozilla's root certificates are used
/// (the system's check there needs a JNI setup from the app).
pub fn http_client() -> reqwest::blocking::ClientBuilder {
    let b = reqwest::blocking::Client::builder();
    #[cfg(target_os = "android")]
    let b = b.tls_certs_only(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().filter_map(|c| reqwest::Certificate::from_der(c.as_ref()).ok()));
    b
}

/// A Host header that is an IP address or "localhost" (with or without a port).
pub fn host_is_address(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map(|(ip, _)| ip).unwrap_or("")
    } else {
        host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host)
    };
    name.eq_ignore_ascii_case("localhost") || name.parse::<std::net::IpAddr>().is_ok()
}

/// A query parameter's value ("" when it isn't there).
pub fn param<'a>(query: &'a str, name: &str) -> &'a str {
    query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v)).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn request(raw: &[u8]) -> Option<Request> {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut c = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        c.write_all(raw).unwrap();
        let (s, _) = l.accept().unwrap();
        read_request(&s)
    }

    #[test]
    fn reads_the_target_and_host() {
        let r = request(b"GET /tv.json?t=k&h=7 HTTP/1.1\r\nHost: 192.168.1.5:8791\r\nX-Other: y\r\n\r\n").unwrap();
        assert_eq!((r.path.as_str(), r.query.as_str(), r.host.as_str()), ("/tv.json", "t=k&h=7", "192.168.1.5:8791"));
        assert_eq!((param(&r.query, "t"), param(&r.query, "h"), param(&r.query, "x")), ("k", "7", ""));
    }

    #[test]
    fn reads_a_small_body_and_the_vit_header() {
        let r = request(b"POST /vit-tv/offer HTTP/1.1\r\nHost: 192.168.1.5:8791\r\nX-VIT: 1\r\nContent-Length: 7\r\n\r\n{\"a\":1}").unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str(), r.vit, r.body.as_slice()), ("POST", "/vit-tv/offer", true, &b"{\"a\":1}"[..]));
        let r = request(b"POST /x HTTP/1.1\r\nContent-Length: 99999\r\n\r\nabc").unwrap();
        assert!(!r.vit && r.body.is_empty(), "too long: not read");
    }

    #[test]
    fn a_huge_request_is_cut_short() {
        // a header with no end: read up to 8 KB, then given up (no unbounded memory)
        let mut raw = b"GET / HTTP/1.1\r\nHost: 127.0.0.1:1\r\nX: ".to_vec();
        raw.extend(std::iter::repeat_n(b'a', 64 * 1024));
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            let _ = c.write_all(&raw);
        });
        let (s, _) = l.accept().unwrap();
        let r = read_request(&s);
        assert!(r.is_some_and(|r| r.path == "/"), "the first line is read, the endless header isn't");
        let _ = t.join();
    }

    #[test]
    fn connections_are_capped() {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let slots: Vec<Slot> = (0..MAX_CONNECTIONS).map(|_| Slot::take(&COUNT).unwrap()).collect();
        assert!(Slot::take(&COUNT).is_none());
        drop(slots);
        assert!(Slot::take(&COUNT).is_some());
    }

    #[test]
    fn the_pages_answer_addresses_only() {
        for ok in ["192.168.1.20:8791", "192.168.1.20", "127.0.0.1:8791", "localhost:8791", "[::1]:8791", "[fe80::1]:8791"] {
            assert!(host_is_address(ok), "{ok}");
        }
        for bad in ["evil.example:8791", "evil.example", "", "192.168.1.20.evil.example:8791", "[evil]:8791"] {
            assert!(!host_is_address(bad), "{bad}");
        }
    }

    // Every answer written by hand ends its header lines in CRLF: a line break inside the literal
    // would be a bare LF on the wire (browsers take it, strict HTTP clients don't).
    #[test]
    fn hand_written_answers_use_crlf() {
        for (file, src) in [("net.rs", include_str!("net.rs")), ("tv_server.rs", include_str!("tv_server.rs"))] {
            // (the answers, not the tests that look at them)
            for line in src.lines().take_while(|l| !l.starts_with("#[cfg(test)]")) {
                // (the pattern is split, so this line isn't one)
                if line.contains(concat!("\"HTTP", "/1.1 ")) {
                    assert!(line.contains("\\r\\n"), "{file}: a header line without \\r\\n: {}", line.trim());
                }
            }
        }
    }
}
